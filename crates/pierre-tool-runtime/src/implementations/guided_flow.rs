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

/// `true` when a guided interview owns the turn and write tools are withheld.
///
/// The conversation is authoritative when the call arrives through the chat
/// pipeline, which puts its id in scope. A `tools/call` on the `/mcp` endpoint
/// has none, so the state is resolved from the athlete's conversations instead —
/// the `conversation_id` argument is model-supplied and cannot be trusted to
/// answer a question about whether this same model may write.
///
/// Public so discovery and execution share one predicate: `/mcp` `tools/list`
/// asks before advertising, `save_training_plan` before running.
///
/// # Errors
///
/// Propagates a repository failure; callers should treat an error as "a flow
/// may be active" rather than advertising.
pub async fn guided_flow_is_active(
    repos: &RepositoryRegistry,
    conv: Option<&ConversationRecord>,
    tenant: TenantId,
    user_id: &str,
) -> AppResult<bool> {
    if let Some(conv) = conv {
        return Ok(OnboardingState::from_column(conv.onboarding_state.as_deref()).is_some());
    }
    let states = repos
        .chat
        .list_user_onboarding_states(user_id, tenant, GUIDED_FLOW_SCAN_LIMIT)
        .await?;
    Ok(states
        .iter()
        .any(|raw| OnboardingState::from_column(Some(raw)).is_some()))
}
