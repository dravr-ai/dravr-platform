// ABOUTME: A leg that failed still sent its prompt, so its usage row must carry that prompt's cost
// ABOUTME: Pins recorded_token_counts across reported, both-sided, prompt-only and blind calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every error path in the tool loop records an `LlmCallRecord` so the failure
//! is visible. Until now each of them handed the recorder no text, and the
//! estimator answered `(0, 0, false)` for anything that was not a completed
//! call — so a timeout, a crash, an empty completion and a retryable error all
//! wrote a row priced at nothing.
//!
//! That is the wrong direction to be wrong in. The prompt is assembled, sent
//! and paid for *before* the provider fails; a headless ACP turn that dispatches
//! its tools and then times out during synthesis has pushed the whole 40-55K
//! token prefix upstream by the time the error arrives. Recording zero makes
//! COGS look best exactly when the system is behaving worst.
//!
//! The completion side stays zero, because it genuinely is absent. Only the
//! prompt is estimated, and the row is marked estimated so billing can see the
//! count did not come from the provider.

// `llm_call_record` is gated behind client-chat; without it there is nothing to test.
#![cfg(feature = "client-chat")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use pierre_core::llm::TokenUsage;
use pierre_tool_runtime::llm_call_record::{recorded_token_counts, LlmCallRecord};

/// A realistic assembled prefix: system prompt + tool surface + history.
/// `48_000` chars is the middle of the 40-55K-token band the ACP path re-sends
/// on every native call.
fn realistic_prefix() -> String {
    "a".repeat(48_000)
}

#[test]
fn a_failed_leg_is_charged_for_the_prompt_it_sent() {
    let prompt = realistic_prefix();
    let (prompt_tokens, completion_tokens, estimated) =
        recorded_token_counts(None, Some(&prompt), None);

    assert_eq!(
        prompt_tokens, 12_000,
        "48_000 chars at 4 chars/token is 12_000 prompt tokens — the abandoned leg pushed \
         every one of them upstream before the provider failed"
    );
    assert_eq!(
        completion_tokens, 0,
        "the completion is genuinely absent on a failed leg; inventing one would be worse \
         than the zero it replaces"
    );
    assert!(
        estimated,
        "the count came from character length, not from the provider, so the row must say so"
    );
}

#[test]
fn a_completed_call_estimates_both_sides() {
    let (prompt_tokens, completion_tokens, estimated) =
        recorded_token_counts(None, Some(&"a".repeat(4_000)), Some(&"b".repeat(400)));

    assert_eq!(prompt_tokens, 1_000);
    assert_eq!(completion_tokens, 100);
    assert!(estimated);
}

#[test]
fn a_provider_that_reports_usage_is_believed_outright() {
    let usage = TokenUsage::new(31, 7, 38);
    let (prompt_tokens, completion_tokens, estimated) =
        recorded_token_counts(Some(&usage), Some(&realistic_prefix()), None);

    assert_eq!(
        (prompt_tokens, completion_tokens),
        (31, 7),
        "a reported count outranks any estimate, even a much larger one"
    );
    assert!(
        !estimated,
        "the provider measured these, so the row must not be flagged as estimated"
    );
}

#[test]
fn a_call_with_no_text_and_no_usage_stays_an_honest_zero() {
    let (prompt_tokens, completion_tokens, estimated) = recorded_token_counts(None, None, None);

    assert_eq!((prompt_tokens, completion_tokens), (0, 0));
    assert!(
        !estimated,
        "nothing was measured and nothing was estimated; marking this estimated would claim \
         a count that was never derived"
    );
}

/// A record shaped like a real call, with the counts left to each test.
fn record(prompt: i64, completion: i64, reasoning: i64, estimated: bool) -> LlmCallRecord {
    LlmCallRecord {
        provider: "copilot_headless".to_owned(),
        model: "claude-sonnet-5".to_owned(),
        prompt_tokens: prompt,
        completion_tokens: completion,
        cached_tokens: 0,
        cached_write_tokens: 0,
        reasoning_tokens: reasoning,
        latency_ms: 1200,
        success: true,
        call_sequence: Some(1),
        token_counts_estimated: estimated,
        tools_called: Vec::new(),
    }
}

#[test]
fn the_historical_shape_is_recognised_as_unaccounted() {
    assert!(
        record(0, 0, 0, false).is_unaccounted(),
        "all counts zero with no estimated marker is exactly the row written 163 times \
         between March and June 2026 — priced at zero and indistinguishable downstream \
         from a measured zero"
    );
}

#[test]
fn a_failed_leg_carrying_its_prompt_is_accounted() {
    assert!(
        !record(12_000, 0, 0, true).is_unaccounted(),
        "the error paths now estimate the prompt they sent; that is a measurement, not a gap"
    );
}

#[test]
fn a_provider_reported_call_is_accounted() {
    assert!(!record(31, 7, 0, false).is_unaccounted());
}

#[test]
fn a_reasoning_only_call_is_accounted() {
    assert!(
        !record(0, 0, 44, false).is_unaccounted(),
        "reasoning tokens are billed at the output rate, so a row carrying only them still \
         accounts for real spend"
    );
}

#[test]
fn an_estimated_zero_is_not_flagged() {
    assert!(
        !record(0, 0, 0, true).is_unaccounted(),
        "an estimator that measured an empty prompt reported a value; the marker says the \
         count was derived, and that is the distinction the flag turns on"
    );
}
