// ABOUTME: Integration tests for TrainingPlanRepository — roundtrip, supersession chains, isolation
// ABOUTME: Content-asserting per the anti-stub rule: real day values, not is_ok() smoke checks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::WorkoutStep;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::training_plans::{PlanAuthor, NO_AUTHOR_AGENT};
use pierre_database::repositories::{
    PlanOutlineInput, PlanWeekInput, SavePlanBundleParams, SaveTrainingPlanParams,
    TrainingPlanRepository,
};
use pierre_memory::training_plans::{
    GoalRace, PlanPhase, PlanStatus, PlannedDay, RacePriority, TrainingPlan, WeekStatus,
};
use pierre_test_support::db::create_test_db;
use sqlx::{Connection, SqliteConnection};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

/// Open the backend under test.
///
/// [`create_test_db`] honours a `PostgreSQL` `DATABASE_URL`, so under
/// `ci-postgres` these tests execute the `PostgreSQL` backend's own SQL: the
/// carry-forward regression tests below — written to prove that superseding an
/// outline does not strand its weeks — run against the backend production uses.
async fn open_test_db() -> Result<Database> {
    Ok(create_test_db().await?)
}

fn big_red() -> GoalRace {
    GoalRace {
        name: "Big Red".to_owned(),
        date: "2026-08-08".to_owned(),
        discipline: "gravel".to_owned(),
        priority: RacePriority::A,
    }
}

fn phases() -> Vec<PlanPhase> {
    vec![
        PlanPhase {
            kind: PhaseKind::Build,
            start: "2026-07-13".to_owned(),
            weeks: 3,
            intent: "volume back up, one moderate day per week".to_owned(),
            target_hours: Some(9.0),
            purpose: String::new(),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: None,
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
        },
        PlanPhase {
            kind: PhaseKind::Taper,
            start: "2026-08-03".to_owned(),
            weeks: 1,
            intent: "shorter, sharper, more rest".to_owned(),
            target_hours: Some(5.0),
            purpose: String::new(),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: None,
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
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
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
        },
        PlannedDay {
            date: "2026-07-14".to_owned(),
            sport: "gravel".to_owned(),
            workout: "tempo 3x8min".to_owned(),
            duration_min: Some(60),
            intensity: "3x8min @ 88-93% FTP".to_owned(),
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
        },
        PlannedDay {
            date: "2026-07-15".to_owned(),
            sport: "mtb".to_owned(),
            workout: "endurance, low HR on climbs".to_owned(),
            duration_min: Some(105),
            intensity: "Z2".to_owned(),
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
        },
    ]
}

fn plan_params<'a>(
    tenant: &'a str,
    user: &'a str,
    race: &'a GoalRace,
    phases: &'a [PlanPhase],
) -> SaveTrainingPlanParams<'a> {
    SaveTrainingPlanParams {
        tenant_id: tenant,
        user_id: user,
        author: PlanAuthor::agent("endurance-coach"),
        goal_fact_id: Some("fact-goal-1"),
        goal_race: race,
        races: Some(&[]),
        strategy: "rest week done; rebuild volume, then race-specific tempo, taper into Aug 8",
        flavour: None,
        season_start: None,
        season_end: None,
        phases,
        source_conversation_id: Some("conv-1"),
    }
}

/// Save an outline through the one write path: a bundle with no weeks.
async fn save_outline(
    plans: &dyn TrainingPlanRepository,
    params: &SaveTrainingPlanParams<'_>,
) -> AppResult<TrainingPlan> {
    let saved = plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: params.tenant_id,
            user_id: params.user_id,
            author: params.author,
            goal_fact_id: params.goal_fact_id,
            replace_season: false,
            outline: Some(PlanOutlineInput {
                goal_race: params.goal_race,
                races: params.races,
                strategy: params.strategy,
                flavour: params.flavour,
                season_start: params.season_start,
                season_end: params.season_end,
                phases: params.phases,
                source_conversation_id: params.source_conversation_id,
            }),
            weeks: &[],
        })
        .await?;
    Ok(saved.plan)
}

