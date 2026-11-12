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
use pierre_core::models::coverage::{Coverage, Pending};
use pierre_core::models::{DailyTrainingState, TenantId};
use pierre_database::repositories::{BackfillJobKind, BackfillJobState, JobProgress};
use pierre_fitness_compute::training_history_compute::MAX_BACKFILL_DAYS;
use pierre_providers::ai_scope;
use pierre_runtime_context::DataContext;
pub use pierre_services::training_history_read::TrainingHistoryRead;
use pierre_services::training_history_read::{
    self as history_read, compute_states, first_vouched_day, load_stored_window,
    resolve_compute_backends, series_coverage, user_timezone, validate_window, walk_source,
    ComputeBackend, HistoryHandles,
};

#[cfg(feature = "tools-data")]
use crate::activity_backfill::history_walk::start_history_walk;
use crate::runtime::ToolRuntime;

/// Default backfill window when the caller does not specify one.
pub const DEFAULT_BACKFILL_DAYS: i64 = 90;

/// Whether a compute may ask the capture rail for the depth it found missing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DepthAsk {
    /// Ask: an athlete surface, a tool or the HTTP route wants the window.
    Capture,
    /// Never ask: the compute runs because a capture just landed. Its ask
    /// would start a capture on each of the athlete's other connections, and
    /// each of those would warm and ask again. On 2026-11-10 that loop ran
    /// about once a second per provider for one athlete with Strava, Wahoo and
    /// Intervals.icu connected, and spent the Strava app's daily budget for
    /// every athlete (carnet#1478). Only a capture constructs it, so it is
    /// gated with the capture rail.
    #[cfg(feature = "tools-data")]
    Withheld,
}

/// The runtime's handles a history read needs, borrowed from `resources` and
/// the configuration snapshot `config` the caller holds for the read.
fn history_handles<'a>(
    resources: &'a Arc<dyn ToolRuntime>,
    config: &'a IntelligenceConfig<true>,
) -> HistoryHandles<'a> {
    HistoryHandles {
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
    /// First day actually computed. Equals `requested_from` when the
    /// coverage is complete.
    pub from: NaiveDate,
    /// Last day of the window, inclusive.
    pub to: NaiveDate,
    /// Daily rows written.
    pub rows_upserted: usize,
    /// How much of the ask the stored history supported, and the history
    /// walk still owed for the missing depth ([`Coverage::pending`]).
    pub coverage: Coverage,
    /// Whether a history walk was started for the missing depth. `false`
    /// when coverage was complete, or when every connection's walk is
    /// already owed, running or ended ([`start_history_walk`]).
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
        self.coverage.is_complete()
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
/// Returns [`AppError`] when the window is invalid, the athlete has neither a
/// provider connected nor an upload, or a repository read/write fails.
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
        resources,
        tenant_id,
        user_id,
        (from, to),
        DepthAsk::Capture,
    ))
    .await
}

/// [`compute_and_persist_history`] for the rollup warm a finished capture
/// runs: the same compute, minus the ask for missing depth ([`DepthAsk::Withheld`]).
///
/// # Errors
///
/// As [`compute_and_persist_history`].
#[cfg(feature = "tools-data")]
pub(crate) async fn compute_history_after_capture(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<TrainingHistoryComputed> {
    ai_scope::unfiltered(compute_and_persist_unfiltered(
        resources,
        tenant_id,
        user_id,
        (from, to),
        DepthAsk::Withheld,
    ))
    .await
}

async fn compute_and_persist_unfiltered(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    (from, to): (NaiveDate, NaiveDate),
    ask: DepthAsk,
) -> AppResult<TrainingHistoryComputed> {
    validate_window(from, to)?;

    let config = resources.cageux_config();
    let sources = history_handles(resources, &config);
    let backends = resolve_compute_backends(resources.repos(), tenant_id, user_id).await?;
    if backends.is_empty() {
        return Err(AppError::no_provider_connected());
    }
    let window = load_stored_window(sources, tenant_id, user_id, backends, from, to).await?;
    let backends = &window.backends;
    let providers = provider_slugs(backends);
    let warmup = window.warmup;

    let Some(oldest_stored) = window.oldest_stored else {
        return empty_cache_outcome(resources, tenant_id, user_id, backends, (from, to), ask).await;
    };

    let trustworthy_from = first_vouched_day(from, oldest_stored, warmup);
    let stored = Some((oldest_stored, trustworthy_from));
    let complete = trustworthy_from <= from;
    // With rows in hand every connection is asked for its missing depth, a
    // lapsed one included: its capture surfaces the reauth itself, and stale
    // history beats no answer.
    let walks = if !complete && ask == DepthAsk::Capture {
        request_walks(resources, tenant_id, user_id, backends.iter()).await
    } else {
        WalkAsk::default()
    };
    let capture_requested = walks.started;

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
            providers = %providers,
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
            coverage: walks.pending_on(series_coverage(from, to, stored, 0)),
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
        providers = %providers,
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
        coverage: walks.pending_on(series_coverage(from, to, stored, rows_upserted)),
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
        history_handles(resources, &config),
        tenant_id,
        user_id,
        from,
        to,
    )
    .await
}

