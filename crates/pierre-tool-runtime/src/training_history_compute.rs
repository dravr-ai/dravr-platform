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

use chrono::{Duration, NaiveDate, Utc};
use dravr_cageux::config::intelligence::IntelligenceConfig;
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use pierre_core::civil_time::{clock_date, resolve_zone};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{DailyTrainingState, TenantId};
use pierre_fitness_compute::training_history_compute::MAX_BACKFILL_DAYS;
use pierre_providers::ai_scope;
#[cfg(feature = "tools-data")]
use pierre_providers::core::ActivityQueryParams;
use pierre_runtime_context::DataContext;
use pierre_services::training_history_read::{
    self as history_read, compute_states, first_vouched_day, load_stored_window,
    resolve_compute_backend, user_timezone, validate_window, ComputeBackend, HistorySources,
};
pub use pierre_services::training_history_read::{HistoryCoverage, TrainingHistoryRead};

#[cfg(feature = "tools-data")]
use crate::activity_backfill::{
    provider_tenant_id_str, spawn_activity_backfill, ActivityBackfillJob,
};
use crate::runtime::ToolRuntime;

/// Default backfill window when the caller does not specify one.
pub const DEFAULT_BACKFILL_DAYS: i64 = 90;

/// The runtime's handles a history read needs, borrowed from `resources` and
/// the configuration snapshot `config` the caller holds for the read.
fn history_sources<'a>(
    resources: &'a Arc<dyn ToolRuntime>,
    config: &'a IntelligenceConfig<true>,
) -> HistorySources<'a> {
    HistorySources {
        repos: resources.repos(),
        terms: resources.provider_registry().as_ref(),
        algorithms: &config.algorithms,
    }
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
    let zone = resolve_zone(user_timezone(resources.repos(), user_id).await?.as_deref());
    let to = clock_date(Utc::now(), zone);
    Ok((to - Duration::days(DEFAULT_BACKFILL_DAYS), to))
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

    let config = resources.cageux_config();
    let sources = history_sources(resources, &config);
    let backend = resolve_compute_backend(resources.repos(), tenant_id, user_id)
        .await?
        .ok_or_else(AppError::no_provider_connected)?;
    let window = load_stored_window(sources, tenant_id, user_id, backend, from, to).await?;
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

    let states = compute_states(sources, tenant_id, user_id, &window, trustworthy_from, to).await?;
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

/// Compute daily training-history rows for `[from, to]` from the durable
/// activity cache, and nothing else: the runtime's binding of
/// [`history_read::read_history_from_cache`], which documents the read.
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
    let config = resources.cageux_config();
    history_read::read_history_from_cache(
        history_sources(resources, &config),
        tenant_id,
        user_id,
        from,
        to,
    )
    .await
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
        .map(|dt| dt.and_utc().timestamp());
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
    history_read::fetch_history_rows(data.repos(), tenant_id, user_id, from, to).await
}

/// The history rows a model, or a caller over an external transport, may read
/// for `[from, to]`: the runtime's binding of
/// [`history_read::history_rows_for_model`], which holds the rule.
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
    let config = resources.cageux_config();
    history_read::history_rows_for_model(
        history_sources(resources, &config),
        tenant_id,
        user_id,
        from,
        to,
    )
    .await
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