#[tokio::test]
async fn save_and_get_roundtrip_preserves_content() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = phases();
    let plan = save_outline(
        repos.training_plans.as_ref(),
        &plan_params(&tenant, &user, &race, &blks),
    )
    .await?;

    let days = week_days("2026-07-13");
    let saved = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume back up",
                days: &days,
                adjustment_reason: "",
                phase_index: None,
            }],
        })
        .await?;
    assert_eq!(saved.plan.id, plan.id);
    assert_eq!(saved.weeks.len(), 1);
    assert_eq!(saved.weeks[0].status, WeekStatus::Active);
    assert_eq!(saved.weeks[0].supersedes_id, None);

    let fetched = repos
        .training_plans
        .get_active_plan(&tenant, &user)
        .await?
        .expect("active plan");
    assert_eq!(fetched.id, plan.id);
    assert_eq!(fetched.goal_race.name, "Big Red");
    assert_eq!(fetched.goal_race.date, "2026-08-08");
    assert_eq!(fetched.goal_race.priority, RacePriority::A);
    assert_eq!(fetched.phases.len(), 2);
    assert_eq!(fetched.phases[0].kind, PhaseKind::Build);
    assert_eq!(fetched.phases[1].kind, PhaseKind::Taper);
    assert_eq!(fetched.phases[0].target_hours, Some(9.0));
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
async fn a_structured_day_round_trips_and_a_prose_row_reads_as_no_steps() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();
    let plan = save_outline(
        repos.training_plans.as_ref(),
        &plan_params(&tenant, &user, &race, &blks),
    )
    .await?;

    let mut days = week_days("2026-07-13");
    days[1].steps = vec![
        WorkoutStep {
            label: "Work".to_owned(),
            duration_seconds: 480,
            distance_meters: None,
            target_zone: "88-93% FTP".to_owned(),
            repeat: 3,
            repeat_group: Some(1),
            note: Some("seated, steady cadence".to_owned()),
        },
        WorkoutStep {
            label: "Recovery".to_owned(),
            duration_seconds: 240,
            distance_meters: Some(1500.0),
            target_zone: "Z1".to_owned(),
            repeat: 3,
            repeat_group: Some(1),
            note: None,
        },
    ];
    repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume back up",
                days: &days,
                adjustment_reason: "",
                phase_index: None,
            }],
        })
        .await?;

    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &plan.id, false)
        .await?;
    assert_eq!(weeks.len(), 1);
    assert_eq!(
        weeks[0].days[1].steps, days[1].steps,
        "every step field survives the days_json column"
    );
    assert!(weeks[0].days[2].steps.is_empty());

    // A row stored before the field existed carries no `steps` key; it reads
    // back as a day with none, not as a deserialization failure — and a
    // prose day still serializes without the key, so its stored JSON is what
    // it was.
    let stored: PlannedDay = serde_json::from_str(
        r#"{"date":"2026-07-14","sport":"gravel","workout":"tempo 3x8min","duration_min":60,"intensity":"3x8min @ 88-93% FTP"}"#,
    )?;
    assert!(stored.steps.is_empty());
    assert_eq!(stored.duration_min, Some(60));
    assert!(!serde_json::to_string(&days[2])?.contains("steps"));
    Ok(())
}

#[tokio::test]
async fn outline_resave_supersedes_previous() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = phases();
    let first = save_outline(
        repos.training_plans.as_ref(),
        &plan_params(&tenant, &user, &race, &blks),
    )
    .await?;

    // Goal moved: new outline replaces the old one atomically.
    let moved = GoalRace {
        date: "2026-08-15".to_owned(),
        ..big_red()
    };
    let second = save_outline(
        repos.training_plans.as_ref(),
        &plan_params(&tenant, &user, &moved, &blks),
    )
    .await?;

    assert_eq!(second.supersedes_id.as_deref(), Some(first.id.as_str()));
    let active = repos
        .training_plans
        .get_active_plan(&tenant, &user)
        .await?
        .expect("active plan");
    assert_eq!(active.id, second.id);
    assert_eq!(active.goal_race.date, "2026-08-15");
    Ok(())
}

