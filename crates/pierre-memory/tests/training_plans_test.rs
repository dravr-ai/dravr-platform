// ABOUTME: Unit tests for training-plan types — statuses, serialization and plan dates
// ABOUTME: Pins the LLM-facing payload shape and strict YYYY-MM-DD date parsing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_core::models::periodization::PhaseKind;
use pierre_memory::training_plans::{
    parse_plan_date, GoalRace, PlanPhase, PlanStatus, PlannedDay, RacePriority, SelectedBy,
    WeekStatus,
};
use std::collections::BTreeMap;

#[test]
fn statuses_roundtrip_through_db_strings() {
    for status in [
        PlanStatus::Active,
        PlanStatus::Superseded,
        PlanStatus::Completed,
        PlanStatus::Abandoned,
    ] {
        assert_eq!(PlanStatus::parse(status.as_str()), Some(status));
    }
    for status in [WeekStatus::Active, WeekStatus::Superseded] {
        assert_eq!(WeekStatus::parse(status.as_str()), Some(status));
    }
    assert_eq!(PlanStatus::parse("bogus"), None);
    assert_eq!(WeekStatus::parse("completed"), None);
}

#[test]
fn goal_race_and_blocks_serialize_stably() {
    let race = GoalRace {
        name: "Big Red".to_owned(),
        date: "2026-08-08".to_owned(),
        discipline: "gravel".to_owned(),
        priority: RacePriority::A,
    };
    let json = serde_json::to_string(&race).unwrap_or_default();
    assert!(json.contains("\"priority\":\"A\""));
    let back = serde_json::from_str::<GoalRace>(&json).ok();
    assert_eq!(back, Some(race));

    let phase = PlanPhase {
        kind: PhaseKind::Build,
        start: "2026-07-13".to_owned(),
        weeks: 2,
        purpose: String::new(),
        intent: "volume back, one moderate day/week".to_owned(),
        target_hours: Some(9.5),
        volume_share_of_peak: None,
        tid_target: None,
        hard_sessions_max: Some(2),
        session_mix: BTreeMap::new(),
        flavour_override: None,
        loading_pattern: None,
        skeleton_id: None,
    };
    let json = serde_json::to_string(&phase).unwrap_or_default();
    assert!(json.contains("\"kind\":\"build\""));
    assert!(
        !json.contains("session_mix"),
        "an empty mix is not serialized"
    );
    let back = serde_json::from_str::<PlanPhase>(&json).ok();
    assert_eq!(back.as_ref().map(|p| p.kind), Some(PhaseKind::Build));
    assert_eq!(back.as_ref().and_then(|p| p.target_hours), Some(9.5));
    assert_eq!(back.and_then(|p| p.hard_sessions_max), Some(2));
    assert_eq!(
        phase.end_exclusive(),
        parse_plan_date("2026-07-27"),
        "two weeks from a Monday ends on the third Monday"
    );
    assert!(phase.covers(parse_plan_date("2026-07-26").unwrap_or_default()));
    assert!(!phase.covers(parse_plan_date("2026-07-27").unwrap_or_default()));
    assert_eq!(SelectedBy::Coach.as_str(), "coach");
}

#[test]
fn rest_day_detection_and_optional_fields() {
    let rest = PlannedDay {
        date: "2026-07-13".to_owned(),
        sport: "Rest".to_owned(),
        workout: "off — legs up".to_owned(),
        duration_min: None,
        intensity: String::new(),
        steps: Vec::new(),
        fueling: None,
        template_slug: None,
        template_params: None,
        template_source: None,
    };
    assert!(rest.is_rest());
    // duration_min: None must not serialize a null (schema hygiene for
    // the LLM-facing tool payloads).
    let json = serde_json::to_string(&rest).unwrap_or_default();
    assert!(!json.is_empty());
    assert!(!json.contains("duration_min"));

    let ride = serde_json::from_str::<PlannedDay>(
        r#"{"date":"2026-07-14","sport":"mtb","workout":"2h endurance","duration_min":120,"intensity":"Z2"}"#,
    )
    .ok();
    assert_eq!(ride.as_ref().map(PlannedDay::is_rest), Some(false));
    assert_eq!(ride.and_then(|d| d.duration_min), Some(120));
}

#[test]
fn plan_dates_parse_strictly() {
    assert!(parse_plan_date("2026-08-08").is_some());
    assert!(parse_plan_date("08/08/2026").is_none());
    assert!(parse_plan_date("2026-8-8").is_none());
    assert!(parse_plan_date("tomorrow").is_none());
}
