// ABOUTME: Integration tests for TrainingPlanRepository — roundtrip, supersession chains, isolation
// ABOUTME: Content-asserting per the anti-stub rule: real day values, not is_ok() smoke checks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
#[cfg(feature = "postgresql")]
use pierre_config::environment::PostgresPoolConfig;
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_database::repositories::{SavePlanWeekParams, SaveTrainingPlanParams};
use pierre_memory::training_plans::{
    BlockPhase, GoalRace, PlanBlock, PlanStatus, PlannedDay, RacePriority, WeekStatus,
};
use uuid::Uuid;

async fn open_in_memory_db() -> Result<Database> {
    let encryption_key = generate_encryption_key().to_vec();
    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await?;
    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key).await?;
    Ok(db)
}

fn big_red() -> GoalRace {
    GoalRace {
        name: "Big Red".to_owned(),
        date: "2026-08-08".to_owned(),
        discipline: "gravel".to_owned(),
        priority: RacePriority::A,
    }
}

fn blocks() -> Vec<PlanBlock> {
    vec![
        PlanBlock {
            phase: BlockPhase::Build,
            start: "2026-07-13".to_owned(),
            weeks: 3,
            intent: "volume back up, one moderate day per week".to_owned(),
            target_hours: Some(9.0),
        },
        PlanBlock {
            phase: BlockPhase::Taper,
            start: "2026-08-03".to_owned(),
            weeks: 1,
            intent: "shorter, sharper, more rest".to_owned(),
            target_hours: Some(5.0),
        },
    ]
}

fn week_days(monday: &str) -> Vec<PlannedDay> {
    // Two real days + a rest day is enough to assert content fidelity.
    vec![
        PlannedDay {
            date: monday.to_owned(),
            sport: "rest".to_owned(),
            workout: "off — legs up".to_owned(),
            duration_min: None,
            intensity: String::new(),
        },
        PlannedDay {
            date: "2026-07-14".to_owned(),
            sport: "gravel".to_owned(),
            workout: "tempo 3x8min".to_owned(),
            duration_min: Some(60),
            intensity: "3x8min @ 88-93% FTP".to_owned(),
        },
        PlannedDay {
            date: "2026-07-15".to_owned(),
            sport: "mtb".to_owned(),
            workout: "endurance, low HR on climbs".to_owned(),
            duration_min: Some(105),
            intensity: "Z2".to_owned(),
        },
    ]
}

fn plan_params<'a>(
    tenant: &'a str,
    user: &'a str,
    race: &'a GoalRace,
    blocks: &'a [PlanBlock],
) -> SaveTrainingPlanParams<'a> {
    SaveTrainingPlanParams {
        tenant_id: tenant,
        user_id: user,
        coach_slug: Some("endurance-coach"),
        goal_fact_id: Some("fact-goal-1"),
        goal_race: race,
        races: &[],
        strategy: "rest week done; rebuild volume, then race-specific tempo, taper into Aug 8",
        blocks,
        source_conversation_id: Some("conv-1"),
    }
}

#[tokio::test]
async fn save_and_get_roundtrip_preserves_content() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = blocks();
    let plan = repos
        .training_plans
        .save_training_plan(&plan_params(&tenant, &user, &race, &blks))
        .await?;

    let days = week_days("2026-07-13");
    let week = repos
        .training_plans
        .save_plan_week(&SavePlanWeekParams {
            tenant_id: &tenant,
            user_id: &user,
            plan_id: &plan.id,
            week_start: "2026-07-13",
            focus: "volume back up",
            days: &days,
            adjustment_reason: "",
        })
        .await?;
    assert_eq!(week.status, WeekStatus::Active);
    assert_eq!(week.supersedes_id, None);

    let fetched = repos
        .training_plans
        .get_active_plan(&tenant, &user, Some("endurance-coach"))
        .await?
        .expect("active plan");
    assert_eq!(fetched.id, plan.id);
    assert_eq!(fetched.goal_race.name, "Big Red");
    assert_eq!(fetched.goal_race.date, "2026-08-08");
    assert_eq!(fetched.goal_race.priority, RacePriority::A);
    assert_eq!(fetched.blocks.len(), 2);
    assert_eq!(fetched.blocks[0].phase, BlockPhase::Build);
    assert_eq!(fetched.blocks[1].phase, BlockPhase::Taper);
    assert_eq!(fetched.blocks[0].target_hours, Some(9.0));
    assert!(fetched.strategy.contains("taper into Aug 8"));
    assert_eq!(fetched.goal_fact_id.as_deref(), Some("fact-goal-1"));
    assert_eq!(fetched.status, PlanStatus::Active);

    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &plan.id, false)
        .await?;
    assert_eq!(weeks.len(), 1);
    assert_eq!(weeks[0].days.len(), 3);
    assert!(weeks[0].days[0].is_rest());
    assert_eq!(weeks[0].days[1].intensity, "3x8min @ 88-93% FTP");
    assert_eq!(weeks[0].days[2].duration_min, Some(105));
    assert_eq!(weeks[0].focus, "volume back up");
    Ok(())
}

