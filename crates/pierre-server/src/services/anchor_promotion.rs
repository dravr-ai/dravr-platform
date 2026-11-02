// ABOUTME: The weekly pass promoting every athlete's empty or unvouched resting HR, VO2max and weight anchors
// ABOUTME: A worker-ledger tick over profile holders and stored-history holders; each promotion recomputes its own history

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The weekly anchor promotion (Athlete Data Consolidation §5, DECISION 5).
//!
//! Once a week, through the worker ledger (once across instances, on schedule
//! across restarts), every athlete who holds a profile row or a stored
//! training-history day has their promotable anchors refreshed from their own
//! measurements ([`promote_anchors`]). An athlete holding stored history but
//! no profile is included: their stored load is scored against the generic
//! defaults until a promotion fills an anchor.
//!
//! One athlete's failure is counted and logged and the pass goes on; the next
//! week's pass reads the measurements afresh, and a promotion is idempotent.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_services::periodic::spawn_periodic;
use pierre_tool_runtime::anchor_promotion::{promote_anchors, AnchorPromotion};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::HistoryRefresh;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::tools::runtime_adapter::into_runtime;

/// The name every log line and the worker ledger row carry.
const WORKER_NAME: &str = "anchor promotion";

/// Pass cadence: DECISION 5 promotes weekly, so the anchors and the load
/// scored against them move at most once a week.
const TICK_INTERVAL: Duration = Duration::from_hours(7 * 24);

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnchorPromotionReport {
    /// Athletes whose anchors were promoted.
    pub promoted: usize,
    /// Of those, athletes whose stored training history was recomputed.
    pub recomputed: usize,
    /// Of those, athletes whose history recompute failed; the anchors stand
    /// and the next capture or save recomputes it.
    pub recompute_failed: usize,
    /// Athletes with nothing to promote.
    pub unchanged: usize,
    /// Athletes whose promoted profile contradicted itself; nothing saved.
    pub refused: usize,
    /// Athletes whose promotion failed on a read or write.
    pub failed: usize,
}

/// Start the promotion on the worker ledger.
///
/// Skipped for an in-memory database, like the other workers, so a throwaway
/// server never writes behind a test's back.
pub fn start_anchor_promotion(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() {
        return;
    }
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    let resources = Arc::clone(resources);
    spawn_periodic(WORKER_NAME, TICK_INTERVAL, ledger, move || {
        let runtime = into_runtime(&resources);
        async move {
            let report = run_anchor_promotion_pass(&runtime).await?;
            info!(
                worker = WORKER_NAME,
                promoted = report.promoted,
                recomputed = report.recomputed,
                recompute_failed = report.recompute_failed,
                unchanged = report.unchanged,
                refused = report.refused,
                failed = report.failed,
                "anchor promotion pass finished"
            );
            Ok(())
        }
    });
}

/// Promote every candidate athlete's anchors once.
///
/// # Errors
/// Returns a database error when the candidates cannot be listed; one
/// athlete's failed promotion is counted and the pass goes on.
pub async fn run_anchor_promotion_pass(
    runtime: &Arc<dyn ToolRuntime>,
) -> AppResult<AnchorPromotionReport> {
    let repos = runtime.repos();
    let mut candidates: HashSet<(TenantId, Uuid)> = repos
        .user_physiological_profile
        .profile_holders()
        .await?
        .into_iter()
        .collect();
    candidates.extend(repos.training_history.training_history_holders().await?);

    let mut report = AnchorPromotionReport::default();
    for (tenant_id, user_id) in candidates {
        match promote_anchors(runtime, tenant_id, user_id).await {
            Ok(AnchorPromotion::Unchanged) => report.unchanged += 1,
            Ok(AnchorPromotion::Refused) => report.refused += 1,
            Ok(AnchorPromotion::Promoted(history)) => {
                report.promoted += 1;
                match history {
                    HistoryRefresh::Recomputed { .. } => report.recomputed += 1,
                    HistoryRefresh::Failed { .. } => report.recompute_failed += 1,
                    HistoryRefresh::Unaffected | HistoryRefresh::NothingStored => {}
                }
            }
            Err(e) => {
                report.failed += 1;
                warn!(
                    worker = WORKER_NAME,
                    user_id = %user_id,
                    tenant_id = %tenant_id,
                    reason = %e.sanitized_message(),
                    "training anchors not promoted"
                );
            }
        }
    }
    Ok(report)
}
