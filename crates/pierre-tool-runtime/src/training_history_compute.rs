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

#[cfg(not(feature = "tools-data"))]
use std::future::{ready, Ready};
use std::sync::Arc;

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use pierre_config::environment::default_provider;
use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, DailyTrainingState, TenantId};
use pierre_fitness_compute::training_history_compute::{
    compute_training_history, warmup_days, AthleteInputs, MAX_BACKFILL_DAYS,
};
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver;
#[cfg(feature = "tools-data")]
use pierre_providers::core::ActivityQueryParams;
use pierre_runtime_context::DataContext;

#[cfg(feature = "tools-data")]
use crate::activity_backfill::{
    provider_tenant_id_str, spawn_activity_backfill, ActivityBackfillJob,
};
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
    /// Rows removed because this run could not stand behind them.
    ///
    /// Non-zero only when an earlier, less careful path had written the span —
    /// the rollup is upsert-only otherwise, so declining to write would have
    /// left those rows readable as current.
    pub rows_cleared: u64,
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
    // The rollup is the athlete's, read by their own surfaces: it is computed
    // from every stored row even when a tool triggers it (carnet#723). What a
    // model reads back goes through `history_rows_for_model`.
    ai_scope::unfiltered(compute_and_persist_unfiltered(
        resources, tenant_id, user_id, from, to,
    ))
    .await
}

async fn compute_and_persist_unfiltered(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<TrainingHistoryComputed> {
    validate_window(from, to)?;

    let backend = resolve_compute_backend(resources, tenant_id, user_id)
        .await?
        .ok_or_else(AppError::no_provider_connected)?;
    let window = load_stored_window(resources, tenant_id, user_id, backend, from, to).await?;
    let backend = &window.backend;
    let warmup = window.warmup;

    let Some(oldest_stored) = window.oldest_stored else {
        return empty_cache_outcome(resources, tenant_id, user_id, backend, from, to, warmup).await;
    };

    let trustworthy_from = first_vouched_day(from, oldest_stored, warmup);
    let complete = trustworthy_from <= from;
    let capture_requested = !complete
        && request_capture(
            resources,
            tenant_id,
            user_id,
            &backend.slug,
            from - Duration::days(warmup),
        )
        .await;

    // Declining to write is not enough: the rollup is upsert-only, so any row an
    // earlier path left in the un-warmable span stays readable and reads as
    // current. Clear exactly what this run just proved it cannot vouch for.
    // The clear is bounded by the ask, not by the warm-up shortfall.
    // `trustworthy_from` can sit far past `to` on a shallow cache, and a delete
    // reaching that far would take days no caller named — days an earlier run
    // with a wider read window legitimately vouched for.
    let clear_end = trustworthy_from.min(to + Duration::days(1));
    let rows_cleared = if complete {
        0
    } else {
        clear_unvouched_span(resources, tenant_id, user_id, from, clear_end).await?
    };

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
            rows_cleared,
        });
    }

    let states =
        compute_states(resources, tenant_id, user_id, &window, trustworthy_from, to).await?;
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
        activities = window.activities.len(),
        rows_upserted,
        rows_cleared,
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
        rows_cleared,
    })
}

/// Daily rows computed from the durable cache for a read that stores nothing.
#[derive(Debug, Clone)]
pub struct TrainingHistoryRead {
    /// One row per day of `[coverage's first vouched day, to]`, oldest first.
    /// Empty when the stored history warms no day of the ask.
    pub states: Vec<DailyTrainingState>,
    /// How much of the ask the stored history supported.
    pub coverage: HistoryCoverage,
}

