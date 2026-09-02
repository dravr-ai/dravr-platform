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
use pierre_chat_pipeline::{dispatch_slash, CommandPersistence, SlashRequest};
use pierre_commands::plan::{PlanShareHandler, PlanShowHandler};
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::chunking::chunk_reply;
use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
use pierre_core::models::groups::{
    CoachingGroup, GroupMember, GroupRespondMode, GroupRole, TranscriptSpeaker,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::training_plans::{BlockPhase, GoalRace, PlanBlock, PlannedDay, RacePriority};
use pierre_messaging::rich_text::{parse_markdown, render_rich_text};
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
        steps: Vec::new(),
        fueling: None,
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
    seed_plan_with(resources, user_id, tenant, None, &[], &[]).await
}

/// The same plan, filed under a specific coach persona slug.
///
/// A coach-agnostic plan renders under ANY coach lookup (`get_active_plan`
/// falls back to the agnostic row), so it cannot tell whether the handler
/// resolved the right coach. A plan under a slug only the athlete's selected
/// coach (or the conversation's) resolves can.
async fn seed_plan_under_coach(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    coach_slug: &str,
) -> Result<()> {
    seed_plan_with(resources, user_id, tenant, Some(coach_slug), &[], &[]).await
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
    coach_slug: Option<&str>,
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
            coach_slug,
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

/// Where the command was typed: which tenant owns the conversation row,
/// whether the athlete is alone with the coach, and whether a messaging
/// channel (a sender id) carried it — the three signals the plan handlers
/// branch on.
struct Surface<'a> {
    conversation_tenant: TenantId,
    is_direct_message: bool,
    sender_id: Option<&'a str>,
}

fn ctx_on(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &str,
    args: Vec<String>,
    surface: &Surface<'_>,
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args,
        raw_text: "/plan".to_owned(),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message: surface.is_direct_message,
        ambient_group_fallback: true,
        conversation_id: Some(conversation_id.to_owned()),
        conversation_tenant_id: surface.conversation_tenant,
        sender_id: surface.sender_id.map(ToOwned::to_owned),
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(resources),
    }
}

/// A messaging DM: the conversation lives under the athlete's own tenant.
fn ctx(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &str,
    args: Vec<String>,
) -> PlatformCommandContext {
    ctx_on(
        resources,
        user_id,
        tenant_id,
        conversation_id,
        args,
        &Surface {
            conversation_tenant: tenant_id,
            is_direct_message: true,
            sender_id: None,
        },
    )
}

/// A shared messaging room, as the ingress files it: the conversation row
/// lives under the BOT's tenant — one the athlete does not belong to — so a
/// lookup under the athlete's own tenant never finds it.
///
/// The bot tenant is another user's tenant; the room conversation is created
/// there for the athlete, bound to `coach_id`, exactly as the messaging
/// session opener does for a group chat.
async fn room_conversation(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    coach_id: Option<&str>,
) -> Result<(TenantId, String)> {
    let bot_tenant = bot_tenant(resources).await?;
    let conversation = room_conversation_in(resources, user_id, bot_tenant, coach_id, None).await?;
    Ok((bot_tenant, conversation))
}

/// The messaging bot's tenant: owned by another user, never joined by the
/// athlete.
async fn bot_tenant(resources: &Arc<ServerContext>) -> Result<TenantId> {
    let bot_email = format!("planbot_{}@example.com", Uuid::new_v4());
    let (bot_owner, _) =
        common::create_test_user_with_email(resources.database(), &bot_email).await?;
    Ok(resources
        .common
        .repos
        .tenants
        .get_all()
        .await?
        .iter()
        .find(|t| t.owner_user_id == bot_owner)
        .map(|t| t.id)
        .expect("bot owner owns a tenant"))
}

async fn room_conversation_in(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    bot_tenant: TenantId,
    coach_id: Option<&str>,
    group_id: Option<&str>,
) -> Result<String> {
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            bot_tenant,
            "room",
            "gemini-2.0-flash",
            coach_id,
            group_id,
        )
        .await?;
    Ok(conversation.id)
}

