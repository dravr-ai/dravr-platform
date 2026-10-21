// ABOUTME: Pins that a plan card's weekly target hours reach the client at f32 precision
// ABOUTME: as_block used serde_json::to_value, which widened 8.3 hours to 8.300000190734863
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#532: `PlanCard::as_block` built the reply block with
//! `serde_json::to_value(self)`, and `PhaseCard::target_hours` is an `f32`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::periodization::PhaseKind;
use pierre_memory::training_plans::{GoalRace, RacePriority};
use pierre_services::plan_card::{PhaseCard, PlanCard};

#[test]
fn a_phase_target_reads_8_3_hours_not_its_f64_widening() {
    let card = PlanCard {
        goal_race: GoalRace {
            name: "Gran Fondo".to_owned(),
            date: "2027-06-06".to_owned(),
            discipline: "cycling".to_owned(),
            priority: RacePriority::A,
        },
        races: vec![],
        season_start: None,
        season_end: None,
        flavour: None,
        phases: vec![PhaseCard {
            kind: PhaseKind::Base,
            start: "2026-10-05".to_owned(),
            end: None,
            weeks: 8,
            purpose: "aerobic base".to_owned(),
            intent: "volume first".to_owned(),
            target_hours: Some(8.3),
            hard_sessions_max: Some(1),
            current: true,
        }],
        current_phase_index: Some(0),
        weeks: vec![],
        weeks_deferred: 0,
    };
    let text = card.as_block("create_training_plan").unwrap().to_string();
    assert!(text.contains(r#""target_hours":8.3"#), "{text}");
    assert!(
        !text.contains("8.30000"),
        "plan card widened an f32: {text}"
    );
}
