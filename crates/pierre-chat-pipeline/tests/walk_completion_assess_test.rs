// ABOUTME: Unit tests for the calibration and season walks' facts-landed checks
// ABOUTME: Pins the time window, shared-kind crediting and which missing answers are named

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use chrono::{Duration, Utc};
use pierre_chat_pipeline::stages::completion::{assess, assess_season};
use pierre_core::models::{CalibrationTopic, SeasonTopic};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode, UserFact};

fn fact(kind: FactKind, age_minutes: i64) -> UserFact {
    let ts = Utc::now() - Duration::minutes(age_minutes);
    UserFact {
        id: format!("{kind:?}-{age_minutes}"),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        agent_id: None,
        scope: MemoryScope::User,
        kind,
        pillar: None,
        predicate_code: PredicateCode::States,
        object: "o".to_owned(),
        confidence: 0.9,
        source: FactSource::Onboarding,
        valid_until: None,
        source_msg_id: None,
        created_at: ts,
        updated_at: ts,
    }
}

#[test]
fn a_silent_interview_reports_zero_and_names_both_safety_gaps() {
    // The failure this whole module exists for: every question asked,
    // nothing extracted. It must not read as success.
    let (captured, missing) = assess(
        &[],
        &CalibrationTopic::CORE,
        Utc::now() - Duration::hours(1),
    );
    assert_eq!(captured, 0);
    assert_eq!(
        missing,
        vec![CalibrationTopic::Injury, CalibrationTopic::RecoverySpeed]
    );
}

#[test]
fn facts_from_before_the_interview_do_not_count() {
    // A pillars walk months ago also wrote `source=onboarding` facts. If
    // those counted, an interview that landed nothing would report a full
    // house and the athlete would never be asked again.
    let started = Utc::now() - Duration::hours(1);
    let stale = vec![
        fact(FactKind::Injury, 60 * 24 * 30),
        fact(FactKind::Physiology, 60 * 24 * 30),
    ];
    let (captured, missing) = assess(&stale, &CalibrationTopic::CORE, started);
    assert_eq!(captured, 0, "month-old facts predate this interview");
    assert_eq!(missing.len(), 2);
}

#[test]
fn a_landed_injury_answer_clears_only_that_safety_gap() {
    let started = Utc::now() - Duration::hours(1);
    let landed = vec![fact(FactKind::Injury, 10)];
    let (captured, missing) = assess(&landed, &CalibrationTopic::CORE, started);
    assert_eq!(captured, 1);
    assert_eq!(
        missing,
        vec![CalibrationTopic::RecoverySpeed],
        "recovery speed is still unanswered and must still be named"
    );
}

#[test]
fn a_complete_interview_names_no_gaps() {
    let started = Utc::now() - Duration::hours(1);
    let landed = vec![
        fact(FactKind::Preference, 50), // progression intent
        fact(FactKind::Preference, 40), // baseline confirm
        fact(FactKind::Schedule, 30),   // availability
        fact(FactKind::Injury, 20),     // injury
        fact(FactKind::Preference, 15), // rpe headroom
        fact(FactKind::Physiology, 10), // recovery speed
    ];
    let (captured, missing) = assess(&landed, &CalibrationTopic::CORE, started);
    assert_eq!(captured, 6, "all six core topics produced a fact");
    assert!(missing.is_empty());
}

#[test]
fn shared_kinds_are_credited_once_per_fact_not_once_per_topic() {
    // Three topics write `preference`. One preference fact must credit one
    // topic, not three — over-reporting is the direction that makes the
    // wrap-up a lie.
    let started = Utc::now() - Duration::hours(1);
    let landed = vec![fact(FactKind::Preference, 10)];
    let (captured, _) = assess(&landed, &CalibrationTopic::CORE, started);
    assert_eq!(captured, 1);

    let landed = vec![
        fact(FactKind::Preference, 10),
        fact(FactKind::Preference, 9),
    ];
    let (captured, _) = assess(&landed, &CalibrationTopic::CORE, started);
    assert_eq!(captured, 2);
}

#[test]
fn a_conditional_topic_that_was_asked_is_counted_in_the_denominator() {
    let started = Utc::now() - Duration::hours(1);
    let mut asked = CalibrationTopic::CORE.to_vec();
    asked.push(CalibrationTopic::EventDemand);
    let landed = vec![fact(FactKind::Goal, 10)];
    let (captured, missing) = assess(&landed, &asked, started);
    assert_eq!(captured, 1, "only the event-demand answer landed");
    assert_eq!(missing.len(), 2, "both safety topics are still missing");
}

#[test]
fn a_season_walk_with_no_goal_fact_is_short_a_calendar() {
    let started = Utc::now() - Duration::minutes(30);
    let landed = vec![
        fact(FactKind::Physiology, 20),
        fact(FactKind::Preference, 15),
        fact(FactKind::Equipment, 10),
        fact(FactKind::Preference, 5),
    ];
    let (captured, goal_missing) = assess_season(&landed, &SeasonTopic::CORE, started);
    assert_eq!(
        captured, 4,
        "bests, background, tools and coaching fit landed"
    );
    assert!(
        goal_missing,
        "no goal fact means no calendar to lay a season on"
    );
}

#[test]
fn a_goal_fact_credits_the_calendar_and_the_horizon_separately() {
    let started = Utc::now() - Duration::minutes(30);
    let landed = vec![fact(FactKind::Goal, 25), fact(FactKind::Goal, 20)];
    let (captured, goal_missing) = assess_season(&landed, &SeasonTopic::CORE, started);
    assert_eq!(captured, 2, "two goal facts credit both goal topics");
    assert!(!goal_missing);

    let one = vec![fact(FactKind::Goal, 25)];
    let (captured, goal_missing) = assess_season(&one, &SeasonTopic::CORE, started);
    assert_eq!(
        captured, 1,
        "one goal fact credits the calendar, asked first"
    );
    assert!(!goal_missing);
}

#[test]
fn season_facts_before_the_window_do_not_count() {
    let started = Utc::now() - Duration::minutes(10);
    let stale = vec![fact(FactKind::Goal, 60), fact(FactKind::Equipment, 45)];
    let (captured, goal_missing) = assess_season(&stale, &SeasonTopic::CORE, started);
    assert_eq!(captured, 0);
    assert!(goal_missing);
}
