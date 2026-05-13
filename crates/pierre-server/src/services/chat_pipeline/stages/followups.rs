// ABOUTME: Tier 4 coach session + followup stages — session attach, pending followups, finalize
// ABOUTME: Extracted from services/chat_orchestration.rs session/followup helpers (2026-04-16)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 4 coach session + followup handling.
//!
//! Three related responsibilities:
//!
//! 1. Session attach (pre-dispatch) — ensure a long-lived coach session
//!    exists for the `(user, coach)` pair and is attached to the conversation.
//!    Idempotent and best-effort; failures do not block the turn.
//! 2. Pending followups (prompt-assembly) — render any pending coach
//!    followups as a system-prompt block so the coach honors commitments it
//!    made on prior turns. Returns the IDs of followups surfaced so they
//!    can be marked delivered after the turn succeeds.
//! 3. Session finalize (post-dispatch) — touch the session's last-active
//!    timestamp and mark any followups that were surfaced as delivered.
//!    Idempotent.

use std::fmt::Write as _;

use pierre_database::database::ConversationRecord;

use crate::context::DataContext;
use crate::models::TenantId;

/// Ensure the conversation has a coach session attached.
///
/// Idempotent — returns the original conversation untouched when there is
/// no coach, when a session is already attached, or when the underlying
/// repository operations fail. The conversation row is mutated in memory
/// and in the database so the rest of the dispatch path can rely on
/// `conv.session_id` being set.
pub async fn ensure_coach_session_attached(
    data: &DataContext,
    mut conv: ConversationRecord,
    tenant_id: TenantId,
) -> ConversationRecord {
    let Some(coach_id) = conv.coach_id.clone() else {
        return conv;
    };
    if conv.session_id.is_some() {
        return conv;
    }

    let session = match data
        .repos()
        .memory
        .get_or_open_coach_session(tenant_id, &conv.user_id, &coach_id)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "coach session resolution failed; continuing without session");
            return conv;
        }
    };

    if let Err(e) = data
        .repos()
        .chat
        .set_conversation_session_id(&conv.id, &session.id, tenant_id)
        .await
    {
        tracing::warn!(error = %e, "failed to attach session_id to conversation");
        return conv;
    }

    conv.session_id = Some(session.id);
    conv
}

/// Render pending coach followups into the prompt.
///
/// Returns the prompt with an injected followups block (when there are
/// any) plus the list of followup IDs that were surfaced this turn so the
/// dispatcher can mark them delivered after the assistant reply lands.
pub async fn inject_pending_followups(
    data: &DataContext,
    tenant_id: TenantId,
    user_id: &str,
    coach_id: Option<&str>,
    base_prompt: String,
) -> (String, Vec<String>) {
    let Some(coach_id) = coach_id else {
        return (base_prompt, Vec::new());
    };
    let followups = match data
        .repos()
        .memory
        .list_pending_followups(tenant_id, user_id, coach_id)
        .await
    {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list pending followups");
            return (base_prompt, Vec::new());
        }
    };
    if followups.is_empty() {
        return (base_prompt, Vec::new());
    }

    let mut block = String::from("\n\n## Pending followups you committed to\n\n");
    let mut ids = Vec::with_capacity(followups.len());
    for f in &followups {
        let due = f
            .due_at
            .map(|d| format!(" (due {})", d.to_rfc3339()))
            .unwrap_or_default();
        let _ = writeln!(block, "- {}{}", f.content, due);
        ids.push(f.id.clone());
    }
    block.push_str(
        "\nAddress these now if relevant; otherwise acknowledge them and explain why later.",
    );
    (format!("{base_prompt}{block}"), ids)
}

/// Post-turn cleanup of session state.
///
/// Touches the active coach session (so "continue where you left off" UI
/// surfaces a fresh timestamp) and marks any followups we surfaced this
/// turn as delivered. Errors are logged and swallowed.
pub async fn finalize_session_state(
    data: &DataContext,
    session_id: Option<&str>,
    delivered_followup_ids: &[String],
    tenant_id: TenantId,
) {
    if let Some(session_id) = session_id {
        if let Err(e) = data
            .repos()
            .memory
            .touch_coach_session(session_id, tenant_id)
            .await
        {
            tracing::warn!(error = %e, "failed to touch coach session");
        }
    }
    for followup_id in delivered_followup_ids {
        if let Err(e) = data
            .repos()
            .memory
            .mark_followup_delivered(followup_id, tenant_id)
            .await
        {
            tracing::warn!(error = %e, followup_id, "failed to mark followup delivered");
        }
    }
}
