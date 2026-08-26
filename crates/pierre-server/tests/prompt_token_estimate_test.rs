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

use pierre_core::config::CompactionConfig;
use pierre_core::tokens::{estimate_chat_tokens, estimate_context_tokens, join_prompt_text};
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

// ============================================================================
// Context budgeting — a different question from usage accounting.
//
// `estimate_chat_tokens` above answers "how big was this turn?" for billing,
// where a symmetric error is acceptable. The compactor asks "may this still be
// sent?", and there under-counting is the dangerous direction: it lets an
// oversized prompt through and the model loses instructions in the middle of
// it. These pin the density-aware estimator that answers the second question.
// ============================================================================

/// Prose is unchanged, so moving the compactor onto the new estimator does not
/// start trimming ordinary conversations early.
#[test]
fn prose_still_estimates_at_four_chars_per_token() {
    let prose = "the athlete ran ten kilometres on sunday morning at an easy pace ".repeat(200);
    let estimate = estimate_context_tokens(&prose);
    let flat = u32::try_from(prose.len() / 4).unwrap();
    let delta = estimate.abs_diff(flat);
    assert!(
        delta * 20 <= flat,
        "prose drifted more than 5% from the flat heuristic: {estimate} vs {flat}"
    );
}

/// Dense JSON costs about twice what the flat heuristic charges it.
#[test]
fn dense_json_estimates_about_twice_the_flat_heuristic() {
    let json =
        r#"{"id":"a1","sport_type":"run","distance_meters":12345,"moving_time":3600},"#.repeat(500);
    let estimate = estimate_context_tokens(&json);
    let flat = u32::try_from(json.len() / 4).unwrap();
    assert!(
        estimate > flat + flat / 2,
        "dense JSON should cost far more than the flat heuristic: {estimate} vs {flat}"
    );
    let dense = u32::try_from(json.len() / 2).unwrap();
    assert!(
        estimate.abs_diff(dense) * 10 <= dense,
        "dense JSON should land near 2 chars/token: {estimate} vs {dense}"
    );
}

/// The production failure, replayed.
///
/// Through August 2026 the messaging pipeline sent prompts averaging 161k real
/// tokens and peaking near 600k, against a compactor configured for a 128k
/// window. It never fired: the prompts are dominated by JSON tool results, the
/// flat 4-chars/token heuristic charged them half price, and a 161k-token
/// prompt read as ~80k — just under the 89.6k warn threshold. The message cap
/// was the only thing bounding the prompt, and 40 dense messages is roughly the
/// 161k that was observed.
#[test]
fn a_prod_sized_json_prompt_now_trips_the_emergency_threshold() {
    let cfg = CompactionConfig::default();
    // ~322k characters of activity JSON — the shape that averaged 161k real
    // tokens in production, at roughly two characters per token.
    let payload =
        r#"{"id":"act-0001","name":"Sortie longue","sport_type":"ride","distance_meters":200000,"#
            .repeat(3_800);

    let flat = u32::try_from(payload.len() / 4).unwrap();
    assert!(
        flat < cfg.warn_tokens(),
        "the flat heuristic is supposed to under-read this prompt — that is the bug \
         being pinned ({flat} should be under {})",
        cfg.warn_tokens()
    );

    let estimate = estimate_context_tokens(&payload);
    assert!(
        estimate >= cfg.emergency_tokens(),
        "a prod-sized JSON prompt must trip the emergency threshold: {estimate} < {}",
        cfg.emergency_tokens()
    );
}

/// Denser content never costs less per character.
#[test]
fn density_never_lowers_the_token_estimate() {
    let len = 4_000;
    let prose = "a".repeat(len);
    let mixed = r#"{"a":1}"#.repeat(len / 7);
    let json = r#"{"a":"b","c":"d","e":"f","g":"h","i":"j","k":"l","m":"n","o":"p","q":"r","#
        .repeat(len / 72);

    let p = estimate_context_tokens(&prose);
    let m = estimate_context_tokens(&mixed);
    let j = estimate_context_tokens(&json);
    assert!(
        p <= m && m <= j,
        "estimate must not fall as content gets denser: prose={p} mixed={m} json={j}"
    );
}

#[test]
fn empty_text_costs_nothing() {
    assert_eq!(estimate_context_tokens(""), 0);
}
