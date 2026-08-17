// ABOUTME: Spawns the coaching background workers — outcome evaluator, archetype aggregation, commitment sweep
// ABOUTME: Keeps the assembly of each worker out of the binary, which only decides that they should start
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coaching background workers.
//!
//! Three fire-and-forget tokio tasks that learn from, and report on, what the
//! athlete actually did. They are grouped here rather than inlined in the
//! binary because two of them need handles assembled from several slices of
//! [`ServerContext`], and the binary's job is to decide *that* they start, not
//! to know how each one is put together.

use std::sync::Arc;

use pierre_services::archetype_aggregation::spawn_archetype_aggregation;
use pierre_services::commitment_sweep::{spawn_commitment_sweep, CommitmentReporter};
use pierre_services::outcome_evaluator::spawn_outcome_evaluator;

#[cfg(feature = "client-messaging")]
use crate::services::commitment_reporter::ServerCommitmentReporter;

use crate::mcp::resources::ServerContext;

/// Start the coaching background workers.
///
/// Skipped wholesale for an in-memory database — that is how the LLM health
/// probe and other throwaway test servers avoid a sweep (and, for the outcome
/// evaluator, an LLM judge call) firing against a scratch database.
pub fn start_coaching_workers(resources: &ServerContext) {
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

    spawn_commitment_sweep(Arc::clone(&resources.common.repos), reporter);
}
