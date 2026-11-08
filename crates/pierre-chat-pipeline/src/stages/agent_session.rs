// ABOUTME: Tier 4 agent session stages — session attach before dispatch, session finalize after it
// ABOUTME: Wires coach_session bookkeeping into chat-turn pre/post hooks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 4 agent session handling.
//!
//! Two related responsibilities:
//!
//! 1. Session attach (pre-dispatch) — ensure a long-lived agent session
//!    exists for the `(user, agent)` pair and is attached to the conversation.
//!    Idempotent and best-effort; failures do not block the turn.
//! 2. Session finalize (post-dispatch) — touch the session's last-active
//!    timestamp. Idempotent.

use pierre_database::database::ConversationRecord;

use pierre_core::models::TenantId;
use pierre_runtime_context::DataContext;

use crate::TurnInput;

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

/// Post-turn cleanup of session state.
///
/// Touches the active agent session (so "continue where you left off" UI
/// surfaces a fresh timestamp). The session lives under the conversation's
/// tenant. Errors are logged and swallowed.
pub async fn finalize_session_state(
    data: &DataContext,
    session_id: Option<&str>,
    input: &TurnInput,
) {
    if let Some(session_id) = session_id {
        if let Err(e) = data
            .repos()
            .memory
            .touch_agent_session(session_id, input.conversation_tenant_id)
            .await
        {
            tracing::warn!(error = %e, "failed to touch coach session");
        }
    }
}
