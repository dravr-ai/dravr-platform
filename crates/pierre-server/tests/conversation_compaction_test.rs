// ABOUTME: Unit tests for the Tier 1 conversation compactor public surface
// ABOUTME: Threshold math, message token estimates, and configuration defaults
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::config::CompactionConfig;
use pierre_llm::{ChatMessage, MessageRole};
use pierre_services::conversation_compaction::{
    decide_action, estimate_messages_tokens, non_system_count, sliding_window_to_fit,
    CompactionAction,
};

fn msgs(items: &[(MessageRole, &str)]) -> Vec<ChatMessage> {
    items
        .iter()
        .map(|(role, content)| match role {
            MessageRole::User => ChatMessage::user(*content),
            MessageRole::Assistant => ChatMessage::assistant(*content),
            MessageRole::System | MessageRole::Tool => ChatMessage::system(*content),
        })
        .collect()
}

#[test]
fn decide_action_routes_over_message_thread_to_summarize_not_slide() {
    let cfg = CompactionConfig::default();
    let (warn, emerg) = (cfg.warn_tokens(), cfg.emergency_tokens());
    // Over the message cap but token-wise under the emergency cliff: summarize
    // (preserve context as a durable block), NOT slide. This is the path a long
    // under-token thread hits — the change that stops it raw-dropping history.
    assert_eq!(
        decide_action(warn / 2, true, warn, emerg),
        CompactionAction::Summarize
    );
}

#[test]
fn decide_action_hard_token_cliff_always_slides() {
    let cfg = CompactionConfig::default();
    let (warn, emerg) = (cfg.warn_tokens(), cfg.emergency_tokens());
    // At/over the emergency token cliff: slide, even when also over the message
    // cap — no budget for a summarizer call before the window would overflow.
    assert_eq!(
        decide_action(emerg, false, warn, emerg),
        CompactionAction::Slide
    );
    assert_eq!(
        decide_action(emerg, true, warn, emerg),
        CompactionAction::Slide
    );
    assert_eq!(
        decide_action(emerg + 1, false, warn, emerg),
        CompactionAction::Slide
    );
}

#[test]
fn decide_action_warn_band_summarizes_below_warn_is_noop() {
    let cfg = CompactionConfig::default();
    let (warn, emerg) = (cfg.warn_tokens(), cfg.emergency_tokens());
    // Warn band (warn <= before < emergency), within the message cap: summarize.
    assert_eq!(
        decide_action(warn, false, warn, emerg),
        CompactionAction::Summarize
    );
    // Below warn and within the message cap: no-op.
    assert_eq!(
        decide_action(warn - 1, false, warn, emerg),
        CompactionAction::NoOp
    );
}

#[test]
fn default_config_matches_gist_plan() {
    let cfg = CompactionConfig::default();
    assert_eq!(cfg.window_tokens, 128_000);
    assert!((cfg.warn_threshold - 0.70).abs() < f32::EPSILON);
    assert!((cfg.emergency_threshold - 0.95).abs() < f32::EPSILON);
}

#[test]
fn warn_and_emergency_threshold_math() {
    let cfg = CompactionConfig::default();
    // 128_000 * 0.70 = 89_600, 128_000 * 0.95 = 121_600
    assert_eq!(cfg.warn_tokens(), 89_600);
    assert_eq!(cfg.emergency_tokens(), 121_600);
    assert!(cfg.emergency_tokens() > cfg.warn_tokens());
}

#[test]
fn emergency_always_strictly_above_warn() {
    let cfg = CompactionConfig {
        window_tokens: 4_000,
        warn_threshold: 0.5,
        emergency_threshold: 0.9,
        ..CompactionConfig::default()
    };
    assert_eq!(cfg.warn_tokens(), 2_000);
    assert_eq!(cfg.emergency_tokens(), 3_600);
}

#[test]
fn estimate_messages_tokens_is_content_sum() {
    // Each content string contributes floor(len / 4) tokens.
    // "aaaa" = 1, "bbbbbbbb" = 2, "cccccccccccc" = 3 → total 6
    let v = msgs(&[
        (MessageRole::System, "aaaa"),
        (MessageRole::User, "bbbbbbbb"),
        (MessageRole::Assistant, "cccccccccccc"),
    ]);
    assert_eq!(estimate_messages_tokens(&v), 6);
}

#[test]
fn estimate_messages_tokens_empty_list_is_zero() {
    assert_eq!(estimate_messages_tokens(&[]), 0);
}

#[test]
fn default_caps_messages_at_forty() {
    assert_eq!(CompactionConfig::default().max_messages, 40);
}

#[test]
fn sliding_window_bounds_by_message_cap_even_at_zero_token_estimate() {
    // 100 two-char turns: each estimates to floor(2/4)=0 tokens, so the whole
    // thread reads as 0 estimated tokens — it would slip through a token-only
    // trigger (the exact bug that let a 168-message thread reach the provider).
    // The message cap must bound it regardless of the under-counting estimate.
    let cfg = CompactionConfig {
        max_messages: 40,
        ..CompactionConfig::default()
    };
    let mut v: Vec<ChatMessage> = (0..100)
        .map(|i| {
            if i % 2 == 0 {
                ChatMessage::user("hi")
            } else {
                ChatMessage::assistant("ok")
            }
        })
        .collect();
    assert_eq!(
        estimate_messages_tokens(&v),
        0,
        "precondition: estimate is 0"
    );

    let dropped = sliding_window_to_fit(&mut v, &cfg);

    assert_eq!(non_system_count(&v), 40, "bounded to the message cap");
    assert_eq!(dropped, 60);
    // The most recent turn is preserved (index 99 was an assistant turn).
    assert_eq!(v.last().unwrap().content, "ok");
}

#[test]
fn sliding_window_preserves_system_prompt_and_recent_pair() {
    let cfg = CompactionConfig {
        max_messages: 2,
        ..CompactionConfig::default()
    };
    let mut v = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("a"),
        ChatMessage::assistant("b"),
        ChatMessage::user("c"),
        ChatMessage::assistant("d"),
    ];

    let dropped = sliding_window_to_fit(&mut v, &cfg);

    assert_eq!(v[0].content, "sys", "system prompt at index 0 is preserved");
    assert_eq!(non_system_count(&v), 2, "keeps only the recent pair");
    assert_eq!(v.last().unwrap().content, "d");
    assert_eq!(dropped, 2);
}

#[test]
fn sliding_window_noop_when_already_within_bounds() {
    let cfg = CompactionConfig::default();
    let mut v = vec![ChatMessage::user("a"), ChatMessage::assistant("b")];
    let dropped = sliding_window_to_fit(&mut v, &cfg);
    assert_eq!(dropped, 0, "a short thread is left untouched");
    assert_eq!(v.len(), 2);
}
