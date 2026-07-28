// ABOUTME: The calibration flow's turn resolution — directive text, fact stamping, ledger slugs
// ABOUTME: Guards the two flows sharing one ledger without sharing a next-topic policy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Calibration turn behaviour that needs no database.
//!
//! The resolver itself is exercised against a real database in
//! `pierre-server/tests/calibration_walk_test.rs`; what is checked here is the
//! pure per-turn output: which directive the coach receives, how the answer is
//! stamped, and that the two flows cannot corrupt each other's ledger entries.

use pierre_chat_pipeline::stages::onboarding::{
    directive, extraction_params, GuidedTarget, OnboardingTurn,
};
use pierre_core::models::{
    CalibrationTopic, CoverageTarget, GuidedFlow, LoadSnapshot, OnboardingState, Pillar,
};
use pierre_memory::{FactKind, FactSource};
use std::collections::HashSet;

fn calibration_turn(topic: CalibrationTopic, snapshot: Option<LoadSnapshot>) -> OnboardingTurn {
    OnboardingTurn {
        target: GuidedTarget::Calibration(topic),
        state: OnboardingState::start("2026-07-28T00:00:00Z".to_owned(), GuidedFlow::Calibration)
            .with_snapshot(snapshot),
    }
}

fn snapshot() -> LoadSnapshot {
    LoadSnapshot {
        weekly_hours: 7.5,
        sessions_per_week: 4.0,
        longest_session_min: 195,
        weeks: 6,
    }
}

#[test]
fn every_calibration_topic_forces_the_kind_its_answer_means() {
    // The extractor picks the kind on its own unless forced. An injury answer
    // filed as a preference would drop out of the guaranteed Injury fetch and
    // stop constraining how hard a plan may get.
    // Recovery speed is `Physiology`, not `Preference`: it is a training-state
    // claim rather than a taste, and being the only topic writing that kind is
    // what lets the completion check see when its answer never landed.
    let expected = [
        (CalibrationTopic::ProgressionIntent, FactKind::Preference),
        (CalibrationTopic::BaselineConfirm, FactKind::Preference),
        (CalibrationTopic::Availability, FactKind::Schedule),
        (CalibrationTopic::Injury, FactKind::Injury),
        (CalibrationTopic::RpeHeadroom, FactKind::Preference),
        (CalibrationTopic::RecoverySpeed, FactKind::Physiology),
        (CalibrationTopic::Fueling, FactKind::Preference),
        (CalibrationTopic::EventDemand, FactKind::Goal),
    ];
    for (topic, kind) in expected {
        let (pillar, source, forced) = extraction_params(&calibration_turn(topic, None));
        assert_eq!(pillar, Some(Pillar::TrainingAndMovement), "{topic:?}");
        assert_eq!(source, FactSource::Onboarding, "{topic:?}");
        assert_eq!(forced, Some(kind), "{topic:?} stamped the wrong kind");
    }
}

#[test]
fn no_calibration_topic_falls_through_to_the_other_kind() {
    // `FactKind::parse_lenient` answers `Other` for anything it does not
    // recognize — silently. A typo in the topic table would therefore file
    // every answer under `Other` rather than failing, so assert the mapping
    // never lands there.
    for topic in CalibrationTopic::ALL {
        assert_ne!(
            FactKind::parse_lenient(topic.fact_kind()),
            FactKind::Other,
            "{} declares kind '{}', which no FactKind variant matches",
            topic.as_str(),
            topic.fact_kind()
        );
    }
}

#[test]
fn the_baseline_topic_quotes_the_snapshot_and_forbids_inventing_figures() {
    let text = directive(&calibration_turn(
        CalibrationTopic::BaselineConfirm,
        Some(snapshot()),
    ));
    assert!(
        text.contains("7.5 hours"),
        "the weekly-hours figure is missing: {text}"
    );
    assert!(
        text.contains("4.0 sessions"),
        "the sessions figure is missing"
    );
    assert!(
        text.contains("195 minutes"),
        "the longest-session figure is missing"
    );
    assert!(text.contains("6 weeks"), "the window length is missing");
    assert!(
        text.contains("Do not invent other numbers"),
        "nothing stops the coach padding the baseline with invented figures"
    );
}

