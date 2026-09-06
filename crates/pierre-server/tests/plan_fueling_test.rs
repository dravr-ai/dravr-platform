// ABOUTME: Pins the fuelling protocol end to end — formatting, persistence, the calendar note, the prompt
// ABOUTME: The coaches emitted this payload for months while nothing rendered or stored it; these keep it landed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The ultra and heat builder coaches attach a `fueling_protocol` to every
//! long session, and the platform validates it against the structured-workout
//! schema — but until 2026-08-30 `PlannedDay` had no field for it and neither
//! plan card rendered it, so the prescription was discarded on the way to the
//! athlete. Each test here asserts a concrete rendered value, because the
//! failure mode being guarded against is silence: a shape that parses, stores
//! and displays nothing looks exactly like success.

use chrono::{NaiveDate, Utc};
use pierre_contremaitre::TrainingCatalogueRegistry;
use pierre_core::models::FuelingProtocol;
use pierre_memory::training_plans::{
    GoalRace, PlanStatus, PlanWeek, PlannedDay, RacePriority, TrainingPlan, WeekStatus,
};
use pierre_services::plan_calendar_push::plan_day_session;
use pierre_services::training_plan_render::render_training_plan_block;
use uuid::Uuid;

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

/// A ride the ultra coach would attach a full protocol to.
fn fuelled_day() -> PlannedDay {
    PlannedDay {
        date: "2026-09-05".to_owned(),
        sport: "gravel".to_owned(),
        workout: "4h endurance with the last hour at tempo".to_owned(),
        duration_min: Some(240),
        intensity: "Z2".to_owned(),
        steps: Vec::new(),
        fueling: Some(FuelingProtocol {
            carbs_g_per_h: 90.0,
            fluid_ml_per_h: 700.0,
            sodium_mg_per_h: Some(600.0),
            carb_source: Some("glucose:fructose 1:0.8".to_owned()),
        }),
        template_slug: None,
        template_params: None,
    }
}

