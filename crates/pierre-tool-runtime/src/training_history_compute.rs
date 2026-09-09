// ABOUTME: Computes daily CTL/ATL/TSB rollups from the durable activity cache, never from a live provider
// ABOUTME: Reports the window it can honestly warm and asks the capture rail for any missing depth

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Daily training-state rollup, sourced from stored activities.
//!
//! ## Why this never calls a provider
//!
//! It used to. `compute_and_persist_history` fetched up to 500 activities live
//! on every call, which on a scrape backend is minutes of browser work inside a
//! chat turn bounded at 90s — and the turn that motivated this change threw a
//! completed 171s scrape away because the bound fired first. The data was
//! already on disk: the same turn's `get_activities` had served the athlete's
//! window from `cached_activities` 51 seconds earlier.
//!
//! So the provider is not this module's business. `get_activities` and the
//! capture rail own provider I/O and the coverage gate; this reads what they
//! persisted and does arithmetic. That makes it a millisecond operation, and it
//! removes the second, ungated door to the provider.
//!
//! ## Why it reports a window rather than a row count
//!
//! `ctl`/`atl`/`tsb` are plain `f64` seeded at zero, with no `None` arm to say
//! "not enough history". Computing a day without
//! [`warmup_days`] of activities behind it therefore emits a chronic load that
//! is confidently wrong low rather than absent — worse than refusing, and
//! invisible to any test that only checks the row count. So the depth actually
//! present in the cache decides the window, the caller is told which days were
//! stood behind, and any missing depth is asked of the capture rail.

use std::sync::Arc;

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use tracing::info;
use uuid::Uuid;

use pierre_config::environment::default_provider;
use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, DailyTrainingState, TenantId};
use pierre_fitness_compute::training_history_compute::{
    compute_training_history, warmup_days, AthleteInputs, MAX_BACKFILL_DAYS,
};
use pierre_providers::backend_resolver;
#[cfg(feature = "tools-data")]
use pierre_providers::core::ActivityQueryParams;
use pierre_runtime_context::DataContext;

#[cfg(feature = "tools-data")]
use crate::activity_backfill::{spawn_activity_backfill, ActivityBackfillJob};
use crate::activity_fetch::HISTORICAL_WINDOW_READ_LIMIT;
use crate::runtime::ToolRuntime;

/// Default backfill window when the caller does not specify one.
pub const DEFAULT_BACKFILL_DAYS: i64 = 90;

/// Slack (days) added to each end of the cache read.
///
/// An activity whose local date sits inside the window must not be missed
/// because its UTC instant falls outside it, and zone offsets reach ±14h. The
/// pure compute re-filters on the athlete's civil date, so reading wide is free
/// and reading tight loses rows.
const CACHE_READ_EDGE_SLACK_DAYS: i64 = 2;

/// How much of the requested window the stored activities could stand behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryCoverage {
    /// Stored history warms the whole requested window.
    Complete,
    /// Stored history begins too late to warm the whole ask; only
    /// `[trustworthy_from, to]` was computed.
    Partial {
        /// First day whose CTL/ATL/TSB the stored history can stand behind.
        trustworthy_from: NaiveDate,
    },
    /// No stored activities in the read window at all. Nothing was computed —
    /// a zero-seeded series would read as a real chronic load.
    NoStoredActivities,
}

/// What a compute-and-persist run actually did.
#[derive(Debug, Clone)]
pub struct TrainingHistoryComputed {
    /// First day the caller asked for.
    pub requested_from: NaiveDate,
    /// First day actually computed. Equals `requested_from` when coverage is
    /// [`HistoryCoverage::Complete`].
    pub from: NaiveDate,
    /// Last day of the window, inclusive.
    pub to: NaiveDate,
    /// Daily rows written.
    pub rows_upserted: usize,
    /// How much of the ask the stored history supported.
    pub coverage: HistoryCoverage,
    /// Whether a background capture was asked for the missing depth. `false`
    /// when coverage was complete, or when a capture for this
    /// `(user, provider)` was already in flight.
    pub capture_requested: bool,
}

impl TrainingHistoryComputed {
    /// Whether every day the caller asked for was stood behind.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.coverage, HistoryCoverage::Complete)
    }
}

/// The default window, ending on the athlete's civil day.
///
/// "Today" is theirs, not the server's: the rollup buckets each activity on the
/// athlete's civil date, so a window ending on the UTC date put their current
/// local day past `to` for every zone ahead of UTC and the session they had
/// just finished was dropped from the series that answers "how am I doing"
/// (registre#260).
///
/// # Errors
///
/// Returns [`AppError`] when the user's timezone cannot be read.
pub async fn default_window(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
) -> AppResult<(NaiveDate, NaiveDate)> {
    let zone = resolve_zone(user_timezone(resources, user_id).await?.as_deref());
    let to = clock_date(Utc::now(), zone);
    Ok((to - Duration::days(DEFAULT_BACKFILL_DAYS), to))
}