/// The athlete's connection slugs, for a log line.
fn provider_slugs(backends: &[ComputeBackend]) -> String {
    backends
        .iter()
        .map(|backend| backend.slug.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// The outcome when the durable cache holds nothing for the read window.
///
/// Computes no rows on purpose: `ctl`/`atl`/`tsb` seeded at zero would read as
/// a real, low chronic load rather than as absent data.
async fn empty_cache_outcome(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    backends: &[ComputeBackend],
    (from, to): (NaiveDate, NaiveDate),
    ask: DepthAsk,
) -> AppResult<TrainingHistoryComputed> {
    // Nothing stored AND every session has lapsed: each capture would fail on
    // auth too, so the honest answer is the reconnect prompt the chat
    // pipeline's `auth_recovery` stage mints from this typed code. With one
    // connection still live, its capture is the way forward, and the lapsed
    // ones are left for their own reconnect prompts. Uploads deliver nothing
    // more on their own, so they neither stand in for a live connection nor
    // call for a reconnect: an athlete with only uploads gets the empty answer.
    let (live, lapsed): (Vec<&ComputeBackend>, Vec<&ComputeBackend>) = backends
        .iter()
        .filter(|b| b.capturable)
        .partition(|b| !b.requires_reauth);
    if live.is_empty() {
        if let Some(lapsed) = lapsed.first() {
            return Err(AppError::provider_auth_required(&lapsed.slug));
        }
    }
    let walks = if ask == DepthAsk::Capture {
        request_walks(resources, tenant_id, user_id, live.into_iter()).await
    } else {
        WalkAsk::default()
    };
    let capture_requested = walks.started;
    // Nothing stored means nothing vouched for anywhere in the ask, so the whole
    // window goes — including rows an earlier path wrote from a provider fetch
    // this one no longer makes.
    let rows_cleared =
        clear_unvouched_span(resources, tenant_id, user_id, from, to + Duration::days(1)).await?;
    info!(
        user_id = %user_id,
        providers = %provider_slugs(backends),
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
        coverage: walks.pending_on(series_coverage(from, to, None, 0)),
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

/// What asking the athlete's connections for their history walks came to.
#[derive(Debug, Default)]
struct WalkAsk {
    /// The walk still owed for the missing depth, as the ledger reads it.
    pending: Option<Pending>,
    /// Whether any new walk was started.
    started: bool,
}

impl WalkAsk {
    /// `coverage` with the walk still owed for it.
    fn pending_on(&self, coverage: Coverage) -> Coverage {
        Coverage {
            pending: self.pending,
            ..coverage
        }
    }
}

/// Ask each of `backends` for a walk of its history, and read the walk still
/// owed for the missing depth.
///
/// One walk per connection: the history is every provider's, so the depth
/// missing from it may sit behind any of them. Each walk is read under the
/// key it is recorded under, its source's ([`walk_source`]), never the
/// connection's own slug. A source already walking is reported, with its
/// progress, as [`Pending::HistoryWalkRunning`]; one whose walk reached the
/// end of the history is not walked again: it holds all there is
/// ([`start_history_walk`]). Any other, never walked or ended before its
/// floor, is asked for a walk, and a started one is
/// [`Pending::HistoryNotWalked`] until it records a step. A connection with
/// no source to walk (uploads, a push) owes nothing.
async fn request_walks<'a>(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    backends: impl Iterator<Item = &'a ComputeBackend>,
) -> WalkAsk {
    let repos = resources.repos();
    let mut running: Vec<JobProgress> = Vec::new();
    let mut started = false;
    // Uploads are not fetched from anywhere: no walk can deepen them.
    for backend in backends.filter(|backend| backend.capturable) {
        let Some(source) = walk_source(repos, user_id, tenant_id, &backend.slug).await else {
            continue;
        };
        let jobs = &repos.activity_backfill_jobs;
        let kind = BackfillJobKind::HistoryWalk;
        match jobs
            .backfill_job_state(user_id, tenant_id, &source.backend, kind)
            .await
        {
            Ok(Some(BackfillJobState::Owed)) => {
                match jobs
                    .job_progress(user_id, tenant_id, &source.backend, kind)
                    .await
                {
                    Ok(progress) => running.push(progress.unwrap_or_default()),
                    Err(e) => warn!(
                        user_id = %user_id,
                        backend = %source.backend,
                        error = %e,
                        "training history: the walk's progress could not be read"
                    ),
                }
            }
            Ok(Some(BackfillJobState::Walked)) => {}
            Ok(None | Some(BackfillJobState::Incomplete)) => {
                started |= request_capture(resources, tenant_id, user_id, &backend.slug).await;
            }
            Err(e) => warn!(
                user_id = %user_id,
                backend = %source.backend,
                error = %e,
                "training history: the walk ledger could not be read"
            ),
        }
    }
    let pending = running
        .into_iter()
        .max_by_key(months_left)
        .map(|progress| walk_running(&progress))
        .or_else(|| started.then_some(Pending::HistoryNotWalked));
    WalkAsk { pending, started }
}

/// Months a walk has left, the longest when its floor is not known yet: the
/// walk the window waits longest on is the one it reports.
fn months_left(progress: &JobProgress) -> i64 {
    progress
        .months_total
        .map_or(i64::MAX, |total| total.saturating_sub(progress.months_done))
}

/// A running walk's [`Pending`] from its recorded progress. Its
/// `secs_per_unit` is the seconds one walked month took, so the ETA is the
/// months left at that pace ([`Pending::running`]).
fn walk_running(progress: &JobProgress) -> Pending {
    let months = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
    Pending::running(
        months(progress.months_done),
        progress.months_total.map(months),
        progress.secs_per_unit,
    )
}

/// The history walk still owed for the athlete's stored history, asking a
/// walk of every connection that never had one
/// ([`Pending::HistoryNotWalked`]).
///
/// The read side of [`compute_and_persist_history`]'s ask: a read of the
/// persisted rollup that finds the window short of history reads and asks
/// the same walks the compute would.
///
/// # Errors
///
/// Returns [`AppError`] when the athlete's connections cannot be read.
pub async fn history_walk_pending(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Option<Pending>> {
    let backends = resolve_compute_backends(resources.repos(), tenant_id, user_id).await?;
    Ok(
        request_walks(resources, tenant_id, user_id, backends.iter())
            .await
            .pending,
    )
}

/// Ask for a walk of `backend`'s history ([`start_history_walk`]), from the
/// first live source of the provider. Returns whether a new walk was
/// started.
///
/// The walk lives behind `tools-data`, alongside the `get_activities` tool
/// that is the other half of the capture story. Reading the cache, computing
/// the series and reporting its coverage need none of that, so the module is
/// not gated with it — only this call is. Without the rail compiled in there
/// is no walk to start and [`TrainingHistoryComputed::capture_requested`] is
/// `false`, which is what happened: nothing was asked for.
#[cfg(feature = "tools-data")]
async fn request_capture(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    backend: &str,
) -> bool {
    start_history_walk(resources, user_id, tenant_id, backend).await
}

/// No walk rail in this build, so no walk is started. Answers as a ready
/// future so the callers await it exactly as they await the rail.
///
/// See the `tools-data` sibling above for why the module is not gated wholesale.
#[cfg(not(feature = "tools-data"))]
fn request_capture(
    _resources: &Arc<dyn ToolRuntime>,
    _tenant_id: TenantId,
    _user_id: Uuid,
    _backend: &str,
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
        history_handles(resources, &config),
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

/// [`recompute_stored_history`], with the error kept typed: `None` when no
/// daily row is stored, the computed window otherwise.
///
/// A caller that must tell the athlete's own state (no connection left, one
/// to reconnect) from a failure reads the error's code here.
///
/// # Errors
///
/// Returns [`AppError`] when a read, the compute or a write fails.
pub async fn try_recompute_stored_history(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Option<TrainingHistoryComputed>> {
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
    match try_recompute_stored_history(resources, tenant_id, user_id).await {
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

/// Recompute the default window of the athlete's daily rollup after their
/// stored activities changed outside a provider capture: an uploaded file was
/// kept, or an uploaded activity deleted (carnet#818).
///
/// A capture warms the rollup when it lands (`activity_backfill`); an upload
/// or a deletion is the same kind of moment, so the persisted series a model
/// and the weekly summaries read moves with it rather than waiting for the
/// next capture. When the athlete has nothing left to stand on — they deleted
/// their only upload and have no connection — the window's rows are cleared:
/// no stored activity vouches for them any more.
///
/// # Errors
///
/// Returns [`AppError`] when a read or a write fails.
pub async fn rewarm_default_window(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<TrainingHistoryComputed> {
    let (from, to) = default_window(resources, user_id).await?;
    if resolve_compute_backends(resources.repos(), tenant_id, user_id)
        .await?
        .is_empty()
    {
        let rows_cleared =
            clear_unvouched_span(resources, tenant_id, user_id, from, to + Duration::days(1))
                .await?;
        return Ok(TrainingHistoryComputed {
            requested_from: from,
            from,
            to,
            rows_upserted: 0,
            coverage: series_coverage(from, to, None, 0),
            capture_requested: false,
            rows_cleared,
        });
    }
    compute_and_persist_history(resources, tenant_id, user_id, from, to).await
}
