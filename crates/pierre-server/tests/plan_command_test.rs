// ABOUTME: /plan command — content assertions for the compact, week, and today views
// ABOUTME: Seeds a real plan through the repository, then asserts the athlete-facing text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `/plan` is a deterministic read of the stored plan — no LLM. These tests
//! assert concrete session text, so a handler that returned an empty string, the
//! empty-state string, or a stub would fail rather than pass on `is_ok()`.
//!
//! The plan is seeded around a fixed "today" so the week selection, the
//! today/tomorrow lookup and the race countdown are all deterministic.

use anyhow::Result;
use pierre_commands::plan::PlanShowHandler;
use pierre_commands::{CommandHandler, PlatformCommandContext};
use pierre_core::chunking::chunk_reply;
use pierre_core::models::TenantId;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::training_plans::{BlockPhase, GoalRace, PlanBlock, PlannedDay, RacePriority};
use pierre_runtime_context::CoachesCtx;
use std::sync::Arc;
use uuid::Uuid;

mod common;

/// Dates are derived from the real current date rather than hardcoded: the
/// handler reads "today" from the clock, so a fixed fixture week would stop
/// straddling today the day after it was written and the week/today views would
/// silently start asserting the empty selection.
fn week_dates() -> (chrono::NaiveDate, Vec<String>) {
    let today = chrono::Utc::now().date_naive();
    // Start two days before today so today and tomorrow both fall inside.
    let start = today - chrono::Days::new(2);
    let dates = (0..7)
        .map(|i| {
            (start + chrono::Days::new(i))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect();
    (start, dates)
}

fn day(
    date: &str,
    sport: &str,
    workout: &str,
    minutes: Option<u32>,
    intensity: &str,
) -> PlannedDay {
    PlannedDay {
        date: date.to_owned(),
        sport: sport.to_owned(),
        workout: workout.to_owned(),
        duration_min: minutes,
        intensity: intensity.to_owned(),
    }
}

async fn setup() -> Result<(Arc<ServerContext>, Uuid, TenantId, String)> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let email = format!("plancmd_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(resources.database(), &email).await?;
    let tenants = resources.common.repos.tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .map(|t| t.id)
        .expect("user owns a tenant");
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "plan",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await?;
    Ok((resources, user_id, tenant, conversation.id))
}

/// Seed one outline plus one full week of concrete sessions.
async fn seed_plan(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> Result<()> {
    seed_plan_with(resources, user_id, tenant, &[], &[]).await
}

/// Seed the same plan behind extra outline blocks and alongside extra stored
/// weeks, so a test can put a row shape of its own choosing in front of the
/// handler. `leading_blocks` come first in the outline because the header
/// scans blocks in order and stops at the one holding today — a block placed
/// after that one is never examined.
async fn seed_plan_with(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    leading_blocks: &[PlanBlock],
    extra_weeks: &[(String, Vec<PlannedDay>)],
) -> Result<()> {
    let goal = GoalRace {
        name: "Unbound XL".to_owned(),
        date: "2026-10-03".to_owned(),
        discipline: "gravel".to_owned(),
        priority: RacePriority::A,
    };
    let (start, dates) = week_dates();
    let mut blocks = leading_blocks.to_vec();
    blocks.push(PlanBlock {
        phase: BlockPhase::Build,
        start: start.format("%Y-%m-%d").to_string(),
        weeks: 4,
        intent: "volume up, one moderate day".to_owned(),
        target_hours: Some(14.0),
    });
    // Index 2 is today, index 3 tomorrow — the compact view must show those two.
    let days = vec![
        day(&dates[0], "rest", "full rest", None, ""),
        day(&dates[1], "gravel", "VO2 intervals", Some(90), "5x4min"),
        day(&dates[2], "gravel", "endurance ride", Some(120), "Z2"),
        day(&dates[3], "run", "easy shakeout", Some(40), "Z1"),
        day(&dates[4], "rest", "full rest", None, ""),
        day(
            &dates[5],
            "gravel",
            "long endurance",
            Some(270),
            "Z2 + tempo",
        ),
        day(&dates[6], "gravel", "recovery spin", Some(90), "Z1"),
    ];
    let week_start = start.format("%Y-%m-%d").to_string();
    let mut weeks = vec![PlanWeekInput {
        week_start: &week_start,
        focus: "build volume",
        days: &days,
        adjustment_reason: "",
    }];
    for (extra_start, extra_days) in extra_weeks {
        weeks.push(PlanWeekInput {
            week_start: extra_start,
            focus: "off the calendar",
            days: extra_days,
            adjustment_reason: "",
        });
    }
    resources
        .common
        .repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant.to_string(),
            user_id: &user_id.to_string(),
            coach_slug: None,
            goal_fact_id: None,
            outline: Some(PlanOutlineInput {
                goal_race: &goal,
                races: &[],
                strategy: "rebuild volume then sharpen",
                blocks: &blocks,
                source_conversation_id: None,
            }),
            weeks: &weeks,
        })
        .await?;
    Ok(())
}

fn ctx(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &str,
    args: Vec<String>,
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args,
        raw_text: "/plan".to_owned(),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message: true,
        conversation_id: Some(conversation_id.to_owned()),
        conversation_tenant_id: tenant_id,
        sender_id: None,
        tool_runtime: Arc::<ServerContext>::clone(resources),
    }
}

