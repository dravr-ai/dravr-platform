// ABOUTME: Spawns the coaching background workers — outcome evaluator, archetype aggregation, commitment sweep, A2A reaper, extraction and backfill resume
// ABOUTME: Keeps the assembly of each worker out of the binary, which only decides that they should start
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coaching background workers.
//!
//! Fire-and-forget tokio tasks that learn from, and report on, what the
//! athlete actually did — and one that finishes the learning a dead instance
//! left owed. They are grouped here rather than inlined in the binary because
//! most need handles assembled from several slices of [`ServerContext`], and
//! the binary's job is to decide *that* they start, not to know how each one
//! is put together.

use std::sync::Arc;

use pierre_services::archetype_aggregation::spawn_archetype_aggregation;
use pierre_services::commitment_sweep::{
    spawn_commitment_sweep, ActivityRefresher, CommitmentReporter,
};
use pierre_services::memory_extraction_resume::start_memory_extraction_resume;
use pierre_services::outcome_evaluator::spawn_outcome_evaluator;

use crate::services::commitment_refresher::ServerActivityRefresher;
#[cfg(feature = "client-messaging")]
use crate::services::commitment_reporter::ServerCommitmentReporter;

use crate::mcp::resources::ServerContext;

/// Start the coaching background workers.
///
/// Takes the composition-root `Arc` rather than a bare reference because the
/// commitment sweep's activity refresher holds the `ToolRuntime` handle for
/// the life of its background fetches.
///
/// Skipped wholesale for an in-memory database — that is how the LLM health
/// probe and other throwaway test servers avoid a sweep (and, for the outcome
/// evaluator, an LLM judge call) firing against a scratch database.
pub fn start_coaching_workers(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() {
        return;
    }

    // Labels due advice from the athlete's real activity/health data and
    // reinforces the matching playbooks. Its hybrid labeler can invoke the LLM
    // judge for ambiguous cases, hence the in-memory skip above.
    spawn_outcome_evaluator(
        Arc::clone(&resources.common.repos),
        resources.common.chat_provider.as_ref().map(Arc::clone),
    );

    // Daily archetype aggregation: rolls per-user playbooks into k-anonymous
    // cross-user priors for cold-start. DB-only (no LLM).
    spawn_archetype_aggregation(Arc::clone(&resources.common.repos));

    // Re-runs the post-turn memory extractions an instance was taken down
    // while running; without it the facts a turn owed vanished with the
    // instance (carnet#461). Same provider and prompt as the turn path.
    start_memory_extraction_resume(
        Arc::clone(&resources.common.repos),
        resources.common.chat_provider.as_ref().map(Arc::clone),
        Arc::clone(&resources.fitness.harness_config_registry),
        resources.memory_extraction_prompt(),
    );

    // Fails the A2A tasks an instance died holding, closes their stream
    // channels and delivers the terminal push; without it a killed
    // return-immediately task reads `working` forever (carnet#462).
    #[cfg(feature = "protocol-a2a")]
    {
        use pierre_a2a::reaper::{reap_stale_tasks, REAP_INTERVAL, STALE_AFTER};
        use pierre_runtime_context::A2ACtx;
        use pierre_services::periodic::spawn_periodic;
        let ctx: Arc<dyn A2ACtx> = Arc::clone(resources) as Arc<dyn A2ACtx>;
        spawn_periodic(
            "a2a task reaper",
            REAP_INTERVAL,
            Arc::clone(&resources.common.repos.worker_runs),
            move || {
                let ctx = Arc::clone(&ctx);
                async move {
                    reap_stale_tasks(&ctx, STALE_AFTER).await?;
                    Ok(())
                }
            },
        );
    }

    // Re-runs the historical activity backfills an instance died holding:
    // the job row a spawn records outlives the instance, and without this
    // sweep the athlete who was told "no need to ask again" gets nothing
    // (carnet#460).
    #[cfg(feature = "tools-data")]
    {
        use pierre_tool_runtime::activity_backfill_resume::start_activity_backfill_resume;
        use pierre_tool_runtime::runtime::ToolRuntime;
        let runtime: Arc<dyn ToolRuntime> = Arc::clone(resources) as Arc<dyn ToolRuntime>;
        start_activity_backfill_resume(runtime, Arc::clone(&resources.common.repos.worker_runs));
    }

    // Counts due athlete commitments against their real activity data and
    // reports the verdict back through the channel the promise was made in.
    // DB-only (no LLM).
    //
    // The reporter is the messaging half, so it exists only on a build with the
    // messaging rail compiled in. Without one the sweep still counts, still
    // labels, and still ages verdicts out, so the unreported queue cannot grow
    // while nothing can deliver.
    #[cfg(feature = "client-messaging")]
    let reporter: Option<Arc<dyn CommitmentReporter>> =
        Some(ServerCommitmentReporter::from_handles(
            Arc::clone(&resources.common.repos),
            Arc::clone(&resources.mcp.messaging_strings_registry),
            #[cfg(feature = "client-notifications")]
            resources.common.notification_service.clone(),
        ));
    #[cfg(not(feature = "client-messaging"))]
    let reporter: Option<Arc<dyn CommitmentReporter>> = None;

    // The refresher is the data half: when a due commitment's activity cache
    // has not caught up, it fetches the window from every provider the athlete
    // has connected — the same authenticated path a chat turn uses — so an
    // athlete who never opens a conversation still gets a verdict.
    let runtime = Arc::clone(resources);
    let refresher: Option<Arc<dyn ActivityRefresher>> =
        Some(Arc::new(ServerActivityRefresher::new(runtime)));

    spawn_commitment_sweep(Arc::clone(&resources.common.repos), reporter, refresher);
}
