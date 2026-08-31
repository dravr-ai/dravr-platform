// ABOUTME: The plan tools' `athlete` argument — a group's human coach acting on a consenting athlete's plan
// ABOUTME: Asserts the row lands under the ATHLETE's tenant/user/coach, and every refusal by its text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `save_training_plan` / `get_training_plan` scope on the caller unless the
//! caller is the human coach (`coaching_groups.coach_user_id`) of a group the
//! named athlete belongs to, the athlete has consented to peer sharing, the
//! athlete lives in exactly one tenant, and the call comes from a direct chat.
//!
//! Every positive assertion reads the athlete's row back through the
//! repository under the athlete's own `(tenant, user, selected coach)` — a
//! stub that ignored `athlete` and saved the coach's own plan would pass a
//! tool-only round trip and fail here. Every refusal is asserted by its text,
//! because the model relays that text to the coach.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::Utc;
use common::create_test_server_resources;
use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_commands::plan::PlanShowHandler;
use pierre_commands::{CommandHandler, PlatformCommandContext};
use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
use pierre_core::models::groups::{
    CoachingGroup, GroupMember, GroupRespondMode, GroupRole, UpdateGroupRequest,
};
use pierre_core::models::{
    Tenant, TenantId, TenantPlan, ToolCatalogEntry, ToolCategory, User, UserStatus,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plans::{
    GetTrainingPlanTool, SaveTrainingPlanTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::task::spawn_blocking;
use uuid::Uuid;

const COACH_WORKOUT: &str = "coach tempo 3x10min";

async fn seed_user(resources: &ServerContext, label: &str, display_name: Option<&str>) -> Uuid {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();
    let mut user = User::new(
        format!("{label}-{}@test.com", Uuid::new_v4()),
        password_hash,
        display_name.map(ToOwned::to_owned),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();
    user_id
}

/// A tenant the user owns — `create` also enrols the owner in `tenant_users`,
/// which is the row `list_for_user` and the selected coach read.
async fn create_tenant_owned_by(resources: &ServerContext, owner_id: Uuid) -> TenantId {
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    let tenant = Tenant {
        id: tenant_id,
        name: "Scope Tenant".to_owned(),
        slug: format!("scope-{tenant_id}"),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: owner_id,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    tenant_id
}

async fn seed_coach_persona(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) -> Uuid {
    resources
        .common
        .repos
        .coaches
        .create_system_coach(
            user_id,
            tenant_id,
            &CreateSystemCoachRequest {
                title: "Scope Coach".to_owned(),
                description: None,
                system_prompt: "Test prompt".to_owned(),
                category: CoachCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: CoachVisibility::Global,
            },
        )
        .await
        .unwrap()
        .id
}

async fn create_group(
    resources: &ServerContext,
    tenant_id: TenantId,
    persona: Uuid,
    owner_id: Uuid,
    coach_user_id: Option<Uuid>,
    peer_data_sharing: bool,
    name: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let group = CoachingGroup {
        id,
        tenant_id: tenant_id.to_string(),
        name: name.to_owned(),
        description: None,
        coach_id: persona.to_string(),
        owner_id,
        coach_user_id,
        peer_data_sharing,
        respond_mode: GroupRespondMode::default(),
        max_members: 20,
        is_active: true,
        channel_type: None,
        channel_chat_id: None,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .groups
        .create_group(tenant_id, &group)
        .await
        .unwrap();
    // The insert never carries the human coach; redeeming a coach invite
    // attaches one through this write, so the fixture does the same.
    if coach_user_id.is_some() {
        resources
            .common
            .repos
            .groups
            .set_group_coach_user(&id.to_string(), coach_user_id, tenant_id)
            .await
            .unwrap();
    }
    id
}

async fn add_member(
    resources: &ServerContext,
    group_id: Uuid,
    user_id: Uuid,
    tenant_id: TenantId,
    role: GroupRole,
    consent: bool,
) {
    let now = Utc::now();
    resources
        .common
        .repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role,
            peer_sharing_consent: consent,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// A human coach with their own tenant, and a consenting athlete in a group
/// the coach is attached to as `coach_user_id`. The athlete selected a coach
/// persona in their own tenant, which is where their plan is filed.
struct Fixture {
    resources: Arc<ServerContext>,
    coach: Uuid,
    coach_tenant: TenantId,
    athlete: Uuid,
    athlete_tenant: TenantId,
    /// The coach persona the athlete selected in their own tenant — the slug
    /// their DM and `/plan` read a plan under. A coach-scoped save must land
    /// under it, never under the coach's own persona or the agnostic row.
    athlete_coach: String,
    group_id: Uuid,
}

async fn coached_athlete(athlete_name: Option<&str>, consent: bool) -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let coach = seed_user(&resources, "coach", Some("Coach Karine")).await;
    let coach_tenant = create_tenant_owned_by(&resources, coach).await;
    let persona = seed_coach_persona(&resources, coach, coach_tenant).await;

    let athlete = seed_user(&resources, "athlete", athlete_name).await;
    let athlete_tenant = create_tenant_owned_by(&resources, athlete).await;
    let athlete_coach = seed_coach_persona(&resources, athlete, athlete_tenant)
        .await
        .to_string();
    resources
        .common
        .repos
        .tenants
        .set_selected_coach(athlete_tenant, athlete, Some(&athlete_coach))
        .await
        .unwrap();

    let group_id = create_group(
        &resources,
        coach_tenant,
        persona,
        coach,
        Some(coach),
        true,
        "Tempo Squad",
    )
    .await;
    add_member(
        &resources,
        group_id,
        athlete,
        athlete_tenant,
        GroupRole::Member,
        consent,
    )
    .await;

    Fixture {
        resources,
        coach,
        coach_tenant,
        athlete,
        athlete_tenant,
        athlete_coach,
        group_id,
    }
}

fn tool_context(user_id: Uuid, tenant_id: TenantId) -> ToolContext {
    ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer")
}

fn structured(response: &ToolResponse) -> Value {
    response
        .structured_content
        .clone()
        .expect("tool result carries structured content")
}

fn error_text(payload: &Value) -> &str {
    payload
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected a refusal, got: {payload}"))
}

/// The week the coach saves: starts two days ago so today falls on the
/// coach-written session and the athlete's compact `/plan` shows it.
fn coach_week() -> Value {
    let today = Utc::now().date_naive();
    let start = today - chrono::Days::new(2);
    let date = |offset: u64| {
        (start + chrono::Days::new(offset))
            .format("%Y-%m-%d")
            .to_string()
    };
    json!({
        "week_start": date(0),
        "focus": "coach-written week",
        "days": [
            { "date": date(0), "sport": "rest", "workout": "full rest", "intensity": "" },
            { "date": date(1), "sport": "run", "workout": "easy 40min", "duration_min": 40, "intensity": "Z1" },
            { "date": date(2), "sport": "run", "workout": COACH_WORKOUT, "duration_min": 60, "intensity": "tempo" },
            { "date": date(3), "sport": "run", "workout": "easy 30min", "duration_min": 30, "intensity": "Z1" },
        ]
    })
}

fn save_payload(athlete: &str) -> Value {
    json!({
        "athlete": athlete,
        "outline": {
            "goal_race": { "name": "Harricana 80", "date": "2026-09-12", "discipline": "trail", "priority": "A" },
            "strategy": "hold the tempo work, taper the last ten days"
        },
        "weeks": [coach_week()],
    })
}

async fn save_as(
    resources: &Arc<ServerContext>,
    user: Uuid,
    tenant: TenantId,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let response = SaveTrainingPlanTool
        .execute(&runtime, &tool_context(user, tenant), args)
        .await;
    structured(&response)
}

async fn get_as(
    resources: &Arc<ServerContext>,
    user: Uuid,
    tenant: TenantId,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let response = GetTrainingPlanTool
        .execute(&runtime, &tool_context(user, tenant), args)
        .await;
    structured(&response)
}

/// The athlete's `/plan`, from a DM conversation under their own tenant that
/// binds no coach — so the selected-coach rung of the ladder is what finds
/// the plan.
async fn athlete_plan_command(fx: &Fixture) -> String {
    let conversation = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &fx.athlete.to_string(),
            fx.athlete_tenant,
            "dm",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await
        .unwrap();
    let ctx = PlatformCommandContext {
        user_id: fx.athlete,
        tenant_id: fx.athlete_tenant,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: "/plan".to_owned(),
        ctx: Arc::<ServerContext>::clone(&fx.resources)
            as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: Some(conversation.id),
        conversation_tenant_id: fx.athlete_tenant,
        sender_id: None,
        tool_runtime: Arc::<ServerContext>::clone(&fx.resources),
    };
    PlanShowHandler.execute(&ctx).await.unwrap().text
}

// ════════════════════════════════════════════════════════════════════════
// The positive path
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn the_coach_saves_a_week_under_the_athletes_own_tenant_user_and_coach() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;

    // A DM-shaped call: no conversation on the tool context at all.
    let saved = save_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        save_payload("phil"),
    )
    .await;
    assert!(
        saved.get("error").is_none(),
        "the attached coach must not be refused, got: {saved}"
    );
    assert_eq!(
        saved.get("athlete").and_then(Value::as_str),
        Some("Phil Tremblay"),
        "the result names whose plan changed: {saved}"
    );
    assert_eq!(saved.get("weeks_saved").and_then(Value::as_u64), Some(1));

    // The row is the ATHLETE's: their tenant, their user id, their selected
    // coach — exactly what their DM reads.
    let repos = &fx.resources.common.repos;
    let plan = repos
        .training_plans
        .get_active_plan(
            &fx.athlete_tenant.to_string(),
            &fx.athlete.to_string(),
            Some(fx.athlete_coach.as_str()),
        )
        .await
        .unwrap()
        .expect("the plan lands under the athlete's tenant/user/selected coach");
    assert_eq!(plan.coach_slug.as_deref(), Some(fx.athlete_coach.as_str()));
    assert_eq!(plan.goal_race.name, "Harricana 80");
    let weeks = repos
        .training_plans
        .list_plan_weeks(
            &fx.athlete_tenant.to_string(),
            &fx.athlete.to_string(),
            &plan.id,
            false,
        )
        .await
        .unwrap();
    assert_eq!(weeks.len(), 1, "one coach-written week");
    assert!(
        weeks[0].days.iter().any(|d| d.workout == COACH_WORKOUT),
        "the coach's session is stored verbatim: {:?}",
        weeks[0].days
    );

    // The coach's own (tenant, user) holds NO plan row — nothing landed on
    // the caller.
    for coach in [None, Some(fx.athlete_coach.as_str())] {
        assert!(
            repos
                .training_plans
                .get_active_plan(&fx.coach_tenant.to_string(), &fx.coach.to_string(), coach)
                .await
                .unwrap()
                .is_none(),
            "the coach must have no plan of their own after saving for an athlete"
        );
    }

    // The read side follows the same scope.
    let fetched = get_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        json!({ "athlete": "Phil" }),
    )
    .await;
    assert_eq!(
        fetched.get("athlete").and_then(Value::as_str),
        Some("Phil Tremblay"),
        "{fetched}"
    );
    let fetched_weeks = fetched
        .get("weeks")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("weeks array present: {fetched}"));
    assert_eq!(fetched_weeks.len(), 1);
    assert!(
        fetched_weeks[0].to_string().contains(COACH_WORKOUT),
        "get_training_plan athlete= returns the coach-written session: {fetched}"
    );

    // And the athlete's own `/plan` renders the session the coach wrote.
    let text = athlete_plan_command(&fx).await;
    assert!(
        text.contains(COACH_WORKOUT),
        "the athlete's /plan must show the coach-saved session: {text}"
    );
}

/// A solo conversation under the coach's own tenant is a direct chat: the
/// gate is about rooms, not about having a conversation at all.
#[tokio::test]
async fn a_solo_conversation_under_the_coachs_tenant_is_a_direct_chat() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;
    let conversation = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &fx.coach.to_string(),
            fx.coach_tenant,
            "coach dm",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let saved = CONVERSATION_ID
        .scope(Some(conversation.id), async {
            save_as(
                &fx.resources,
                fx.coach,
                fx.coach_tenant,
                save_payload("phil"),
            )
            .await
        })
        .await;

    assert!(
        saved.get("error").is_none(),
        "a solo thread is a direct chat, got: {saved}"
    );
    assert_eq!(
        saved.get("athlete").and_then(Value::as_str),
        Some("Phil Tremblay")
    );
}

