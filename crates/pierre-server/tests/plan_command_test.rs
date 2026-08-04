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
