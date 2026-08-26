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

/// Characters per token for dense structured content.
///
/// JSON tool results tokenize far denser than prose: `{"distance_m":12345,`
/// splits on almost every punctuation mark, so braces, quotes, colons and
/// commas each cost a token while contributing one character. Measured across
/// the activity payloads this pipeline actually sends, dense spans land near
/// two characters per token — half of [`CHARS_PER_TOKEN`].
pub const CHARS_PER_TOKEN_DENSE: usize = 2;

/// Fraction of structural punctuation at which content is treated as fully
/// dense. Well-formed JSON runs about 15% structural characters; English prose
/// with normal punctuation stays under 3%.
const FULLY_DENSE_STRUCTURAL_RATIO: f64 = 0.15;

/// Estimate tokens for **context budgeting**, where under-counting is the
/// dangerous direction.
///
/// [`estimate_prompt_tokens`] answers "roughly how many tokens was this?" for
/// usage and cost records, where a symmetric error is fine. Budgeting asks a
/// different question — "may this still be sent?" — and there the two errors
/// are not symmetric. Over-estimating compacts a thread slightly early; under-
/// estimating lets an oversized prompt through, and the model silently loses
/// instructions in the middle of it.
///
/// The flat 4 chars/token heuristic under-counts JSON-heavy prompts by about
/// half, which is how a compactor configured for a 128k window let 161k-token
/// prompts through in production (2026-08, averaged over a week) with peaks
/// near 600k: the real prompt estimated at ~80k and sat just under the 89.6k
/// warn threshold, so compaction never ran.
///
/// Scales between the two ratios on how structural the text is, rather than
/// picking one, so a long prose conversation is not compacted early to protect
/// against JSON it does not contain.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn estimate_context_tokens(text: &str) -> u32 {
    let len = text.len();
    if len == 0 {
        return 0;
    }
    let structural = text
        .bytes()
        .filter(|b| matches!(b, b'{' | b'}' | b'[' | b']' | b'"' | b':' | b','))
        .count();
    let ratio = (structural as f64 / len as f64).min(FULLY_DENSE_STRUCTURAL_RATIO);
    let density = ratio / FULLY_DENSE_STRUCTURAL_RATIO;
    // Linear between prose and dense; `density` is already clamped to 0..=1.
    let chars_per_token =
        (CHARS_PER_TOKEN_DENSE as f64).mul_add(density, CHARS_PER_TOKEN as f64 * (1.0 - density));
    // chars_per_token is bounded below by CHARS_PER_TOKEN_DENSE (2.0), so this
    // never divides by zero.
    (len as f64 / chars_per_token) as u32
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

/// Join every part of an outgoing prompt into one string for the estimator.
///
/// `prompt_tokens` means the whole prompt: system prompt, replayed history,
/// injected grounding, all of it. Sizing it from the newest user message alone
/// under-counts by orders of magnitude — an athlete's "et en 2022?" is ~70
/// characters against a prompt in the hundreds of thousands, and the resulting
/// `*_estimated` usage rows then read as near-zero.
#[must_use]
pub fn join_prompt_text<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut joined = String::new();
    for part in parts {
        joined.push_str(part);
        joined.push('\n');
    }
    joined
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
