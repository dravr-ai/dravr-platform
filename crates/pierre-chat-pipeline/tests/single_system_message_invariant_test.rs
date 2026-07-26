// ABOUTME: Pins the one-system-message invariant every injected prompt block depends on
// ABOUTME: Regression for the silent System-message drop on the copilot_headless wire (2026-07-26)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! **The platform emits exactly one `System` message, at index 0. Every other
//! injected context block rides in a `User` message, in position.**
//!
//! Why this file exists. The live provider is `copilot_headless`, and embacle's
//! `build_prompt_blocks` keeps only the FIRST system message as the prompt and
//! filters every other `System` message out of history. The platform was
//! emitting five: the compaction replay, the same-turn compaction splice, the
//! turn-1 activity pre-load, the Stage 12b activity refresh, and the guardian
//! planner prompt. Four of them were discarded on every turn, silently, for
//! months.
//!
//! The activity injections are the expensive ones: the text that never arrived
//! reads *"base your analysis and any plan on these specific activities, cite
//! them by name and date … do not answer from memory"* — i.e. the entire
//! deterministic grounding mechanism. The loss was invisible because the coach
//! can still call `get_activities` on its own initiative, which produces the
//! same observable result. `c1ffca625` shipped Stage 12b as "validated live on
//! Telegram" against the exact embacle rev that drops it; the feature was
//! confirmed working by watching an outcome that two different mechanisms
//! produce, and the older sibling test in `prefetch_grounding_test.rs`
//! *asserted the System role as correct*.
//!
//! So these tests deliberately check the wire shape — role and position — not
//! whether a reply looks good. A behavioural assertion cannot tell the two
//! mechanisms apart; this can.

use pierre_chat_pipeline::stages::prefetch::inject_activity_refresh;
use pierre_llm::{ChatMessage, MessageRole};
use pierre_services::conversation_compaction::REPLAYED_SUMMARY_PREFIX;

/// The invariant itself, as a reusable assertion.
fn assert_single_leading_system(messages: &[ChatMessage], context: &str) {
    let system_positions: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == MessageRole::System)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        system_positions,
        vec![0],
        "{context}: expected exactly one System message at index 0, found them at {system_positions:?} \
         — every System message after the first is silently dropped by the live provider"
    );
}

const WINDOW: &str = r#"{"count":1,"activity_list":"1. [Ride] Gravel du matin - 2026-07-25 - 62.40 km","activities":[]}"#;

#[test]
fn activity_refresh_preserves_the_invariant_and_survives_as_user_text() {
    let mut messages = vec![
        ChatMessage::system("coach persona + tool catalogue"),
        ChatMessage::user("what should I ride this weekend?"),
    ];

    assert!(inject_activity_refresh(&mut messages, WINDOW));

    assert_single_leading_system(&messages, "after Stage 12b refresh");
    // The grounding block must actually be on the wire, carrying both the
    // instruction and the data — this is the assertion whose absence let the
    // drop survive for months.
    let grounding = messages
        .iter()
        .find(|m| m.content.contains("Gravel du matin"))
        .expect("the injected activity data must still be present in the message vector");
    assert_eq!(
        grounding.role,
        MessageRole::User,
        "grounding must ride as User text or the provider discards it"
    );
    assert!(
        grounding.content.contains("do not answer from memory"),
        "the grounding instruction must travel with the data, not just the data"
    );
}

#[test]
fn activity_refresh_never_displaces_the_system_prompt_from_index_zero() {
    // Degenerate vector: system prompt only. `len - 1` would be index 0, which
    // would push the system prompt to index 1 — where `non_system_count` and
    // `sliding_window_to_fit` (both of which infer the prompt from
    // `messages.first()`) would treat it as ordinary droppable history.
    let mut messages = vec![ChatMessage::system("coach persona")];

    assert!(inject_activity_refresh(&mut messages, WINDOW));

    assert_single_leading_system(&messages, "degenerate single-message vector");
    assert_eq!(
        messages[0].content, "coach persona",
        "the system prompt must stay at index 0"
    );
}

#[test]
fn an_empty_window_injects_nothing_and_leaves_the_shape_untouched() {
    let mut messages = vec![
        ChatMessage::system("coach persona"),
        ChatMessage::user("plan my week"),
    ];

    let injected = inject_activity_refresh(&mut messages, r#"{"count":0,"activities":[]}"#);

    assert!(!injected, "an empty window must not be injected");
    assert_eq!(messages.len(), 2, "no message added");
    assert_single_leading_system(&messages, "empty window");
}

#[test]
fn compaction_summary_framing_is_shared_and_says_it_is_recovered_history() {
    // Both the same-turn splice (conversation_compaction::apply_summary) and the
    // later-turn replay (prompt_builder::build_llm_messages) render a summary
    // with this prefix, so one summary reads identically on the turn it is born
    // and on every turn after it. It must not carry the UI-only
    // COMPACTION_MARKER, which would reach the model as render scaffolding.
    assert!(
        REPLAYED_SUMMARY_PREFIX.contains("Earlier conversation summary"),
        "the wire prefix must name what the text is"
    );
    assert!(
        REPLAYED_SUMMARY_PREFIX.contains("not a new message from the athlete"),
        "a User-role summary must disclaim that the athlete said it"
    );
    assert!(
        !REPLAYED_SUMMARY_PREFIX.contains("[pierre:compaction]"),
        "the UI render marker must never travel on the wire"
    );
}
