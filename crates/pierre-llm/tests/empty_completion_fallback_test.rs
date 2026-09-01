// ABOUTME: An empty completion from the chain's primary must fall back, not be reported as a success
// ABOUTME: Empty content WITH tool calls is an ordinary mid-loop turn and must not trigger fallback
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The runtime fallback chain used to match `Ok(response)` and take the success
//! path regardless of what the response contained. A primary that returned an
//! empty completion — the Copilot ACP "empty turn", observed 2026-08-31 — was
//! therefore recorded as primary *health*, the secondary was never consulted,
//! and the athlete received the platform's lost-turn apology while two working
//! providers sat unused (carnet#165).
//!
//! The classification these tests pin is the whole fix. Getting it wrong in the
//! permissive direction restores the bug; getting it wrong in the strict
//! direction is worse — treating a tool-call turn as empty would fall back on
//! every tool-using turn in every conversation, against a paid provider.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use embacle::ToolCallRequest;
use pierre_llm::{is_empty_completion, ChatResponse};
use serde_json::json;

fn response(content: &str, tool_calls: Option<Vec<ToolCallRequest>>) -> ChatResponse {
    ChatResponse {
        content: content.to_owned(),
        model: "test-model".to_owned(),
        usage: None,
        finish_reason: Some("stop".to_owned()),
        warnings: None,
        tool_calls,
    }
}

fn a_tool_call() -> ToolCallRequest {
    ToolCallRequest {
        id: "call_1".to_owned(),
        function_name: "get_activities".to_owned(),
        arguments: json!({"limit": 10}),
    }
}

/// The live shape: `finish_reason: "stop"`, no content, no tool calls.
#[test]
fn a_bare_empty_turn_is_empty() {
    assert!(
        is_empty_completion(&response("", None)),
        "an empty turn with no tool calls is the failure this exists to catch"
    );
}

/// Whitespace is not content. Every surface trims before rendering, so a reply
/// of three spaces reaches the athlete as nothing at all — and would otherwise
/// slip past a bare `is_empty()` check.
#[test]
fn whitespace_only_content_is_empty() {
    assert!(is_empty_completion(&response("   \n\t  ", None)));
}

/// An empty `tool_calls` vec is the same as none. Providers differ on which
/// they send, and a `Some(vec![])` reading as "has tool calls" would silently
/// restore the original bug for whichever provider serialises it that way.
#[test]
fn an_empty_tool_call_vec_is_still_empty() {
    assert!(is_empty_completion(&response("", Some(vec![]))));
}

/// The critical negative case. A model that calls a tool instead of speaking
/// returns empty content by design; treating that as a failure would fall back
/// on every tool-using turn — the common case, against the paid provider.
#[test]
fn empty_content_with_tool_calls_is_not_empty() {
    assert!(
        !is_empty_completion(&response("", Some(vec![a_tool_call()]))),
        "a tool-call turn is an ordinary mid-loop turn, not a lost one"
    );
}

/// Whitespace content alongside a tool call is still a tool-call turn.
#[test]
fn whitespace_content_with_tool_calls_is_not_empty() {
    assert!(!is_empty_completion(&response(
        "  ",
        Some(vec![a_tool_call()])
    )));
}

/// An ordinary answer is never empty — guards against a classifier that returns
/// true for everything, which would route all traffic to the secondary.
#[test]
fn a_real_reply_is_not_empty() {
    assert!(!is_empty_completion(&response(
        "Tu as couru 42 km ce mois-ci.",
        None
    )));
}
