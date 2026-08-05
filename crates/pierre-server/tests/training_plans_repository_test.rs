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
use pierre_database::repositories::{
    PlanOutlineInput, PlanWeekInput, SavePlanBundleParams, SaveTrainingPlanParams,
};
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
    let saved = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume back up",
                days: &days,
                adjustment_reason: "",
            }],
        })
        .await?;
    assert_eq!(saved.plan.id, plan.id);
    assert_eq!(saved.weeks.len(), 1);
    assert_eq!(saved.weeks[0].status, WeekStatus::Active);
    assert_eq!(saved.weeks[0].supersedes_id, None);

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
    let days2 = week_days("2026-07-20");
    let first = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: None,
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-07-13",
                    focus: "volume",
                    days: &days1,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: "2026-07-20",
                    focus: "tempo",
                    days: &days2,
                    adjustment_reason: "",
                },
            ],
        })
        .await?;
    assert_eq!(first.plan.id, plan.id);
    let w1 = &first.weeks[0];

    // "Move Tuesday to Wednesday": whole-week re-save of week 1 only.
    let mut adjusted = week_days("2026-07-13");
    adjusted[1].date = "2026-07-15".to_owned();
    adjusted[2].date = "2026-07-14".to_owned();
    let resaved = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &adjusted,
                adjustment_reason: "tempo moved to Wednesday — legs heavy Tuesday",
            }],
        })
        .await?;
    let w1b = &resaved.weeks[0];
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

    // A week can never attach across the tenant boundary: the save resolves
    // the plan itself, tenant-scoped, so tenant B reaches no plan at all.
    let days = week_days("2026-07-13");
    let cross = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant_b,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &days,
                adjustment_reason: "",
            }],
        })
        .await;
    let Err(err) = cross else {
        panic!("cross-tenant week save must be rejected");
    };
    assert!(
        err.to_string().contains("no active plan"),
        "unexpected error: {err}"
    );

    // And nothing landed on tenant A's plan.
    let untouched = repos
        .training_plans
        .list_plan_weeks(&tenant_a, &user, &plan.id, true)
        .await?;
    assert!(untouched.is_empty(), "tenant A's plan must be untouched");
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

#[tokio::test]
async fn a_coach_bound_plan_is_invisible_to_a_coachless_lookup() -> Result<()> {
    // The fallback runs one way only: `coach_slug IN (?3, '')` with no coach
    // binds `?3` to `''`, so it matches coach-agnostic plans alone. Every plan
    // `save_training_plan` writes from a conversation carries that
    // conversation's coach, so a caller that asks without one sees nothing and
    // concludes the athlete has no plan — which is what told the athletes most
    // likely to hold one, at the end of a calibration interview, to go build it.
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = blocks();
    let coach_bound = repos
        .training_plans
        .save_training_plan(&plan_params(&tenant, &user, &race, &blks))
        .await?;
    assert_eq!(coach_bound.coach_slug.as_deref(), Some("endurance-coach"));

    assert!(
        repos
            .training_plans
            .get_active_plan(&tenant, &user, None)
            .await?
            .is_none(),
        "a coachless lookup must not be relied on to find a coach-bound plan"
    );

    let seen = repos
        .training_plans
        .get_active_plan(&tenant, &user, Some("endurance-coach"))
        .await?
        .expect("the conversation's own coach finds it");
    assert_eq!(seen.id, coach_bound.id);
    assert_eq!(seen.goal_race.name, "Big Red");
    Ok(())
}

fn outline_input<'a>(race: &'a GoalRace, blocks: &'a [PlanBlock]) -> PlanOutlineInput<'a> {
    PlanOutlineInput {
        goal_race: race,
        races: &[],
        strategy: "rebuild volume, race-specific tempo, taper into Aug 8",
        blocks,
        source_conversation_id: Some("conv-1"),
    }
}

#[tokio::test]
async fn bundle_saves_outline_and_weeks_in_one_call() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = blocks();
    let days = week_days("2026-07-13");

    let saved = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: Some("fact-goal-1"),
            outline: Some(outline_input(&race, &blks)),
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume back up",
                days: &days,
                adjustment_reason: "",
            }],
        })
        .await?;
    assert_eq!(saved.weeks.len(), 1);
    assert!(saved.superseded_plan_id.is_none());
    assert_eq!(saved.plan.goal_race.name, "Big Red");

    // Both the outline and the week are durably present.
    let got = repos
        .training_plans
        .get_active_plan(&tenant, &user, Some("endurance-coach"))
        .await?
        .expect("plan present");
    assert_eq!(got.id, saved.plan.id);
    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &got.id, false)
        .await?;
    assert_eq!(weeks.len(), 1);
    assert_eq!(weeks[0].days[1].intensity, "3x8min @ 88-93% FTP");
    Ok(())
}