#[tokio::test]
async fn plan_with_no_saved_plan_reports_the_empty_state() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let response = PlanShowHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation_id, vec![]))
        .await?;
    assert!(
        response.text.contains("No plan saved yet"),
        "empty state must name the gap, got: {}",
        response.text
    );
    Ok(())
}

#[tokio::test]
async fn bare_plan_shows_the_goal_countdown_and_two_days() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation_id, vec![]))
        .await?;
    let text = response.text;

    // Goal line with the race and its date.
    assert!(text.contains("Unbound XL"), "goal race missing: {text}");
    assert!(text.contains("2026-10-03"), "race date missing: {text}");
    // Current block, from the outline.
    assert!(
        text.to_lowercase().contains("build"),
        "current block phase missing: {text}"
    );
    assert!(text.contains("14"), "weekly hours missing: {text}");
    // Exactly two day lines — today and tomorrow, never the whole week.
    assert!(
        text.contains("Today") && text.contains("Tomorrow"),
        "compact view must label today and tomorrow: {text}"
    );
    // Today is index 2 and tomorrow index 3, so those two sessions appear and
    // the rest of the week must not.
    assert!(
        text.contains("endurance ride"),
        "compact view must show today's session: {text}"
    );
    assert!(
        text.contains("easy shakeout"),
        "compact view must show tomorrow's session: {text}"
    );
    for other in ["VO2 intervals", "long endurance", "recovery spin"] {
        assert!(
            !text.contains(other),
            "compact view must not render the whole week ({other} leaked): {text}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn plan_week_renders_every_seeded_session() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            vec!["week".to_owned()],
        ))
        .await?;
    let text = response.text;

    assert!(text.contains("build volume"), "week focus missing: {text}");
    for session in [
        "VO2 intervals",
        "endurance ride",
        "easy shakeout",
        "long endurance",
        "recovery spin",
    ] {
        assert!(text.contains(session), "week must list {session}: {text}");
    }
    // Durations and intensities come through, not just names.
    assert!(text.contains("270min"), "duration missing: {text}");
    assert!(text.contains("5x4min"), "intensity missing: {text}");
    // Rest days read as rest, not as a blank session.
    assert!(text.contains("rest"), "rest day missing: {text}");
    Ok(())
}

#[tokio::test]
async fn plan_today_renders_a_single_day() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            vec!["today".to_owned()],
        ))
        .await?;
    let text = response.text;

    assert!(text.contains("Today"), "today label missing: {text}");
    assert!(
        text.contains("endurance ride"),
        "today view must show today's session: {text}"
    );
    assert!(
        !text.contains("easy shakeout"),
        "today view must not include tomorrow's session: {text}"
    );
    // The goal header still frames it.
    assert!(text.contains("Unbound XL"), "goal race missing: {text}");
    Ok(())
}