// ════════════════════════════════════════════════════════════════════════
// Refusals, each by its text
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a_non_consenting_athlete_is_refused() {
    let fx = coached_athlete(Some("Phil Tremblay"), false).await;

    let saved = save_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        save_payload("phil"),
    )
    .await;
    let err = error_text(&saved);
    assert!(
        err.contains("hasn't shared") && err.contains("/group consent yes"),
        "consent is the athlete's grant, got: {err}"
    );
    assert!(
        fx.resources
            .common
            .repos
            .training_plans
            .get_active_plan(
                &fx.athlete_tenant.to_string(),
                &fx.athlete.to_string(),
                Some(fx.athlete_coach.as_str())
            )
            .await
            .unwrap()
            .is_none(),
        "nothing may be written for a non-consenting athlete"
    );

    let fetched = get_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        json!({ "athlete": "phil" }),
    )
    .await;
    assert!(
        error_text(&fetched).contains("hasn't shared"),
        "the read side applies the same gate: {fetched}"
    );
}

#[tokio::test]
async fn the_groups_kill_switch_refuses_despite_consent() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;
    let repos = &fx.resources.common.repos;
    // A second, sharing-disabled group coached by the same coach with the
    // same athlete: the resolver prefers the open row, so switch the only
    // group off instead.
    let group = repos
        .groups
        .get_group(&fx.group_id.to_string(), fx.coach_tenant)
        .await
        .unwrap()
        .unwrap();
    repos
        .groups
        .update_group(
            &group.id.to_string(),
            fx.coach_tenant,
            &UpdateGroupRequest {
                name: None,
                description: None,
                coach_id: None,
                max_members: None,
                peer_data_sharing: Some(false),
                respond_mode: None,
                is_active: None,
            },
        )
        .await
        .unwrap();

    let saved = save_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        save_payload("phil"),
    )
    .await;
    assert!(
        error_text(&saved).contains("disabled for group"),
        "got: {saved}"
    );
}

