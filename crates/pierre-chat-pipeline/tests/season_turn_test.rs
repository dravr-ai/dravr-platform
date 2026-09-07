// ABOUTME: The season walk's turn resolution — directive text, fact stamping, ledger slugs, the release
// ABOUTME: Guards the third fixed-list flow sharing one ledger with the other two without sharing a policy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Season turn behaviour that needs no database.
//!
//! The resolver itself is exercised against a real database in
//! `pierre-server/tests/season_command_test.rs`; what is checked here is the
//! pure per-turn output: which directive the coach receives, how each answer
//! is stamped, that the calendar turn leaves its kind to the extractor, and
//! that three flows cannot corrupt each other's ledger entries.

use pierre_chat_pipeline::stages::onboarding::{
    answered_target, directive, extraction_params, release_directive, GuidedTarget, OnboardingTurn,
};
use pierre_core::models::{
    CalibrationTopic, CoverageTarget, GuidedFlow, LoadSnapshot, OnboardingState, Pillar,
    SeasonConditions, SeasonTopic, WalkAudience,
};
use pierre_memory::{FactKind, FactSource};
use std::collections::HashSet;

const STARTED_AT: &str = "2026-09-07T00:00:00Z";

fn season_turn(topic: SeasonTopic, audience: WalkAudience) -> OnboardingTurn {
    OnboardingTurn {
        target: GuidedTarget::Season(topic),
        state: OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Season)
            .with_audience(audience),
    }
}

fn multi_sport_snapshot() -> LoadSnapshot {
    LoadSnapshot {
        weekly_hours: 9.0,
        sessions_per_week: 6.0,
        longest_session_min: 150,
        weeks: 6,
        sport_families: 3,
    }
}

#[test]
fn every_season_topic_but_the_calendar_forces_the_kind_its_answer_means() {
    for topic in SeasonTopic::ALL {
        let (pillar, source, forced) = extraction_params(GuidedTarget::Season(topic));
        assert_eq!(
            pillar,
            Some(Pillar::TrainingAndMovement),
            "{}",
            topic.as_str()
        );
        assert_eq!(source, FactSource::Onboarding);
        match topic {
            SeasonTopic::RaceCalendar => assert_eq!(
                forced, None,
                "the calendar turn also carries the quoted-back availability: a correction is \
                 a schedule fact while the races are goals, so forcing either mis-files the other"
            ),
            SeasonTopic::GoalHorizon => assert_eq!(forced, Some(FactKind::Goal)),
            SeasonTopic::PerformanceBaseline => assert_eq!(forced, Some(FactKind::Physiology)),
            SeasonTopic::Background | SeasonTopic::CoachingFit => {
                assert_eq!(forced, Some(FactKind::Preference));
            }
            SeasonTopic::MeasurementTools | SeasonTopic::FacilityAccess => {
                assert_eq!(forced, Some(FactKind::Equipment));
            }
        }
    }
}

#[test]
fn every_turn_of_the_walk_stamps_the_question_it_answers() {
    // Drive the whole walk turn by turn, multi-sport so the conditional is in
    // play, and check the stamping at each step against the question asked
    // one turn earlier.
    let conditions = SeasonConditions { multi_sport: true };
    let mut state = OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Season)
        .with_snapshot(Some(multi_sport_snapshot()));
    let mut asked_last_turn: Option<SeasonTopic> = None;
    let mut steps = 0;

    while let Some(asking_now) =
        SeasonTopic::next_target(&state.probed, conditions, WalkAudience::Private)
    {
        assert_eq!(
            answered_target(&state),
            asked_last_turn.map(GuidedTarget::Season),
            "turn {steps} stamped the wrong topic"
        );
        if let Some(previous) = asked_last_turn {
            assert_ne!(
                previous, asking_now,
                "turn {steps} answers and asks the same topic"
            );
        }
        asked_last_turn = Some(asking_now);
        state = state.with_delivered_probe(asking_now.slug());
        steps += 1;
    }

    assert_eq!(
        steps,
        SeasonTopic::ALL.len(),
        "six core questions plus facilities"
    );
    assert_eq!(asked_last_turn, Some(SeasonTopic::FacilityAccess));
    let answered = answered_target(&state).expect("the last probe is in the ledger");
    assert_eq!(answered, GuidedTarget::Season(SeasonTopic::FacilityAccess));
    let (_, _, forced) = extraction_params(answered);
    assert_eq!(forced, Some(FactKind::Equipment));
}

