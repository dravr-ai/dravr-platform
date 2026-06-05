// ABOUTME: Verifies insight content-validation fails closed on unparseable or
// ABOUTME: unknown LLM verdicts so unvalidated content is never shared.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_routes_social::SocialRoutes;

const ORIGINAL: &str = "Ran 10k at threshold today.";

#[test]
fn valid_verdict_passes_original_through() {
    let raw = r#"{"verdict":"valid"}"#;
    let out = SocialRoutes::parse_validation_json_response(raw, ORIGINAL).unwrap();
    assert_eq!(out, ORIGINAL);
}

#[test]
fn improved_verdict_returns_improved_content() {
    let raw = r#"{"verdict":"improved","improved_content":"Ran a 10k at lactate threshold."}"#;
    let out = SocialRoutes::parse_validation_json_response(raw, ORIGINAL).unwrap();
    assert_eq!(out, "Ran a 10k at lactate threshold.");
}

#[test]
fn rejected_verdict_is_an_error() {
    let raw = r#"{"verdict":"rejected","reason":"no data"}"#;
    assert!(SocialRoutes::parse_validation_json_response(raw, ORIGINAL).is_err());
}

#[test]
fn unparseable_response_fails_closed() {
    // Malformed LLM output must NOT be accepted — the gate did not run.
    let raw = "I'm not JSON at all, just chatty prose.";
    assert!(
        SocialRoutes::parse_validation_json_response(raw, ORIGINAL).is_err(),
        "unparseable validator output must be rejected, not shared"
    );
}

#[test]
fn unknown_verdict_fails_closed() {
    let raw = r#"{"verdict":"maybe_ok"}"#;
    assert!(
        SocialRoutes::parse_validation_json_response(raw, ORIGINAL).is_err(),
        "unrecognized verdict must be rejected, not shared"
    );
}