#[tokio::test]
async fn outline_resave_supersedes_previous() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = blocks();
    let first = repos
        .training_plans
        .save_training_plan(&plan_params(&tenant, &user, &race, &blks))
        .await?;

    // Goal moved: new outline replaces the old one atomically.
    let moved = GoalRace {
        date: "2026-08-15".to_owned(),
        ..big_red()
    };
    let second = repos
        .training_plans
        .save_training_plan(&plan_params(&tenant, &user, &moved, &blks))
        .await?;

    assert_eq!(second.supersedes_id.as_deref(), Some(first.id.as_str()));
    let active = repos
        .training_plans
        .get_active_plan(&tenant, &user, Some("endurance-coach"))
        .await?
        .expect("active plan");
    assert_eq!(active.id, second.id);
    assert_eq!(active.goal_race.date, "2026-08-15");
    Ok(())
}

#[tokio::test]
async fn week_resave_supersedes_that_week_only() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = blocks();
    let plan = repos
        .training_plans
        .save_training_plan(&plan_params(&tenant, &user, &race, &blks))
        .await?;

    let days1 = week_days("2026-07-13");
    let w1 = repos
        .training_plans
        .save_plan_week(&SavePlanWeekParams {
            tenant_id: &tenant,
            user_id: &user,
            plan_id: &plan.id,
            week_start: "2026-07-13",
            focus: "volume",
            days: &days1,
            adjustment_reason: "",
        })
        .await?;
    let days2 = week_days("2026-07-20");
    repos
        .training_plans
        .save_plan_week(&SavePlanWeekParams {
            tenant_id: &tenant,
            user_id: &user,
            plan_id: &plan.id,
            week_start: "2026-07-20",
            focus: "tempo",
            days: &days2,
            adjustment_reason: "",
        })
        .await?;

    // "Move Tuesday to Wednesday": whole-week re-save of week 1 only.
    let mut adjusted = week_days("2026-07-13");
    adjusted[1].date = "2026-07-15".to_owned();
    adjusted[2].date = "2026-07-14".to_owned();
    let w1b = repos
        .training_plans
        .save_plan_week(&SavePlanWeekParams {
            tenant_id: &tenant,
            user_id: &user,
            plan_id: &plan.id,
            week_start: "2026-07-13",
            focus: "volume",
            days: &adjusted,
            adjustment_reason: "tempo moved to Wednesday — legs heavy Tuesday",
        })
        .await?;
    assert_eq!(w1b.supersedes_id.as_deref(), Some(w1.id.as_str()));

    let active = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &plan.id, false)
        .await?;
    assert_eq!(active.len(), 2, "one active row per calendar week");
    assert_eq!(active[0].id, w1b.id, "week 1 is the adjusted row");
    assert_eq!(
        active[0].adjustment_reason,
        "tempo moved to Wednesday — legs heavy Tuesday"
    );
    assert_eq!(active[0].days[1].date, "2026-07-15");
    assert_eq!(active[1].focus, "tempo", "week 2 untouched");

    let with_history = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &plan.id, true)
        .await?;
    assert_eq!(with_history.len(), 3, "superseded week kept as audit trail");
    let superseded: Vec<_> = with_history
        .iter()
        .filter(|w| w.status == WeekStatus::Superseded)
        .collect();
    assert_eq!(superseded.len(), 1);
    assert_eq!(superseded[0].id, w1.id);
    Ok(())
}

#[tokio::test]
async fn tenant_and_user_isolation_enforced() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = blocks();
    let plan = repos
        .training_plans
        .save_training_plan(&plan_params(&tenant_a, &user, &race, &blks))
        .await?;

    // Another tenant must see nothing, even for the same user id.
    assert!(repos
        .training_plans
        .get_active_plan(&tenant_b, &user, Some("endurance-coach"))
        .await?
        .is_none());

    // A week can never attach across the tenant boundary.
    let days = week_days("2026-07-13");
    let cross = repos
        .training_plans
        .save_plan_week(&SavePlanWeekParams {
            tenant_id: &tenant_b,
            user_id: &user,
            plan_id: &plan.id,
            week_start: "2026-07-13",
            focus: "volume",
            days: &days,
            adjustment_reason: "",
        })
        .await;
    assert!(cross.is_err(), "cross-tenant week save must be rejected");
    Ok(())
}

#[tokio::test]
async fn coach_scoped_plan_prefers_specific_over_agnostic() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = blocks();
    let mut agnostic = plan_params(&tenant, &user, &race, &blks);
    agnostic.coach_slug = None;
    repos.training_plans.save_training_plan(&agnostic).await?;

    // A coach without their own plan falls back to the agnostic one.
    let seen = repos
        .training_plans
        .get_active_plan(&tenant, &user, Some("endurance-coach"))
        .await?
        .expect("agnostic fallback");
    assert_eq!(seen.coach_slug, None);

    // Once the coach saves their own, it wins.
    let specific = repos
        .training_plans
        .save_training_plan(&plan_params(&tenant, &user, &race, &blks))
        .await?;
    let seen = repos
        .training_plans
        .get_active_plan(&tenant, &user, Some("endurance-coach"))
        .await?
        .expect("coach-specific plan");
    assert_eq!(seen.id, specific.id);
    assert_eq!(seen.coach_slug.as_deref(), Some("endurance-coach"));
    Ok(())
}
