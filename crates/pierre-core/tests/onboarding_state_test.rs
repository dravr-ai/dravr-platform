// ABOUTME: Tests for OnboardingState and CoverageMap
// ABOUTME: Topic coverage, probe limits, visibility and the load snapshot round-trip

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::onboarding::{
    CoverageMap, CoverageTarget, GuidedFlow, LoadSnapshot, OnboardingState, TopicSlug,
    TopicVisibility, WalkAudience, MAX_PROBE_ATTEMPTS,
};
use pierre_core::models::{Dossier, DossierFact, Pillar};
use uuid::Uuid;

/// Delivered-probe history for the given targets, in ask order.
fn probed(targets: &[CoverageTarget]) -> Vec<TopicSlug> {
    targets.iter().map(|t| t.slug()).collect()
}

fn covered_fact() -> DossierFact {
    DossierFact {
        kind: "goal".to_owned(),
        predicate_code: "states".to_owned(),
        object: "x".to_owned(),
        confidence: 0.9,
        source: "onboarding".to_owned(),
        updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap_or_default(),
        valid_until: None,
        stale: false,
    }
}

#[test]
fn empty_dossier_targets_north_star_first() {
    let d = Dossier::empty(Uuid::nil(), Uuid::nil());
    let cov = CoverageMap::from_dossier(&d);
    assert_eq!(
        cov.next_target(&[], WalkAudience::Private),
        Some(CoverageTarget::NorthStar)
    );
    assert!(!cov.is_complete());
    assert_eq!(cov.covered_count(), 0);
}

#[test]
fn north_star_then_first_uncovered_pillar() {
    let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
    d.north_star = vec![covered_fact()];
    let cov = CoverageMap::from_dossier(&d);
    assert_eq!(
        cov.next_target(&[], WalkAudience::Private),
        Some(CoverageTarget::Pillar(Pillar::TrainingAndMovement))
    );
}

#[test]
fn stale_fact_does_not_count_as_covered() {
    let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
    let mut f = covered_fact();
    f.stale = true;
    d.north_star = vec![f];
    let cov = CoverageMap::from_dossier(&d);
    assert!(!cov.north_star_covered);
}

#[test]
fn complete_when_all_seven_covered() {
    let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
    d.north_star = vec![covered_fact()];
    for pillar in Pillar::ALL {
        d.pillars.insert(pillar, vec![covered_fact()]);
    }
    let cov = CoverageMap::from_dossier(&d);
    assert!(cov.is_complete());
    assert_eq!(cov.next_target(&[], WalkAudience::Private), None);
    assert_eq!(cov.covered_count(), 7);
}

#[test]
fn onboarding_state_roundtrip() {
    let s = OnboardingState::start("2026-06-17T00:00:00Z".to_owned(), GuidedFlow::Pillars);
    let json = serde_json::to_string(&s).unwrap_or_default();
    let back = OnboardingState::from_column(Some(&json));
    assert!(back.is_some());
    assert!(OnboardingState::from_column(None).is_none());
    assert!(OnboardingState::from_column(Some("not json")).is_none());
}

#[test]
fn flow_survives_the_column_round_trip() {
    let json = OnboardingState::start_now_column(GuidedFlow::Calibration);
    assert_eq!(
        OnboardingState::from_column(Some(&json)).map(|s| s.flow),
        Some(GuidedFlow::Calibration),
        "a calibration start that came back as a pillars walk would ask the wrong questions"
    );
    let json = OnboardingState::start_now_column(GuidedFlow::Pillars);
    assert_eq!(
        OnboardingState::from_column(Some(&json)).map(|s| s.flow),
        Some(GuidedFlow::Pillars)
    );
}

#[test]
fn the_serialization_fallback_literals_carry_their_flow() {
    // `start_now_column` degrades to a hand-written literal if serde ever
    // fails. Both literals must parse back to the flow that produced them —
    // a flow-less fallback would silently default a calibration start to
    // the pillars walk.
    for (literal, expected) in [
        (
            r#"{"active":true,"started_at":"","flow":"pillars"}"#,
            GuidedFlow::Pillars,
        ),
        (
            r#"{"active":true,"started_at":"","flow":"calibration"}"#,
            GuidedFlow::Calibration,
        ),
    ] {
        assert_eq!(
            OnboardingState::from_column(Some(literal)).map(|s| s.flow),
            Some(expected),
            "fallback literal {literal} did not round-trip"
        );
    }
}

