// ABOUTME: Tier 6 text guardrail tests — disclaimer prefix, blocked topics, length caps
// ABOUTME: Pure-Rust, no provider dependencies; lives in tests/ per the project rule
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::config::text_guardrails::{
    GuardrailOutcome, GuardrailRejection, TextGuardrails,
};

fn rules() -> TextGuardrails {
    TextGuardrails {
        max_response_chars: 200,
        blocked_topics: vec!["self-harm".to_owned()],
        disclaimer_triggers: vec!["injury".to_owned(), "pain".to_owned()],
        disclaimer_text: "Disclaimer.".to_owned(),
    }
}

#[test]
fn allowed_when_no_trigger() {
    let r = rules();
    let outcome = r.apply("How was your training week?");
    assert_eq!(
        outcome,
        GuardrailOutcome::Allowed("How was your training week?".to_owned())
    );
}

#[test]
fn disclaimer_prepended_on_injury_keyword() {
    let r = rules();
    let outcome = r.apply("Watch your achilles for any injury signs.");
    if let GuardrailOutcome::Allowed(text) = outcome {
        assert!(text.starts_with("Disclaimer."));
        assert!(text.contains("achilles"));
    } else {
        panic!("expected allowed with disclaimer");
    }
}

#[test]
fn disclaimer_prepended_on_pain_keyword_case_insensitive() {
    let r = rules();
    let outcome = r.apply("If you feel any PAIN, stop the workout.");
    if let GuardrailOutcome::Allowed(text) = outcome {
        assert!(text.starts_with("Disclaimer."));
    } else {
        panic!("expected allowed with disclaimer");
    }
}

#[test]
fn rejected_when_response_exceeds_cap() {
    let r = rules();
    let long = "a".repeat(300);
    match r.apply(&long) {
        GuardrailOutcome::Rejected(GuardrailRejection::TooLong { length, cap }) => {
            assert_eq!(length, 300);
            assert_eq!(cap, 200);
        }
        other => panic!("expected TooLong rejection, got {other:?}"),
    }
}

#[test]
fn rejected_when_response_mentions_blocked_topic() {
    let r = rules();
    match r.apply("Please don't engage in self-harm.") {
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            assert_eq!(topic, "self-harm");
        }
        other => panic!("expected BlockedTopic rejection, got {other:?}"),
    }
}

#[test]
fn empty_response_passes_through_unchanged() {
    let r = rules();
    assert_eq!(r.apply(""), GuardrailOutcome::Allowed(String::new()));
}

#[test]
fn safe_default_includes_medical_keywords() {
    let r = TextGuardrails::safe_default();
    let outcome = r.apply("If your knee pain persists, please see a doctor.");
    if let GuardrailOutcome::Allowed(text) = outcome {
        assert!(text.contains("Medical disclaimer"));
        assert!(text.contains("knee pain"));
    } else {
        panic!("expected allowed with disclaimer prepended");
    }
}

#[test]
fn safe_default_caps_long_responses() {
    let r = TextGuardrails::safe_default();
    let huge = "x".repeat(10_000);
    matches!(
        r.apply(&huge),
        GuardrailOutcome::Rejected(GuardrailRejection::TooLong { .. })
    );
}