fn plan() -> TrainingPlan {
    TrainingPlan {
        id: "plan-1".to_owned(),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        coach_slug: None,
        goal_fact_id: None,
        goal_race: GoalRace {
            name: "Harricana".to_owned(),
            date: "2026-09-12".to_owned(),
            discipline: "trail".to_owned(),
            priority: RacePriority::A,
        },
        races: Vec::new(),
        strategy: "build then taper".to_owned(),
        flavour: None,
        season_start: None,
        season_end: None,
        phases: Vec::new(),
        status: PlanStatus::Active,
        supersedes_id: None,
        source_conversation_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn week_with(days: Vec<PlannedDay>) -> PlanWeek {
    PlanWeek {
        id: "week-1".to_owned(),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        plan_id: "plan-1".to_owned(),
        week_start: "2026-08-31".to_owned(),
        focus: "last big week".to_owned(),
        days,
        status: WeekStatus::Active,
        supersedes_id: None,
        adjustment_reason: String::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        phase_index: None,
    }
}

/// Sodium is worded as a loss, and simply absent when nothing measured it.
///
/// The wording is the assertion, not decoration. Hew-Butler 2008 — the source
/// the coaches used to cite here — found hyponatremia is driven by fluid
/// intake above sweat rate and that sodium supplementation does not prevent
/// it, so a prescribed mg/h target inverts the evidence.
#[test]
fn summary_calls_sodium_a_loss_and_omits_it_when_unmeasured() {
    let full = FuelingProtocol {
        carbs_g_per_h: 90.0,
        fluid_ml_per_h: 700.0,
        sodium_mg_per_h: Some(600.0),
        carb_source: Some("glucose:fructose 1:0.8".to_owned()),
    };
    assert_eq!(
        full.summary(),
        "90 g/h carbs · 700 ml/h fluid · ~600 mg/h sodium lost · glucose:fructose 1:0.8"
    );

    let unmeasured = FuelingProtocol {
        carbs_g_per_h: 60.0,
        fluid_ml_per_h: 500.0,
        sodium_mg_per_h: None,
        carb_source: None,
    };
    assert_eq!(unmeasured.summary(), "60 g/h carbs · 500 ml/h fluid");
    assert!(
        !unmeasured.summary().contains("sodium"),
        "an unmeasured sodium loss must be absent, never rendered as zero"
    );
}

/// A saved plan keeps the prescription. This is the half that had no field at
/// all: the coach prescribed, the plan stored, and the fuelling vanished.
#[test]
fn a_planned_day_round_trips_its_fuelling_protocol() {
    let json = serde_json::to_string(&fuelled_day()).unwrap();
    let back: PlannedDay = serde_json::from_str(&json).unwrap();

    let fuel = back
        .fueling
        .expect("the fuelling protocol must survive a save/load cycle");
    assert!((fuel.carbs_g_per_h - 90.0).abs() < f32::EPSILON);
    assert!((fuel.fluid_ml_per_h - 700.0).abs() < f32::EPSILON);
    assert_eq!(fuel.sodium_mg_per_h, Some(600.0));
    assert_eq!(fuel.carb_source.as_deref(), Some("glucose:fructose 1:0.8"));
}

/// A day that needs no fuelling stores no key for it, rather than a null.
#[test]
fn a_day_without_fuelling_writes_no_fuelling_key() {
    let mut day = fuelled_day();
    day.fueling = None;
    let json = serde_json::to_string(&day).unwrap();
    assert!(
        !json.contains("fueling"),
        "an absent protocol must not serialise a key: {json}"
    );
}

/// The athlete's own calendar is where the protocol has to arrive.
///
/// Every provider calendar renders the note, so this is the one surface that
/// reaches the athlete on the day, off their phone, mid-ride.
#[test]
fn the_calendar_note_carries_the_fuelling_line() {
    let session = plan_day_session(Uuid::new_v4(), &fuelled_day(), 0)
        .expect("a fuelled training day is not a rest day");

    assert!(
        session.notes.contains("90 g/h carbs"),
        "the calendar note must carry the carbohydrate rate:\n{}",
        session.notes
    );
    assert!(
        session.notes.contains("~600 mg/h sodium lost"),
        "the calendar note must word sodium as a loss:\n{}",
        session.notes
    );
    assert!(
        session.notes.contains("4h endurance"),
        "the coach's own prose must survive alongside it:\n{}",
        session.notes
    );
}

/// A rest day has nothing to fuel and produces no session at all.
#[test]
fn a_rest_day_produces_no_session_to_fuel() {
    let rest = PlannedDay {
        date: "2026-09-06".to_owned(),
        sport: "rest".to_owned(),
        workout: "off".to_owned(),
        duration_min: None,
        intensity: String::new(),
        steps: Vec::new(),
        fueling: None,
        template_slug: None,
        template_params: None,
    };
    assert!(plan_day_session(Uuid::new_v4(), &rest, 0).is_none());
}

/// The coach reads its own prescription back on the next turn.
///
/// Without this the model re-invents a rate every time it is asked, which is
/// how three different carbohydrate ceilings came to ship at once.
#[test]
fn the_prompt_renders_the_fuelling_clause() {
    let week = week_with(vec![fuelled_day()]);
    let out = render_training_plan_block(
        &plan(),
        &[week],
        date("2026-09-03"),
        &TrainingCatalogueRegistry::new(),
    )
    .expect("an active plan renders a block");

    assert!(
        out.contains("fuel: 90 g/h carbs"),
        "the prompt must carry the fuelling clause so the coach quotes its own \
         prescription instead of improvising a new one:\n{out}"
    );
    assert!(
        out.contains("~600 mg/h sodium lost"),
        "sodium reaches the prompt worded as a loss:\n{out}"
    );
}

/// A day with no protocol renders no empty clause.
#[test]
fn a_day_without_fuelling_renders_no_clause() {
    let mut day = fuelled_day();
    day.fueling = None;
    let out = render_training_plan_block(
        &plan(),
        &[week_with(vec![day])],
        date("2026-09-03"),
        &TrainingCatalogueRegistry::new(),
    )
    .expect("an active plan renders a block");

    assert!(
        !out.contains("fuel:"),
        "an unfuelled day must render no clause at all:\n{out}"
    );
}