#[tokio::test]
async fn bundle_weeks_only_attaches_to_active_plan() -> Result<()> {
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
    let saved = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &days,
                adjustment_reason: "",
            }],
        })
        .await?;
    assert_eq!(saved.plan.id, plan.id, "weeks attach to the active plan");
    assert_eq!(saved.weeks.len(), 1);
    Ok(())
}

#[tokio::test]
async fn bundle_weeks_only_with_no_plan_errors_and_writes_nothing() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let days = week_days("2026-07-13");

    let result = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &days,
                adjustment_reason: "",
            }],
        })
        .await;
    assert!(result.is_err(), "weeks with no active plan must error");

    // The transaction rolled back: no plan, no orphan week.
    assert!(repos
        .training_plans
        .get_active_plan(&tenant, &user, Some("endurance-coach"))
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn outline_resave_carries_the_active_weeks_onto_the_new_plan() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = blocks();

    let d1 = week_days("2026-07-13");
    let d2 = week_days("2026-07-20");
    let d3 = week_days("2026-07-27");
    let first = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-07-13",
                    focus: "volume back up",
                    days: &d1,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: "2026-07-20",
                    focus: "race-specific tempo",
                    days: &d2,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: "2026-07-27",
                    focus: "peak then unload",
                    days: &d3,
                    adjustment_reason: "",
                },
            ],
        })
        .await?;
    assert_eq!(first.weeks.len(), 3);

    // The goal moved, so the coach re-saves the OUTLINE alone — the encouraged
    // "adjust one part of the plan" call. The day-by-day schedule must follow.
    let moved = GoalRace {
        date: "2026-08-15".to_owned(),
        ..big_red()
    };
    let second = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&moved, &blks)),
            weeks: &[],
        })
        .await?;
    assert_ne!(second.plan.id, first.plan.id);

    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &second.plan.id, false)
        .await?;
    assert_eq!(weeks.len(), 3, "every week follows the new outline");
    assert_eq!(weeks[0].week_start, "2026-07-13");
    assert_eq!(weeks[1].week_start, "2026-07-20");
    assert_eq!(weeks[2].week_start, "2026-07-27");
    assert_eq!(weeks[1].focus, "race-specific tempo");
    assert_eq!(weeks[2].focus, "peak then unload");
    assert_eq!(weeks[1].status, WeekStatus::Active);

    // Content, not just count: the day rows survive the re-parenting verbatim.
    assert_eq!(weeks[1].days.len(), 3);
    assert!(weeks[1].days[0].is_rest());
    assert_eq!(weeks[1].days[1].workout, "tempo 3x8min");
    assert_eq!(weeks[1].days[1].intensity, "3x8min @ 88-93% FTP");
    assert_eq!(weeks[1].days[2].duration_min, Some(105));

    // Each carried row supersedes the row it replaces, so the audit chain from
    // the new outline back to the old one is unbroken.
    let carried: Vec<_> = weeks
        .iter()
        .map(|w| w.supersedes_id.clone().unwrap_or_default())
        .collect();
    let originals: Vec<_> = first.weeks.iter().map(|w| w.id.clone()).collect();
    assert_eq!(
        carried, originals,
        "carried weeks chain back to the originals"
    );

    // Nothing active is stranded on the superseded outline.
    let stranded = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &first.plan.id, false)
        .await?;
    assert!(
        stranded.is_empty(),
        "no active week left on the superseded outline"
    );
    let history = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &first.plan.id, true)
        .await?;
    assert_eq!(history.len(), 3, "the old rows stay as audit trail");
    assert!(history.iter().all(|w| w.status == WeekStatus::Superseded));
    Ok(())
}

