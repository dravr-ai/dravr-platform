// ABOUTME: The calibration flow's turn resolution — directive text, fact stamping, ledger slugs
// ABOUTME: Guards the two flows sharing one ledger without sharing a next-topic policy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Calibration turn behaviour that needs no database.
//!
//! The resolver itself is exercised against a real database in
//! `pierre-server/tests/calibration_walk_test.rs`; what is checked here is the
//! pure per-turn output: which directive the coach receives, how the answer is
//! stamped, and that the two flows cannot corrupt each other's ledger entries.

use pierre_chat_pipeline::stages::onboarding::{
    answered_target, directive, extraction_params, GuidedTarget, OnboardingTurn,
};
use pierre_core::models::{
    CalibrationConditions, CalibrationTopic, CoverageTarget, GuidedFlow, LoadSnapshot,
    OnboardingState, Pillar, TopicSlug,
};
use pierre_memory::{FactKind, FactSource};
use std::collections::HashSet;

/// Interview start stamp shared by every fixture here.
const STARTED_AT: &str = "2026-07-28T00:00:00Z";

fn calibration_turn(topic: CalibrationTopic, snapshot: Option<LoadSnapshot>) -> OnboardingTurn {
    OnboardingTurn {
        target: GuidedTarget::Calibration(topic),
        state: OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Calibration)
            .with_snapshot(snapshot),
    }
}

/// The flow state as a turn loads it, wound forward to the turn that asks
/// `topic` — i.e. every earlier topic recorded as delivered, and `topic` not.
///
/// This is the real turn geometry: `record_delivered_probe` appends at the END
/// of the turn that asks a question, so the state a turn starts with names the
/// question the athlete's inbound message is answering.
fn state_when_asking(topic: CalibrationTopic) -> OnboardingState {
    let mut state = OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Calibration);
    loop {
        let next = CalibrationTopic::next_target(&state.probed, CalibrationConditions::default())
            .expect("the interview ran out of topics before reaching the fixture's");
        if next == topic {
            return state;
        }
        state = state.with_delivered_probe(next.slug());
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
        let (pillar, source, forced) = extraction_params(GuidedTarget::Calibration(topic));
        assert_eq!(pillar, Some(Pillar::TrainingAndMovement), "{topic:?}");
        assert_eq!(source, FactSource::Onboarding, "{topic:?}");
        assert_eq!(forced, Some(kind), "{topic:?} stamped the wrong kind");
    }
}

#[test]
fn the_injury_turn_files_the_availability_answer_as_a_schedule_fact() {
    // Two consecutive guided turns. The turn that ASKS about injuries carries
    // the athlete's answer to the previous question — availability — because a
    // probe is recorded as delivered at the end of the turn that asks it, so
    // the next-topic policy has already advanced by the time this state loads.
    //
    // Stamping the inbound message with the topic being asked filed "8 h/week,
    // Tuesdays protected" as `FactKind::Injury`. That is what the dossier then
    // hands the coach as the athlete's injury history, and what the completion
    // check counts as the safety answer landing — Injury being the sole writer
    // of its kind is precisely what makes the mis-file invisible.
    let state = state_when_asking(CalibrationTopic::Injury);
    assert_eq!(
        state.probed.last().map(TopicSlug::as_str),
        Some(CalibrationTopic::Availability.as_str()),
        "fixture geometry: the previous turn must have asked about availability"
    );

    let answered = answered_target(&state).expect("the previous turn's probe is in the ledger");
    assert_eq!(
        answered,
        GuidedTarget::Calibration(CalibrationTopic::Availability),
        "the message being extracted answers the question the previous turn asked"
    );

    let (pillar, source, forced) = extraction_params(answered);
    assert_eq!(
        forced,
        Some(FactKind::Schedule),
        "an availability answer is a schedule fact"
    );
    assert_ne!(
        forced,
        Some(FactKind::Injury),
        "stamping with the topic being asked is the mis-file this test exists for"
    );
    assert_eq!(pillar, Some(Pillar::TrainingAndMovement));
    assert_eq!(source, FactSource::Onboarding);
}

#[test]
fn the_opening_message_of_a_guided_flow_answers_no_topic() {
    // The first guided turn's inbound message arrived before any question was
    // asked. Stamping it with a topic would file whatever the athlete opened
    // with as that topic's answer.
    let fresh = OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Calibration);
    assert_eq!(answered_target(&fresh), None);
}

#[test]
fn every_turn_of_the_walk_stamps_the_question_it_answers() {
    // Drive the whole core interview turn by turn and check the stamping at
    // each step against the question actually asked one turn earlier.
    let conditions = CalibrationConditions::default();
    let mut state = OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Calibration);
    let mut asked_last_turn: Option<CalibrationTopic> = None;
    let mut steps = 0;

    while let Some(asking_now) = CalibrationTopic::next_target(&state.probed, conditions) {
        assert_eq!(
            answered_target(&state),
            asked_last_turn.map(GuidedTarget::Calibration),
            "turn {steps} stamped the wrong topic"
        );
        if let Some(previous) = asked_last_turn {
            let (_, _, forced) = extraction_params(GuidedTarget::Calibration(previous));
            assert_eq!(
                forced,
                Some(FactKind::parse_lenient(previous.fact_kind())),
                "turn {steps} forced a kind the answered topic does not mean"
            );
            assert_ne!(
                previous, asking_now,
                "turn {steps} answers and asks the same topic"
            );
        }
        asked_last_turn = Some(asking_now);
        state = state.with_delivered_probe(asking_now.slug());
        steps += 1;
    }

    assert_eq!(steps, CalibrationTopic::CORE.len(), "six core questions");
    // The turn that finds nothing left to ask is the one that answers the last
    // question, and the last question is recovery speed: safety-critical, sole
    // writer of `physiology`, and the answer the deterministic wrap-up turn
    // still has to extract even though it never calls the model.
    assert_eq!(asked_last_turn, Some(CalibrationTopic::RecoverySpeed));
    let answered = answered_target(&state).expect("the last probe is in the ledger");
    assert_eq!(
        answered,
        GuidedTarget::Calibration(CalibrationTopic::RecoverySpeed)
    );
    let (_, _, forced) = extraction_params(answered);
    assert_eq!(forced, Some(FactKind::Physiology));
}

#[test]
fn a_pillars_ledger_resolves_back_to_its_own_targets() {
    // The two flows share one ledger, so the answered-topic lookup has to read
    // a pillar slug and a North Star slug as well as a calibration one.
    for target in [
        GuidedTarget::Coverage(CoverageTarget::NorthStar),
        GuidedTarget::Coverage(CoverageTarget::Pillar(Pillar::Fuelling)),
        GuidedTarget::Coverage(CoverageTarget::Pillar(Pillar::TrainingAndMovement)),
    ] {
        let state = OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Pillars)
            .with_delivered_probe(target.slug());
        assert_eq!(
            answered_target(&state),
            Some(target),
            "{target:?} does not survive a round trip through the ledger"
        );
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
