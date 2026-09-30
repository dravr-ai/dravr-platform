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
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

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

#[cfg(test)]
mod tests {
    //! Guardian confirm-required render stage.
    //!
    //! Pins the Confirm HITL UX: when the tool loop surfaces a
    //! `ToolLoopResult::guardian_confirm` (a destructive tool parked at the
    //! runtime chokepoint under `TaintedDestructive::Confirm`), the stage replaces
    //! the reply with the locale-resolved `KEY_GUARDIAN_CONFIRM_PROMPT` carrying
    //! the tool name and the `/confirm` claim token, and reports that it fired. A
    //! clean turn must be a no-op.

    use std::sync::Arc;

    use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
    use pierre_tool_runtime::tool_loop_io::{GuardianConfirmRequest, ToolLoopResult};

    use super::apply_guardian_confirm;

    fn loop_result(confirm: Option<GuardianConfirmRequest>) -> ToolLoopResult {
        ToolLoopResult {
            content: String::new(),
            usage: None,
            finish_reason: Some("guardian_confirm".to_owned()),
            activity_list: None,
            tool_calls_count: 1,
            tools_called: vec!["disconnect_provider".to_owned()],
            pending_provider_auth_required: None,
            served_without_provider: None,
            guardian_denied: None,
            guardian_confirm: confirm,
            capability_claim_unverified: false,
            provider_warnings: Vec::new(),
        }
    }

    #[test]
    fn park_renders_localized_prompt_with_tool_and_token() {
        let registry = Arc::new(MessagingStringsRegistry::new());
        let mut result = loop_result(Some(GuardianConfirmRequest {
            tool_name: "disconnect_provider".to_owned(),
            pending_id: "abc123def456".to_owned(),
        }));

        let fired = apply_guardian_confirm(&registry, "en", &mut result);

        assert!(fired, "the stage must fire when a tool was parked");
        assert!(
            result.content.contains("disconnect_provider"),
            "the prompt must name the parked tool, got: {}",
            result.content
        );
        assert!(
            result.content.contains("/confirm abc123def456"),
            "the prompt must carry the /confirm claim token, got: {}",
            result.content
        );
        assert!(
            result.content.contains("/deny abc123def456"),
            "the prompt must carry the /deny claim token, got: {}",
            result.content
        );
    }

    #[test]
    fn park_respects_resolved_locale() {
        let registry = Arc::new(MessagingStringsRegistry::new());
        let mut result = loop_result(Some(GuardianConfirmRequest {
            tool_name: "disconnect_provider".to_owned(),
            pending_id: "abc123".to_owned(),
        }));

        let fired = apply_guardian_confirm(&registry, "fr", &mut result);

        assert!(fired);
        assert!(
            result.content.contains("Par sécurité"),
            "a French turn must get the French prompt, got: {}",
            result.content
        );
    }

    #[test]
    fn clean_turn_is_a_no_op() {
        let registry = Arc::new(MessagingStringsRegistry::new());
        let mut result = loop_result(None);
        result.content = "normal reply".to_owned();

        let fired = apply_guardian_confirm(&registry, "en", &mut result);

        assert!(!fired, "no park, no fire");
        assert_eq!(result.content, "normal reply", "content must be untouched");
    }
}