/// A stored plan can carry a date at the far edge of the proleptic calendar:
/// `parse_plan_date` checks the `YYYY-MM-DD` shape and chrono round-trips a
/// signed six-digit year, so `+262142-12-31` (`NaiveDate::MAX`) persists and is
/// read back intact. Closing a 255-week block or a 7-day week from there leaves
/// the calendar, and `/plan` must still answer the athlete.
#[tokio::test]
async fn plan_survives_a_stored_date_at_the_calendar_edge() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let edge_block = PlanBlock {
        phase: BlockPhase::Base,
        start: "+262142-01-01".to_owned(),
        weeks: 255,
        intent: "far side of the calendar".to_owned(),
        target_hours: None,
    };
    let edge_week = (
        "+262142-12-31".to_owned(),
        vec![day(
            "+262142-12-31",
            "gravel",
            "off-calendar ride",
            Some(60),
            "Z2",
        )],
    );
    seed_plan_with(&resources, user_id, tenant, &[edge_block], &[edge_week]).await?;

    for args in [vec![], vec!["week".to_owned()], vec!["today".to_owned()]] {
        let view = args
            .first()
            .cloned()
            .unwrap_or_else(|| "compact".to_owned());
        let response = PlanShowHandler
            .execute(&ctx(&resources, user_id, tenant, &conversation_id, args))
            .await?;
        let text = response.text;
        assert!(
            text.contains("Unbound XL"),
            "{view} view must still render the goal: {text}"
        );
        assert!(
            !text.contains("off-calendar ride"),
            "{view} view must not surface the unclosable week: {text}"
        );
    }

    // The real block still places today, so the header keeps naming its phase —
    // the edge block is passed over rather than shadowing it.
    let response = PlanShowHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation_id, vec![]))
        .await?;
    assert!(
        response.text.contains("Block: build, ~14h/wk"),
        "current block phase missing: {}",
        response.text
    );
    assert!(
        !response.text.contains("base"),
        "an off-calendar block contains no date, today included: {}",
        response.text
    );
    Ok(())
}

/// Seed a plan whose stored weeks all begin after today, with the outline's
/// first block starting alongside them.
///
/// This is the shape a plan takes when nothing covers the present — the coach
/// left a stretch unstructured, or the weeks that covered it were lost. `/plan`
/// must name the gap rather than render it as an ordinary empty day.
///
/// Returns the resume week's start date, as stored.
async fn seed_future_only_plan(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Result<String> {
    let today = chrono::Utc::now().date_naive();
    let resume = today + chrono::Days::new(5);
    let goal = GoalRace {
        name: "Harricana".to_owned(),
        date: (today + chrono::Days::new(120))
            .format("%Y-%m-%d")
            .to_string(),
        discipline: "trail run".to_owned(),
        priority: RacePriority::A,
    };
    let resume_start = resume.format("%Y-%m-%d").to_string();
    let second_start = (resume + chrono::Days::new(7))
        .format("%Y-%m-%d")
        .to_string();
    let blocks = vec![PlanBlock {
        phase: BlockPhase::Base,
        start: resume_start.clone(),
        weeks: 4,
        intent: "reintroduce structure after the break".to_owned(),
        target_hours: Some(9.0),
    }];
    let first_days: Vec<PlannedDay> = (0..7)
        .map(|i| {
            let d = (resume + chrono::Days::new(i))
                .format("%Y-%m-%d")
                .to_string();
            day(&d, "trail", "reintro trail", Some(60), "Z2")
        })
        .collect();
    let second_days: Vec<PlannedDay> = (0..7)
        .map(|i| {
            let d = (resume + chrono::Days::new(7 + i))
                .format("%Y-%m-%d")
                .to_string();
            day(&d, "trail", "build trail", Some(75), "Z2")
        })
        .collect();
    resources
        .common
        .repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant.to_string(),
            user_id: &user_id.to_string(),
            coach_slug: None,
            goal_fact_id: None,
            outline: Some(PlanOutlineInput {
                goal_race: &goal,
                races: &[],
                strategy: "rest, then rebuild toward the A race",
                blocks: &blocks,
                source_conversation_id: None,
            }),
            weeks: &[
                PlanWeekInput {
                    week_start: &resume_start,
                    focus: "reintroduction",
                    days: &first_days,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: &second_start,
                    focus: "build",
                    days: &second_days,
                    adjustment_reason: "",
                },
            ],
        })
        .await?;
    Ok(resume_start)
}