/// The group's Owner is not its human coach: on a channel-bound group the
/// owner is whoever spoke first.
#[tokio::test]
async fn a_group_owner_who_is_not_the_attached_coach_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let owner = seed_user(&resources, "owner", Some("Owner Olga")).await;
    let owner_tenant = create_tenant_owned_by(&resources, owner).await;
    let persona = seed_coach_persona(&resources, owner, owner_tenant).await;
    let athlete = seed_user(&resources, "athlete", Some("Phil Tremblay")).await;
    let athlete_tenant = create_tenant_owned_by(&resources, athlete).await;

    let gid = create_group(
        &resources,
        owner_tenant,
        persona,
        owner,
        None,
        true,
        "Tempo Squad",
    )
    .await;
    add_member(&resources, gid, owner, owner_tenant, GroupRole::Owner, true).await;
    add_member(
        &resources,
        gid,
        athlete,
        athlete_tenant,
        GroupRole::Member,
        true,
    )
    .await;

    let saved = save_as(&resources, owner, owner_tenant, save_payload("phil")).await;
    let err = error_text(&saved);
    assert!(
        err.contains("No athlete matching") && err.contains("coach"),
        "an owner without the coach attachment is refused, got: {err}"
    );
}

#[tokio::test]
async fn a_name_matching_two_coached_athletes_lists_both() {
    let fx = coached_athlete(Some("Marc Dubois"), true).await;
    let persona = seed_coach_persona(&fx.resources, fx.coach, fx.coach_tenant).await;
    let other = seed_user(&fx.resources, "athlete2", Some("Marc Tremblay")).await;
    let other_tenant = create_tenant_owned_by(&fx.resources, other).await;
    let second = create_group(
        &fx.resources,
        fx.coach_tenant,
        persona,
        fx.coach,
        Some(fx.coach),
        true,
        "Hill Squad",
    )
    .await;
    add_member(
        &fx.resources,
        second,
        other,
        other_tenant,
        GroupRole::Member,
        true,
    )
    .await;

    let saved = save_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        save_payload("marc"),
    )
    .await;
    let err = error_text(&saved);
    assert!(
        err.contains("ambiguous") && err.contains("Marc Dubois") && err.contains("Marc Tremblay"),
        "both display names must be listed, got: {err}"
    );
}