/// Read the athlete's configured timezone, if any.
async fn user_timezone(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
) -> AppResult<Option<String>> {
    Ok(resources
        .repos()
        .users
        .get_global(user_id)
        .await?
        .and_then(|u| u.timezone))
}

/// Compute and persist daily training-history rows for `[from, to]` from the
/// durable activity cache.
///
/// Computes only the days the stored history can honestly warm, persists those,
/// and asks the capture rail for any depth it was missing. Never calls a
/// provider — see the module docs.
///
/// # Errors
///
/// Returns [`AppError`] when the window is invalid, no provider is connected,
/// or a repository read/write fails.
pub async fn compute_and_persist_history(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<TrainingHistoryComputed> {
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    if (to - from).num_days() > MAX_BACKFILL_DAYS {
        return Err(AppError::invalid_input(format!(
            "backfill window exceeds the maximum of {MAX_BACKFILL_DAYS} days"
        )));
    }

    let backend = resolve_compute_backend(resources, tenant_id, user_id).await?;

    // The rollup buckets on the athlete's civil day. Persisting UTC-day buckets
    // shifted the whole CTL/ATL/TSB series against their own calendar for
    // anyone training in the evening (registre#200).
    let timezone = user_timezone(resources, user_id).await?;
    let zone = resolve_zone(timezone.as_deref());

    let warmup = warmup_days(
        resources
            .cageux_config()
            .algorithms
            .params
            .training_load_ctl_days,
    );
    let activities = read_cached_window(
        resources,
        tenant_id,
        user_id,
        &backend.slug,
        from - Duration::days(warmup),
        to,
    )
    .await?;

    let oldest_stored = activities
        .iter()
        .map(|a: &Activity| local_date(a.start_date(), zone))
        .min();
    let Some(oldest_stored) = oldest_stored else {
        return empty_cache_outcome(resources, tenant_id, user_id, &backend, from, to, warmup);
    };

    // The first day the stored history can stand behind: anything earlier would
    // be computed against a partly-empty warm-up and read as a real, low CTL.
    let trustworthy_from = from.max(oldest_stored + Duration::days(warmup));
    let complete = trustworthy_from <= from;
    let capture_requested = !complete
        && request_capture(
            resources,
            tenant_id,
            user_id,
            &backend.slug,
            from - Duration::days(warmup),
        );

    if trustworthy_from > to {
        // Stored history is too shallow to warm even the last day of the ask.
        info!(
            user_id = %user_id,
            provider = %backend.slug,
            requested_from = %from,
            to = %to,
            oldest_stored = %oldest_stored,
            capture_requested,
            "training history: stored history too shallow to warm any requested day"
        );
        return Ok(TrainingHistoryComputed {
            requested_from: from,
            from: trustworthy_from,
            to,
            rows_upserted: 0,
            coverage: HistoryCoverage::Partial { trustworthy_from },
            capture_requested,
        });
    }

    let inputs = athlete_inputs(resources, tenant_id, user_id).await?;
    let states = compute_training_history(
        &activities,
        inputs,
        trustworthy_from,
        to,
        &resources.cageux_config().algorithms,
        timezone.as_deref(),
    );
    let rows_upserted = states.len();
    if rows_upserted > 0 {
        resources
            .repos()
            .training_history
            .upsert_training_history_batch(tenant_id, user_id, &states)
            .await?;
    }

    info!(
        user_id = %user_id,
        provider = %backend.slug,
        requested_from = %from,
        computed_from = %trustworthy_from,
        to = %to,
        oldest_stored = %oldest_stored,
        activities = activities.len(),
        rows_upserted,
        complete,
        capture_requested,
        "training history: computed from durable cache"
    );

    Ok(TrainingHistoryComputed {
        requested_from: from,
        from: trustworthy_from,
        to,
        rows_upserted,
        coverage: if complete {
            HistoryCoverage::Complete
        } else {
            HistoryCoverage::Partial { trustworthy_from }
        },
        capture_requested,
    })
}

/// The outcome when the durable cache holds nothing for the read window.
///
/// Computes no rows on purpose: `ctl`/`atl`/`tsb` seeded at zero would read as
/// a real, low chronic load rather than as absent data.
fn empty_cache_outcome(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    backend: &ComputeBackend,
    from: NaiveDate,
    to: NaiveDate,
    warmup: i64,
) -> AppResult<TrainingHistoryComputed> {
    // Nothing stored AND the session has lapsed: the capture below would fail
    // on auth too, so the honest answer is the reconnect prompt the chat
    // pipeline's `auth_recovery` stage mints from this typed code. With rows in
    // hand we serve them instead and let the capture surface the reauth itself,
    // because stale history beats no answer.
    if backend.requires_reauth {
        return Err(AppError::provider_auth_required(&backend.slug));
    }
    let capture_requested = request_capture(
        resources,
        tenant_id,
        user_id,
        &backend.slug,
        from - Duration::days(warmup),
    );
    info!(
        user_id = %user_id,
        provider = %backend.slug,
        requested_from = %from,
        to = %to,
        capture_requested,
        "training history: no stored activities in the window; computed nothing"
    );
    Ok(TrainingHistoryComputed {
        requested_from: from,
        from,
        to,
        rows_upserted: 0,
        coverage: HistoryCoverage::NoStoredActivities,
        capture_requested,
    })
}

/// The backend this compute reads, and whether its connection is still live.
struct ComputeBackend {
    /// Canonical slug the cached rows are keyed by.
    slug: String,
    /// Whether the athlete must reconnect before any new history can arrive.
    requires_reauth: bool,
}

/// Resolve the backend slug whose cached rows this compute reads.
///
/// Canonicalised through [`backend_resolver::resolve_backend`] because that is
/// what the write side keys on: a Garmin athlete's rows are written under
/// `sciotte_garmin` while the connection says `garmin`, and reading the raw
/// connection slug would miss every row it wrote.
async fn resolve_compute_backend(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<ComputeBackend> {
    let (requested, requires_reauth) = if let Some(p) = default_provider() {
        (p, false)
    } else if let Some(conn) = resources
        .repos()
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await?
    {
        let requires_reauth = conn.status.requires_reauth();
        (conn.provider, requires_reauth)
    } else {
        return Err(AppError::no_provider_connected());
    };
    Ok(ComputeBackend {
        slug: backend_resolver::resolve_backend(
            &resources.repos().auth_repos(),
            user_id,
            Some(tenant_id),
            &requested,
        )
        .await,
        requires_reauth,
    })
}

/// Read the athlete's stored activities covering `[start, to]`, newest first.
async fn read_cached_window(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    backend: &str,
    start: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<Activity>> {
    let slack = Duration::days(CACHE_READ_EDGE_SLACK_DAYS);
    let start_ts = Utc.from_utc_datetime(&(start - slack).and_hms_opt(0, 0, 0).unwrap_or_default());
    let end_ts = Utc.from_utc_datetime(&(to + slack).and_hms_opt(0, 0, 0).unwrap_or_default());
    let limit = i64::try_from(HISTORICAL_WINDOW_READ_LIMIT).unwrap_or(i64::MAX);
    resources
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant_id, Some(backend), start_ts, end_ts, limit)
        .await
}

/// Per-user physiology, defaulted where the profile is silent.
async fn athlete_inputs(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<AthleteInputs> {
    Ok(resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await?
        .as_ref()
        .map_or_else(AthleteInputs::default, |p| AthleteInputs {
            ftp_watts: p.ftp_watts.map(f64::from),
            lthr: p
                .lactate_threshold_percentage
                .and_then(|pct| p.max_hr.map(|mhr| f64::from(mhr) * pct)),
            max_hr: p.max_hr.map(f64::from),
            resting_hr: p.resting_hr.map(f64::from),
            weight_kg: p.weight,
        }))
}

/// Ask the capture rail to page the provider back to `floor`.
///
/// Rides the rail `get_activities` already uses, so a capture for this
/// `(user, provider)` is deduplicated against one already in flight and its
/// completion notice reaches the conversation that asked. Returns whether a new
/// job was started.
///
/// The rail lives behind `tools-data`, alongside the `get_activities` tool that
/// is the other half of the capture story. Reading the cache, computing the
/// series and reporting its coverage need none of that, so the module is not
/// gated with it — only this call is. Without the rail compiled in there is no
/// capture to start and [`TrainingHistoryComputed::capture_requested`] is
/// `false`, which is what happened: nothing was asked for.
#[cfg(feature = "tools-data")]
fn request_capture(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    backend: &str,
    floor: NaiveDate,
) -> bool {
    let after = floor
        .and_hms_opt(0, 0, 0)
        .map(|dt| Utc.from_utc_datetime(&dt).timestamp());
    spawn_activity_backfill(ActivityBackfillJob {
        resources: resources.clone(),
        user_id,
        tenant_id,
        tenant_id_str: Some(tenant_id.to_string()),
        provider_name: backend.to_owned(),
        query_params: ActivityQueryParams {
            after,
            ..ActivityQueryParams::default()
        },
        // This compute is reached from the tool, the HTTP route and the capture
        // rail's own warm hook; none of them owns a conversation to push to.
        // The rail's completion notice is driven by the fetch that has one.
        pierre_conversation_id: None,
    })
}

/// No capture rail in this build, so no capture is started.
///
/// See the `tools-data` sibling above for why the module is not gated wholesale.
#[cfg(not(feature = "tools-data"))]
const fn request_capture(
    _resources: &Arc<dyn ToolRuntime>,
    _tenant_id: TenantId,
    _user_id: Uuid,
    _backend: &str,
    _floor: NaiveDate,
) -> bool {
    false
}

/// Read-only fetch of persisted rows in `[from, to]`.
///
/// The read side of the rollup: no cache read, no compute, no provider. What
/// `get_training_history` answers with, and what the HTTP route serves once the
/// window has been computed.
///
/// # Errors
///
/// Returns [`AppError`] when the window is inverted or the repository read fails.
pub async fn fetch_history_rows(
    data: &DataContext,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<DailyTrainingState>> {
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    data.repos()
        .training_history
        .get_training_history(tenant_id, user_id, from, to)
        .await
}
