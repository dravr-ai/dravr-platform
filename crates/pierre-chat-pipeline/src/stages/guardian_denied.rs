// ABOUTME: Guardian-denied stage — short-circuits a turn when the runtime Guardian blocked a tool
// ABOUTME: Renders a deterministic locale-aware block reply saying what already went through, never an LLM paraphrase
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guardian-denied recovery for the chat pipeline.
//!
//! When the multi-turn tool loop detects that the runtime Guardian blocked a
//! consequential tool in `enforce` mode (see
//! [`pierre_tool_runtime::tool_execution`]), it exits immediately and
//! propagates the offending tool + machine reason via
//! [`ToolLoopResult::guardian_denied`]. This stage observes that signal and
//! overrides [`ToolLoopResult::content`] with a deterministic localized reply:
//!
//! - [`KEY_GUARDIAN_DENIED_AFTER_ACTIONS`] when the turn already committed
//!   something, naming those actions by their display titles, so an athlete whose
//!   first delete landed is never told only that "that action was blocked".
//! - [`KEY_GUARDIAN_DENIED`] when nothing went through.
//!
//! Neither reply names the blocked tool or echoes an argument.
//!
//! This mirrors [`super::auth_recovery`]: a special, out-of-band tool outcome is
//! rendered deterministically rather than fed back to the LLM, so the block can
//! never be softened, misrepresented, or hallucinated away by a follow-up model
//! turn. In `observe` mode the runtime Guardian denies no tool call.

use std::sync::Arc;

use tracing::warn;

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_GUARDIAN_DENIED, KEY_GUARDIAN_DENIED_AFTER_ACTIONS,
};
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