#[test]
fn a_row_written_before_flow_existed_is_a_pillars_walk() {
    // Live rows predate both `probed` and `flow`. They must keep parsing,
    // and they are all pillars walks — calibration did not exist.
    let legacy = r#"{"active":true,"started_at":"2026-06-17T00:00:00Z"}"#;
    let parsed = OnboardingState::from_column(Some(legacy));
    assert_eq!(parsed.as_ref().map(|s| s.flow), Some(GuidedFlow::Pillars));
    assert_eq!(parsed.as_ref().map(|s| s.probed.len()), Some(0));
    assert!(parsed.is_some_and(|s| s.snapshot.is_none()));
}

#[test]
fn a_row_written_with_probed_but_no_flow_still_parses() {
    // Rows written by the pillars Phase A build: `probed` present, `flow`
    // absent. Both defaults must apply independently.
    let mid = r#"{"active":true,"started_at":"2026-07-26T00:00:00Z","probed":["north_star"]}"#;
    let parsed = OnboardingState::from_column(Some(mid));
    assert_eq!(parsed.as_ref().map(|s| s.flow), Some(GuidedFlow::Pillars));
    assert_eq!(parsed.map(|s| s.probed.len()), Some(1));
}

#[test]
fn the_snapshot_survives_the_column_round_trip() {
    let snapshot = LoadSnapshot {
        weekly_hours: 7.5,
        sessions_per_week: 4.0,
        longest_session_min: 195,
        weeks: 6,
        sport_families: 1,
    };
    let state = OnboardingState::start("2026-07-28T00:00:00Z".to_owned(), GuidedFlow::Calibration)
        .with_snapshot(Some(snapshot.clone()));
    let column = state.to_column().unwrap_or_default();
    assert_eq!(
        OnboardingState::from_column(Some(&column)).and_then(|s| s.snapshot),
        Some(snapshot),
        "later turns quote the baseline back; a lost snapshot would refetch and drift"
    );
}

#[test]
fn probed_north_star_advances_before_its_fact_lands() {
    // The athlete answered the North Star but extraction has not landed:
    // coverage still says uncovered. One delivered probe is enough to move
    // the walk to Training & Movement instead of re-asking.
    let d = Dossier::empty(Uuid::nil(), Uuid::nil());
    let cov = CoverageMap::from_dossier(&d);
    assert_eq!(
        cov.next_target(&probed(&[CoverageTarget::NorthStar]), WalkAudience::Private),
        Some(CoverageTarget::Pillar(Pillar::TrainingAndMovement))
    );
}

#[test]
fn unprobed_topics_are_swept_before_a_second_attempt() {
    let d = Dossier::empty(Uuid::nil(), Uuid::nil());
    let cov = CoverageMap::from_dossier(&d);
    // Every topic asked exactly once, none extracted: the sweep restarts at
    // the North Star for attempt two rather than stopping.
    let mut history = vec![CoverageTarget::NorthStar];
    history.extend(Pillar::ALL.map(CoverageTarget::Pillar));
    assert_eq!(
        cov.next_target(&probed(&history), WalkAudience::Private),
        Some(CoverageTarget::NorthStar)
    );
}

#[test]
fn walk_terminates_once_every_topic_burns_its_attempts() {
    let d = Dossier::empty(Uuid::nil(), Uuid::nil());
    let cov = CoverageMap::from_dossier(&d);
    let mut history = Vec::new();
    for _ in 0..MAX_PROBE_ATTEMPTS {
        history.push(CoverageTarget::NorthStar);
        history.extend(Pillar::ALL.map(CoverageTarget::Pillar));
    }
    assert_eq!(
        cov.next_target(&probed(&history), WalkAudience::Private),
        None,
        "nothing covered, but every topic is out of attempts — walk must end"
    );
    // Coverage is still the honest 0/7: /pillars can re-screen later.
    assert!(!cov.is_complete());
    assert_eq!(cov.covered_count(), 0);
}

#[test]
fn covered_topic_is_skipped_even_with_attempts_left() {
    let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
    d.north_star = vec![covered_fact()];
    let cov = CoverageMap::from_dossier(&d);
    assert_eq!(
        cov.next_target(&probed(&[CoverageTarget::NorthStar]), WalkAudience::Private),
        Some(CoverageTarget::Pillar(Pillar::TrainingAndMovement))
    );
}

