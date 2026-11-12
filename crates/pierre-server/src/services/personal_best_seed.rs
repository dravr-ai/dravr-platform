// ABOUTME: The periodic walk of each athlete's stored history that seeds their all-time best efforts before any is announced
// ABOUTME: Waits for the history walk; runs on the worker ledger, leasing each athlete, its samples on the walk's budget share

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Boot wiring and passes for the personal-best seed.
//!
//! A personal record is only announced against the athlete's whole running
//! history, so each athlete's history is walked once, in the background,
//! before any is (see [`pierre_services::personal_bests`]). The history is
//! read from the activity store once a history walk copied it there; each
//! pass looks only at athletes whose walk ended, so one waiting on theirs
//! never takes another's turn. Each provider's runs are measured on the
//! samples of the backend [`seed_stream_sources`] names for it: the Strava
//! grant for a Strava history, whichever key the store holds it under. The
//! walk runs on [`spawn_periodic`], the loop every background worker shares:
//! the worker ledger runs a due pass once across instances, and a pass that
//! dies leaves its progress in the database — the cursor the walk saved and
//! the runs `best_effort_scans` holds — for the next one to resume from.
//!
//! This is a worker rather than a job of the historical-activity backfill
//! pipeline because that pipeline serves one athlete's ask for a window,
//! records a job per ask, writes the activity cache and tells the
//! conversation that asked when it is done. The seed has no ask and no
//! conversation, writes no activity, and owes every athlete one walk
//! whoever they are: a sweep over the athletes whose walk is incomplete.
//!
//! Every request a pass makes is admitted against the background share of
//! the signing app's budget in the
//! [`ProviderRateLimiter`](pierre_providers::request_budget::ProviderRateLimiter),
//! whose windows (Strava's 15-minute and daily ones) are counted in the database, so
//! every instance of the backend draws on the same windows — Cloud Run runs up
//! to three — and together they never spend more than the walk's share: when
//! it is spent the pass ends, and a later pass picks up in the next window
//! with the athlete it stopped on.
//!
//! Each athlete is walked under a lease in the worker ledger, so two passes —
//! on two instances, or one outliving its tick's lease — never walk the same
//! athlete at once, and a sync's scan waits for the walk to let go. A lease
//! an instance died holding runs out after
//! [`ATHLETE_LEASE`](pierre_services::personal_bests::ATHLETE_LEASE) and the
//! next pass takes the athlete over.
//!
//! The backend scales to zero, and the walk runs only on a live instance: it
//! advances while traffic keeps an instance up and waits, where it stopped,
//! while none is. Nothing is lost in between — the cursor and the scanned
//! runs are in the database — only the seed takes longer to complete for an
//! athlete of a quiet deployment.

use std::sync::Arc;
use std::time::Duration;

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_services::training_history_read::history_store_keys;
use pierre_providers::core::FitnessProvider;
use pierre_services::periodic::spawn_periodic;
use pierre_services::personal_bests::{seed_stream_sources, SeedPause, SeedProgress};
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The name every log line and the worker ledger row carry.
pub const WORKER_NAME: &str = "personal best seed";

/// Pass cadence.
///
/// Shorter than Strava's 15-minute window, so a pass lands soon after each
/// window opens; a pass that finds the background share spent does nothing.
pub const TICK_INTERVAL: Duration = Duration::from_mins(5);

/// The most athletes of one provider one pass looks at. A pass usually ends
/// on the budget long before, and the least recently advanced come first
/// next time.
const CANDIDATES_PER_PASS: i64 = 50;

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeedPassReport {
    /// Athletes whose walk reached their first activity in this pass.
    pub completed: usize,
    /// Athletes whose walk stopped part-way and resumes on a later pass.
    pub paused: usize,
    /// Athletes passed over because another batch held them: a walk on
    /// another instance, or a sync's scan.
    pub held: usize,
    /// Athletes passed over because no history walk has filled the store
    /// for them yet.
    pub awaiting_history: usize,
    /// Whether a provider's turn in the pass ended because the background
    /// share of its shared budget was spent, rather than on a refusal or with
    /// no athlete left to walk.
    pub budget_spent: bool,
}