#[tokio::test]
async fn week_resave_supersedes_that_week_only() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = phases();
    let plan = save_outline(
        repos.training_plans.as_ref(),
        &plan_params(&tenant, &user, &race, &blks),
    )
    .await?;

    let days1 = week_days("2026-07-13");
    let days2 = week_days("2026-07-20");
    let first = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: None,
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-07-13",
                    focus: "volume",
                    days: &days1,
                    adjustment_reason: "",
                    phase_index: None,
                },
                PlanWeekInput {
                    week_start: "2026-07-20",
                    focus: "tempo",
                    days: &days2,
                    adjustment_reason: "",
                    phase_index: None,
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
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &adjusted,
                adjustment_reason: "tempo moved to Wednesday — legs heavy Tuesday",
                phase_index: None,
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
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let race = big_red();
    let blks = phases();
    let plan = save_outline(
        repos.training_plans.as_ref(),
        &plan_params(&tenant_a, &user, &race, &blks),
    )
    .await?;

    // Another tenant must see nothing, even for the same user id.
    assert!(repos
        .training_plans
        .get_active_plan(&tenant_b, &user)
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
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &days,
                adjustment_reason: "",
                phase_index: None,
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

/// A weeks-only bundle for one author.
fn weeks_by<'a>(
    tenant: &'a str,
    user: &'a str,
    author: PlanAuthor<'a>,
    weeks: &'a [PlanWeekInput<'a>],
) -> SavePlanBundleParams<'a> {
    SavePlanBundleParams {
        tenant_id: tenant,
        user_id: user,
        author,
        goal_fact_id: None,
        replace_season: false,
        outline: None,
        weeks,
    }
}

/// One week input with a recognisable focus.
fn week_input<'a>(
    week_start: &'a str,
    focus: &'a str,
    days: &'a [PlannedDay],
) -> PlanWeekInput<'a> {
    PlanWeekInput {
        week_start,
        focus,
        days,
        adjustment_reason: "",
        phase_index: None,
    }
}

/// Who wrote each active week, by `week_start`.
async fn week_authors(
    plans: &dyn TrainingPlanRepository,
    tenant: &str,
    user: &str,
    plan_id: &str,
) -> Result<Vec<(String, Option<String>)>> {
    Ok(plans
        .list_plan_weeks(tenant, user, plan_id, false)
        .await?
        .into_iter()
        .map(|w| (w.week_start, w.author_agent_id))
        .collect())
}

#[test]
fn only_the_author_or_anyone_over_an_agnostic_season_may_re_lay_it_unasked() {
    let endurance = PlanAuthor::agent("endurance-agent");
    assert!(endurance.may_resave_outline(None), "an agnostic season");
    assert!(
        endurance.may_resave_outline(Some("endurance-agent")),
        "its own season"
    );
    assert!(
        !endurance.may_resave_outline(Some("taper-builder-agent")),
        "another agent's"
    );
    assert!(PlanAuthor::none().may_resave_outline(None));
    assert!(
        !PlanAuthor::none().may_resave_outline(Some("endurance-agent")),
        "a writer with no agent may not re-lay an agent's season"
    );
    assert_eq!(PlanAuthor::none().stored(), NO_AUTHOR_AGENT);
    assert_eq!(endurance.stored(), "endurance-agent");
    assert_eq!(PlanAuthor::from_agent(None), PlanAuthor::none());
}

#[tokio::test]
async fn a_plan_saved_under_one_agent_is_the_plan_every_read_returns() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let plans = repos.training_plans.as_ref();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();

    let season = save_outline(plans, &plan_params(&tenant, &user, &race, &blks)).await?;
    assert_eq!(season.author_agent_id.as_deref(), Some("endurance-coach"));

    let read = plans
        .get_active_plan(&tenant, &user)
        .await?
        .expect("the athlete's season");
    assert_eq!(read.id, season.id);
    assert_eq!(read.author_agent_id.as_deref(), Some("endurance-coach"));
    assert_eq!(read.goal_race.name, "Big Red");

    // Weeks written by another agent, and by a call with no agent at all,
    // land on that same season rather than opening a plan of their own.
    let d1 = week_days("2026-07-13");
    let d2 = week_days("2026-07-20");
    let taper_week = [week_input("2026-07-13", "taper-written", &d1)];
    let direct_week = [week_input("2026-07-20", "direct call", &d2)];
    let by_taper = plans
        .save_plan_bundle(&weeks_by(
            &tenant,
            &user,
            PlanAuthor::agent("taper-builder-agent"),
            &taper_week,
        ))
        .await?;
    let by_nobody = plans
        .save_plan_bundle(&weeks_by(&tenant, &user, PlanAuthor::none(), &direct_week))
        .await?;
    assert_eq!(by_taper.plan.id, season.id);
    assert_eq!(by_nobody.plan.id, season.id);
    assert_eq!(
        week_authors(plans, &tenant, &user, &season.id).await?,
        vec![
            (
                "2026-07-13".to_owned(),
                Some("taper-builder-agent".to_owned())
            ),
            ("2026-07-20".to_owned(), None),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_same_author_outline_supersedes_and_carries_weeks_with_their_authors() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let plans = repos.training_plans.as_ref();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();

    let d1 = week_days("2026-07-13");
    let d2 = week_days("2026-07-20");
    let first = plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[week_input("2026-07-13", "season week", &d1)],
        })
        .await?;
    plans
        .save_plan_bundle(&weeks_by(
            &tenant,
            &user,
            PlanAuthor::agent("taper-builder-agent"),
            &[week_input("2026-07-20", "taper week", &d2)],
        ))
        .await?;

    // The season's author re-lays its outline unasked.
    let moved = GoalRace {
        date: "2026-08-15".to_owned(),
        ..big_red()
    };
    let second = save_outline(plans, &plan_params(&tenant, &user, &moved, &blks)).await?;
    assert_eq!(
        second.supersedes_id.as_deref(),
        Some(first.plan.id.as_str())
    );
    assert_eq!(second.author_agent_id.as_deref(), Some("endurance-coach"));
    assert_eq!(
        week_authors(plans, &tenant, &user, &second.id).await?,
        vec![
            ("2026-07-13".to_owned(), Some("endurance-coach".to_owned())),
            (
                "2026-07-20".to_owned(),
                Some("taper-builder-agent".to_owned())
            ),
        ],
        "every carried week keeps the agent that wrote it"
    );
    Ok(())
}

