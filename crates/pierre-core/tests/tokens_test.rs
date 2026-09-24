// ABOUTME: Tests for token estimation
// ABOUTME: Chars-per-token ratio, prompt estimates and the chat floor

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::tokens::{estimate_chat_tokens, estimate_prompt_tokens, CHARS_PER_TOKEN};

#[test]
fn chars_per_token_is_four() {
    assert_eq!(CHARS_PER_TOKEN, 4);
}

#[test]
fn prompt_tokens_divides_by_four() {
    assert_eq!(estimate_prompt_tokens(""), 0);
    assert_eq!(estimate_prompt_tokens("abc"), 0);
    assert_eq!(estimate_prompt_tokens("abcd"), 1);
    assert_eq!(estimate_prompt_tokens("hello world!"), 3);
}

#[test]
fn chat_tokens_floor_at_one() {
    assert_eq!(estimate_chat_tokens("", ""), (1, 1));
    assert_eq!(estimate_chat_tokens("abc", "xy"), (1, 1));
    assert_eq!(estimate_chat_tokens("hello world!", "ok"), (3, 1));
}

#[test]
fn chat_tokens_scale_with_length() {
    let prompt = "a".repeat(400);
    let completion = "b".repeat(200);
    assert_eq!(estimate_chat_tokens(&prompt, &completion), (100, 50));
}
