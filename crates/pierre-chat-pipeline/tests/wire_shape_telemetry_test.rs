// ABOUTME: Pins that an assembled prompt presents exactly one System message to the provider
// ABOUTME: The observable half of P1 — system_message_count is logged at the dispatch boundary
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `log_wire_shape` emits `system_message_count` at both dispatch paths so the
//! question "did the block we injected actually reach the model?" is answerable
//! from Cloud Logging instead of by inference.
//!
//! The field matters because `copilot_headless` keeps only the FIRST system
//! message and filters the rest. The platform emitted five for months — the
//! compaction replay, the same-turn splice, the turn-1 activity pre-load, the
//! Stage 12b refresh, and the guardian planner — and four were discarded on
//! every turn with nothing logging the loss.
//!
//! The logging itself is a side effect and not directly assertable here. What
//! IS assertable, and what the operator will compare the logged number against,
//! is the invariant: a prompt built by this pipeline presents exactly one
//! System message, at index 0. `system_message_count > 1` in production means a
//! new emitter appeared and its block is being silently dropped.

use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages;
use pierre_core::models::MessageRecord;
use pierre_llm::MessageRole;

fn row(id: &str, role: &str, content: &str) -> MessageRecord {
    MessageRecord {
        id: id.to_owned(),
        conversation_id: "conv-1".to_owned(),
        role: role.to_owned(),
        content: content.to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: None,
        structured_content: None,
        created_at: "2026-08-03T12:00:00Z".to_owned(),
    }
}

/// Count roles exactly as `log_wire_shape` does, so the assertion and the
/// telemetry cannot drift apart.
fn system_message_count(messages: &[pierre_llm::ChatMessage]) -> usize {
    messages
        .iter()
        .filter(|m| m.role == MessageRole::System)
        .count()
}

#[test]
fn an_assembled_prompt_presents_exactly_one_system_message() {
    let history = vec![
        row("m1", "user", "quel est mon plan?"),
        row("m2", "assistant", "Voici ton bloc de la semaine."),
        row(
            "m3",
            "tool_result",
            "<tool_result>{\"count\":3}</tool_result>",
        ),
        row("m4", "user", "et samedi?"),
    ];

    let (messages, _) = build_llm_messages(Some("coach persona + tool catalogue"), &history);

    assert_eq!(
        system_message_count(&messages),
        1,
        "the prompt must present exactly one System message — every one after \
         the first is silently dropped by the live provider, which is what \
         system_message_count exists to surface"
    );
    assert_eq!(
        messages[0].role,
        MessageRole::System,
        "the single System message must be at index 0"
    );
}

#[test]
fn a_history_with_no_system_prompt_reports_zero() {
    // The counter must be honest in the degenerate direction too: a missing
    // system prompt is its own defect class (the coach ships with no persona),
    // and it should read 0 rather than being masked.
    let history = vec![row("m1", "user", "salut")];

    let (messages, _) = build_llm_messages(None, &history);

    assert_eq!(system_message_count(&messages), 0);
}