#[tokio::test]
async fn an_outline_over_another_authors_season_is_refused_already_exists_and_changes_nothing(
) -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let plans = repos.training_plans.as_ref();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();
    let days = week_days("2026-07-13");

    let season = plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[week_input("2026-07-13", "season week", &days)],
        })
        .await?;

    let taper_race = GoalRace {
        name: "Taper Race".to_owned(),
        ..big_red()
    };
    let taper_days = week_days("2026-07-20");
    for author in [PlanAuthor::agent("taper-builder-agent"), PlanAuthor::none()] {
        let refused = plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &tenant,
                user_id: &user,
                author,
                goal_fact_id: None,
                replace_season: false,
                outline: Some(outline_input(&taper_race, &blks)),
                weeks: &[week_input("2026-07-20", "taper week", &taper_days)],
            })
            .await;
        let Err(err) = refused else {
            panic!("an outline over another author's season must be refused ({author:?})");
        };
        assert_eq!(err.code, ErrorCode::ResourceAlreadyExists, "{err}");
        assert!(
            err.message.contains("changed while this save ran")
                && err.message.contains("get_training_plan"),
            "{err}"
        );
    }

    // Nothing moved: the same season, its one week, no taper week, no history.
    let active = plans
        .get_active_plan(&tenant, &user)
        .await?
        .expect("the season survives");
    assert_eq!(active.id, season.plan.id);
    assert_eq!(active.goal_race.name, "Big Red");
    assert_eq!(
        week_authors(plans, &tenant, &user, &season.plan.id).await?,
        vec![("2026-07-13".to_owned(), Some("endurance-coach".to_owned()))]
    );
    assert_eq!(
        plans
            .list_plan_weeks(&tenant, &user, &season.plan.id, true)
            .await?
            .len(),
        1,
        "no week was superseded"
    );
    Ok(())
}

#[tokio::test]
async fn replace_season_supersedes_another_authors_outline() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let plans = repos.training_plans.as_ref();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();
    let days = week_days("2026-07-13");

    let season = plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[week_input("2026-07-13", "season week", &days)],
        })
        .await?;

    let taper_race = GoalRace {
        name: "Taper Race".to_owned(),
        ..big_red()
    };
    let relaid = plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("taper-builder-agent"),
            goal_fact_id: None,
            replace_season: true,
            outline: Some(outline_input(&taper_race, &blks)),
            weeks: &[],
        })
        .await?;
    assert_eq!(
        relaid.superseded_plan_id.as_deref(),
        Some(season.plan.id.as_str())
    );
    assert_eq!(
        relaid.plan.author_agent_id.as_deref(),
        Some("taper-builder-agent")
    );

    let active = plans
        .get_active_plan(&tenant, &user)
        .await?
        .expect("the re-laid season");
    assert_eq!(active.id, relaid.plan.id);
    assert_eq!(active.goal_race.name, "Taper Race");
    assert_eq!(
        active.author_agent_id.as_deref(),
        Some("taper-builder-agent")
    );
    assert_eq!(
        week_authors(plans, &tenant, &user, &relaid.plan.id).await?,
        vec![("2026-07-13".to_owned(), Some("endurance-coach".to_owned()))],
        "the carried week keeps its author"
    );

    // An agnostic season is anyone's to re-lay, no flag needed.
    let other = Uuid::new_v4().to_string();
    let mut agnostic = plan_params(&tenant, &other, &race, &blks);
    agnostic.author = PlanAuthor::none();
    let agnostic = save_outline(plans, &agnostic).await?;
    assert_eq!(agnostic.author_agent_id, None);
    let mut by_taper = plan_params(&tenant, &other, &taper_race, &blks);
    by_taper.author = PlanAuthor::agent("taper-builder-agent");
    let taken = save_outline(plans, &by_taper).await?;
    assert_eq!(taken.supersedes_id.as_deref(), Some(agnostic.id.as_str()));
    assert_eq!(
        taken.author_agent_id.as_deref(),
        Some("taper-builder-agent")
    );
    Ok(())
}