/// A coaching group under the bot tenant with the athlete as a member — the
/// shape a channel-bound room takes once the bot has enrolled its speakers.
async fn bound_room_group(
    resources: &Arc<ServerContext>,
    bot_tenant: TenantId,
    user_id: Uuid,
    user_tenant: TenantId,
) -> Result<Uuid> {
    let persona = seed_persona(resources, user_id, user_tenant, "Room Persona").await?;
    let group_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    resources
        .common
        .repos
        .groups
        .create_group(
            bot_tenant,
            &CoachingGroup {
                id: group_id,
                tenant_id: bot_tenant.to_string(),
                name: "Room Squad".to_owned(),
                description: None,
                coach_id: persona,
                owner_id: user_id,
                coach_user_id: None,
                peer_data_sharing: true,
                respond_mode: GroupRespondMode::default(),
                max_members: 20,
                is_active: true,
                channel_type: Some("telegram".to_owned()),
                channel_chat_id: Some("-100123".to_owned()),
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
    resources
        .common
        .repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: bot_tenant.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await?;
    Ok(group_id)
}

const ROOM_SENDER: &str = "telegram-user-42";
const SHARED_MARKER: &str = "shared with the room";

/// A coach persona in the athlete's tenant. The selected-coach pointer is a
/// foreign key onto `coaches`, so a plan's coach slug has to be a real
/// persona id, not a made-up string.
async fn seed_persona(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    title: &str,
) -> Result<String> {
    let coach = resources
        .common
        .repos
        .coaches
        .create_system_coach(
            user_id,
            tenant,
            &CreateSystemCoachRequest {
                title: title.to_owned(),
                description: None,
                system_prompt: "Test prompt".to_owned(),
                category: CoachCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: CoachVisibility::Global,
            },
        )
        .await?;
    Ok(coach.id.to_string())
}

/// An athlete whose plan was built in their DM under their selected coach —
/// the row a bare tenant/None lookup never finds.
async fn athlete_with_a_coached_plan(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    display_name: &str,
) -> Result<()> {
    resources
        .common
        .repos
        .users
        .update_display_name(user_id, display_name)
        .await?;
    let selected = seed_persona(resources, user_id, tenant, "Share Coach").await?;
    resources
        .common
        .repos
        .tenants
        .set_selected_coach(tenant, user_id, Some(&selected))
        .await?;
    seed_plan_under_coach(resources, user_id, tenant, &selected).await
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
    seed_plan_with(
        &resources,
        user_id,
        tenant,
        None,
        &[edge_block],
        &[edge_week],
    )
    .await?;

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

/// `/plan share` in a messaging room opens with the athlete's name and says
/// the plan is shared, then renders the plan their DM built under their
/// selected coach — read from a room conversation the athlete's own tenant
/// cannot resolve, with no coach bound to it.
#[tokio::test]
async fn plan_share_in_a_messaging_room_posts_the_header_and_the_week() -> Result<()> {
    let (resources, user_id, tenant, _dm) = setup().await?;
    athlete_with_a_coached_plan(&resources, user_id, tenant, "Phil Tremblay").await?;
    let (bot_tenant, room) = room_conversation(&resources, user_id, None).await?;

    let response = PlanShareHandler
        .execute(&ctx_on(
            &resources,
            user_id,
            tenant,
            &room,
            vec!["week".to_owned()],
            &Surface {
                conversation_tenant: bot_tenant,
                is_direct_message: false,
                sender_id: Some(ROOM_SENDER),
            },
        ))
        .await?;
    let text = response.text;

    assert!(
        response.is_rich_text,
        "the header carries **, so the reply is rich text"
    );
    assert!(
        text.starts_with("📋"),
        "the header must open the reply so attribution comes first: {text}"
    );
    assert!(
        text.contains("**Phil Tremblay**") && text.contains(SHARED_MARKER),
        "the room must read whose plan it is and that it was shared: {text}"
    );
    for session in ["VO2 intervals", "endurance ride", "long endurance"] {
        assert!(
            text.contains(session),
            "the week built under the selected coach must render ({session}): {text}"
        );
    }
    assert!(
        !text.contains("No plan saved yet"),
        "a plan filed under the selected coach must be found from the room: {text}"
    );
    Ok(())
}

/// In a DM there is no room to share with, so `/plan share` is `/plan`.
#[tokio::test]
async fn plan_share_in_a_dm_renders_exactly_like_plan() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    athlete_with_a_coached_plan(&resources, user_id, tenant, "Phil Tremblay").await?;

    let share = PlanShareHandler
        .execute(&ctx_on(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            vec![],
            &Surface {
                conversation_tenant: tenant,
                is_direct_message: true,
                sender_id: Some(ROOM_SENDER),
            },
        ))
        .await?
        .text;
    let plain = PlanShowHandler
        .execute(&ctx(&resources, user_id, tenant, &conversation_id, vec![]))
        .await?
        .text;

    assert!(
        !share.contains(SHARED_MARKER) && !share.contains("Phil Tremblay"),
        "a DM has no room, so no header: {share}"
    );
    assert!(
        share.contains("endurance ride"),
        "the plan itself still renders: {share}"
    );
    assert_eq!(
        share, plain,
        "the DM share reply is byte-identical to /plan"
    );
    Ok(())
}

/// Regression: `/plan` in a room looked the conversation up under the
/// caller's tenant, missed the bot-tenant row, and fell back to the
/// coach-agnostic plan — an athlete whose plan lived under their selected
/// coach read "No plan saved yet" in the room.
#[tokio::test]
async fn plan_in_a_room_finds_the_plan_built_under_the_selected_coach() -> Result<()> {
    let (resources, user_id, tenant, _dm) = setup().await?;
    athlete_with_a_coached_plan(&resources, user_id, tenant, "Phil Tremblay").await?;
    let (bot_tenant, room) = room_conversation(&resources, user_id, None).await?;

    let text = PlanShowHandler
        .execute(&ctx_on(
            &resources,
            user_id,
            tenant,
            &room,
            vec![],
            &Surface {
                conversation_tenant: bot_tenant,
                is_direct_message: false,
                sender_id: Some(ROOM_SENDER),
            },
        ))
        .await?
        .text;

    assert!(
        text.contains("endurance ride") && text.contains("easy shakeout"),
        "today's and tomorrow's sessions must render from the room: {text}"
    );
    assert!(
        !text.contains("No plan saved yet"),
        "the selected-coach rung of the ladder must find the plan: {text}"
    );
    Ok(())
}

/// A room conversation that DOES bind a coach — read under the tenant that
/// owns the row — wins over the athlete's selection, matching how the plan
/// injection keys on the conversation's coach.
#[tokio::test]
async fn a_room_conversation_bound_to_a_coach_reads_that_coachs_plan() -> Result<()> {
    let (resources, user_id, tenant, _dm) = setup().await?;
    // The selection points at a coach with no plan; only the conversation's
    // coach has one, so a ladder in the wrong order renders the empty state.
    let other_coach = seed_persona(&resources, user_id, tenant, "Other Coach").await?;
    let room_coach = seed_persona(&resources, user_id, tenant, "Room Coach").await?;
    resources
        .common
        .repos
        .tenants
        .set_selected_coach(tenant, user_id, Some(&other_coach))
        .await?;
    seed_plan_under_coach(&resources, user_id, tenant, &room_coach).await?;
    let (bot_tenant, room) = room_conversation(&resources, user_id, Some(&room_coach)).await?;

    let text = PlanShowHandler
        .execute(&ctx_on(
            &resources,
            user_id,
            tenant,
            &room,
            vec!["today".to_owned()],
            &Surface {
                conversation_tenant: bot_tenant,
                is_direct_message: false,
                sender_id: Some(ROOM_SENDER),
            },
        ))
        .await?
        .text;

    assert!(
        text.contains("endurance ride"),
        "the conversation's coach must be honoured from the bot tenant: {text}"
    );
    assert!(!text.contains("No plan saved yet"), "{text}");
    Ok(())
}

/// The header is inline markdown and the name is user-set: a `*` in a display
/// name would otherwise open an emphasis run and swallow the header. Angle
/// brackets are the channel egress's business — each renderer escapes its own
/// text nodes — so the handler leaves them alone.
#[tokio::test]
async fn plan_share_escapes_the_display_name_in_the_header() -> Result<()> {
    let (resources, user_id, tenant, _dm) = setup().await?;
    athlete_with_a_coached_plan(&resources, user_id, tenant, "Marc *<3* vélo & co").await?;
    let (bot_tenant, room) = room_conversation(&resources, user_id, None).await?;

    let text = PlanShareHandler
        .execute(&ctx_on(
            &resources,
            user_id,
            tenant,
            &room,
            vec![],
            &Surface {
                conversation_tenant: bot_tenant,
                is_direct_message: false,
                sender_id: Some(ROOM_SENDER),
            },
        ))
        .await?
        .text;

    assert!(
        text.contains(r"**Marc \*<3\* vélo & co**"),
        "the name's markdown metacharacters must be escaped inside the bold run: {text}"
    );
    let dialect = render_rich_text(&parse_markdown(&text));
    assert!(
        dialect.contains("<b>Marc *<3* vélo & co</b>"),
        "the channel egress must read the name back verbatim inside one bold span: {dialect}"
    );
    Ok(())
}

/// An in-app group thread is not a DM, but it persists the reply into the
/// caller's own conversation alone — nothing is posted to a room, so the
/// header would misstate its audience. No sender id, no header.
#[tokio::test]
async fn plan_share_on_the_in_app_surface_renders_no_header() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    athlete_with_a_coached_plan(&resources, user_id, tenant, "Phil Tremblay").await?;

    let text = PlanShareHandler
        .execute(&ctx_on(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            vec![],
            &Surface {
                conversation_tenant: tenant,
                is_direct_message: false,
                sender_id: None,
            },
        ))
        .await?
        .text;

    assert!(
        !text.contains(SHARED_MARKER) && !text.contains("**Phil Tremblay**"),
        "web renders exactly like /plan, header-free: {text}"
    );
    assert!(text.contains("endurance ride"), "{text}");
    Ok(())
}

/// An athlete with nothing saved still shares legibly: the room learns whose
/// (absent) plan it is reading rather than an unattributed empty state.
#[tokio::test]
async fn plan_share_with_no_plan_in_a_room_still_names_the_athlete() -> Result<()> {
    let (resources, user_id, tenant, _dm) = setup().await?;
    resources
        .common
        .repos
        .users
        .update_display_name(user_id, "Phil Tremblay")
        .await?;
    let (bot_tenant, room) = room_conversation(&resources, user_id, None).await?;

    let text = PlanShareHandler
        .execute(&ctx_on(
            &resources,
            user_id,
            tenant,
            &room,
            vec![],
            &Surface {
                conversation_tenant: bot_tenant,
                is_direct_message: false,
                sender_id: Some(ROOM_SENDER),
            },
        ))
        .await?
        .text;

    assert!(
        text.contains("**Phil Tremblay**") && text.contains("No plan saved yet"),
        "header then the empty state: {text}"
    );
    Ok(())
}

/// A `/plan share` typed in a shared room is the room's history too: both
/// rows fan out to the group transcript — what the ambient block a later
/// room turn reads is built from — so a coach can discuss the plan the
/// athlete just shared. Bare `/plan` in the same room is answered privately
/// and leaves no trace there.
#[tokio::test]
async fn plan_share_in_a_room_lands_in_the_group_transcript_and_plan_does_not() -> Result<()> {
    let (resources, user_id, tenant, _dm) = setup().await?;
    athlete_with_a_coached_plan(&resources, user_id, tenant, "Phil Tremblay").await?;
    let bot = bot_tenant(&resources).await?;
    let group_id = bound_room_group(&resources, bot, user_id, tenant).await?;
    let room =
        room_conversation_in(&resources, user_id, bot, None, Some(&group_id.to_string())).await?;
    let pipeline = resources.chat_pipeline_context();
    let request = |text: &'static str| SlashRequest {
        user_id,
        tenant_id: tenant,
        conversation_id: &room,
        conversation_tenant_id: bot,
        channel_type: "telegram",
        locale: "en",
        is_direct_message: false,
        ambient_group_fallback: true,
        persistence: CommandPersistence::RoomVisibleOnly,
        sender_id: Some(ROOM_SENDER),
        text,
    };

    let shared = dispatch_slash(&pipeline, &request("/plan share week"))
        .await?
        .expect("/plan share is a catalogued command");
    assert_eq!(shared.command_name.as_deref(), Some("plan-share"));
    assert!(
        shared.persisted.is_some(),
        "a room-visible turn is written to the room conversation"
    );

    let entries = resources
        .common
        .repos
        .groups
        .list_transcript_visible_to(&group_id.to_string(), user_id, 20)
        .await?;
    let reply = entries
        .iter()
        .find(|e| e.speaker == TranscriptSpeaker::Coach)
        .expect("the shared plan reaches the group transcript as the coach line");
    assert!(
        reply.content.contains("**Phil Tremblay**")
            && reply.content.contains(SHARED_MARKER)
            && reply.content.contains("long endurance"),
        "the transcript carries the header and the week: {}",
        reply.content
    );
    assert!(
        entries
            .iter()
            .any(|e| e.speaker == TranscriptSpeaker::Member && e.content == "/plan share week"),
        "the athlete's own `/plan share` line is in the transcript: {entries:?}"
    );
    let shared_rows = entries.len();

    let private = dispatch_slash(&pipeline, &request("/plan"))
        .await?
        .expect("/plan is a catalogued command");
    assert_eq!(private.command_name.as_deref(), Some("plan"));
    assert!(
        private.persisted.is_none(),
        "a privately answered command is not the room's history"
    );
    let after = resources
        .common
        .repos
        .groups
        .list_transcript_visible_to(&group_id.to_string(), user_id, 20)
        .await?;
    assert_eq!(
        after.len(),
        shared_rows,
        "bare /plan must add nothing to the group transcript: {after:?}"
    );
    Ok(())
}