/// On a channel-bound group the coach is also auto-enrolled as a member, so
/// their own name is on the roster they coach.
#[tokio::test]
async fn the_coachs_own_name_is_refused_as_self() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;
    add_member(
        &fx.resources,
        fx.group_id,
        fx.coach,
        fx.coach_tenant,
        GroupRole::Member,
        true,
    )
    .await;

    let saved = save_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        save_payload("karine"),
    )
    .await;
    let err = error_text(&saved);
    assert!(
        err.contains("is you") && err.contains("omit"),
        "a self-name names the mistake, got: {err}"
    );
}

/// `GroupMember.display_name` is the raw e-mail; the resolver and its
/// refusals must render the roster name (the local part) instead.
#[tokio::test]
async fn an_athlete_without_a_display_name_is_never_named_by_email() {
    let fx = coached_athlete(None, false).await;
    let email = fx
        .resources
        .common
        .repos
        .users
        .get_global(fx.athlete)
        .await
        .unwrap()
        .unwrap()
        .email;
    let local_part = email.split('@').next().unwrap().to_owned();

    let saved = save_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        save_payload("athlete"),
    )
    .await;
    let err = error_text(&saved);
    assert!(
        err.contains("hasn't shared"),
        "the athlete resolved by their roster name and was refused on consent: {err}"
    );
    assert!(
        err.contains(&local_part),
        "the refusal names the roster name the room already knows: {err}"
    );
    assert!(
        !err.contains('@'),
        "an e-mail address must never reach a tool error: {err}"
    );
}

