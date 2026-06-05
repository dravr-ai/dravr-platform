// ABOUTME: Verifies the pure prompt-builder and LLM-response parser used by the
// ABOUTME: LLM-powered insight adaptation path (audience keying + fail-open fallback).

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::{InsightType, ShareVisibility, SharedInsight, TrainingPhase};
use pierre_intelligence::insight_adapter::UserTrainingContext;
use pierre_services::social_insights::{build_adaptation_prompt, parse_adaptation_response};
use uuid::Uuid;

const FALLBACK: &str = "From a friend: Ran 10k. For you: keep building your base!";

fn sample_insight() -> SharedInsight {
    let mut insight = SharedInsight::new(
        Uuid::new_v4(),
        InsightType::TrainingTip,
        "Held 4:30/km for a 10k tempo today.".to_owned(),
        ShareVisibility::default(),
    );
    insight.sport_type = Some("Running".to_owned());
    insight
}

#[test]
fn prompt_carries_audience_keying_inputs() {
    let insight = sample_insight();
    let context = UserTrainingContext::default()
        .with_fitness_score(75.0)
        .with_training_phase(TrainingPhase::Build)
        .with_weekly_volume(6.5)
        .with_primary_sport("Running".to_owned())
        .with_training_goal("sub-40 10k".to_owned());

    let prompt = build_adaptation_prompt(&insight, &context, Some("training for a spring race"));

    // Insight type is surfaced (description form).
    assert!(prompt.contains("Training recommendation"));
    // Audience-keying fields the deterministic adapter uses are all present.
    assert!(prompt.contains("Advanced"), "fitness level missing");
    assert!(prompt.contains("build"), "training phase missing");
    assert!(prompt.contains("6.5"), "weekly volume missing");
    assert!(prompt.contains("Running"), "primary sport missing");
    assert!(prompt.contains("sub-40 10k"), "training goal missing");
    assert!(
        prompt.contains("training for a spring race"),
        "additional context missing"
    );
    // Fact-preservation + JSON-only contract is stated.
    assert!(prompt.contains("adapted_content"));
    assert!(prompt.to_lowercase().contains("preserve"));
}

#[test]
fn prompt_omits_unset_optional_fields() {
    let insight = sample_insight();
    let context = UserTrainingContext::default();

    let prompt = build_adaptation_prompt(&insight, &context, None);

    // No fitness score => Unknown level, and no phase/volume/sport/goal lines.
    assert!(prompt.contains("Unknown"));
    assert!(!prompt.contains("training phase:"));
    assert!(!prompt.contains("weekly training volume:"));
    assert!(!prompt.contains("primary sport:"));
    assert!(!prompt.contains("training goal:"));
    assert!(!prompt.contains("Additional context"));
}

#[test]
fn parses_plain_json_response() {
    let raw = r#"{"adapted_content":"Nice tempo! At your level, try matching that 4:30/km."}"#;
    let out = parse_adaptation_response(raw, FALLBACK);
    assert_eq!(out, "Nice tempo! At your level, try matching that 4:30/km.");
}

#[test]
fn parses_json_wrapped_in_markdown_fence() {
    let raw = "Here you go:\n```json\n{\"adapted_content\": \"Adapted line.\"}\n```";
    let out = parse_adaptation_response(raw, FALLBACK);
    assert_eq!(out, "Adapted line.");
}

#[test]
fn empty_adapted_content_falls_back() {
    let raw = r#"{"adapted_content":"   "}"#;
    let out = parse_adaptation_response(raw, FALLBACK);
    assert_eq!(out, FALLBACK, "blank rewrite must fall back, never blank");
}

#[test]
fn unparseable_response_falls_back() {
    let raw = "I'm just chatty prose, no JSON here.";
    let out = parse_adaptation_response(raw, FALLBACK);
    assert_eq!(
        out, FALLBACK,
        "unparseable rewrite must fall back, never blank"
    );
}

#[test]
fn missing_field_falls_back() {
    let raw = r#"{"something_else":"oops"}"#;
    let out = parse_adaptation_response(raw, FALLBACK);
    assert_eq!(out, FALLBACK);
}