#[test]
fn state_without_probed_field_parses_and_records_probes() {
    // Rows written before `probed` existed must keep loading.
    let legacy = r#"{"active":true,"started_at":"2026-06-17T00:00:00Z"}"#;
    let parsed = OnboardingState::from_column(Some(legacy));
    assert!(
        parsed.is_some(),
        "a row written before `probed` existed must still parse"
    );
    assert_eq!(parsed.as_ref().map(|s| s.probed.len()), Some(0));

    let recorded = parsed
        .map(|s| s.with_delivered_probe(CoverageTarget::Pillar(Pillar::Fuelling).slug()))
        .and_then(|s| s.to_column().ok())
        .and_then(|column| OnboardingState::from_column(Some(&column)));
    assert_eq!(
        recorded.map(|s| s.probed),
        Some(probed(&[CoverageTarget::Pillar(Pillar::Fuelling)])),
        "a recorded probe must survive the column round trip"
    );
    assert_eq!(
        CoverageTarget::Pillar(Pillar::Fuelling).slug().as_str(),
        "fuelling"
    );
}

#[test]
fn unknown_slug_in_probed_matches_no_topic() {
    let d = Dossier::empty(Uuid::nil(), Uuid::nil());
    let cov = CoverageMap::from_dossier(&d);
    let foreign = vec![TopicSlug::new("topic_from_a_later_build".to_owned())];
    assert_eq!(
        cov.next_target(&foreign, WalkAudience::Private),
        Some(CoverageTarget::NorthStar)
    );
}
#[test]
fn old_row_json_parses_with_private_defaults() {
    // Every stored row predates the subject/audience fields. They must
    // parse as an unbound private walk — today's semantics exactly.
    let legacy = r#"{"active":true,"started_at":"2026-06-17T00:00:00Z","flow":"calibration"}"#;
    let parsed = OnboardingState::from_column(Some(legacy));
    assert_eq!(
        parsed.as_ref().map(|s| s.subject_user_id.clone()),
        Some(None)
    );
    assert_eq!(
        parsed.map(|s| s.audience),
        Some(WalkAudience::Private),
        "a legacy row read as a room walk would probe room-safe topics only"
    );
}

#[test]
fn subject_and_audience_survive_the_column_round_trip() {
    let state = OnboardingState::start("2026-08-31T00:00:00Z".to_owned(), GuidedFlow::Pillars)
        .with_subject("6a938a90-2b31-49b5-8b9a-000000000001".to_owned())
        .with_audience(WalkAudience::Room);
    let column = state.to_column().unwrap_or_default();
    let back = OnboardingState::from_column(Some(&column));
    assert_eq!(
        back.as_ref().and_then(|s| s.subject_user_id.clone()),
        Some("6a938a90-2b31-49b5-8b9a-000000000001".to_owned()),
        "a lost subject binding would let any member's message advance the walk"
    );
    assert_eq!(back.map(|s| s.audience), Some(WalkAudience::Room));
}

#[test]
fn room_audience_excludes_dm_only_pillars_from_the_walk() {
    // Nothing covered: a room walk must sweep the North Star plus the four
    // room-safe pillars and never surface the two DM-only ones — and it
    // must TERMINATE once those five burn their attempts, because `None`
    // is the walk's completion signal.
    let d = Dossier::empty(Uuid::nil(), Uuid::nil());
    let cov = CoverageMap::from_dossier(&d);
    let mut history = Vec::new();
    while let Some(target) = cov.next_target(&probed(&history), WalkAudience::Room) {
        assert_ne!(
            target.visibility(),
            TopicVisibility::DmOnly,
            "a DM-only topic surfaced in a room walk: {:?}",
            target.slug().as_str()
        );
        history.push(target);
        assert!(
            history.len() <= 5 * MAX_PROBE_ATTEMPTS,
            "room walk did not terminate"
        );
    }
    assert_eq!(
        history.len(),
        5 * MAX_PROBE_ATTEMPTS,
        "expected exactly the 5 room-safe targets, each probed to its budget"
    );
    // The same dossier walked privately still reaches all seven.
    assert_eq!(
        cov.next_target(&probed(&history), WalkAudience::Private)
            .map(CoverageTarget::visibility),
        Some(TopicVisibility::DmOnly),
        "the private walk must still owe the DM-only pillars"
    );
}