#[tokio::test]
async fn a_weeks_only_save_by_a_second_author_stamps_only_its_weeks() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let plans = repos.training_plans.as_ref();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();
    let d1 = week_days("2026-07-13");
    let d2 = week_days("2026-07-20");
    let d3 = week_days("2026-07-27");

    let season = plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[
                week_input("2026-07-13", "season week one", &d1),
                week_input("2026-07-20", "season week two", &d2),
            ],
        })
        .await?;
    let original_week_two = season.weeks[1].id.clone();

    // The taper agent re-saves week two and adds week three, no outline.
    let taper = plans
        .save_plan_bundle(&weeks_by(
            &tenant,
            &user,
            PlanAuthor::agent("taper-builder-agent"),
            &[
                week_input("2026-07-20", "taper week two", &d2),
                week_input("2026-07-27", "taper week three", &d3),
            ],
        ))
        .await?;
    assert_eq!(taper.plan.id, season.plan.id);
    assert_eq!(
        taper.plan.author_agent_id.as_deref(),
        Some("endurance-coach"),
        "a weeks-only save leaves the outline's author alone"
    );
    assert!(taper
        .weeks
        .iter()
        .all(|w| w.author_agent_id.as_deref() == Some("taper-builder-agent")));
    assert_eq!(
        week_authors(plans, &tenant, &user, &season.plan.id).await?,
        vec![
            ("2026-07-13".to_owned(), Some("endurance-coach".to_owned())),
            (
                "2026-07-20".to_owned(),
                Some("taper-builder-agent".to_owned())
            ),
            (
                "2026-07-27".to_owned(),
                Some("taper-builder-agent".to_owned())
            ),
        ]
    );

    // The week it replaced keeps its own author in the history.
    let history = plans
        .list_plan_weeks(&tenant, &user, &season.plan.id, true)
        .await?;
    let replaced = history
        .iter()
        .find(|w| w.id == original_week_two)
        .expect("the replaced week stays as history");
    assert_eq!(replaced.status, WeekStatus::Superseded);
    assert_eq!(replaced.author_agent_id.as_deref(), Some("endurance-coach"));
    assert_eq!(replaced.focus, "season week two");
    Ok(())
}

