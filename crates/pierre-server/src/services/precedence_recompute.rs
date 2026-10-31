// ABOUTME: Recomputes every athlete's training-history rollup once after the rule their workouts merge by changed
// ABOUTME: A worker-ledger pass compares the stored precedence generation with this build's and recomputes on a mismatch

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The derived-data recompute a precedence change owes (Athlete Data
//! Consolidation S3).
//!
//! Which copy of a workout stands for its session follows the one precedence
//! rule (`pierre_providers::deduplication::merge_duplicates`), and the
//! session's figures follow that copy: a Garmin watch's ride is Garmin's
//! distance and heart rate, not Strava's re-derivation. The daily
//! training-history rollup is scored when computed and persisted, so a day
//! computed before the rule changed keeps the old copy's load until it is
//! recomputed; a capture only re-warms its default window.
//!
//! So the rule carries a generation, [`PRECEDENCE_GENERATION`], bumped by
//! every change to how a workout's copies rank — the change itself and its
//! rollback alike. A
//! pass on [`spawn_periodic`] (once across instances, through the worker
//! ledger) reads the generation the stored rollups were computed under from
//! `system_settings`; on a mismatch it recomputes every stored day of every
//! athlete holding one (`recompute_stored_history`) and then records this
//! build's generation, so later passes read one setting and stop. An athlete
//! whose recompute fails leaves the generation unrecorded, and the next pass
//! tries every athlete again: a recompute is idempotent. An athlete with
//! nothing to recompute from (every connection gone and nothing uploaded or
//! imported, or every connection to reconnect with no workout cached) is
//! skipped and never holds the generation back.
//!
//! Rolling the rule back is a forward change that bumps the generation again,
//! never a `git revert` of the change: a revert deletes this worker with it,
//! so the days computed under the reverted rule would stand.
//!
//! The gear ledger needs no pass: each read moves an attributed workout's
//! distance to its merged session's (`gear_ledger`).

use std::sync::Arc;
use std::time::Duration;

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::backends::factory::Database;
use pierre_services::periodic::spawn_periodic;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::try_recompute_stored_history;
use tracing::{info, warn};

use crate::mcp::resources::ServerContext;
use crate::tools::runtime_adapter::into_runtime;

/// The name every log line and the worker ledger row carry.
pub const WORKER_NAME: &str = "precedence recompute";

/// Pass cadence: a pass with nothing owed reads one setting.
pub const TICK_INTERVAL: Duration = Duration::from_mins(30);

/// The `system_settings` key holding the generation the stored rollups were
/// computed under.
pub const GENERATION_SETTING_KEY: &str = "derived.precedence_generation";

/// The generation of the precedence rule this build merges workouts by.
///
/// 1: Athlete Data Consolidation S3 — the recording service's own copy of a
/// workout stands over Strava's unless the athlete edited it on Strava
/// (carnet#1274); before it, the copy carrying a route stood.
pub const PRECEDENCE_GENERATION: u32 = 1;

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrecedenceRecomputeReport {
    /// Athletes whose stored history was recomputed.
    pub recomputed: usize,
    /// Athletes holding nothing a read can reach any more.
    pub nothing_stored: usize,
    /// Athletes holding stored days but nothing to recompute them from:
    /// every connection gone and nothing of their own brought in, or every
    /// connection to reconnect with no workout cached. Their days stand until
    /// a connection's capture recomputes them; they never hold the
    /// generation back.
    pub no_source: usize,
    /// Athletes whose recompute failed; the next pass tries again.
    pub failed: usize,
    /// Whether this build's generation was recorded.
    pub recorded: bool,
}

/// Start the recompute on the worker ledger.
///
/// Skipped for an in-memory database, like the other workers, so a
/// throwaway server never writes behind a test's back.
pub fn start_precedence_recompute(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() {
        return;
    }
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    let resources = Arc::clone(resources);
    spawn_periodic(WORKER_NAME, TICK_INTERVAL, ledger, move || {
        let database = Arc::clone(&resources.agent.database);
        let runtime = into_runtime(&resources);
        async move {
            let report = run_precedence_recompute_pass(&database, &runtime).await?;
            if report != PrecedenceRecomputeReport::default() {
                info!(
                    worker = WORKER_NAME,
                    recomputed = report.recomputed,
                    nothing_stored = report.nothing_stored,
                    no_source = report.no_source,
                    failed = report.failed,
                    recorded = report.recorded,
                    generation = PRECEDENCE_GENERATION,
                    "precedence recompute pass finished"
                );
            }
            Ok(())
        }
    });
}

/// Whether a recompute refused for the athlete's own state rather than
/// failed: no source is left to compute from, or every connection needs
/// reconnecting with no workout cached. Nothing the rule could re-rank is
/// there, the next pass would refuse the same way, and the athlete's
/// reconnect brings a capture that recomputes their window.
fn owes_no_recompute(error: &AppError) -> bool {
    matches!(
        error.code,
        ErrorCode::NoProviderConnected | ErrorCode::ProviderAuthRequired
    )
}

/// Recompute every athlete's stored training history when the stored
/// generation is not [`PRECEDENCE_GENERATION`], then record it.
///
/// # Errors
/// Returns a database error when the setting or the athletes holding a
/// rollup cannot be read, or the generation cannot be recorded; one
/// athlete's failed recompute is counted and the pass goes on.
pub async fn run_precedence_recompute_pass(
    database: &Database,
    runtime: &Arc<dyn ToolRuntime>,
) -> AppResult<PrecedenceRecomputeReport> {
    let stored = database
        .get_system_setting(GENERATION_SETTING_KEY)
        .await?
        .and_then(|setting| setting.value.parse::<u32>().ok());
    if stored == Some(PRECEDENCE_GENERATION) {
        return Ok(PrecedenceRecomputeReport::default());
    }
    let mut report = PrecedenceRecomputeReport::default();
    let holders = runtime
        .repos()
        .training_history
        .training_history_holders()
        .await?;
    for (tenant_id, user_id) in holders {
        match try_recompute_stored_history(runtime, tenant_id, user_id).await {
            Ok(Some(_)) => report.recomputed += 1,
            Ok(None) => report.nothing_stored += 1,
            Err(e) if owes_no_recompute(&e) => report.no_source += 1,
            Err(e) => {
                report.failed += 1;
                warn!(
                    worker = WORKER_NAME,
                    user_id = %user_id,
                    tenant_id = %tenant_id,
                    reason = %e.sanitized_message(),
                    "training history not recomputed under the current precedence"
                );
            }
        }
    }
    if report.failed == 0 {
        database
            .set_system_setting(GENERATION_SETTING_KEY, &PRECEDENCE_GENERATION.to_string())
            .await?;
        report.recorded = true;
    }
    Ok(report)
}