/// Seed a plan whose current week spans today but prescribes nothing on it —
/// a deliberate empty day inside a covered week.
async fn seed_week_missing_today(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Result<()> {
    let today = chrono::Utc::now().date_naive();
    let (start, dates) = week_dates();
    let goal = GoalRace {
        name: "Unbound XL".to_owned(),
        date: (today + chrono::Days::new(90))
            .format("%Y-%m-%d")
            .to_string(),
        discipline: "gravel".to_owned(),
        priority: RacePriority::A,
    };
    let blocks = vec![PlanBlock {
        phase: BlockPhase::Build,
        start: start.format("%Y-%m-%d").to_string(),
        weeks: 4,
        intent: "volume up".to_owned(),
        target_hours: Some(14.0),
    }];
    // Index 2 is today — every other day of the week is listed, today is not.
    let days: Vec<PlannedDay> = dates
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 2)
        .map(|(_, d)| day(d, "gravel", "endurance ride", Some(90), "Z2"))
        .collect();
    let week_start = start.format("%Y-%m-%d").to_string();
    resources
        .common
        .repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant.to_string(),
            user_id: &user_id.to_string(),
            coach_slug: None,
            goal_fact_id: None,
            outline: Some(PlanOutlineInput {
                goal_race: &goal,
                races: &[],
                strategy: "build volume",
                blocks: &blocks,
                source_conversation_id: None,
            }),
            weeks: &[PlanWeekInput {
                week_start: &week_start,
                focus: "build volume",
                days: &days,
                adjustment_reason: "",
            }],
        })
        .await?;
    Ok(())
}

/// A date no stored week spans must read as a hole in the plan, not as a
/// scheduled easy day. These two states used to share one string, so a plan
/// that had lost the weeks covering the present rendered exactly like a working
/// plan — the failure that made a data gap look like a broken command.
#[tokio::test]
async fn a_date_outside_every_stored_week_reports_no_coverage() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_future_only_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation_id, vec![]))
        .await?;
    let text = response.text;

    assert!(
        text.contains("not covered by the plan"),
        "an uncovered date must say the plan does not reach it: {text}"
    );
    assert!(
        !text.contains("nothing scheduled"),
        "an uncovered date must not borrow the empty-day wording: {text}"
    );
    // The plan is real — the goal still frames the answer.
    assert!(text.contains("Harricana"), "goal race missing: {text}");
    Ok(())
}

/// Knowing the plan does not cover today is only half an answer; the command
/// already selected the week it resumes in, so it must name that date.
#[tokio::test]
async fn a_gap_names_the_date_the_plan_resumes() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let resume_start = seed_future_only_plan(&resources, user_id, tenant).await?;

    for args in [vec![], vec!["today".to_owned()]] {
        let view = args
            .first()
            .cloned()
            .unwrap_or_else(|| "compact".to_owned());
        let response = PlanShowHandler
            .execute(&ctx(&resources, user_id, tenant, &conversation_id, args))
            .await?;
        let text = response.text;
        assert!(
            text.contains(&format!("The plan resumes on {resume_start}")),
            "{view} view must name the resume date {resume_start}: {text}"
        );
    }
    Ok(())
}

/// When today sits in a gap the outline does not phase, the header falls back
/// to the block covering the first week actually shown — dropping the line is
/// how the phase context went silently missing whenever a plan resumed later.
#[tokio::test]
async fn a_gap_still_names_the_phase_of_the_week_shown() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_future_only_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation_id, vec![]))
        .await?;
    assert!(
        response.text.contains("Block: base, ~9h/wk"),
        "header must name the phase of the week it is about to show: {}",
        response.text
    );
    Ok(())
}