/// A weeks-only save that starts while an outline save holds the season
/// lands on the outline that save commits, never on the one it superseded.
///
/// The held transaction is the outline save's own shape — the season row
/// flipped to `superseded`, a new active row inserted — on a second
/// connection, committed while the weeks-only save waits on it. On Postgres
/// the weeks save's claim waits on the row lock, finds its row superseded and
/// claims again; on SQLite it waits on the write lock and claims the new row.
#[tokio::test]
async fn a_weeks_only_save_waiting_on_an_outline_supersede_lands_on_the_new_outline() -> Result<()>
{
    let db = open_test_db().await?;
    let repos = db.repositories();
    let plans = repos.training_plans.as_ref();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();
    let season = save_outline(plans, &plan_params(&tenant, &user, &race, &blks)).await?;
    let relaid = Uuid::new_v4().to_string();

    let supersede = "UPDATE training_plans SET status = 'superseded', updated_at = $1 \
                     WHERE id = $2";
    let insert = "INSERT INTO training_plans (id, tenant_id, user_id, author_agent_id, \
                  goal_race_json, races_json, strategy, phases_json, status, supersedes_id, \
                  created_at, updated_at) \
                  SELECT $1, tenant_id, user_id, author_agent_id, goal_race_json, races_json, \
                  'the re-laid season', phases_json, 'active', id, created_at, updated_at \
                  FROM training_plans WHERE id = $2";
    let now = chrono::Utc::now().timestamp();
    let days = week_days("2026-07-13");
    let week = [week_input(
        "2026-07-13",
        "written while the season moved",
        &days,
    )];
    let weeks_save = weeks_by(
        &tenant,
        &user,
        PlanAuthor::agent("taper-builder-agent"),
        &week,
    );

    let saved = match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            let options = sqlite.pool().connect_options();
            let mut conn = SqliteConnection::connect_with(&options).await?;
            let mut held = conn.begin().await?;
            sqlx::query(supersede)
                .bind(now)
                .bind(&season.id)
                .execute(&mut *held)
                .await?;
            sqlx::query(insert)
                .bind(&relaid)
                .bind(&season.id)
                .execute(&mut *held)
                .await?;
            let (saved, committed) = tokio::join!(plans.save_plan_bundle(&weeks_save), async {
                sleep(Duration::from_millis(300)).await;
                held.commit().await
            });
            committed?;
            saved?
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            let mut held = pg.pool().begin().await?;
            sqlx::query(supersede)
                .bind(now)
                .bind(&season.id)
                .execute(&mut *held)
                .await?;
            sqlx::query(insert)
                .bind(&relaid)
                .bind(&season.id)
                .execute(&mut *held)
                .await?;
            let (saved, committed) = tokio::join!(plans.save_plan_bundle(&weeks_save), async {
                sleep(Duration::from_millis(300)).await;
                held.commit().await
            });
            committed?;
            saved?
        }
    };

    assert_eq!(
        saved.plan.id, relaid,
        "the week lands on the outline that won"
    );
    assert_eq!(saved.plan.strategy, "the re-laid season");
    assert_eq!(
        week_authors(plans, &tenant, &user, &relaid).await?,
        vec![(
            "2026-07-13".to_owned(),
            Some("taper-builder-agent".to_owned())
        )]
    );
    assert!(
        plans
            .list_plan_weeks(&tenant, &user, &season.id, true)
            .await?
            .is_empty(),
        "nothing was written onto the superseded outline"
    );
    Ok(())
}

fn outline_input<'a>(race: &'a GoalRace, phases: &'a [PlanPhase]) -> PlanOutlineInput<'a> {
    PlanOutlineInput {
        goal_race: race,
        races: Some(&[]),
        strategy: "rebuild volume, race-specific tempo, taper into Aug 8",
        flavour: None,
        season_start: None,
        season_end: None,
        phases,
        source_conversation_id: Some("conv-1"),
    }
}

#[tokio::test]
async fn bundle_saves_outline_and_weeks_in_one_call() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();
    let days = week_days("2026-07-13");

    let saved = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: Some("fact-goal-1"),
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume back up",
                days: &days,
                adjustment_reason: "",
                phase_index: None,
            }],
        })
        .await?;
    assert_eq!(saved.weeks.len(), 1);
    assert!(saved.superseded_plan_id.is_none());
    assert_eq!(saved.plan.goal_race.name, "Big Red");

    // Both the outline and the week are durably present.
    let got = repos
        .training_plans
        .get_active_plan(&tenant, &user)
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
async fn a_phase_index_past_the_column_is_refused_as_input_and_writes_nothing() -> Result<()> {
    // `phase_index` is an `INTEGER` column, `int4` on PostgreSQL. A caller's
    // index past `i32::MAX` names no phase any outline could hold, so the
    // repository refuses it as invalid input before either engine sees it —
    // and refuses it identically on both, rather than SQLite storing what
    // PostgreSQL would reject with a driver error.
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();
    let days = week_days("2026-07-13");

    let result = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &days,
                adjustment_reason: "",
                phase_index: Some(u32::MAX),
            }],
        })
        .await;
    let Err(err) = result else {
        panic!("a phase index past int4 is refused");
    };
    assert_eq!(err.code, ErrorCode::InvalidInput, "{err}");
    assert!(err.message.contains("phase_index"), "{err}");

    // One transaction: the outline that was to carry the week rolled back too.
    assert!(repos
        .training_plans
        .get_active_plan(&tenant, &user)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn a_phase_index_round_trips_through_a_supersede() -> Result<()> {
    // The carried week keeps its phase index when an outline re-save moves it
    // onto the new plan: the value is read from the stored row and bound
    // again, on both engines, with no conversion between.
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();
    let days = week_days("2026-07-13");

    let first = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &days,
                adjustment_reason: "",
                phase_index: Some(1),
            }],
        })
        .await?;
    assert_eq!(first.weeks[0].phase_index, Some(1));

    let second = save_outline(
        repos.training_plans.as_ref(),
        &plan_params(&tenant, &user, &race, &blks),
    )
    .await?;
    assert_eq!(
        second.supersedes_id.as_deref(),
        Some(first.plan.id.as_str())
    );
    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &second.id, false)
        .await?;
    assert_eq!(weeks.len(), 1);
    assert_eq!(weeks[0].phase_index, Some(1));
    assert_eq!(
        weeks[0].supersedes_id.as_deref(),
        Some(first.weeks[0].id.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn bundle_weeks_only_attaches_to_active_plan() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();

    let plan = save_outline(
        repos.training_plans.as_ref(),
        &plan_params(&tenant, &user, &race, &blks),
    )
    .await?;

    let days = week_days("2026-07-13");
    let saved = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &days,
                adjustment_reason: "",
                phase_index: None,
            }],
        })
        .await?;
    assert_eq!(saved.plan.id, plan.id, "weeks attach to the active plan");
    assert_eq!(saved.weeks.len(), 1);
    Ok(())
}

