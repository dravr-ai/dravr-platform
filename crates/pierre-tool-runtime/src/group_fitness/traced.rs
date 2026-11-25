// ABOUTME: Member snapshots each traced to what its reads drew on, for a copy posted into a messaging group chat
// ABOUTME: A snapshot drawn from members-only data (Garmin) is marked so a group-chat copy leaves its member out (carnet#1170)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Traced member snapshots. The parent module re-exports them.

use std::sync::Arc;

use futures_util::future::join_all;
use pierre_core::models::groups::MemberFitnessSnapshot;
use pierre_core::models::TenantId;
use pierre_providers::ai_scope;
use uuid::Uuid;

#[cfg(doc)]
use super::fetch_member_snapshots_in;
use super::fetch_single_member_snapshot;
use crate::runtime::ToolRuntime;

/// One member's snapshot and whether it was drawn from data kept to Dravr's
/// members (carnet#1170).
#[derive(Debug, Clone)]
pub struct TracedSnapshot {
    /// The member's snapshot.
    pub snapshot: MemberFitnessSnapshot,
    /// Whether any of it was drawn from a provider whose terms keep its data
    /// to Dravr's members: a copy posted into a messaging group chat leaves
    /// this member out.
    pub members_only: bool,
}

/// [`fetch_member_snapshots_in`], each snapshot with what its own reads drew
/// on.
///
/// Each member's reads accumulate into their own provenance, which is handed
/// on to the derivation running here as well, so a caller stamping the whole
/// batch still sees everything it served.
pub async fn fetch_member_snapshots_traced(
    runtime: &Arc<dyn ToolRuntime>,
    members: &[(Uuid, TenantId)],
) -> Vec<TracedSnapshot> {
    if members.is_empty() {
        return Vec::new();
    }

    let futures: Vec<_> = members
        .iter()
        .map(|&(user_id, tenant_id)| {
            let runtime = Arc::clone(runtime);
            async move {
                let provenance = ai_scope::Provenance::new();
                let snapshot = Box::pin(ai_scope::tracking(
                    provenance.clone(),
                    ai_scope::reading_about(
                        user_id,
                        fetch_single_member_snapshot(&runtime, user_id, tenant_id),
                    ),
                ))
                .await;
                TracedSnapshot {
                    snapshot,
                    members_only: provenance.members_only(),
                }
            }
        })
        .collect();

    join_all(futures).await
}
