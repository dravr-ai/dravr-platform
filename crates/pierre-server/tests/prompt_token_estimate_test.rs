// ABOUTME: Pins that the fallback token estimator sizes the WHOLE prompt
// ABOUTME: Regression cover for *_estimated llm_usage rows reading near-zero
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression cover for the fallback prompt-token estimator.
//!
//! Copilot ACP returns no usage, so every `*_estimated` row in `llm_usage`
//! comes from `estimate_chat_tokens`. What that estimator is handed decides
//! whether the billing tables describe the turn or a rounding error of it.

use std::iter::empty;

use pierre_core::tokens::{estimate_chat_tokens, join_prompt_text};
use pierre_llm::ChatMessage;

/// The newest user message is a rounding error against the prompt it rides in.
///
/// Copilot ACP returns `usage: None`, so these rows are what billing sees. When
/// the estimator was fed only the last user message, a turn carrying an 80k-char
/// system prompt and 40k of replayed history recorded ~2 prompt tokens.
#[test]
fn test_estimate_sizes_the_whole_prompt_not_the_last_user_message() {
    let system = "S".repeat(80_000);
    let history = "H".repeat(40_000);
    let latest = "et en 2022?";

    let messages = [
        ChatMessage::system(&system),
        ChatMessage::user(&history),
        ChatMessage::user(latest),
    ];

    let joined = join_prompt_text(messages.iter().map(|m| m.content.as_str()));
    // Every message, plus the newline each one is terminated with.
    assert_eq!(joined.len(), 80_000 + 40_000 + latest.len() + 3);

    let (whole_prompt_tokens, _) = estimate_chat_tokens(&joined, "ok");
    // 120_011 chars / 4 chars-per-token.
    assert_eq!(whole_prompt_tokens, 30_003);

    // What the old code recorded for the very same turn.
    let (last_message_only, _) = estimate_chat_tokens(latest, "ok");
    assert_eq!(last_message_only, 2);
    assert!(
        whole_prompt_tokens / last_message_only > 10_000,
        "estimator must not under-count the prompt by orders of magnitude: \
         whole={whole_prompt_tokens} last_only={last_message_only}"
    );
}

/// An empty message list must not panic or claim a zero-token prompt.
#[test]
fn test_estimate_handles_an_empty_message_list() {
    let joined = join_prompt_text(empty());
    assert_eq!(joined, "");
    let (prompt_tokens, _) = estimate_chat_tokens(&joined, "ok");
    assert_eq!(prompt_tokens, 1, "estimator floors an empty prompt at 1");
}