#[tokio::test]
async fn bundle_weeks_only_with_no_plan_errors_and_writes_nothing() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let days = week_days("2026-07-13");

    let result = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: None,
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume",
                days: &days,
                adjustment_reason: "",
                phase_index: None,
            }],
        })
        .await;
    assert!(result.is_err(), "weeks with no active plan must error");

    // The transaction rolled back: no plan, no orphan week.
    assert!(repos
        .training_plans
        .get_active_plan(&tenant, &user)
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn outline_resave_carries_the_active_weeks_onto_the_new_plan() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();

    let d1 = week_days("2026-07-13");
    let d2 = week_days("2026-07-20");
    let d3 = week_days("2026-07-27");
    let first = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-07-13",
                    focus: "volume back up",
                    days: &d1,
                    adjustment_reason: "",
                    phase_index: None,
                },
                PlanWeekInput {
                    week_start: "2026-07-20",
                    focus: "race-specific tempo",
                    days: &d2,
                    adjustment_reason: "",
                    phase_index: None,
                },
                PlanWeekInput {
                    week_start: "2026-07-27",
                    focus: "peak then unload",
                    days: &d3,
                    adjustment_reason: "",
                    phase_index: None,
                },
            ],
        })
        .await?;
    assert_eq!(first.weeks.len(), 3);

    // The goal moved, so the agent re-saves the OUTLINE alone — the encouraged
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
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
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
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let blks = phases();

    // Plan A — the first goal, with the two weeks around it.
    let a1 = week_days("2026-07-28");
    let a2 = week_days("2026-08-04");
    let race_a = big_red();
    let plan_a = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race_a, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-07-28",
                    focus: "race week",
                    days: &a1,
                    adjustment_reason: "",
                    phase_index: None,
                },
                PlanWeekInput {
                    week_start: "2026-08-04",
                    focus: "post-race easy",
                    days: &a2,
                    adjustment_reason: "",
                    phase_index: None,
                },
            ],
        })
        .await?;
    assert_eq!(plan_a.weeks.len(), 2);

    // Plan B — the goal changes and the agent lays out the whole run to it.
    let b1 = week_days("2026-08-10");
    let b2 = week_days("2026-08-17");
    // The peak week is the one plan C never re-sends, so it is the week whose
    // survival this test exists to prove. Its days are deliberately unlike
    // every other week's: `week_days` writes the same two sessions everywhere,
    // so asserting those would pass on any week's payload arriving here.
    let b3 = vec![
        PlannedDay {
            date: "2026-08-24".to_owned(),
            sport: "rest".to_owned(),
            workout: "off before the peak block".to_owned(),
            duration_min: None,
            intensity: String::new(),
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
        },
        PlannedDay {
            date: "2026-08-26".to_owned(),
            sport: "trail".to_owned(),
            workout: "VO2max 6x3min".to_owned(),
            duration_min: Some(75),
            intensity: "6x3min @ 95-100%".to_owned(),
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
        },
        PlannedDay {
            date: "2026-08-29".to_owned(),
            sport: "trail".to_owned(),
            workout: "long trail simulating race pace and aid stations".to_owned(),
            duration_min: Some(110),
            intensity: "race effort".to_owned(),
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
        },
    ];
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
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race_b, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-08-10",
                    focus: "reintroduction",
                    days: &b1,
                    adjustment_reason: "",
                    phase_index: None,
                },
                PlanWeekInput {
                    week_start: "2026-08-17",
                    focus: "build",
                    days: &b2,
                    adjustment_reason: "",
                    phase_index: None,
                },
                PlanWeekInput {
                    week_start: "2026-08-24",
                    focus: "peak",
                    days: &b3,
                    adjustment_reason: "",
                    phase_index: None,
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
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race_b, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-08-10",
                    focus: "reintroduction, revised",
                    days: &c1,
                    adjustment_reason: "",
                    phase_index: None,
                },
                PlanWeekInput {
                    week_start: "2026-08-17",
                    focus: "build, revised",
                    days: &c2,
                    adjustment_reason: "",
                    phase_index: None,
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
    // The re-sent weeks carry the revision, and the week that only ever lived on
    // plan B keeps its own content rather than an empty placeholder or another
    // week's payload — these values appear on no other week in this test.
    assert_eq!(weeks[2].focus, "reintroduction, revised");
    assert_eq!(weeks[3].focus, "build, revised");
    assert_eq!(weeks[4].focus, "peak");
    assert_eq!(weeks[4].days.len(), 3);
    assert_eq!(weeks[4].days[1].workout, "VO2max 6x3min");
    assert_eq!(
        weeks[4].days[2].workout,
        "long trail simulating race pace and aid stations"
    );
    assert_eq!(weeks[4].days[2].duration_min, Some(110));

    // Neither superseded outline retains an active week, and both keep their
    // rows as audit trail rather than losing them.
    for (label, id) in [("A", &plan_a.plan.id), ("B", &plan_b.plan.id)] {
        let stranded = repos
            .training_plans
            .list_plan_weeks(&tenant, &user, id, false)
            .await?;
        assert!(
            stranded.is_empty(),
            "plan {label} still holds active weeks: {stranded:?}"
        );
        let history = repos
            .training_plans
            .list_plan_weeks(&tenant, &user, id, true)
            .await?;
        assert!(
            !history.is_empty() && history.iter().all(|w| w.status == WeekStatus::Superseded),
            "plan {label}'s rows must survive as superseded audit trail: {history:?}"
        );
    }

    // The audit chain is unbroken across BOTH hops: the peak week reachable
    // from plan C points at the row plan B held, not at plan B's plan id, not
    // at the original on plan A, and not at nothing. A carry-forward that
    // rebound this wrong after two hops still satisfies every count above.
    let b_history = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &plan_b.plan.id, true)
        .await?;
    let b_peak = b_history
        .iter()
        .find(|w| w.week_start == "2026-08-24")
        .expect("plan B wrote the peak week");
    assert_eq!(
        weeks[4].supersedes_id.as_deref(),
        Some(b_peak.id.as_str()),
        "the twice-carried peak week must chain back to the row it replaced"
    );
    Ok(())
}