/// Seed a plan whose every stored week already ended — the state a plan reaches
/// once its last week is behind the athlete and nothing has replaced it.
async fn seed_expired_plan(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Result<()> {
    let today = chrono::Utc::now().date_naive();
    let first = today - chrono::Days::new(30);
    let second = today - chrono::Days::new(23);
    let goal = GoalRace {
        name: "Unbound XL".to_owned(),
        date: (today - chrono::Days::new(16))
            .format("%Y-%m-%d")
            .to_string(),
        discipline: "gravel".to_owned(),
        priority: RacePriority::A,
    };
    let blocks = vec![PlanBlock {
        phase: BlockPhase::Taper,
        start: first.format("%Y-%m-%d").to_string(),
        weeks: 2,
        intent: "sharpen into the race".to_owned(),
        target_hours: Some(8.0),
    }];
    let first_start = first.format("%Y-%m-%d").to_string();
    let second_start = second.format("%Y-%m-%d").to_string();
    let first_days: Vec<PlannedDay> = (0..7)
        .map(|i| {
            let d = (first + chrono::Days::new(i))
                .format("%Y-%m-%d")
                .to_string();
            day(&d, "gravel", "taper spin", Some(45), "Z1")
        })
        .collect();
    let second_days: Vec<PlannedDay> = (0..7)
        .map(|i| {
            let d = (second + chrono::Days::new(i))
                .format("%Y-%m-%d")
                .to_string();
            day(&d, "gravel", "race week", Some(30), "Z1")
        })
        .collect();
    resources
        .common
        .repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant.to_string(),
            user_id: &user_id.to_string(),
            coach_slug: None,
            goal_fact_id: None,
            outline: Some(PlanOutlineInput {
                goal_race: &goal,
                races: &[],
                strategy: "taper and race",
                blocks: &blocks,
                source_conversation_id: None,
            }),
            weeks: &[
                PlanWeekInput {
                    week_start: &first_start,
                    focus: "taper",
                    days: &first_days,
                    adjustment_reason: "",
                },
                PlanWeekInput {
                    week_start: &second_start,
                    focus: "race week",
                    days: &second_days,
                    adjustment_reason: "",
                },
            ],
        })
        .await?;
    Ok(())
}

/// A plan whose weeks have all run out reports the gap without inventing a
/// resume date, and must stay distinguishable from having no plan at all — the
/// athlete still has a stored plan, it has simply ended.
#[tokio::test]
async fn a_plan_whose_weeks_have_all_ended_reports_no_coverage_and_no_resume() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_expired_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation_id, vec![]))
        .await?;
    let text = response.text;

    assert!(
        text.contains("not covered by the plan"),
        "an expired plan must report the gap: {text}"
    );
    assert!(
        !text.contains("The plan resumes on"),
        "there is no later week to resume into: {text}"
    );
    assert!(
        !text.contains("No plan saved yet"),
        "a stored plan that ended is not the same as no plan: {text}"
    );
    assert!(
        text.contains("Unbound XL"),
        "the stored plan still frames the answer: {text}"
    );
    Ok(())
}

/// `/plan week` on a plan that has not started must say so before it shows the
/// week. The header names a date, not a tense, so seven sessions rendered under
/// a future Monday read exactly like the current week — the athlete could train
/// next week's plan today believing it was prescribed for now.
#[tokio::test]
async fn the_week_view_reports_the_gap_before_showing_an_upcoming_week() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let resume_start = seed_future_only_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            vec!["week".to_owned()],
        ))
        .await?;
    let text = response.text;

    assert!(
        text.contains("not covered by the plan"),
        "the week view must say today is not covered: {text}"
    );
    assert!(
        text.contains(&format!("The plan resumes on {resume_start}")),
        "the week view must name the resume date {resume_start}: {text}"
    );
    // The week itself is still the point of the view.
    assert!(
        text.contains(&format!("Week of {resume_start}")),
        "the upcoming week must still be rendered: {text}"
    );
    Ok(())
}