#[tokio::test]
async fn an_athlete_in_two_tenants_is_refused() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;
    create_tenant_owned_by(&fx.resources, fx.athlete).await;

    let saved = save_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        save_payload("phil"),
    )
    .await;
    assert!(
        error_text(&saved).contains("several tenants"),
        "got: {saved}"
    );
}

/// A tenant that disabled the tool by configuration keeps it disabled for a
/// plan filed there — the dispatch chokepoint only checked the coach's tenant.
#[tokio::test]
async fn a_disabled_tool_in_the_athletes_tenant_refuses_the_cross_tenant_write() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;
    let now = Utc::now();
    fx.resources
        .common
        .repos
        .tool_selection
        .upsert_tool_catalog_entry(&ToolCatalogEntry {
            id: Uuid::new_v4().to_string(),
            tool_name: "save_training_plan".to_owned(),
            display_name: "Save training plan".to_owned(),
            description: "test catalog row".to_owned(),
            category: ToolCategory::Fitness,
            is_enabled_by_default: true,
            requires_provider: None,
            min_plan: TenantPlan::Starter,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    fx.resources
        .tool_selection()
        .set_tool_override(
            fx.athlete_tenant,
            "save_training_plan",
            false,
            fx.coach,
            None,
        )
        .await
        .unwrap();

    let saved = save_as(
        &fx.resources,
        fx.coach,
        fx.coach_tenant,
        save_payload("phil"),
    )
    .await;
    assert!(
        error_text(&saved).contains("disabled for this tenant"),
        "got: {saved}"
    );
    assert_eq!(
        saved.get("reason").and_then(Value::as_str),
        Some("tenant_disabled"),
        "the refusal carries the chokepoint's shape: {saved}"
    );
    assert!(
        fx.resources
            .common
            .repos
            .training_plans
            .get_active_plan(
                &fx.athlete_tenant.to_string(),
                &fx.athlete.to_string(),
                Some(fx.athlete_coach.as_str())
            )
            .await
            .unwrap()
            .is_none(),
        "nothing may be written into a tenant that disabled the tool"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Rooms
// ════════════════════════════════════════════════════════════════════════

/// A shared messaging room files its conversation under the bot tenant — a
/// row the coach's own tenant cannot resolve. That unresolvable conversation
/// IS the room signal, and the reply there would go to every member.
#[tokio::test]
async fn a_room_conversation_under_the_bot_tenant_is_refused() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;
    let bot_owner = seed_user(&fx.resources, "bot", Some("Dravr Bot")).await;
    let bot_tenant = create_tenant_owned_by(&fx.resources, bot_owner).await;
    let room = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &fx.coach.to_string(),
            bot_tenant,
            "room",
            "gemini-2.0-flash",
            None,
            Some(&fx.group_id.to_string()),
        )
        .await
        .unwrap();

    let saved = CONVERSATION_ID
        .scope(Some(room.id.clone()), async {
            save_as(
                &fx.resources,
                fx.coach,
                fx.coach_tenant,
                save_payload("phil"),
            )
            .await
        })
        .await;
    let err = error_text(&saved);
    assert!(
        err.contains("direct chat") && err.contains("/plan share"),
        "a room turn is refused and pointed at the share flow, got: {err}"
    );

    let fetched = CONVERSATION_ID
        .scope(Some(room.id), async {
            get_as(
                &fx.resources,
                fx.coach,
                fx.coach_tenant,
                json!({ "athlete": "phil" }),
            )
            .await
        })
        .await;
    assert!(
        error_text(&fetched).contains("direct chat"),
        "reading in a room would publish the plan too: {fetched}"
    );
}

/// An in-app group thread resolves under the coach's own tenant but binds a
/// group — every member reads it, so it is a room as well.
#[tokio::test]
async fn a_group_bound_conversation_under_the_coachs_tenant_is_refused() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;
    let thread = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &fx.coach.to_string(),
            fx.coach_tenant,
            "group thread",
            "gemini-2.0-flash",
            None,
            Some(&fx.group_id.to_string()),
        )
        .await
        .unwrap();

    let saved = CONVERSATION_ID
        .scope(Some(thread.id), async {
            save_as(
                &fx.resources,
                fx.coach,
                fx.coach_tenant,
                save_payload("phil"),
            )
            .await
        })
        .await;
    assert!(error_text(&saved).contains("direct chat"), "got: {saved}");
}

