// ABOUTME: Which guided flow owns the turn, and what it withholds — the one lookup discovery and execution share
// ABOUTME: Lives apart from any tool so `/mcp` tools/list and save_training_plan cannot drift

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One lookup, shared by discovery and execution.
//!
//! A guided flow withholds plan-writing for its duration when the flow is an
//! interview — which every flow that exists today is. Enforcing that needs
//! the same answer in two places: `/mcp` `tools/list` decides whether to
//! advertise the tool, and the tool itself decides whether to run.
//!
//! They used to answer separately. Advertisement was filtered in
//! `build_mcp_tools`, which the native ACP path never reads — visibility there
//! comes from `tools/list` — so the filter silently no-opped the moment
//! `COPILOT_HEADLESS_MCP_TOOL_CALLING` was re-enabled, and a walk could see the
//! tool it was meant to be denied. One predicate, in a module that belongs to
//! neither caller, is what stops that recurring.

use pierre_core::errors::AppResult;
use pierre_core::models::{ConversationRecord, GuidedFlow, OnboardingState, TenantId};
use pierre_database::RepositoryRegistry;
use tracing::warn;

/// Newest conversations consulted when resolving an athlete's guided-flow state
/// without a conversation in scope. An interview keeps its conversation the
/// most recently updated one, so a running walk is always inside this window.
const GUIDED_FLOW_SCAN_LIMIT: i64 = 50;

/// The guided flow that owns the turn FOR THIS USER, when one does.
///
/// The flow, not a verdict: whether it withholds the write tools is
/// [`flow_withholds_writes`]'s question, asked of what this returns.
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
/// Public so discovery and execution share one lookup: `/mcp` `tools/list`
/// asks before advertising, `save_training_plan` before running.
///
/// LIMITATION(registre#168): a bare `/mcp` `tools/call` carries no
/// conversation, and the fallback scan below runs under the caller's home
/// tenant — so an active ROOM walk, whose row lives under the channel tenant,
/// is invisible to `active_guided_flow` on that one path. The chat
/// pipeline, the Guardian `/confirm` re-dispatch, and every other
/// conversation-carrying call are covered.
///
/// # Errors
///
/// Propagates a repository failure; callers should treat an error as "a flow
/// may be active" rather than advertising.
pub async fn active_guided_flow(
    repos: &RepositoryRegistry,
    conv: Option<&ConversationRecord>,
    conversation_ref: Option<(&str, TenantId)>,
    tenant: TenantId,
    user_id: &str,
) -> AppResult<Option<GuidedFlow>> {
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
    Ok(states.iter().find_map(|raw| walk_binds(Some(raw), user_id)))
}

/// The flow that binds this user, when one does.
///
/// Returns the flow rather than a bool because what a walk withholds is a
/// property *of that walk*: an interview must not write a plan, and a walk
/// whose whole purpose is to write one must. Discarding the flow here is
/// what made the withhold list flow-agnostic, and a writing flow
/// unbuildable.
fn walk_binds(raw: Option<&str>, user_id: &str) -> Option<GuidedFlow> {
    OnboardingState::from_column(raw).and_then(|state| {
        state
            .subject_user_id
            .as_deref()
            .is_none_or(|subject| subject == user_id)
            .then_some(state.flow)
    })
}

/// Tools withheld from the model while a withholding guided flow owns the
/// turn. Which flows those are is [`flow_withholds_writes`]'s question; this
/// is the list they keep back.
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

/// Whether a flow withholds the plan-writing tools while it owns the turn.
///
/// Every walk is an interview — it asks a question and records the answer —
/// so every one of them withholds, which for the four walks is the same
/// behaviour the flow-agnostic list had. It takes the flow because the rule is
/// about what a flow is *for*: one that exists to write a plan cannot be
/// denied the tool that writes it, and the list being flow-agnostic is what
/// made such a flow unbuildable.
///
/// [`GuidedFlow::Fortnight`] is that flow, and it is the reason this is a
/// function. It never owns a turn today — `/fortnight` decides in Rust and
/// retires its marker in the same breath — so the answer it gives is not read
/// on any live path; it is stated here because the alternative is deciding it
/// implicitly, and a rail whose whole purpose is to save two weeks must never
/// inherit "save is withheld" from a list it was never considered for.
#[must_use]
pub const fn flow_withholds_writes(flow: GuidedFlow) -> bool {
    flow.is_interview()
}

/// Whether `tool_name` is one a withholding flow keeps back.
#[must_use]
pub fn is_withheld_tool(tool_name: &str) -> bool {
    GUIDED_FLOW_WITHHELD_TOOLS.contains(&tool_name)
}

/// Whether `tool_name` is withheld while `flow` owns the turn.
#[must_use]
pub fn is_withheld_during_guided_flow(flow: GuidedFlow, tool_name: &str) -> bool {
    flow_withholds_writes(flow) && is_withheld_tool(tool_name)
}

/// Whether this turn withholds the plan-writing tools, read from a lookup
/// that may have failed.
///
/// An `Err` reads as *a flow may be running, and it withholds*. A repository
/// failure is not evidence that no walk is active, and advertising a write
/// tool into a live interview is the derail this guard exists to prevent, so
/// the unknown case fails closed.
///
/// It is logged rather than swallowed, because failing closed stopped being
/// free the moment a walk could exist whose purpose is to write: withholding
/// from *that* walk breaks it, and a silent `unwrap_or(true)` would make that
/// look like the walk simply refusing to work.
#[must_use]
pub fn turn_withholds_writes(lookup: &AppResult<Option<GuidedFlow>>) -> bool {
    match lookup {
        Ok(flow) => flow.is_some_and(flow_withholds_writes),
        Err(e) => {
            // An unknown flow withholds. This is the right answer for both
            // kinds now that a writing flow exists, and it is a decision
            // rather than an accident (carnet#416 asked for it explicitly).
            //
            // The Err arm carries no flow, so it cannot tell an interview
            // from the fortnight rail — and it does not have to. This
            // governs ADVERTISEMENT: never advertising a write tool into a
            // turn that might be an interview is the derail this exists to
            // prevent, and the cost to a fortnight turn is a tool it was not
            // offered rather than one it silently failed to use.
            //
            // The rail is covered on the other side. Enforcement resolves the
            // walk with `?` (`training_plans::save_training_plan`), so the
            // same repository failure returns a real error from the call
            // instead of dropping the save — the agent reports what happened
            // rather than telling the athlete two weeks were written when
            // nothing was. Threading the flow through here to refuse the turn
            // outright would change shipped failure semantics for the four
            // live walks to serve the fifth.
            warn!(
                error = %e,
                "guided-flow lookup failed; withholding the write tools for this turn"
            );
            true
        }
    }
}
