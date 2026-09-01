// ABOUTME: Whether a guided flow owns the turn — the one predicate discovery and execution share
// ABOUTME: Lives apart from any tool so `/mcp` tools/list and save_training_plan cannot drift

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One predicate, shared by discovery and execution.
//!
//! A guided flow (the `/pillars` walk) withholds plan-writing for its
//! duration. Enforcing that needs the same answer in two places: `/mcp`
//! `tools/list` decides whether to advertise the tool, and the tool itself
//! decides whether to run.
//!
//! They used to answer separately. Advertisement was filtered in
//! `build_mcp_tools`, which the native ACP path never reads — visibility there
//! comes from `tools/list` — so the filter silently no-opped the moment
//! `COPILOT_HEADLESS_MCP_TOOL_CALLING` was re-enabled, and a walk could see the
//! tool it was meant to be denied. One predicate, in a module that belongs to
//! neither caller, is what stops that recurring.

use pierre_core::errors::AppResult;
use pierre_core::models::{ConversationRecord, OnboardingState, TenantId};
use pierre_database::RepositoryRegistry;

/// Newest conversations consulted when resolving an athlete's guided-flow state
/// without a conversation in scope. An interview keeps its conversation the
/// most recently updated one, so a running walk is always inside this window.
const GUIDED_FLOW_SCAN_LIMIT: i64 = 50;

/// `true` when a guided interview owns the turn FOR THIS USER and write tools
/// are withheld.
///
/// The conversation is authoritative when the call arrives through the chat
/// pipeline, which puts its id (and, since room walks exist, the tenant that
/// owns its row) in scope. A `tools/call` on the `/mcp` endpoint has neither,
/// so the state is resolved from the athlete's conversations instead — the
/// `conversation_id` argument is model-supplied and cannot be trusted to
/// answer a question about whether this same model may write.
///
/// `conversation_ref` is the `(id, owning tenant)` pair from
/// [`crate::context::ToolExecutionContext::conversation_ref`]. It matters when
/// `conv` is `None` despite a conversation id being in scope: a shared room
/// files its row under the channel tenant, so the caller's own-tenant lookup
/// missed it — this predicate then retries under the owning tenant, which is
/// exactly the case where a room walk's withhold used to fall through to the
/// scan and silently never fire.
///
/// A walk bound to a subject withholds from that member alone: everyone else
/// on the thread is not mid-interview, and refusing the room's human coach a
/// plan save because their athlete is calibrating would be the wrong refusal.
///
/// Public so discovery and execution share one predicate: `/mcp` `tools/list`
/// asks before advertising, `save_training_plan` before running.
///
/// LIMITATION(registre#168): a bare `/mcp` `tools/call` carries no
/// conversation, and the fallback scan below runs under the caller's home
/// tenant — so an active ROOM walk, whose row lives under the channel tenant,
/// is invisible to `guided_flow_is_active` on that one path. The chat
/// pipeline, the Guardian `/confirm` re-dispatch, and every other
/// conversation-carrying call are covered.
///
/// # Errors
///
/// Propagates a repository failure; callers should treat an error as "a flow
/// may be active" rather than advertising.
pub async fn guided_flow_is_active(
    repos: &RepositoryRegistry,
    conv: Option<&ConversationRecord>,
    conversation_ref: Option<(&str, TenantId)>,
    tenant: TenantId,
    user_id: &str,
) -> AppResult<bool> {
    if let Some(conv) = conv {
        return Ok(walk_binds(conv.onboarding_state.as_deref(), user_id));
    }
    if let Some((conv_id, conv_tenant)) = conversation_ref {
        if conv_tenant != tenant {
            if let Some(conv) = repos
                .chat
                .get_conversation(conv_id, user_id, conv_tenant)
                .await?
            {
                return Ok(walk_binds(conv.onboarding_state.as_deref(), user_id));
            }
        }
    }
    let states = repos
        .chat
        .list_user_onboarding_states(user_id, tenant, GUIDED_FLOW_SCAN_LIMIT)
        .await?;
    Ok(states.iter().any(|raw| walk_binds(Some(raw), user_id)))
}

/// Whether the stored column holds an active walk that binds `user_id`.
///
/// A walk with no subject predates the binding (or was auto-started) and
/// belongs to the conversation's owner, which is who every pre-room caller
/// asked about — so it binds.
fn walk_binds(raw: Option<&str>, user_id: &str) -> bool {
    OnboardingState::from_column(raw).is_some_and(|state| {
        state
            .subject_user_id
            .as_deref()
            .is_none_or(|subject| subject == user_id)
    })
}

/// Tools withheld from the model while a guided conversational flow — today the
/// `/pillars` profile walk — owns the turn.
///
/// A profile interview asks one question and records the answer; writing a
/// training plan mid-interview is what the 2026-07-24 derail did instead of
/// moving to the second pillar. Read tools stay available so the athlete can
/// still ask "what did I ride yesterday?" without leaving the walk.
///
/// Single source of truth for the two surfaces that must agree: the native
/// function declarations filtered in `tool_dispatch.rs`, and the server-side
/// refusal in `SaveTrainingPlanTool::execute`. There used to be a third — a
/// prose "Available Tools" list generated into the system prompt — and keeping
/// three surfaces aligned is why this constant exists; that list is deleted, so
/// advertisement is now one surface rather than two that could disagree.
/// Declarations are advertisement, which is not enforcement; the refusal is
/// what covers the native-MCP path, where tool visibility comes from the
/// `/mcp` endpoint rather than from these
/// declarations.
pub const GUIDED_FLOW_WITHHELD_TOOLS: &[&str] = &["save_training_plan"];

/// Whether `tool_name` is withheld while a guided flow owns the turn.
#[must_use]
pub fn is_withheld_during_guided_flow(tool_name: &str) -> bool {
    GUIDED_FLOW_WITHHELD_TOOLS.contains(&tool_name)
}