/// Start the seed on the worker ledger.
///
/// Skipped for an in-memory database, like the other workers, so a
/// throwaway server never walks a history against a scratch database.
pub fn start_personal_best_seed(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() {
        return;
    }
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    let resources = Arc::clone(resources);
    spawn_periodic(WORKER_NAME, TICK_INTERVAL, ledger, move || {
        let resources = Arc::clone(&resources);
        async move {
            let report = run_personal_best_seed_pass(&resources).await?;
            if report != SeedPassReport::default() {
                info!(
                    worker = WORKER_NAME,
                    completed = report.completed,
                    paused = report.paused,
                    held = report.held,
                    awaiting_history = report.awaiting_history,
                    budget_spent = report.budget_spent,
                    "personal best seed pass finished"
                );
            }
            Ok(())
        }
    });
}

/// Advance the seed of every athlete whose history walk has ended.
///
/// Provider by provider ([`seed_stream_sources`]), each athlete whose seed is
/// incomplete and whose history a walk copied into the store, least recently
/// advanced first, until the background share of that provider's budget is
/// spent or it refuses.
///
/// An athlete whose provider cannot be built is passed over and the next one
/// is walked. A walk that pauses ends that provider's turn: on the budget,
/// nothing is left to spend until the next window; on a refusal, the athlete
/// it stopped on goes to the back of the queue, since its walk stamped its
/// progress. Another provider's budget is its own, so its turn still runs.
///
/// # Errors
/// Returns a database error when the athletes owed a walk cannot be listed;
/// one athlete's failure is logged and the pass goes on.
pub async fn run_personal_best_seed_pass(
    resources: &Arc<ServerContext>,
) -> AppResult<SeedPassReport> {
    let mut report = SeedPassReport::default();
    for source in seed_stream_sources(resources.fitness.provider_registry.as_ref()) {
        seed_provider_pass(resources, source, &mut report).await?;
    }
    Ok(report)
}

/// [`run_personal_best_seed_pass`]'s turn for the athletes whose samples
/// `source` serves.
async fn seed_provider_pass(
    resources: &Arc<ServerContext>,
    source: &str,
    report: &mut SeedPassReport,
) -> AppResult<()> {
    let keys = history_store_keys(source);
    let paired = keys.iter().copied().find(|key| *key != source);
    let candidates = resources
        .common
        .repos
        .personal_bests
        .list_personal_best_seed_candidates(source, paired, CANDIDATES_PER_PASS)
        .await?;
    let bests = resources.personal_bests();
    for candidate in candidates {
        let Ok(tenant_id) = TenantId::parse_str(&candidate.tenant_id) else {
            warn!(user_id = %candidate.user_id, provider = source, "token carries an unparseable tenant id; its history is not walked");
            continue;
        };
        let Some(provider) =
            exact_provider(resources, source, candidate.user_id, &candidate.tenant_id).await
        else {
            continue;
        };
        match bests
            .advance_seed(provider.as_ref(), candidate.user_id, tenant_id)
            .await
        {
            Ok(SeedProgress::Complete) => report.completed += 1,
            Ok(SeedProgress::Held) => report.held += 1,
            Ok(SeedProgress::AwaitingHistory) => report.awaiting_history += 1,
            Ok(SeedProgress::Paused(pause)) => {
                report.paused += 1;
                report.budget_spent |= pause == SeedPause::Budget;
                // A refusal is as likely the provider's as the athlete's (a
                // spent app quota, an outage), and would then refuse the
                // next athlete too. The walk stamped this athlete's
                // progress, so the next pass starts with the others.
                break;
            }
            Err(e) => {
                warn!(user_id = %candidate.user_id, provider = source, error = %e, "personal best seed could not advance");
            }
        }
    }
    Ok(())
}

/// The athlete's authenticated `provider` handle exactly as named, or `None`
/// (logged) when their connection cannot produce one.
///
/// The backend itself, never the one the athlete's live reads are planned
/// to: a Strava grant is never swapped for the session the reads serve
/// first, since the samples are the API's and the seed must never start a
/// scrape (carnet#1494).
pub async fn exact_provider(
    resources: &Arc<ServerContext>,
    provider: &str,
    user_id: Uuid,
    tenant_id: &str,
) -> Option<Box<dyn FitnessProvider>> {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
    match AuthService::new(runtime)
        .create_exact_provider(provider, user_id, Some(tenant_id))
        .await
    {
        Ok(provider) => Some(provider),
        Err(response) => {
            warn!(user_id = %user_id, provider, error = ?response.error, "provider unavailable; runs not measured");
            None
        }
    }
}