#[tokio::test]
async fn outline_resave_with_weeks_keeps_one_active_row_per_week() -> Result<()> {
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();

    let d1 = week_days("2026-07-13");
    let d2 = week_days("2026-07-20");
    repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[
                PlanWeekInput {
                    week_start: "2026-07-13",
                    focus: "volume back up",
                    days: &d1,
                    adjustment_reason: "",
                    phase_index: None,
                },
                PlanWeekInput {
                    week_start: "2026-07-20",
                    focus: "race-specific tempo",
                    days: &d2,
                    adjustment_reason: "",
                    phase_index: None,
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
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[PlanWeekInput {
                week_start: "2026-07-13",
                focus: "volume back up",
                days: &rewritten,
                adjustment_reason: "illness — intervals pulled",
                phase_index: None,
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
    let db = open_test_db().await?;
    let repos = db.repositories();
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let race = big_red();
    let blks = phases();

    let first = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(outline_input(&race, &blks)),
            weeks: &[],
        })
        .await?;

    let second = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant,
            user_id: &user,
            author: PlanAuthor::agent("endurance-coach"),
            goal_fact_id: None,
            replace_season: false,
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
        .get_active_plan(&tenant, &user)
        .await?
        .expect("active plan");
    assert_eq!(active.id, second.plan.id);
    assert_eq!(active.status, PlanStatus::Active);
    Ok(())
}