/// Compute daily training-history rows for `[from, to]` from the durable
/// activity cache, and nothing else.
///
/// The read a page makes: the same stored activities, thresholds, civil days
/// and warm-up rule as [`compute_and_persist_history`], so the page and the
/// agent read one series — but it writes no row, clears none, and asks the
/// capture rail for nothing. A page is opened many times a day by an athlete
/// who asked for no history; a capture per visit would page a scrape backend
/// again every time a thin history was looked at.
///
/// An athlete with no connected provider has no stored activities, and is
/// answered [`HistoryCoverage::NoStoredActivities`] rather than refused.
///
/// # Errors
///
/// Returns [`AppError`] when the window is invalid or a repository read fails.
pub async fn read_history_from_cache(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<TrainingHistoryRead> {
    validate_window(from, to)?;
    let nothing_stored = TrainingHistoryRead {
        states: Vec::new(),
        coverage: HistoryCoverage::NoStoredActivities,
    };
    let Some(backend) = resolve_compute_backend(resources, tenant_id, user_id).await? else {
        return Ok(nothing_stored);
    };
    let window = load_stored_window(resources, tenant_id, user_id, backend, from, to).await?;
    let Some(oldest_stored) = window.oldest_stored else {
        return Ok(nothing_stored);
    };
    let trustworthy_from = first_vouched_day(from, oldest_stored, window.warmup);
    if trustworthy_from > to {
        return Ok(TrainingHistoryRead {
            states: Vec::new(),
            coverage: HistoryCoverage::Partial { trustworthy_from },
        });
    }
    let states =
        compute_states(resources, tenant_id, user_id, &window, trustworthy_from, to).await?;
    Ok(TrainingHistoryRead {
        states,
        coverage: if trustworthy_from <= from {
            HistoryCoverage::Complete
        } else {
            HistoryCoverage::Partial { trustworthy_from }
        },
    })
}

/// Refuse a window that is inverted or longer than the compute is bounded to.
fn validate_window(from: NaiveDate, to: NaiveDate) -> AppResult<()> {
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    if (to - from).num_days() > MAX_BACKFILL_DAYS {
        return Err(AppError::invalid_input(format!(
            "backfill window exceeds the maximum of {MAX_BACKFILL_DAYS} days"
        )));
    }
    Ok(())
}

/// The athlete's stored activities behind a window, with what computing from
/// them needs.
struct StoredWindow {
    /// The backend whose cached rows were read.
    backend: ComputeBackend,
    /// The athlete's configured timezone, if any.
    timezone: Option<String>,
    /// Days of history a day needs behind it, at the configured chronic window.
    warmup: i64,
    /// Stored activities covering the window and its warm-up.
    activities: Vec<Activity>,
    /// The athlete's civil date of the oldest of them; `None` when the read
    /// held nothing.
    oldest_stored: Option<NaiveDate>,
}

/// Read the stored activities behind `[from, to]` and its warm-up.
async fn load_stored_window(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    backend: ComputeBackend,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<StoredWindow> {
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
    Ok(StoredWindow {
        backend,
        timezone,
        warmup,
        activities,
        oldest_stored,
    })
}

/// The first day the stored history can stand behind: anything earlier would
/// be computed against a partly-empty warm-up and read as a real, low CTL.
fn first_vouched_day(from: NaiveDate, oldest_stored: NaiveDate, warmup: i64) -> NaiveDate {
    from.max(oldest_stored + Duration::days(warmup))
}

/// The daily rows of `[from, to]` from a stored window, each session scored
/// against the athlete's saved thresholds.
async fn compute_states(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    window: &StoredWindow,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<DailyTrainingState>> {
    let inputs = athlete_inputs(resources, tenant_id, user_id).await?;
    compute_training_history(
        &window.activities,
        inputs,
        from,
        to,
        &resources.cageux_config().algorithms,
        window.timezone.as_deref(),
    )
    .map_err(|e| AppError::internal(format!("training-load series: {e}")))
}

/// The outcome when the durable cache holds nothing for the read window.
///
/// Computes no rows on purpose: `ctl`/`atl`/`tsb` seeded at zero would read as
/// a real, low chronic load rather than as absent data.
async fn empty_cache_outcome(
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
    )
    .await;
    // Nothing stored means nothing vouched for anywhere in the ask, so the whole
    // window goes — including rows an earlier path wrote from a provider fetch
    // this one no longer makes.
    let rows_cleared =
        clear_unvouched_span(resources, tenant_id, user_id, from, to + Duration::days(1)).await?;
    info!(
        user_id = %user_id,
        provider = %backend.slug,
        requested_from = %from,
        to = %to,
        capture_requested,
        rows_cleared,
        "training history: no stored activities in the window; computed nothing"
    );
    Ok(TrainingHistoryComputed {
        requested_from: from,
        from,
        to,
        rows_upserted: 0,
        coverage: HistoryCoverage::NoStoredActivities,
        capture_requested,
        rows_cleared,
    })
}

/// Delete persisted rows in `[from, exclusive_end)` — the span this run could
/// not stand behind.
///
/// `exclusive_end` is the first day that IS vouched for, so the delete stops one
/// day short of it. Returns the number of rows removed, which is zero on the
/// normal path: only an earlier write leaves anything here.
async fn clear_unvouched_span(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    exclusive_end: NaiveDate,
) -> AppResult<u64> {
    let last = exclusive_end - Duration::days(1);
    if last < from {
        return Ok(0);
    }
    resources
        .repos()
        .training_history
        .delete_training_history_range(tenant_id, user_id, from, last)
        .await
}

/// The backend this compute reads, and whether its connection is still live.
struct ComputeBackend {
    /// Canonical slug the cached rows are keyed by.
    slug: String,
    /// Whether the athlete must reconnect before any new history can arrive.
    requires_reauth: bool,
}

/// Resolve the backend slug whose cached rows this compute reads, or `None`
/// when the athlete has no provider connected.
///
/// Canonicalised through [`backend_resolver::resolve_backend`] because that is
/// what the write side keys on: a Garmin athlete's rows are written under
/// `sciotte_garmin` while the connection says `garmin`, and reading the raw
/// connection slug would miss every row it wrote.
async fn resolve_compute_backend(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Option<ComputeBackend>> {
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
        return Ok(None);
    };
    Ok(Some(ComputeBackend {
        slug: backend_resolver::resolve_backend(
            &resources.repos().auth_repos(),
            user_id,
            Some(tenant_id),
            &requested,
        )
        .await,
        requires_reauth,
    }))
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
    let rows = resources
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant_id, Some(backend), start_ts, end_ts, limit)
        .await?;
    // A history computed for a model sums only what it may see; the persisted
    // rollup is computed unfiltered (`compute_and_persist_history`).
    Ok(ai_scope::filter_activities(
        resources.provider_registry().as_ref(),
        rows,
    ))
}

/// Per-user physiology, absent where the profile is silent.
async fn athlete_inputs(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<AthleteInputs> {
    let profile = resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await?;
    Ok(AthleteInputs::from_profile(profile.as_ref()))
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
async fn request_capture(
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
        tenant_id_str: provider_tenant_id_str(tenant_id),
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
    .await
}

/// No capture rail in this build, so no capture is started. Answers as a
/// ready future so the callers await it exactly as they await the rail.
///
/// See the `tools-data` sibling above for why the module is not gated wholesale.
#[cfg(not(feature = "tools-data"))]
fn request_capture(
    _resources: &Arc<dyn ToolRuntime>,
    _tenant_id: TenantId,
    _user_id: Uuid,
    _backend: &str,
    _floor: NaiveDate,
) -> Ready<bool> {
    ready(false)
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

/// The history rows a model, or a caller over an external transport, may read
/// for `[from, to]`.
///
/// The persisted rollup sums every stored activity. Under a gate whose window
/// holds activities a provider's terms withhold — from AI (carnet#723) or from
/// this transport (carnet#724) — the rows are computed on the fly from the
/// permitted activities instead, under the same gates: the same warm-up rule
/// as the rollup, so with nothing withheld the two agree and the stored rows
/// are served as before.
///
/// # Errors
///
/// Returns an error when the window is inverted or a read fails.
pub async fn history_rows_for_model(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<DailyTrainingState>> {
    if ai_scope::policies_apply() {
        let (computed, withheld) = ai_scope::tallied(read_history_from_cache(
            resources, tenant_id, user_id, from, to,
        ))
        .await;
        if !withheld.is_empty() {
            ai_scope::record_withheld(withheld);
            return Ok(computed?.states);
        }
    }
    fetch_history_rows(&resources.data(), tenant_id, user_id, from, to).await
}

/// What happened to the athlete's stored daily rollup when the thresholds it
/// is scored against changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HistoryRefresh {
    /// The change moved no number training load is scored against.
    Unaffected,
    /// No daily rows were stored, so none could be stale.
    NothingStored,
    /// The stored days were recomputed against the saved thresholds.
    Recomputed {
        /// First day recomputed, `YYYY-MM-DD`.
        from: String,
        /// Last day recomputed, `YYYY-MM-DD`.
        to: String,
        /// Daily rows written.
        rows_upserted: usize,
    },
    /// Recomputing failed. `get_training_history` reads rows scored against
    /// the previous thresholds until `compute_training_history` runs.
    Failed {
        /// Why, safe to relay to the athlete.
        reason: String,
    },
}

/// Recompute every stored daily row of the athlete, from the oldest one a
/// read can reach to their civil today, against the thresholds now saved.
///
/// The rollup is scored when it is computed, not when it is read, so a row
/// written before a threshold changed would otherwise keep the previous
/// scoring while every tool that computes load live used the new one. Days the
/// stored activities can no longer stand behind are cleared, as every compute
/// run clears them.
pub async fn recompute_stored_history(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> HistoryRefresh {
    let outcome = async {
        let (_, to) = default_window(resources, user_id).await?;
        let stored = resources
            .repos()
            .training_history
            .get_training_history(
                tenant_id,
                user_id,
                to - Duration::days(MAX_BACKFILL_DAYS),
                to,
            )
            .await?;
        let Some(oldest) = stored.first() else {
            return Ok(None);
        };
        compute_and_persist_history(resources, tenant_id, user_id, oldest.date, to)
            .await
            .map(Some)
    }
    .await;
    match outcome {
        Ok(None) => HistoryRefresh::NothingStored,
        Ok(Some(computed)) => HistoryRefresh::Recomputed {
            from: computed.from.format("%Y-%m-%d").to_string(),
            to: computed.to.format("%Y-%m-%d").to_string(),
            rows_upserted: computed.rows_upserted,
        },
        Err(e) => {
            warn!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                error = %e,
                "recomputing the stored training history after a threshold change failed"
            );
            HistoryRefresh::Failed {
                reason: e.sanitized_message(),
            }
        }
    }
}
