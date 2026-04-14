// ABOUTME: Unit tests for the Tier 1 conversation compactor public surface
// ABOUTME: Threshold math, message token estimates, and configuration defaults
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_llm::{ChatMessage, MessageRole};
use pierre_mcp_server::services::conversation_compaction::{
    estimate_messages_tokens, CompactionConfig,
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