/// The committed calls as the athlete reads them: display titles in first-seen
/// order, a repeated title counted (`Delete recipe ×2`).
fn committed_actions(tools: &ToolRegistry, committed: &[String]) -> String {
    let mut titles: Vec<(String, usize)> = Vec::new();
    for tool_name in committed {
        let title = tools.display_name(tool_name);
        match titles.iter_mut().find(|(seen, _)| *seen == title) {
            Some((_, count)) => *count += 1,
            None => titles.push((title, 1)),
        }
    }
    titles
        .into_iter()
        .map(|(title, count)| {
            if count > 1 {
                format!("{title} ×{count}")
            } else {
                title
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Apply Guardian-denied recovery in place.
///
/// When `result.guardian_denied` is set, render the localized reply and replace
/// `result.content` with it.
///
/// Returns `true` when the stage fired so callers can skip LLM-content-aware
/// post-processing (text guardrails, claim verification); returns `false` when
/// no tool was Guardian-denied and downstream stages should run normally.
///
/// `locale` is the turn's resolved BCP-47 short code, taken from
/// [`crate::SurfaceProfile::locale`] — resolution happened at the ingress
/// boundary, so this stage renders rather than re-derives.
pub(crate) fn apply_guardian_denied(
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    tools: &ToolRegistry,
    locale: &str,
    result: &mut ToolLoopResult,
) -> bool {
    let Some(denial) = result.guardian_denied.as_ref() else {
        return false;
    };

    let message = if denial.committed.is_empty() {
        messaging_strings_registry.render(KEY_GUARDIAN_DENIED, locale, &[])
    } else {
        messaging_strings_registry.render(
            KEY_GUARDIAN_DENIED_AFTER_ACTIONS,
            locale,
            &[&committed_actions(tools, &denial.committed)],
        )
    };

    // The blocked tool's name and the machine reason are for operators only.
    warn!(
        tool_name = %denial.tool_name,
        guardian.reason = %denial.reason,
        committed = denial.committed.len(),
        locale = %locale,
        "guardian_denied: rendering localized block reply (enforce mode)"
    );

    result.content = message;
    true
}

#[cfg(test)]
mod tests {
    //! Guardian-denied render stage.
    //!
    //! When the tool loop surfaces a `ToolLoopResult::guardian_denied` (a tool
    //! blocked at the runtime chokepoint in `enforce` mode), the stage replaces the
    //! reply with the locale-resolved string and reports that it fired (so the
    //! pipeline skips LLM post-processing). A clean turn (`guardian_denied == None`)
    //! must be a no-op so `observe` mode and ordinary turns are untouched.

    use std::sync::Arc;

    use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_GUARDIAN_DENIED};
    use pierre_tool_runtime::implementations::connection::DisconnectProviderTool;
    use pierre_tool_runtime::registry::ToolRegistry;
    use pierre_tool_runtime::tool_loop_io::{GuardianDenial, ToolLoopResult};

    use super::apply_guardian_denied;

    fn loop_result(denied: Option<GuardianDenial>) -> ToolLoopResult {
        ToolLoopResult {
            content: String::new(),
            usage: None,
            finish_reason: Some("guardian_denied".to_owned()),
            activity_list: None,
            tool_calls_count: 1,
            tools_called: vec!["disconnect_provider".to_owned()],
            pending_provider_auth_required: None,
            served_without_provider: None,
            guardian_denied: denied,
            capability_claim_unverified: false,
            provider_warnings: Vec::new(),
        }
    }

    #[test]
    fn denial_renders_localized_reply_and_short_circuits() {
        let registry = Arc::new(MessagingStringsRegistry::new());
        let mut result = loop_result(Some(GuardianDenial {
            tool_name: "disconnect_provider".to_owned(),
            reason: "budget_exceeded".to_owned(),
            committed: Vec::new(),
        }));

        let fired = apply_guardian_denied(&registry, &ToolRegistry::new(), "en", &mut result);

        assert!(fired, "the stage must fire when a tool was guardian-denied");
        assert_eq!(
            result.content,
            registry.get(KEY_GUARDIAN_DENIED, "en"),
            "the reply must be the locale-resolved guardian-denied string"
        );
        assert!(result.content.contains("blocked for safety"));
    }

    #[test]
    fn denial_respects_resolved_locale() {
        let registry = Arc::new(MessagingStringsRegistry::new());
        let mut result = loop_result(Some(GuardianDenial {
            tool_name: "delete_agent".to_owned(),
            reason: "budget_exceeded".to_owned(),
            committed: Vec::new(),
        }));

        assert!(
            apply_guardian_denied(&registry, &ToolRegistry::new(), "fr", &mut result),
            "the stage must fire when a tool was guardian-denied"
        );
        assert_eq!(
            result.content,
            registry.get(KEY_GUARDIAN_DENIED, "fr"),
            "a French turn must get the French guardian-denied string"
        );
        assert!(result.content.contains("bloquée par sécurité"));
    }

    #[test]
    fn unsupported_locale_falls_back_to_the_default_string() {
        // The surface profile always carries a locale, so the stage never sees an
        // absent one. It can still be handed a code the string catalogue does not
        // stock (a channel reporting a language nobody translated), and the reply
        // must then read as the default locale rather than as an empty message.
        let registry = Arc::new(MessagingStringsRegistry::new());
        let mut result = loop_result(Some(GuardianDenial {
            tool_name: "disconnect_provider".to_owned(),
            reason: "egress_forbidden".to_owned(),
            committed: Vec::new(),
        }));

        assert!(
            apply_guardian_denied(&registry, &ToolRegistry::new(), "is", &mut result),
            "the stage must fire when a tool was guardian-denied"
        );
        assert_eq!(
            result.content,
            registry.get(KEY_GUARDIAN_DENIED, "fr"),
            "an unstocked locale must render DEFAULT_LOCALE's string, never empty"
        );
        assert!(result.content.contains("bloquée"));
    }

    /// The live failure on dev: two deletes asked, the first committed, and the
    /// reply said only that the action was blocked.
    #[test]
    fn a_denial_after_commits_says_what_went_through() {
        let registry = Arc::new(MessagingStringsRegistry::new());
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(DisconnectProviderTool));
        let mut result = loop_result(Some(GuardianDenial {
            tool_name: "delete_recipe".to_owned(),
            reason: "budget_exceeded".to_owned(),
            committed: vec![
                "disconnect_provider".to_owned(),
                "disconnect_provider".to_owned(),
            ],
        }));

        assert!(
            apply_guardian_denied(&registry, &tools, "en", &mut result),
            "the stage must fire when a tool was guardian-denied"
        );

        assert!(
            result.content.contains("Disconnect fitness provider ×2"),
            "the reply must name what went through in words, got: {}",
            result.content
        );
        assert!(
            !result.content.contains("delete_recipe")
                && !result.content.contains("disconnect_provider"),
            "registry ids are not for the athlete, got: {}",
            result.content
        );
    }

    #[test]
    fn clean_turn_is_a_no_op() {
        // observe mode (and every ordinary turn) never sets guardian_denied — the
        // stage must not fire and must not touch the (empty) content the LLM path
        // will fill in downstream.
        let registry = Arc::new(MessagingStringsRegistry::new());
        let mut result = loop_result(None);

        let fired = apply_guardian_denied(&registry, &ToolRegistry::new(), "en", &mut result);

        assert!(
            !fired,
            "a clean turn must not trip the guardian-denied stage"
        );
        assert!(
            result.content.is_empty(),
            "the stage must leave content untouched when no tool was denied"
        );
    }
}
