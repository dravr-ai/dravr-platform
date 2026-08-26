// ABOUTME: Guardian-confirm stage — short-circuits a turn when the Guardian parked a tool pending confirmation
// ABOUTME: Renders a deterministic locale-aware prompt carrying the /confirm claim token, never an LLM paraphrase

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guardian confirm-required recovery for the chat pipeline.
//!
//! When the tool loop detects that the runtime Guardian parked a destructive
//! tool call for human confirmation (`TaintedDestructive::Confirm`, see
//! [`pierre_tool_runtime::tool_execution`]), it exits immediately and
//! propagates the tool + claim token via
//! [`ToolLoopResult::guardian_confirm`]. This stage observes that signal and
//! renders the localized confirmation prompt deterministically, exactly like
//! [`super::guardian_denied`] — the ask can never be softened,
//! misrepresented, or hallucinated away by a follow-up model turn.
//!
//! The prompt carries the tool's registry identifier (a static, platform-owned
//! name — meaningful consent needs to say WHAT is being confirmed) and the
//! opaque claim token. It never echoes the tool arguments: they can carry the
//! very injected content the taint rule fired on.

use std::sync::Arc;

use tracing::warn;

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_GUARDIAN_CONFIRM_PROMPT,
};
use pierre_tool_runtime::tool_execution::ToolLoopResult;

/// Apply Guardian confirm-required recovery in place.
///
/// When `result.guardian_confirm` is set, render the localized confirmation
/// prompt (tool name + claim token) and replace `result.content` with it.
///
/// Returns `true` when the stage fired so callers can skip LLM-content-aware
/// post-processing (text guardrails, claim verification); returns `false`
/// when no tool was parked and downstream stages should run normally.
///
/// `locale` is the turn's resolved BCP-47 short code, taken from
/// [`crate::SurfaceProfile::locale`] — resolution happened at the ingress
/// boundary, so this stage renders rather than re-derives.
pub fn apply_guardian_confirm(
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    locale: &str,
    result: &mut ToolLoopResult,
) -> bool {
    let Some(confirm) = result.guardian_confirm.as_ref() else {
        return false;
    };

    let message = messaging_strings_registry.render(
        KEY_GUARDIAN_CONFIRM_PROMPT,
        locale,
        &[&confirm.tool_name, &confirm.pending_id],
    );

    warn!(
        tool_name = %confirm.tool_name,
        pending_id = %confirm.pending_id,
        locale = %locale,
        "guardian_confirm: rendering localized confirmation prompt (enforce mode)"
    );

    result.content = message;
    true
}
