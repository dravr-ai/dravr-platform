// ABOUTME: Repository trait for guardian_pending_actions — parked destructive tool calls awaiting /confirm
// ABOUTME: Single-use owner-checked claims with expiry at resolution; the Guardian Confirm HITL storage seam

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;

/// A destructive tool call the Guardian parked pending user confirmation.
///
/// `arguments` is the tool-call JSON verbatim — stored to re-dispatch on
/// `/confirm`, never echoed to the user (it can carry the very injected
/// content the taint rule fired on). Ids are uuid-simple tokens (122-bit
/// entropy); ownership is still enforced on claim.
#[derive(Debug, Clone)]
pub struct PendingGuardianAction {
    /// Opaque uuid-simple claim token, surfaced to the user in the prompt.
    pub id: String,
    /// Stringified tenant of the dispatch that was parked.
    pub tenant_id: String,
    /// Stringified user the confirmation belongs to.
    pub user_id: String,
    /// Originating conversation, when the dispatch had one (chat surfaces).
    pub conversation_id: Option<String>,
    /// Registry identifier of the parked tool.
    pub tool_name: String,
    /// Tool-call arguments JSON, re-dispatched verbatim on confirm.
    pub arguments: serde_json::Value,
    /// The Guardian deny reason that triggered the park (`tainted_sink`).
    pub deny_reason: String,
}

/// Result of an atomic claim attempt on a pending action.
#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// The caller won the single-use claim; the action payload follows.
    /// Boxed: the payload dwarfs the unit variants (`large_enum_variant`).
    Claimed(Box<PendingGuardianAction>),
    /// The row exists and belongs to the caller but its TTL elapsed; it has
    /// been marked `expired`.
    Expired,
    /// No claimable row: unknown id, another user's row, or already resolved.
    /// Collapsed into one variant on purpose — distinguishing "someone
    /// else's id" from "unknown id" would let ids be probed for existence.
    NotFound,
}

/// Persistent store behind the Guardian's Confirm human-in-the-loop flow.
#[async_trait]
pub trait GuardianPendingActionsRepository: Send + Sync {
    /// Park a destructive tool call until `expires_at`.
    async fn create_pending_action(
        &self,
        action: &PendingGuardianAction,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Atomically claim a pending action for `user_id`/`tenant_id`, flipping
    /// `pending` → `resolution` (`confirmed` or `denied`). Single-use: of two
    /// concurrent claims exactly one wins. Expiry is checked here, at
    /// resolution time (the `short_links` pattern) — an elapsed row is marked
    /// `expired` and reported as [`ClaimOutcome::Expired`].
    async fn claim_pending_action(
        &self,
        id: &str,
        user_id: &str,
        tenant_id: &str,
        resolution: &str,
    ) -> AppResult<ClaimOutcome>;

    /// Delete rows whose TTL elapsed, returning how many were removed.
    ///
    /// Claims already filter expired rows, so this is storage hygiene only;
    /// it is invoked opportunistically when a new action is parked.
    async fn delete_expired_pending_actions(&self) -> AppResult<u64>;
}