#[test]
fn the_opening_message_of_the_walk_answers_no_topic() {
    let fresh = OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Season);
    assert_eq!(answered_target(&fresh), None);
}

#[test]
fn the_season_directive_names_the_mode_the_topic_and_the_no_plan_clause() {
    let text = directive(&season_turn(
        SeasonTopic::RaceCalendar,
        WalkAudience::Private,
    ));
    assert!(text.contains("# Season mode"), "{text}");
    assert!(text.contains("overrides every other instruction"), "{text}");
    assert!(
        text.contains("the events on their calendar and which one matters"),
        "{text}"
    );
    assert!(
        text.contains(SeasonTopic::RaceCalendar.probe_hint()),
        "{text}"
    );
    assert!(
        text.contains("Do not build, propose, or save a training plan on this turn"),
        "the walk captures; the season is laid out by the rule after it: {text}"
    );
    assert!(!text.contains("Calibration mode"), "{text}");
}

#[test]
fn every_season_topic_reaches_the_directive_with_its_own_hint() {
    for topic in SeasonTopic::ALL {
        let text = directive(&season_turn(topic, WalkAudience::Private));
        assert!(
            text.contains(topic.probe_hint()),
            "{} hint missing: {text}",
            topic.as_str()
        );
    }
}

#[test]
fn a_room_walk_carries_the_audience_line_and_skips_coaching_fit() {
    let text = directive(&season_turn(SeasonTopic::RaceCalendar, WalkAudience::Room));
    assert!(text.contains("shared room the athlete chose"), "{text}");

    let mut probed = Vec::new();
    let mut asked = Vec::new();
    while let Some(next) =
        SeasonTopic::next_target(&probed, SeasonConditions::default(), WalkAudience::Room)
    {
        asked.push(next);
        probed.push(next.slug());
    }
    assert!(
        !asked.contains(&SeasonTopic::CoachingFit),
        "what the last coach got wrong is said to a coach alone: {asked:?}"
    );
    assert_eq!(asked.len(), SeasonTopic::CORE.len() - 1);
}

#[test]
fn the_three_flows_write_disjoint_ledger_slugs() {
    // All three walks append to one `probed` list. If a season slug ever
    // equalled a pillar or calibration slug, finishing one flow would
    // silently mark a topic of another as delivered.
    let season: Vec<String> = SeasonTopic::ALL
        .iter()
        .map(|t| t.slug().as_str().to_owned())
        .collect();
    let mut others: Vec<String> = CalibrationTopic::ALL
        .iter()
        .map(|t| t.slug().as_str().to_owned())
        .collect();
    others.extend(
        Pillar::ALL
            .iter()
            .map(|p| CoverageTarget::Pillar(*p).slug().as_str().to_owned()),
    );
    others.push(CoverageTarget::NorthStar.slug().as_str().to_owned());

    for slug in &season {
        assert!(
            !others.contains(slug),
            "season slug '{slug}' collides with another flow"
        );
    }
    assert_eq!(
        season.len(),
        season.iter().collect::<HashSet<_>>().len(),
        "two season topics share a slug"
    );

    // And a ledger holding the other flows' slugs resolves to THEIR targets,
    // never to a season topic.
    let calibration_state = OnboardingState::start(STARTED_AT.to_owned(), GuidedFlow::Calibration)
        .with_delivered_probe(CalibrationTopic::Injury.slug());
    assert_eq!(
        answered_target(&calibration_state),
        Some(GuidedTarget::Calibration(CalibrationTopic::Injury))
    );
}

#[test]
fn the_season_release_directive_hands_the_yes_to_the_rule() {
    let retired = |flow: GuidedFlow| {
        OnboardingState::start(STARTED_AT.to_owned(), flow)
            .completed(STARTED_AT.to_owned())
            .to_column()
            .expect("serialize completed state")
    };
    let text = release_directive(Some(&retired(GuidedFlow::Season)));
    assert!(text.contains("recommend_plan_flavour"), "{text}");
    assert!(text.contains("Never choose a flavour yourself"), "{text}");
    assert!(
        !release_directive(Some(&retired(GuidedFlow::Pillars))).contains("recommend_plan_flavour"),
        "only the season wrap-up offers to lay the season out"
    );
}
