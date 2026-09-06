// ABOUTME: The vision half of a training plan — flavour with provenance, season window, phases with targets,
// ABOUTME: a week's phase index and a day's template — survives storage and reaches the prompt as a phase header
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Annual Vision Phase 2, slice 1: the plan outline grown into the vision.
//! Storage round trip on `SQLite` through the repository, then the prompt
//! render — the phase header names the current phase's targets and the
//! catalogue templates that fit it, from the seeded registry.

use std::collections::BTreeMap;

use anyhow::Result;
use chrono::NaiveDate;
use pierre_contremaitre::TrainingCatalogueRegistry;
use pierre_core::models::periodization::{
    FlavourFamily, LoadingPattern, PhaseKind, Sequencing, Share, TidTarget, WorkoutPurpose,
};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_memory::training_plans::{
    FlavourSelection, GoalRace, PlanPhase, PlanStatus, PlannedDay, RacePriority, SelectedBy,
    TemplateParams, TrainingPlan,
};
use pierre_services::training_plan_render::render_training_plan_block;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_default()
}

fn goal() -> GoalRace {
    GoalRace {
        name: "Harricana 65".to_owned(),
        date: "2026-10-10".to_owned(),
        discipline: "trail".to_owned(),
        priority: RacePriority::A,
    }
}

fn flavour() -> FlavourSelection {
    FlavourSelection {
        id: "polarized-classic".to_owned(),
        family: FlavourFamily::Polarized,
        sequencing: Sequencing::Linear,
        modifiers: Vec::new(),
        selected_by: SelectedBy::Coach,
        override_reason: Some("she wants the two hard days she is used to".to_owned()),
    }
}

/// A build phase with every target stated, the way `build_skeleton` will
/// state them, followed by a taper stated by hand.
fn phases() -> Vec<PlanPhase> {
    let mut mix = BTreeMap::new();
    mix.insert(WorkoutPurpose::Vo2maxLong, 2);
    mix.insert(WorkoutPurpose::EnduranceLong, 3);
    mix.insert(WorkoutPurpose::Endurance, 4);
    vec![
        PlanPhase {
            kind: PhaseKind::Build,
            start: "2026-08-24".to_owned(),
            weeks: 5,
            purpose: "Race-specific intensity on a held aerobic base.".to_owned(),
            intent: "two hard days, long run grows to 2h30".to_owned(),
            target_hours: Some(9.0),
            volume_share_of_peak: Some(Share {
                min: 0.80,
                max: 1.0,
            }),
            tid_target: Some(TidTarget {
                z1: Share {
                    min: 0.75,
                    max: 0.85,
                },
                z2: Share {
                    min: 0.0,
                    max: 0.05,
                },
                z3: Share {
                    min: 0.15,
                    max: 0.20,
                },
            }),
            hard_sessions_max: Some(2),
            session_mix: mix,
            flavour_override: None,
            loading_pattern: Some(LoadingPattern {
                load_weeks: 3,
                recovery_weeks: 1,
            }),
            skeleton_id: Some("run-5k-10k".to_owned()),
        },
        PlanPhase {
            kind: PhaseKind::Taper,
            start: "2026-09-28".to_owned(),
            weeks: 2,
            purpose: String::new(),
            intent: "volume down, one sharp session kept".to_owned(),
            target_hours: Some(5.0),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: Some(1),
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
        },
    ]
}

fn day(
    date: &str,
    sport: &str,
    workout: &str,
    template: Option<(&str, TemplateParams)>,
) -> PlannedDay {
    let (template_slug, template_params) = match template {
        Some((slug, params)) => (Some(slug.to_owned()), Some(params)),
        None => (None, None),
    };
    PlannedDay {
        date: date.to_owned(),
        sport: sport.to_owned(),
        workout: workout.to_owned(),
        duration_min: Some(60),
        intensity: "Z2".to_owned(),
        steps: Vec::new(),
        fueling: None,
        template_slug,
        template_params,
    }
}

