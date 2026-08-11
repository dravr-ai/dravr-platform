// ABOUTME: Guardian-denied stage — short-circuits a turn when the runtime Guardian blocked a tool
// ABOUTME: Renders a deterministic locale-aware "blocked for safety" reply instead of an LLM paraphrase
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guardian-denied recovery for the chat pipeline.
//!
//! When the multi-turn tool loop detects that the runtime Guardian blocked a
//! consequential tool in `enforce` mode (see
//! [`pierre_tool_runtime::tool_execution`]), it exits immediately and
//! propagates the offending tool + machine reason via
//! [`ToolLoopResult::guardian_denied`]. This stage observes that signal and:
//!
//! 1. Renders [`pierre_contremaitre::messaging_strings::KEY_GUARDIAN_DENIED`]
//!    in the user's resolved locale (the message carries no placeholders — it
//!    deliberately never echoes the tool name or arguments back to the user).
//! 2. Overrides [`ToolLoopResult::content`] with that deterministic reply so
//!    downstream stages (`post_process`, `persistence`) see a clean,
//!    user-appropriate message instead of the empty string the short-circuited
//!    tool loop produced.
//!
//! This mirrors [`super::auth_recovery`] exactly: a special, out-of-band tool
//! outcome is rendered deterministically rather than fed back to the LLM, so
//! the security block can never be softened, misrepresented, or hallucinated
//! away by a follow-up model turn. In `off` and `observe` modes (`enforce` is
//! the default) the Guardian never denies, so `guardian_denied` stays `None`
//! and this stage is a no-op.

use std::sync::Arc;

use tracing::warn;

use crate::turn::TurnInput;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, DEFAULT_LOCALE, KEY_GUARDIAN_DENIED,
};
use pierre_tool_runtime::tool_execution::ToolLoopResult;

/// Apply Guardian-denied recovery in place.
///
/// When `result.guardian_denied` is set, render the localized "blocked for
/// safety" reply and replace `result.content` with it.
///
/// Returns `true` when the stage fired so callers can skip LLM-content-aware
/// post-processing (text guardrails, claim verification); returns `false` when
/// no tool was Guardian-denied and downstream stages should run normally.
pub fn apply_guardian_denied(
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
) -> bool {
    let Some(denial) = result.guardian_denied.as_ref() else {
        return false;
    };

    let locale = input
        .locale
        .as_deref()
        .filter(|l| !l.is_empty())
        .unwrap_or(DEFAULT_LOCALE);

    // The user-facing string is placeholder-free by design: it must not leak
    // the blocked tool name or arguments back into the conversation. The tool
    // name and machine reason are logged for operators only.
    let message = messaging_strings_registry.render(KEY_GUARDIAN_DENIED, locale, &[]);

    warn!(
        tool_name = %denial.tool_name,
        guardian.reason = %denial.reason,
        locale = %locale,
        "guardian_denied: rendering localized block reply (enforce mode)"
    );

    result.content = message;
    true
}