/// Two consecutive outline re-saves, which is how a plan actually loses weeks.
///
/// Carrying weeks forward across a *single* supersede is covered above. The
/// shape that stranded a real athlete's schedule was three outlines in a row:
/// the second save re-outlined and wrote its own weeks, the third re-outlined
/// again and wrote only the first two of them. Every week the second plan held
/// but the third did not re-send has to arrive on the third plan, or the build
/// and taper the athlete was promised become unreachable while the outline
/// still advertises those phases.
#[tokio::test]
async fn two_consecutive_outline_resaves_strand_no_week() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let blks = blocks();

    // Plan A — the first goal, with the two weeks around it.
    let a1 = week_days("2026-07-28");
    let a2 = week_days("2026-08-04");
    let race_a = big_red();
    let plan_a = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&race_a, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-07-28",
                    focus: "race week",
                    days: &a1,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: "2026-08-04",
                    focus: "post-race easy",
                    days: &a2,
                    adjustment_reason: "",
                },
            ],
        })
        .await?;
    assert_eq!(plan_a.weeks.len(), 2);

    // Plan B — the goal changes and the coach lays out the whole run to it.
    let b1 = week_days("2026-08-10");
    let b2 = week_days("2026-08-17");
    let b3 = week_days("2026-08-24");
    let race_b = GoalRace {
        name: "Harricana".to_owned(),
        date: "2026-09-11".to_owned(),
        ..big_red()
    };
    let plan_b = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&race_b, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-08-10",
                    focus: "reintroduction",
                    days: &b1,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: "2026-08-17",
                    focus: "build",
                    days: &b2,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: "2026-08-24",
                    focus: "peak",
                    days: &b3,
                    adjustment_reason: "",
                },
            ],
        })
        .await?;
    assert_ne!(plan_b.plan.id, plan_a.plan.id);

    // Plan C — the same outline again, re-sending only the first two weeks.
    // `2026-08-24` is not in this payload and exists only on plan B.
    let c1 = week_days("2026-08-10");
    let c2 = week_days("2026-08-17");
    let plan_c = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&race_b, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-08-10",
                    focus: "reintroduction, revised",
                    days: &c1,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: "2026-08-17",
                    focus: "build, revised",
                    days: &c2,
                    adjustment_reason: "",
                },
            ],
        })
        .await?;
    assert_ne!(plan_c.plan.id, plan_b.plan.id);

    // Every week ever written is reachable from the plan the athlete now reads.
    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &plan_c.plan.id, false)
        .await?;
    let starts: Vec<&str> = weeks.iter().map(|w| w.week_start.as_str()).collect();
    assert_eq!(
        starts,
        vec![
            "2026-07-28",
            "2026-08-04",
            "2026-08-10",
            "2026-08-17",
            "2026-08-24"
        ],
        "a week the third outline did not re-send must still arrive on it"
    );
    assert!(weeks.iter().all(|w| w.status == WeekStatus::Active));

    // The re-sent weeks carry the revision, and the week that only ever lived on
    // plan B keeps its original content rather than an empty placeholder.
    assert_eq!(weeks[2].focus, "reintroduction, revised");
    assert_eq!(weeks[3].focus, "build, revised");
    assert_eq!(weeks[4].focus, "peak");
    assert_eq!(weeks[4].days.len(), 3);
    assert_eq!(weeks[4].days[1].workout, "tempo 3x8min");
    assert_eq!(weeks[4].days[2].duration_min, Some(105));

    // Neither superseded outline retains an active week.
    for (label, id) in [("A", &plan_a.plan.id), ("B", &plan_b.plan.id)] {
        let stranded = repos
            .training_plans
            .list_plan_weeks(&tenant, &user, id, false)
            .await?;
        assert!(
            stranded.is_empty(),
            "plan {label} still holds active weeks: {stranded:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn outline_resave_with_weeks_keeps_one_active_row_per_week() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = blocks();

    let d1 = week_days("2026-07-13");
    let d2 = week_days("2026-07-20");
    repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-07-13",
                    focus: "volume back up",
                    days: &d1,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: "2026-07-20",
                    focus: "race-specific tempo",
                    days: &d2,
                    adjustment_reason: "",
                },
            ],
        })
        .await?;

    // New outline AND a rewritten week 1: the rewrite wins, week 2 is carried.
    let mut rewritten = week_days("2026-07-13");
    rewritten[1].workout = "endurance, no intervals".to_owned();
    let second = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume back up",
                days: &rewritten,
                adjustment_reason: "illness — intervals pulled",
            }],
        })
        .await?;

    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &second.plan.id, false)
        .await?;
    assert_eq!(weeks.len(), 2, "one active row per calendar week");
    assert_eq!(weeks[0].adjustment_reason, "illness — intervals pulled");
    assert_eq!(weeks[0].days[1].workout, "endurance, no intervals");
    assert_eq!(weeks[1].focus, "race-specific tempo", "week 2 carried over");
    assert_eq!(weeks[1].days[1].workout, "tempo 3x8min");
    Ok(())
}

#[tokio::test]
async fn bundle_resaving_outline_supersedes_the_previous_plan() -> Result<()> {
    let db = open_in_memory_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = blocks();

    let first = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[],
        })
        .await?;

    let second = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            coach_slug: Some("endurance-coach"),
            goal_fact_id: None,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[],
        })
        .await?;
    assert_eq!(
        second.superseded_plan_id.as_deref(),
        Some(first.plan.id.as_str()),
        "the re-save supersedes the first plan"
    );
    assert_ne!(second.plan.id, first.plan.id);

    // Exactly one active plan remains.
    let active = repos
        .training_plans
        .get_active_plan(&tenant, &user, Some("endurance-coach"))
        .await?
        .expect("active plan");
    assert_eq!(active.id, second.plan.id);
    assert_eq!(active.status, PlanStatus::Active);
    Ok(())
}
