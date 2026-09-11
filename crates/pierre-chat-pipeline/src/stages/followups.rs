// ABOUTME: Tier 4 agent session + followup stages — session attach, pending followups, finalize
// ABOUTME: Wires coach_session bookkeeping into chat-turn pre/post hooks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 4 agent session + followup handling.
//!
//! Three related responsibilities:
//!
//! 1. Session attach (pre-dispatch) — ensure a long-lived agent session
//!    exists for the `(user, agent)` pair and is attached to the conversation.
//!    Idempotent and best-effort; failures do not block the turn.
//! 2. Pending followups (prompt-assembly) — render any pending agent
//!    followups as a system-prompt block so the agent honors commitments it
//!    made on prior turns. Returns the IDs of followups surfaced so they
//!    can be marked delivered after the turn succeeds.
//! 3. Session finalize (post-dispatch) — touch the session's last-active
//!    timestamp and mark any followups that were surfaced as delivered.
//!    Idempotent.

use std::fmt::Write as _;

use pierre_database::database::ConversationRecord;

use pierre_core::models::TenantId;
use pierre_runtime_context::DataContext;

/// Ensure the conversation has an agent session attached.
///
/// Idempotent — returns the original conversation untouched when there is
/// no agent, when a session is already attached, or when the underlying
/// repository operations fail. The conversation row is mutated in memory
/// and in the database so the rest of the dispatch path can rely on
/// `conv.session_id` being set.
///
/// The session is the conversation's own agent's, read from `conv.agent_id`:
/// a `@handle` turn answers as another agent but never attaches that agent's
/// session to the row, which is what keeps a per-turn mention from leaving a
/// durable mark on the conversation.
pub async fn ensure_agent_session_attached(
    data: &DataContext,
    mut conv: ConversationRecord,
    tenant_id: TenantId,
) -> ConversationRecord {
    let Some(agent_id) = conv.agent_id.clone() else {
        return conv;
    };
    if conv.session_id.is_some() {
        return conv;
    }

    let session = match data
        .repos()
        .memory
        .get_or_open_agent_session(tenant_id, &conv.user_id, &agent_id)
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

/// Render pending agent followups into the prompt.
///
/// Returns the prompt with an injected followups block (when there are
/// any) plus the list of followup IDs that were surfaced this turn so the
/// dispatcher can mark them delivered after the assistant reply lands.
pub async fn inject_pending_followups(
    data: &DataContext,
    tenant_id: TenantId,
    user_id: &str,
    agent_id: Option<&str>,
    base_prompt: String,
) -> (String, Vec<String>) {
    let Some(agent_id) = agent_id else {
        return (base_prompt, Vec::new());
    };
    let followups = match data
        .repos()
        .memory
        .list_pending_followups(tenant_id, user_id, agent_id)
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
/// Touches the active agent session (so "continue where you left off" UI
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
            .touch_agent_session(session_id, tenant_id)
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