#[tokio::test]
async fn the_vision_round_trips_through_storage() -> Result<()> {
    let db = create_test_db().await?;
    let plans = db.repositories().training_plans;
    let selection = flavour();
    let outline_phases = phases();
    let params = TemplateParams {
        sets: None,
        reps: Some(4),
        work_seconds: Some(480),
        rest_seconds: Some(120),
        duration_minutes: None,
    };
    let days = vec![
        day("2026-08-24", "rest", "off", None),
        day(
            "2026-08-25",
            "run",
            "4 × 8 min at ~90% HRmax",
            Some(("vo2max_4x8", params)),
        ),
    ];
    let bundle = plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: "tenant-v",
            user_id: "user-v",
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(PlanOutlineInput {
                goal_race: &goal(),
                races: &[],
                strategy: "polarized build into a two-week taper",
                flavour: Some(&selection),
                season_start: Some("2026-08-24"),
                season_end: Some("2026-10-11"),
                phases: &outline_phases,
                source_conversation_id: None,
            }),
            weeks: &[PlanWeekInput {
                week_start: "2026-08-24",
                focus: "first build week",
                days: &days,
                adjustment_reason: "",
                phase_index: Some(0),
            }],
        })
        .await?;

    let fetched = plans
        .get_active_plan("tenant-v", "user-v", Some("endurance-coach"))
        .await?
        .expect("the outline just saved is the active plan");
    assert_eq!(fetched.id, bundle.plan.id);
    assert_eq!(fetched.flavour, Some(selection));
    assert_eq!(fetched.season_start.as_deref(), Some("2026-08-24"));
    assert_eq!(fetched.season_end.as_deref(), Some("2026-10-11"));
    assert_eq!(
        fetched.phases, outline_phases,
        "every phase target survives"
    );
    assert_eq!(fetched.phases[0].kind, PhaseKind::Build);
    assert_eq!(
        fetched.phases[0].loading_pattern.map(|p| p.to_string()),
        Some("3:1".to_owned())
    );
    assert_eq!(
        fetched.phases[0]
            .session_mix
            .get(&WorkoutPurpose::Vo2maxLong),
        Some(&2)
    );

    let weeks = plans
        .list_plan_weeks("tenant-v", "user-v", &fetched.id, false)
        .await?;
    assert_eq!(weeks.len(), 1);
    assert_eq!(weeks[0].phase_index, Some(0));
    assert_eq!(
        weeks[0].days[1].template_slug.as_deref(),
        Some("vo2max_4x8")
    );
    assert_eq!(weeks[0].days[1].template_params, Some(params));
    assert_eq!(weeks[0].days[0].template_slug, None);

    // A superseding outline carries the week forward with its phase index.
    let second = plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: "tenant-v",
            user_id: "user-v",
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(PlanOutlineInput {
                goal_race: &goal(),
                races: &[],
                strategy: "same season, one more recovery week",
                flavour: None,
                season_start: None,
                season_end: None,
                phases: &outline_phases,
                source_conversation_id: None,
            }),
            weeks: &[],
        })
        .await?;
    assert_eq!(
        second.superseded_plan_id.as_deref(),
        Some(bundle.plan.id.as_str())
    );
    let carried = plans
        .list_plan_weeks("tenant-v", "user-v", &second.plan.id, false)
        .await?;
    assert_eq!(carried.len(), 1, "the week followed the new outline");
    assert_eq!(carried[0].phase_index, Some(0));
    assert_eq!(
        second.plan.flavour, None,
        "a plan saved without a flavour has none"
    );
    Ok(())
}

#[tokio::test]
async fn the_prompt_carries_the_current_phase_header() -> Result<()> {
    let catalogue = TrainingCatalogueRegistry::new();
    let plan = TrainingPlan {
        id: "plan-v".to_owned(),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        coach_slug: Some("endurance-coach".to_owned()),
        goal_fact_id: None,
        goal_race: goal(),
        races: Vec::new(),
        strategy: "polarized build into a two-week taper".to_owned(),
        flavour: Some(flavour()),
        season_start: Some("2026-08-24".to_owned()),
        season_end: None,
        phases: phases(),
        status: PlanStatus::Active,
        supersedes_id: None,
        source_conversation_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let block = render_training_plan_block(&plan, &[], d("2026-09-02"), &catalogue)
        .expect("a plan renders");

    assert!(
        block.contains("Flavour: polarized-classic (polarized · linear), chosen by the coach — she wants the two hard days she is used to"),
        "flavour line with provenance: {block}"
    );
    assert!(
        block.contains("Season: 2026-08-24 to the goal race"),
        "season window: {block}"
    );
    assert!(
        block.contains("- [current] build × 5wk from 2026-08-24, ~9h/wk: two hard days, long run grows to 2h30 — Race-specific intensity on a held aerobic base."),
        "phase line with purpose: {block}"
    );
    assert!(
        block.contains("Current phase: build (1 of 2), 4 week(s) left — Race-specific intensity on a held aerobic base."),
        "phase header: {block}"
    );
    assert!(
        block.contains("Time-in-zone target: below LT1 75–85%, between 0–5%, above LT2 15–20%"),
        "TID target: {block}"
    );
    assert!(
        block.contains("Limits: at most 2 hard session(s) a week; loading 3:1; volume 80–100% of the peak week"),
        "limits: {block}"
    );
    // The seeded catalogue carries four vo2max_long templates that fit a build
    // phase; the header names them by weight order, heaviest purpose first.
    assert!(
        block.contains("- endurance (weight 4):"),
        "the heaviest purpose comes first: {block}"
    );
    assert!(
        block.contains("- vo2max_long (weight 2): "),
        "each purpose lists its carriers: {block}"
    );
    let vo2_line = block
        .lines()
        .find(|l| l.starts_with("- vo2max_long (weight 2): "))
        .unwrap_or_default();
    assert!(
        vo2_line.contains("vo2max_4x8") && vo2_line.contains("vo2_5x3"),
        "both long-interval templates fit a build phase: {vo2_line}"
    );
    Ok(())
}

#[tokio::test]
async fn a_phase_without_a_mix_lists_every_template_that_fits_it() -> Result<()> {
    let catalogue = TrainingCatalogueRegistry::new();
    let mut plan = TrainingPlan {
        id: "plan-t".to_owned(),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        coach_slug: None,
        goal_fact_id: None,
        goal_race: goal(),
        races: Vec::new(),
        strategy: "taper".to_owned(),
        flavour: None,
        season_start: None,
        season_end: None,
        phases: phases(),
        status: PlanStatus::Active,
        supersedes_id: None,
        source_conversation_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    plan.phases.remove(0);
    let block = render_training_plan_block(&plan, &[], d("2026-09-29"), &catalogue)
        .expect("a plan renders");
    assert!(
        block.contains("Current phase: taper (1 of 1), 2 week(s) left"),
        "header without a purpose: {block}"
    );
    assert!(
        block.contains("Templates that fit this phase (list_workout_templates phase=taper): "),
        "no mix means the whole fitting set is named: {block}"
    );
    assert!(
        block.contains("race_pace_long"),
        "the race-pace long session fits a taper in the seed: {block}"
    );
    assert!(
        !block.contains("Time-in-zone target"),
        "a phase stating no target renders none: {block}"
    );
    Ok(())
}