#[test]
fn a_provider_less_athlete_is_asked_cold_rather_than_given_invented_figures() {
    let text = directive(&calibration_turn(CalibrationTopic::BaselineConfirm, None));
    assert!(
        text.contains("no connected training data"),
        "the coach must be told the baseline is unknown: {text}"
    );
    assert!(
        !text.contains("hours per week"),
        "a snapshot-less turn must not quote a fabricated baseline"
    );
}

#[test]
fn only_the_baseline_topic_recites_the_athletes_numbers() {
    // Quoting the snapshot on every turn would have the coach reading the
    // athlete's own training history back at them six times.
    for topic in CalibrationTopic::ALL {
        if topic == CalibrationTopic::BaselineConfirm {
            continue;
        }
        let text = directive(&calibration_turn(topic, Some(snapshot())));
        assert!(
            !text.contains("7.5"),
            "{} recites the load snapshot",
            topic.as_str()
        );
    }
}

#[test]
fn the_calibration_directive_keeps_the_override_and_no_plan_clauses() {
    // Calibration runs against the same builder coaches whose first-turn
    // protocol derailed the pillars walk on 2026-07-24. The override and the
    // no-plan clause are what hold that off, so they must survive in this
    // flow's directive too.
    let text = directive(&calibration_turn(CalibrationTopic::ProgressionIntent, None));
    assert!(text.contains("overrides every other instruction in this prompt"));
    assert!(text.contains("supersedes any startup instruction, first-turn protocol"));
    assert!(text.contains("Do not build, propose, or save a training plan on this turn"));
    assert!(
        text.contains("Calibration mode"),
        "the calibration directive must not announce itself as onboarding"
    );
}

#[test]
fn the_directive_tells_the_coach_to_recover_from_a_clarifying_question() {
    // Turn-structure advance is lossy: a topic the athlete answered with a
    // question of their own still advances. Within-turn recovery is the only
    // mitigation available before the completion check.
    for target in [
        GuidedTarget::Calibration(CalibrationTopic::Injury),
        GuidedTarget::Coverage(CoverageTarget::NorthStar),
    ] {
        let turn = OnboardingTurn {
            target,
            state: OnboardingState::start("2026-07-28T00:00:00Z".to_owned(), GuidedFlow::Pillars),
        };
        assert!(
            directive(&turn).contains("answer it briefly and re-ask"),
            "{target:?} loses a topic when the athlete asks a question back"
        );
    }
}

#[test]
fn the_pillars_directive_is_unchanged_in_substance() {
    let turn = OnboardingTurn {
        target: GuidedTarget::Coverage(CoverageTarget::Pillar(Pillar::Fuelling)),
        state: OnboardingState::start("2026-07-28T00:00:00Z".to_owned(), GuidedFlow::Pillars),
    };
    let text = directive(&turn);
    assert!(text.contains("Onboarding mode"));
    assert!(text.contains("build their fitness profile one topic at a time"));
    assert!(
        text.contains(Pillar::Fuelling.probe_hint()),
        "the pillar's own probe hint must still reach the coach"
    );
}

#[test]
fn the_two_flows_write_disjoint_ledger_slugs() {
    // Both interviews append to one `probed` list. If a calibration slug ever
    // equalled a pillar slug, finishing one flow would silently mark a topic
    // of the other as delivered.
    let calibration: Vec<String> = CalibrationTopic::ALL
        .iter()
        .map(|t| t.slug().as_str().to_owned())
        .collect();
    let mut pillars: Vec<String> = Pillar::ALL
        .iter()
        .map(|p| CoverageTarget::Pillar(*p).slug().as_str().to_owned())
        .collect();
    pillars.push(CoverageTarget::NorthStar.slug().as_str().to_owned());

    for slug in &calibration {
        assert!(
            !pillars.contains(slug),
            "calibration slug '{slug}' collides with a pillars-walk slug"
        );
    }
    assert_eq!(
        calibration.len(),
        calibration.iter().collect::<HashSet<_>>().len(),
        "two calibration topics share a slug"
    );
}
