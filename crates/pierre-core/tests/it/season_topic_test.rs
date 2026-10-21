// ABOUTME: Tests for the season walk topics
// ABOUTME: Ask order, conditional topics, slug prefixes and probe hints

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::onboarding::{TopicSlug, TopicVisibility, WalkAudience};
use pierre_core::models::season::{SeasonConditions, SeasonTopic};

fn probed(topics: &[SeasonTopic]) -> Vec<TopicSlug> {
    topics.iter().map(|t| t.slug()).collect()
}

#[test]
fn the_calendar_is_asked_first() {
    assert_eq!(
        SeasonTopic::next_target(&[], SeasonConditions::default(), WalkAudience::Private),
        Some(SeasonTopic::RaceCalendar),
        "every later answer is read against the event it points at"
    );
}

#[test]
fn topics_advance_in_ask_order_and_terminate() {
    let conditions = SeasonConditions::default();
    let mut history = Vec::new();
    for expected in SeasonTopic::CORE {
        assert_eq!(
            SeasonTopic::next_target(&history, conditions, WalkAudience::Private),
            Some(expected)
        );
        history.push(expected.slug());
    }
    assert_eq!(
        SeasonTopic::next_target(&history, conditions, WalkAudience::Private),
        None,
        "the walk ends after its six core topics — there is no re-ask budget"
    );
}

#[test]
fn facility_access_is_asked_only_of_a_multi_sport_athlete() {
    let single = SeasonTopic::for_conditions(SeasonConditions::default());
    assert_eq!(single.len(), 6);
    assert!(!single.contains(&SeasonTopic::FacilityAccess));

    let multi = SeasonTopic::for_conditions(SeasonConditions { multi_sport: true });
    assert_eq!(multi, SeasonTopic::ALL.to_vec());
}

#[test]
fn a_qualifying_conditional_is_still_asked_after_the_core_six() {
    let history = probed(&SeasonTopic::CORE);
    assert_eq!(
        SeasonTopic::next_target(
            &history,
            SeasonConditions { multi_sport: true },
            WalkAudience::Private
        ),
        Some(SeasonTopic::FacilityAccess)
    );
}

#[test]
fn slugs_are_prefixed_so_they_never_collide_with_another_flow() {
    for topic in SeasonTopic::ALL {
        assert!(
            topic.as_str().starts_with("season_"),
            "{} shares the ledger with pillar and calibration slugs",
            topic.as_str()
        );
        assert_eq!(SeasonTopic::parse(topic.as_str()), Some(topic));
    }
    assert_eq!(SeasonTopic::parse("calibration_availability"), None);
    assert_eq!(SeasonTopic::parse("training_and_movement"), None);
}

#[test]
fn a_foreign_slug_in_the_ledger_does_not_satisfy_a_season_topic() {
    let stale = vec![
        TopicSlug::new("north_star".to_owned()),
        TopicSlug::new("calibration_availability".to_owned()),
        TopicSlug::new("fuelling".to_owned()),
    ];
    assert_eq!(
        SeasonTopic::next_target(&stale, SeasonConditions::default(), WalkAudience::Private),
        Some(SeasonTopic::RaceCalendar)
    );
}

#[test]
fn the_calendar_turn_leaves_the_kind_to_the_extractor() {
    // It carries the quoted-back availability too: a correction is a
    // schedule fact while the races are goals, so forcing either would
    // mis-file the other.
    assert_eq!(SeasonTopic::RaceCalendar.fact_kind(), None);
    assert_eq!(SeasonTopic::RaceCalendar.landed_kind(), "goal");
    assert_eq!(SeasonTopic::GoalHorizon.fact_kind(), Some("goal"));
    assert_eq!(
        SeasonTopic::PerformanceBaseline.fact_kind(),
        Some("physiology")
    );
    assert_eq!(SeasonTopic::MeasurementTools.fact_kind(), Some("equipment"));
    assert_eq!(SeasonTopic::FacilityAccess.fact_kind(), Some("equipment"));
    assert_eq!(SeasonTopic::Background.fact_kind(), Some("preference"));
    assert_eq!(SeasonTopic::CoachingFit.fact_kind(), Some("preference"));
}

#[test]
fn every_topic_has_a_usable_probe_hint() {
    for topic in SeasonTopic::ALL {
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
fn a_room_walk_skips_coaching_fit_and_nothing_else() {
    let private: Vec<_> = SeasonTopic::for_conditions(SeasonConditions { multi_sport: true });
    let mut history = Vec::new();
    let mut asked_in_room = Vec::new();
    while let Some(next) = SeasonTopic::next_target(
        &history,
        SeasonConditions { multi_sport: true },
        WalkAudience::Room,
    ) {
        asked_in_room.push(next);
        history.push(next.slug());
    }
    let expected: Vec<_> = private
        .into_iter()
        .filter(|t| *t != SeasonTopic::CoachingFit)
        .collect();
    assert_eq!(asked_in_room, expected);
    assert_eq!(
        SeasonTopic::CoachingFit.visibility(),
        TopicVisibility::DmOnly
    );
}