/// Without `athlete`, both tools act on the caller exactly as before — the
/// coach's own plan, under the coach's own tenant.
#[tokio::test]
async fn omitting_athlete_keeps_self_scope() {
    let fx = coached_athlete(Some("Phil Tremblay"), true).await;
    let mut args = save_payload("phil");
    args.as_object_mut().unwrap().remove("athlete");

    let saved = save_as(&fx.resources, fx.coach, fx.coach_tenant, args).await;
    assert!(saved.get("error").is_none(), "got: {saved}");
    assert!(
        saved.get("athlete").is_some_and(Value::is_null),
        "self scope names nobody: {saved}"
    );
    let repos = &fx.resources.common.repos;
    assert!(
        repos
            .training_plans
            .get_active_plan(&fx.coach_tenant.to_string(), &fx.coach.to_string(), None)
            .await
            .unwrap()
            .is_some(),
        "the coach's own plan row exists"
    );
    assert!(
        repos
            .training_plans
            .get_active_plan(
                &fx.athlete_tenant.to_string(),
                &fx.athlete.to_string(),
                Some(fx.athlete_coach.as_str())
            )
            .await
            .unwrap()
            .is_none(),
        "the athlete's plan is untouched"
    );
}

/// The athlete resolves and consents, and there is still nowhere to act: they
/// belong to no tenant, so no surface of theirs could ever read the plan. The
/// refusal names exactly that, and nothing is written anywhere for them.
#[tokio::test]
async fn an_athlete_with_no_tenant_membership_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = seed_user(&resources, "coach", Some("Coach Karine")).await;
    let coach_tenant = create_tenant_owned_by(&resources, coach).await;
    let persona = seed_coach_persona(&resources, coach, coach_tenant).await;
    // The athlete gets NO tenant: the tenant helper is deliberately not
    // called for them, so `list_for_user` answers with an empty set.
    let athlete = seed_user(&resources, "athlete", Some("Phil Tremblay")).await;

    let group_id = create_group(
        &resources,
        coach_tenant,
        persona,
        coach,
        Some(coach),
        true,
        "Tempo Squad",
    )
    .await;
    // The membership row carries the GROUP's tenant — the athlete brings none.
    add_member(
        &resources,
        group_id,
        athlete,
        coach_tenant,
        GroupRole::Member,
        true,
    )
    .await;

    let saved = save_as(&resources, coach, coach_tenant, save_payload("phil")).await;
    let err = error_text(&saved);
    assert!(
        err.contains("belongs to no tenant") && err.contains("nowhere to keep their plan"),
        "the refusal must name the missing home tenant, got: {err}"
    );

    // Nothing may have been written for that user under ANY tenant — with no
    // home there is nowhere correct, so any row at all would be the bug.
    let pool = resources
        .coach
        .database
        .sqlite_pool()
        .expect("test fixture runs against SQLite");
    let plans_for_athlete: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM training_plans WHERE user_id = ?1")
            .bind(athlete.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        plans_for_athlete, 0,
        "no plan row may exist for a tenant-less athlete"
    );

    // The read side is gated identically.
    let fetched = get_as(
        &resources,
        coach,
        coach_tenant,
        json!({ "athlete": "phil" }),
    )
    .await;
    assert!(
        error_text(&fetched).contains("belongs to no tenant"),
        "the read side applies the same gate: {fetched}"
    );
}
