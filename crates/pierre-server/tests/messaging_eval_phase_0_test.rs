// ABOUTME: Botium plan Phase 0 spike — synthetic activities → fixture → asserter round trip
// ABOUTME: Validates assertion-adapter + fixture-generator approach end-to-end without a full pipeline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Phase 0 of the Botium messaging-eval plan.
//!
//! This test exercises the minimum viable round trip described in the
//! plan's assertion-adapter approach:
//!
//! 1. Seed a tenant's activities with `SyntheticGenerator` (the existing
//!    helpers crate).
//! 2. Derive a [`CitationFixture`] from the seeded activities.
//! 3. Assert that a reply citing the derived numbers is grounded.
//! 4. Assert that a reply citing hallucinated numbers fails the
//!    grounding check.
//!
//! It deliberately skips wiring the full Slack webhook → canot → chat
//! pipeline: that path is already covered by
//! `conversation_turn_e2e_test.rs` and adds no new signal for this
//! spike. Phase 1 layers the full pipeline on top by injecting a
//! mock-LLM reply that pretends to have called the activity tools.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

use helpers::messaging_eval::{
    assert_citation_grounded, assert_guardrail, CitationFixture, CitationMismatch, Tolerances,
};
use helpers::synthetic_data::SyntheticDataBuilder;
use pierre_mcp_server::models::Activity;

/// Seed ten synthetic runs, each exactly 5 km. Makes the expected
/// fixture trivially `{ count: 10, total_distance_km: 50.0 }` and
/// keeps the assertions independent of the synthetic generator's
/// per-run distance heuristics.
fn seed_ten_identical_runs() -> Vec<Activity> {
    let mut builder = SyntheticDataBuilder::new(42);
    (0..10)
        .map(|_| builder.generate_run().distance_km(5.0).build())
        .collect()
}

#[test]
fn fixture_from_synthetic_activities_matches_seeded_totals() {
    let activities = seed_ten_identical_runs();
    let fixture = CitationFixture::from_activities(&activities);

    assert_eq!(fixture.activity_count, 10);
    assert!(
        (fixture.total_distance_km - 50.0).abs() < f64::EPSILON,
        "expected exactly 50 km, got {}",
        fixture.total_distance_km
    );
}

#[test]
fn citation_grounded_for_reply_matching_seeded_fixture() {
    let activities = seed_ten_identical_runs();
    let fixture = CitationFixture::from_activities(&activities);

    // Simulates a coach reply that correctly cites the seeded data.
    let reply = "Over the past block you logged 10 runs totaling 50 km. \
                 Solid base for building 5 K speed.";

    assert!(
        assert_citation_grounded(reply, &fixture, Tolerances::loose()).is_ok(),
        "reply citing fixture numbers should be grounded"
    );
}

#[test]
fn citation_catches_hallucinated_distance_in_seeded_tenant() {
    let activities = seed_ten_identical_runs();
    let fixture = CitationFixture::from_activities(&activities);

    // Simulates a hallucinating coach: right run count, invented volume.
    let reply = "Over the past block you logged 10 runs totaling 120 km.";

    let err = assert_citation_grounded(reply, &fixture, Tolerances::strict())
        .expect_err("120 km ≠ 50 km, must be flagged");
    match err {
        CitationMismatch::DistanceOutOfTolerance {
            cited_km,
            expected_km,
            ..
        } => {
            assert!((cited_km - 120.0).abs() < f64::EPSILON);
            assert!((expected_km - 50.0).abs() < f64::EPSILON);
        }
        CitationMismatch::ActivityCountWrong { cited, expected } => {
            panic!("expected distance mismatch, got ActivityCountWrong(cited={cited}, expected={expected})")
        }
    }
}

#[test]
fn citation_catches_hallucinated_activity_count_in_seeded_tenant() {
    let activities = seed_ten_identical_runs();
    let fixture = CitationFixture::from_activities(&activities);

    // Simulates the "you've done 25 runs this block" class of hallucination.
    let reply = "You've done 25 runs this block. Let's talk intervals.";

    let err = assert_citation_grounded(reply, &fixture, Tolerances::strict())
        .expect_err("25 ≠ 10, must be flagged");
    assert_eq!(
        err,
        CitationMismatch::ActivityCountWrong {
            cited: 25,
            expected: 10,
        }
    );
}

#[test]
fn guardrail_scope_refusal_detected_in_reply() {
    // The scope refusal string comes from
    // `dravr-contremaitre/messaging_strings/{en,fr}/scope_refusal.md`
    // and is emitted by the LLM when it obeys the capability-discipline
    // rules. This test uses the expected canonical English form.
    let reply = "That's outside what I can help with — I'm your fitness assistant.";
    assert!(assert_guardrail(reply, "outside what I can help with").is_ok());
}
