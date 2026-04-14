// ABOUTME: Character-based LLM token estimation shared across crates
// ABOUTME: Single source of truth for the ~4 chars/token heuristic used as a fallback
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Token Estimation
//!
//! Character-based LLM token estimation used as a fallback when providers
//! don't return real token counts (e.g., CLI runners, headless providers,
//! stored system prompts). Most English text averages ~4 characters per
//! token across major tokenizers (GPT, Gemini, LLaMA), making this a
//! reasonable heuristic for budgeting and usage tracking.
//!
//! This is the single source of truth for the heuristic — do not
//! reintroduce `CHARS_PER_TOKEN` in downstream crates.

/// Average characters per token for LLM tokenizers.
pub const CHARS_PER_TOKEN: usize = 4;

/// Estimate token count for a single text span.
///
/// Divides character length by [`CHARS_PER_TOKEN`]. Used when sizing a
/// stored prompt, a compaction block, or any isolated text fragment.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub const fn estimate_prompt_tokens(text: &str) -> u32 {
    (text.len() / CHARS_PER_TOKEN) as u32
}

/// Estimate `(prompt, completion)` token counts for a chat turn.
///
/// Each side is floored to at least 1 token so that cost math never
/// divides by zero on empty strings. Used when recording usage for
/// providers that don't return real token counts.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub const fn estimate_chat_tokens(prompt_text: &str, completion_text: &str) -> (u32, u32) {
    let prompt = {
        let raw = prompt_text.len() / CHARS_PER_TOKEN;
        if raw == 0 {
            1
        } else {
            raw as u32
        }
    };
    let completion = {
        let raw = completion_text.len() / CHARS_PER_TOKEN;
        if raw == 0 {
            1
        } else {
            raw as u32
        }
    };
    (prompt, completion)
}

#[cfg(test)]
mod tests {
    use super::{estimate_chat_tokens, estimate_prompt_tokens, CHARS_PER_TOKEN};

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
}
