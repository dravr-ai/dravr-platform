// ABOUTME: Tests for the calibration walk topics
// ABOUTME: Ask order, conditional topics, slug prefixes and safety-critical topics

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::calibration::{CalibrationConditions, CalibrationTopic};
use pierre_core::models::onboarding::TopicSlug;
use pierre_core::models::onboarding::{TopicVisibility, WalkAudience};

fn probed(topics: &[CalibrationTopic]) -> Vec<TopicSlug> {
    topics.iter().map(|t| t.slug()).collect()
}

#[test]
fn intent_is_asked_first() {
    assert_eq!(
        CalibrationTopic::next_target(&[], CalibrationConditions::default(), WalkAudience::Private),
        Some(CalibrationTopic::ProgressionIntent),
        "every later answer is interpreted against the progression intent"
    );
}

#[test]
fn topics_advance_in_ask_order_and_terminate() {
    let conditions = CalibrationConditions::default();
    let mut history = Vec::new();
    for expected in CalibrationTopic::CORE {
        assert_eq!(
            CalibrationTopic::next_target(&history, conditions, WalkAudience::Private),
            Some(expected)
        );
        history.push(expected.slug());
    }
    assert_eq!(
        CalibrationTopic::next_target(&history, conditions, WalkAudience::Private),
        None,
        "the core interview ends after its six topics — there is no re-ask budget"
    );
}

#[test]
fn conditionals_only_appear_when_they_apply() {
    let core = CalibrationTopic::for_conditions(CalibrationConditions::default());
    assert_eq!(core.len(), 6);
    assert!(!core.contains(&CalibrationTopic::Fueling));

    let fueling_only = CalibrationTopic::for_conditions(CalibrationConditions {
        long_sessions: true,
        dated_goal: false,
    });
    assert_eq!(fueling_only.len(), 7);
    assert!(fueling_only.contains(&CalibrationTopic::Fueling));
    assert!(!fueling_only.contains(&CalibrationTopic::EventDemand));

    let both = CalibrationTopic::for_conditions(CalibrationConditions {
        long_sessions: true,
        dated_goal: true,
    });
    assert_eq!(both, CalibrationTopic::ALL.to_vec());
}

#[test]
fn a_dated_goal_alone_earns_the_fueling_question() {
    // Either signal qualifies: an athlete building toward an event needs
    // the fueling answer before their sessions get long, not after.
    let goal_only = CalibrationTopic::for_conditions(CalibrationConditions {
        long_sessions: false,
        dated_goal: true,
    });
    assert!(goal_only.contains(&CalibrationTopic::Fueling));
    assert!(goal_only.contains(&CalibrationTopic::EventDemand));
    assert_eq!(goal_only.len(), 8);
}

#[test]
fn a_qualifying_conditional_is_still_asked_after_the_core_six() {
    let conditions = CalibrationConditions {
        long_sessions: true,
        dated_goal: false,
    };
    let history = probed(&CalibrationTopic::CORE);
    assert_eq!(
        CalibrationTopic::next_target(&history, conditions, WalkAudience::Private),
        Some(CalibrationTopic::Fueling),
        "the core six are done but this athlete still owes the fueling answer"
    );
}

#[test]
fn slugs_are_prefixed_so_they_never_collide_with_a_pillar() {
    for topic in CalibrationTopic::ALL {
        assert!(
            topic.as_str().starts_with("calibration_"),
            "{} shares the ledger with pillar slugs",
            topic.as_str()
        );
        assert_eq!(CalibrationTopic::parse(topic.as_str()), Some(topic));
    }
    assert_eq!(CalibrationTopic::parse("fuelling"), None);
    assert_eq!(CalibrationTopic::parse("north_star"), None);
}

#[test]
fn a_pillar_slug_in_the_ledger_does_not_satisfy_a_calibration_topic() {
    // A conversation that ran a pillars walk before calibrating carries
    // pillar slugs in the shared ledger; none of them may mark a
    // calibration topic delivered.
    let stale = vec![
        TopicSlug::new("north_star".to_owned()),
        TopicSlug::new("training_and_movement".to_owned()),
        TopicSlug::new("fuelling".to_owned()),
    ];
    assert_eq!(
        CalibrationTopic::next_target(
            &stale,
            CalibrationConditions::default(),
            WalkAudience::Private
        ),
        Some(CalibrationTopic::ProgressionIntent)
    );
}

#[test]
fn safety_critical_topics_are_the_two_that_bound_load() {
    let critical: Vec<_> = CalibrationTopic::ALL
        .into_iter()
        .filter(|t| t.is_safety_critical())
        .collect();
    assert_eq!(
        critical,
        vec![CalibrationTopic::Injury, CalibrationTopic::RecoverySpeed]
    );
}

#[test]
fn fact_kinds_match_the_topic_semantics() {
    assert_eq!(CalibrationTopic::Availability.fact_kind(), "schedule");
    assert_eq!(CalibrationTopic::Injury.fact_kind(), "injury");
    assert_eq!(CalibrationTopic::EventDemand.fact_kind(), "goal");
    assert_eq!(CalibrationTopic::RecoverySpeed.fact_kind(), "physiology");
    assert_eq!(
        CalibrationTopic::ProgressionIntent.fact_kind(),
        "preference"
    );
}

#[test]
fn each_safety_critical_topic_owns_its_fact_kind_alone() {
    // The completion check notices a missing safety answer by looking for
    // that topic's kind among the facts the interview landed. That only
    // works while no other topic writes the same kind — otherwise a
    // sibling's answer would mask the gap and the interview would report
    // success having never learned whether the athlete is injured.
    for critical in CalibrationTopic::ALL
        .into_iter()
        .filter(|t| t.is_safety_critical())
    {
        let sharers: Vec<_> = CalibrationTopic::ALL
            .into_iter()
            .filter(|t| *t != critical && t.fact_kind() == critical.fact_kind())
            .collect();
        assert!(
            sharers.is_empty(),
            "{} shares kind '{}' with {sharers:?}, so its absence cannot be detected",
            critical.as_str(),
            critical.fact_kind()
        );
    }
}

#[test]
fn every_topic_has_a_usable_probe_hint() {
    for topic in CalibrationTopic::ALL {
        let hint = topic.probe_hint();
        assert!(
            hint.len() > 40,
            "{} has no usable probe hint",
            topic.as_str()
        );
        assert!(
            !hint.ends_with('?'),
            "{} reads as a verbatim question; hints describe what to explore",
            topic.as_str()
        );
    }
}
#[test]
fn every_calibration_topic_declares_itself_room_safe() {
    // Pins that adding a sensitive topic forces a conscious visibility
    // decision: a new variant lands here red until someone chooses.
    assert!(
        CalibrationTopic::ALL
            .into_iter()
            .all(|t| t.visibility() == TopicVisibility::RoomSafe),
        "a DM-only calibration topic exists now — decide what a room walk says about it"
    );
    assert_eq!(
        CalibrationTopic::next_target(&[], CalibrationConditions::default(), WalkAudience::Room),
        CalibrationTopic::next_target(&[], CalibrationConditions::default(), WalkAudience::Private),
        "with every topic room-safe, both audiences must walk the same list"
    );
}