/// `/plan week` on a plan whose weeks have all ended has no week to render at
/// all. It used to return the goal line alone — no sessions, no explanation —
/// while the compact view on the same data reported the gap.
#[tokio::test]
async fn the_week_view_reports_the_gap_when_no_week_is_left_to_show() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_expired_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            vec!["week".to_owned()],
        ))
        .await?;
    let text = response.text;

    assert!(
        text.contains("not covered by the plan"),
        "an expired plan must report the gap in the week view too: {text}"
    );
    assert!(
        !text.contains("The plan resumes on"),
        "there is no later week to resume into: {text}"
    );
    assert!(
        !text.contains("Week of"),
        "there is no week left to render: {text}"
    );
    Ok(())
}

/// The week view must stay silent about coverage when the plan does cover
/// today: the gap lines are for holes, and adding them to a healthy week would
/// make every athlete read a warning about a plan that is working.
#[tokio::test]
async fn the_week_view_says_nothing_about_coverage_when_today_is_covered() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_plan(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            vec!["week".to_owned()],
        ))
        .await?;
    let text = response.text;

    assert!(
        !text.contains("not covered by the plan"),
        "a covered week must not report a gap: {text}"
    );
    assert!(
        !text.contains("The plan resumes on"),
        "a plan that already covers today has nothing to resume: {text}"
    );
    assert!(
        text.contains("endurance ride"),
        "the current week's sessions must still render: {text}"
    );
    Ok(())
}

/// The mirror of the gap case: a week that spans today and simply prescribes
/// nothing on it is a rest day, and must keep saying so. Without this the
/// no-coverage wording could be introduced by relabelling every empty day,
/// which would lose exactly the distinction it was added to make.
#[tokio::test]
async fn an_empty_day_inside_a_covered_week_still_reports_no_session() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_week_missing_today(&resources, user_id, tenant).await?;

    let response = PlanShowHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation_id, vec![]))
        .await?;
    let text = response.text;

    assert!(
        text.contains("nothing scheduled"),
        "a covered day with no session is a rest day: {text}"
    );
    assert!(
        !text.contains("not covered by the plan"),
        "a covered day must not be reported as a hole in the plan: {text}"
    );
    assert!(
        !text.contains("The plan resumes on"),
        "a plan that already covers today has nothing to resume: {text}"
    );
    Ok(())
}

/// The plan arrives whole, and the surface's ceiling splits it rather than
/// cutting it.
///
/// Two regressions meet in this one reply. `/plan` used to cut every channel
/// at one hardcoded floor, spending Slack's 40,000-character headroom as if it
/// were 2,000 (registre#1); it then cut at the *right* number but still cut,
/// appending a "truncated" marker and telling the athlete to run a different
/// command to see the rest (registre#2). The handler renders the whole plan
/// now, and the egress splits it into messages each channel will accept —
/// sized by that channel's own number, never a constant.
#[tokio::test]
async fn the_whole_plan_is_rendered_and_the_surface_ceiling_only_splits_it() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    seed_plan(&resources, user_id, tenant).await?;

    let text = PlanShowHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            vec!["week".to_owned()],
        ))
        .await?
        .text;

    assert!(
        text.contains("long endurance"),
        "the tail of the week must survive the render: {text}"
    );
    assert!(
        !text.contains("truncated"),
        "nothing is cut, so nothing announces a cut: {text}"
    );
    assert!(
        text.chars().count() > 240,
        "fixture must be long enough to split: {} chars",
        text.chars().count()
    );

    // The same body, laid out for two channels an order of magnitude apart.
    let cramped = chunk_reply(&text, 240);
    let roomy = chunk_reply(&text, 40_000);

    assert!(
        cramped.len() > 1,
        "a 240-character surface receives several messages, not a truncated one"
    );
    assert_eq!(
        roomy.len(),
        1,
        "a 40,000-character surface carries the whole plan in one"
    );
    for (index, part) in cramped.iter().enumerate() {
        assert!(
            part.chars().count() <= 240,
            "part {index} is {} characters, over the surface ceiling",
            part.chars().count()
        );
    }
    let split: String = cramped
        .concat()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let whole: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(split, whole, "the split loses nothing the render produced");
    assert!(
        cramped.iter().any(|part| part.contains("long endurance")),
        "the tail the old cap dropped now arrives in a later message"
    );
    Ok(())
}
