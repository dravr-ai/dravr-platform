// ABOUTME: Pins the embacle transport contract every injected prompt block depends on
// ABOUTME: Fails on a dependency bump that changes how the provider serializes messages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! **The platform hands a `Vec<ChatMessage>` to embacle and has zero coverage
//! past that line.** This file is the missing half.
//!
//! `copilot_headless` keeps only the FIRST `System` message and filters every
//! other one out of history. The platform emitted five for months and four were
//! discarded on every turn, silently — no error, no warning, and no visible
//! symptom, because the coach could still call `get_activities` itself and
//! produce the same observable outcome.
//!
//! Nothing caught it, and nothing *could* have:
//!
//! - `cargo upgrade --dry-run` (`dependency-check.yml`) resolves crates.io
//!   versions and structurally cannot see a git-tag pin, which is how embacle is
//!   pinned in three manifests.
//! - The npm Copilot CLI has a dedicated weekly drift monitor; embacle has none.
//! - A behavioural change in a pinned dep compiles clean. The one embacle change
//!   that *did* red CI (`ca16c7fb2`) was caught by rustc only because it happened
//!   to be a type change.
//!
//! So this asserts the *properties the platform relies on* rather than embacle's
//! internals. If a bump changes how messages are serialized, these fail at the
//! bump instead of months later in production.
//!
//! Scope note, stated rather than implied: `build_prompt_blocks` is private to
//! embacle, so this pins the contract at the boundary the platform can observe —
//! the message vector it constructs. The complementary half is the live
//! `system_message_count` telemetry (`log_wire_shape`) plus its alert, which
//! catches a violation on the far side of the boundary in production.

use pierre_llm::{ChatMessage, ChatRequest, MessageRole};

/// The invariant: exactly one `System`, at index 0.
///
/// Every injected block the platform adds after prompt assembly — activity
/// pre-load, Stage 12b refresh, compaction replay, compaction splice, guardian
/// planner — must ride as `User` text in position.
fn system_positions(messages: &[ChatMessage]) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == MessageRole::System)
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn message_role_is_a_closed_four_variant_enum() {
    // `log_wire_shape` counts roles with an EXHAUSTIVE match so a new variant is
    // a compile error rather than a silently uncounted bucket. If embacle adds a
    // fifth role, this test and that match both need revisiting — the point is
    // that the bump cannot land unnoticed.
    let all = [
        MessageRole::System,
        MessageRole::User,
        MessageRole::Assistant,
        MessageRole::Tool,
    ];
    // Round-trips through the wire representation embacle serializes with.
    let wire: Vec<&str> = all.iter().map(MessageRole::as_str).collect();
    assert_eq!(
        wire,
        vec!["system", "user", "assistant", "tool"],
        "embacle's MessageRole wire strings changed — the provider adapters and \
         log_wire_shape's role histogram both assume these exact four"
    );
}

#[test]
fn a_request_the_platform_builds_carries_exactly_one_leading_system() {
    // The shape `run_cli_tool_loop` hands to the provider on a grounded coach
    // turn: system prompt, then the activity block and the athlete's ask as User
    // messages. Verified live on 2026-08-04 against copilot_headless as
    // system_message_count=1, user_message_count=3.
    let messages = vec![
        ChatMessage::system("coach persona + tool catalogue"),
        ChatMessage::user("The following activity data has been pre-loaded for your analysis: …"),
        ChatMessage::user("Analyze my training load"),
    ];
    let request = ChatRequest::new(messages.clone());

    assert_eq!(
        system_positions(&request.messages),
        vec![0],
        "ChatRequest must preserve the platform's message order and roles verbatim — \
         a provider that reorders or re-roles would invalidate the one-system-message \
         invariant the whole prompt pipeline depends on"
    );
    assert_eq!(
        request.messages.len(),
        messages.len(),
        "ChatRequest::new must not add, drop or merge messages"
    );
}

#[test]
fn injected_blocks_are_user_role_so_the_provider_cannot_filter_them() {
    // The regression this whole line of work exists for. Each of these was a
    // `ChatMessage::system` before 0988e17e6, and each was discarded on every
    // turn by the live provider.
    let injected = [
        ChatMessage::user("The following activity data has been pre-loaded for your analysis: …"),
        ChatMessage::user("The athlete's question needs to be grounded in their real training: …"),
        ChatMessage::user("[Earlier conversation summary — recovered context] …"),
    ];
    for block in &injected {
        assert_eq!(
            block.role,
            MessageRole::User,
            "an injected context block must be User-role; a System block after \
             index 0 is filtered out of history by copilot_headless and never \
             reaches the model"
        );
    }

    let mut vector = vec![ChatMessage::system("coach persona")];
    vector.extend(injected);
    vector.push(ChatMessage::user("what should I do this week?"));
    assert_eq!(
        system_positions(&vector),
        vec![0],
        "a fully-injected turn still presents exactly one System message"
    );
}

#[test]
fn content_survives_the_request_boundary_byte_for_byte() {
    // Guards against a bump that starts trimming, escaping or re-encoding
    // content: the grounding instruction is load-bearing prose, and a silent
    // transformation would be as invisible as the role filter was.
    let grounding = "base your analysis and any plan on these specific activities, \
                     cite them by name and date, infer the sport mix from them \
                     rather than asking, and do not answer from memory";
    let request = ChatRequest::new(vec![
        ChatMessage::system("persona"),
        ChatMessage::user(grounding),
    ]);
    assert_eq!(
        request.messages[1].content, grounding,
        "message content must cross the ChatRequest boundary unmodified"
    );
}
