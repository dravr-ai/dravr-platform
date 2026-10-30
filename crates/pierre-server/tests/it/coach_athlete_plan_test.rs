// ABOUTME: A coach reads a coached athlete's published plan off the plan thread — Group info's view, the room's plan status, /plan <athlete>
// ABOUTME: Pins each surface's shape and the gates they share: coach attachment, membership, consent, the athlete's tenant allowing the plan read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1244. Outside their plan thread, a coach reaches an athlete's
//! published plan from three surfaces, each through
//! `coached_plan_read_access`:
//!
//! - `GET /api/chat/athletes/{id}/plan?group_id=` — Group info's read-only
//!   view, the live plan in the home shape;
//! - `get_group_member_plan_status` — what the coach's room may see: whether
//!   there is a plan, its goal and season, never a week, and where the coach
//!   reads it;
//! - `/plan <athlete>` — the plan itself, answered to the coach alone.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Days, NaiveDate, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_commands::plan::PlanShowHandler;
use pierre_commands::{ConversationRotation, PlatformCommandContext};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::group_member_plan::GetGroupMemberPlanStatusTool;
use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

const ATHLETE_NAME: &str = "Mira Athlete";
const GOAL_RACE: &str = "Demi de Montréal";

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

/// A group owned by `owner` and coached by `coach`, with `athlete` a
/// consenting member who has a published season, `bare` a consenting member
/// with none, and `outsider` a member of nothing.
struct Fixture {
    res: Arc<ServerContext>,
    owner: Person,
    coach: Person,
    athlete: Person,
    bare: Person,
    outsider: Person,
    group: CoachingGroup,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@coach-athlete-plan.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    let token = generate_test_token(res, &user).await;
    Person {
        id,
        tenant,
        auth: format!("Bearer {token}"),
    }
}

fn monday(weeks: u64) -> NaiveDate {
    use chrono::Datelike;

    let today = Utc::now().date_naive();
    let this = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    this + Days::new(7 * weeks)
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn week(start: NaiveDate) -> Value {
    let days: Vec<Value> = (0..7)
        .map(|offset| {
            json!({
                "date": ymd(start + Days::new(offset)),
                "sport": "run",
                "workout": "easy aerobic run",
                "duration_min": 40,
                "intensity": "Z2",
            })
        })
        .collect();
    json!({ "week_start": ymd(start), "focus": "aerobic base", "days": days })
}

async fn create_group(
    repos: &RepositoryRegistry,
    owner: &Person,
    coach: &Person,
    name: &str,
) -> CoachingGroup {
    let agent_id = create_group_agent(repos, owner.id, owner.tenant)
        .await
        .unwrap();
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: name.to_owned(),
                description: None,
                agent_id,
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    assert!(
        repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
            .await
            .unwrap(),
        "fixture: the coach is attached to the group"
    );
    group
}

async fn join(repos: &RepositoryRegistry, group: &CoachingGroup, who: &Person) {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: who.id,
            tenant_id: who.tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// Run `tool` as `who`, from `conversation` when one is named.
async fn call<T: McpTool<dyn ToolRuntime>>(
    res: &Arc<ServerContext>,
    who: &Person,
    tool: &T,
    conversation: Option<&str>,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(res);
    let ctx = ToolContext::new()
        .with_user(who.id.to_string())
        .with_tenant(who.tenant.to_string())
        .with_auth_method("jwt_bearer");
    CONVERSATION_ID
        .scope(conversation.map(ToOwned::to_owned), async {
            tool.execute(&runtime, &ctx, args)
                .await
                .structured_content
                .expect("tool result carries structured content")
        })
        .await
}

impl Fixture {
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let owner = person(&res, "owner", "Olive Owner").await;
        let coach = person(&res, "coach", "Casey Coach").await;
        let athlete = person(&res, "athlete", ATHLETE_NAME).await;
        let bare = person(&res, "bare", "Bruno Bare").await;
        let outsider = person(&res, "outsider", "Otto Outsider").await;
        let repos = &res.common.repos;
        let group = create_group(repos, &owner, &coach, "Coacher Mira").await;
        join(repos, &group, &athlete).await;
        join(repos, &group, &bare).await;
        let fx = Self {
            res,
            owner,
            coach,
            athlete,
            bare,
            outsider,
            group,
        };
        let saved = call(
            &fx.res,
            &fx.athlete,
            &SaveTrainingPlanTool,
            None,
            json!({
                "outline": {
                    "goal_race": { "name": GOAL_RACE, "date": "2027-02-14", "discipline": "run", "priority": "A" },
                    "strategy": "an aerobic winter toward the half",
                },
                "weeks": [week(monday(0)), week(monday(1))],
            }),
        )
        .await;
        assert!(
            saved.get("error").is_none(),
            "fixture: the athlete's season is saved: {saved}"
        );
        fx
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// `GET /api/chat/athletes/{athlete}/plan?group_id=` as `auth`.
    async fn view(&self, athlete: Uuid, auth: &str) -> (StatusCode, Value) {
        let resp = AxumTestRequest::get(&format!(
            "/api/chat/athletes/{athlete}/plan?group_id={}",
            self.group.id
        ))
        .header("authorization", auth)
        .send(ChatRoutes::routes(Arc::clone(&self.res)))
        .await;
        (resp.status_code(), resp.json())
    }

    /// The coach's own conversation in the group's room.
    async fn room(&self, group: &CoachingGroup) -> String {
        self.repos()
            .chat
            .create_conversation(
                &self.coach.id.to_string(),
                self.coach.tenant,
                "room",
                "gemini-2.0-flash",
                None,
                Some(&group.id.to_string()),
            )
            .await
            .unwrap()
            .id
    }

    /// `/plan <args>` as `who`, in a messaging DM of theirs.
    async fn plan_command(&self, who: &Person, args: &[&str]) -> String {
        use pierre_commands::CommandHandler;

        let conversation = self
            .repos()
            .chat
            .create_conversation(
                &who.id.to_string(),
                who.tenant,
                "dm",
                "gemini-2.0-flash",
                None,
                None,
            )
            .await
            .unwrap();
        let ctx = PlatformCommandContext {
            user_id: who.id,
            tenant_id: who.tenant,
            channel_type: "telegram".to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            raw_text: format!("/plan {}", args.join(" ")),
            ctx: Arc::<ServerContext>::clone(&self.res),
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: Some(conversation.id),
            conversation_tenant_id: who.tenant,
            sender_id: None,
            rotation: ConversationRotation::default(),
            tool_runtime: Arc::<ServerContext>::clone(&self.res),
        };
        PlanShowHandler.execute(&ctx).await.unwrap().text
    }
}

/// The `details.reason` of a refused route call.
fn reason(body: &Value) -> &str {
    body["details"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal naming its reason, got {body}"))
}

#[tokio::test]
async fn the_view_shows_the_coach_the_live_plan_in_the_home_shape() {
    let fx = Fixture::new().await;

    let (status, body) = fx.view(fx.athlete.id, &fx.coach.auth).await;
    assert_eq!(status, StatusCode::OK, "the coach reads the plan: {body}");
    assert_eq!(
        body["athlete"],
        json!({ "user_id": fx.athlete.id, "display_name": ATHLETE_NAME }),
        "the view names whose plan it is: {body}"
    );
    assert_eq!(
        body["today"],
        ymd(Utc::now().date_naive()),
        "the plan is projected on the athlete's today: {body}"
    );
    assert_eq!(
        body["live"]["goal_race"]["name"], GOAL_RACE,
        "the live plan is the athlete's season: {body}"
    );
    assert!(
        body["live"]["weeks"]
            .as_array()
            .is_some_and(|weeks| !weeks.is_empty()),
        "the live plan carries its weeks: {body}"
    );
    // Read-only: no draft, and nothing a client could publish.
    assert!(
        body.get("draft").is_none() && body.get("draft_state").is_none(),
        "the read-only view carries no draft: {body}"
    );

    let (status, body) = fx.view(fx.bare.id, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an athlete with no plan is no refusal: {body}"
    );
    assert!(body["live"].is_null(), "an athlete with no plan: {body}");
}

#[tokio::test]
async fn the_view_refuses_each_gate_by_its_reason() {
    let fx = Fixture::new().await;

    // The owner is not the group's coach.
    let (status, body) = fx.view(fx.athlete.id, &fx.owner.auth).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the owner is refused: {body}"
    );
    assert_eq!(
        reason(&body),
        "not_group_coach",
        "the owner is not the coach"
    );

    // A person who is not a member of the group.
    let (status, body) = fx.view(fx.outsider.id, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-member is not found: {body}"
    );
    assert_eq!(
        reason(&body),
        "not_a_member",
        "the outsider is no member of the group"
    );

    // The athlete's own workspace turns the plan read off.
    fx.res
        .tool_selection()
        .set_tool_override(
            fx.athlete.tenant,
            "get_training_plan",
            false,
            fx.athlete.id,
            None,
        )
        .await
        .unwrap();
    let (status, body) = fx.view(fx.athlete.id, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a workspace with plans off is refused: {body}"
    );
    assert_eq!(
        reason(&body),
        "plans_disabled",
        "the athlete's workspace turned the plan read off"
    );

    // The athlete stops sharing with the coach.
    assert!(
        fx.repos()
            .groups
            .update_coach_sharing_consent(&fx.group.id.to_string(), fx.bare.id, false)
            .await
            .unwrap(),
        "fixture: the athlete's consent is revoked"
    );
    let (status, body) = fx.view(fx.bare.id, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a revoked consent is refused: {body}"
    );
    assert_eq!(
        reason(&body),
        "coach_sharing_revoked",
        "the athlete stopped sharing with the coach"
    );
}

#[tokio::test]
async fn the_room_reads_the_plan_status_and_never_a_week() {
    let fx = Fixture::new().await;
    let room = fx.room(&fx.group).await;

    let status = call(
        &fx.res,
        &fx.coach,
        &GetGroupMemberPlanStatusTool,
        Some(&room),
        json!({ "member": "Mira" }),
    )
    .await;
    assert_eq!(
        status["member"], ATHLETE_NAME,
        "the status names the athlete by roster name: {status}"
    );
    assert_eq!(
        status["group"], "Coacher Mira",
        "the status names the room's group: {status}"
    );
    assert_eq!(
        status["published"], true,
        "the athlete has a published plan: {status}"
    );
    assert_eq!(
        status["goal_race"]["name"], GOAL_RACE,
        "the status carries the goal race: {status}"
    );
    assert_eq!(
        status["goal_race"]["date"], "2027-02-14",
        "the status carries the race day: {status}"
    );
    assert!(
        status.get("weeks").is_none() && !status.to_string().contains("easy aerobic run"),
        "the room's status carries no week of the plan: {status}"
    );
    let pointer = status["where_to_read"].as_str().unwrap();
    assert!(
        pointer.contains("never posted in a group"),
        "the pointer says why the plan is not in the room: {pointer}"
    );
    assert!(
        pointer.contains(&format!("`/plan {ATHLETE_NAME}`")),
        "the pointer names the private /plan reply: {pointer}"
    );

    let none = call(
        &fx.res,
        &fx.coach,
        &GetGroupMemberPlanStatusTool,
        Some(&room),
        json!({ "member": "Bruno" }),
    )
    .await;
    assert_eq!(
        none["published"], false,
        "an athlete with no plan reads unpublished: {none}"
    );
    assert!(none["goal_race"].is_null(), "no plan, no goal race: {none}");
}

#[tokio::test]
async fn the_plan_status_is_the_rooms_coach_alone_and_pinned_to_the_room() {
    let fx = Fixture::new().await;
    let room = fx.room(&fx.group).await;

    // The athlete and the owner coach no group: nothing is read for them.
    for who in [&fx.athlete, &fx.owner] {
        let refused = call(
            &fx.res,
            who,
            &GetGroupMemberPlanStatusTool,
            None,
            json!({ "member": "Mira" }),
        )
        .await;
        assert_eq!(
            refused["reason"], "not_a_coach",
            "someone who coaches no group reads nothing: {refused}"
        );
    }

    // An athlete the coach coaches in another group never reaches this room.
    let other = create_group(fx.repos(), &fx.owner, &fx.coach, "Other Squad").await;
    join(fx.repos(), &other, &fx.outsider).await;
    let pinned = call(
        &fx.res,
        &fx.coach,
        &GetGroupMemberPlanStatusTool,
        Some(&room),
        json!({ "member": "Otto" }),
    )
    .await;
    assert_eq!(
        pinned["reason"], "no_match",
        "another group's athlete is out of this room's reach: {pinned}"
    );
    // From the coach's own conversation the same athlete is in reach.
    let private = call(
        &fx.res,
        &fx.coach,
        &GetGroupMemberPlanStatusTool,
        None,
        json!({ "member": "Otto" }),
    )
    .await;
    assert_eq!(
        private["published"], false,
        "the coach's own conversation reaches every group they coach: {private}"
    );

    // An athlete who stopped sharing is refused by name.
    assert!(
        fx.repos()
            .groups
            .update_coach_sharing_consent(&fx.group.id.to_string(), fx.athlete.id, false)
            .await
            .unwrap(),
        "fixture: the athlete's consent is revoked"
    );
    let revoked = call(
        &fx.res,
        &fx.coach,
        &GetGroupMemberPlanStatusTool,
        Some(&room),
        json!({ "member": "Mira" }),
    )
    .await;
    assert_eq!(
        revoked["reason"], "coach_sharing_revoked",
        "an athlete who stopped sharing is refused: {revoked}"
    );
}

#[tokio::test]
async fn plan_with_an_athletes_name_answers_their_coach_with_their_plan() {
    let fx = Fixture::new().await;

    let reply = fx.plan_command(&fx.coach, &["Mira"]).await;
    assert!(
        reply.starts_with(&format!("📋 **{ATHLETE_NAME}**'s plan")),
        "the reply is headed with whose plan it is: {reply}"
    );
    assert!(
        reply.contains(GOAL_RACE),
        "the reply is the athlete's plan: {reply}"
    );

    let week = fx.plan_command(&fx.coach, &["Mira", "week"]).await;
    assert!(
        week.contains("easy aerobic run"),
        "the week view carries the athlete's sessions: {week}"
    );

    let none = fx.plan_command(&fx.coach, &["Bruno"]).await;
    assert_eq!(
        none, "Bruno Bare has no published plan yet.",
        "an athlete with no plan is named as having none"
    );

    let miss = fx.plan_command(&fx.coach, &["Zed"]).await;
    assert_eq!(
        miss, "No athlete matching “Zed” in a group where you're the human coach.",
        "a name matching no coached athlete says so"
    );

    assert!(
        fx.repos()
            .groups
            .update_coach_sharing_consent(&fx.group.id.to_string(), fx.athlete.id, false)
            .await
            .unwrap(),
        "fixture: the athlete's consent is revoked"
    );
    let revoked = fx.plan_command(&fx.coach, &["Mira"]).await;
    assert!(
        revoked.starts_with(&format!(
            "{ATHLETE_NAME} stopped sharing their data with you"
        )),
        "a revoked consent is named: {revoked}"
    );
    assert!(
        !revoked.contains(GOAL_RACE),
        "a revoked consent shows nothing of the plan: {revoked}"
    );
}

#[tokio::test]
async fn plan_with_a_name_from_someone_who_coaches_no_group_is_their_own_plan() {
    let fx = Fixture::new().await;

    // The athlete coaches nobody: a name after /plan is not read, and their
    // own plan answers.
    let own = fx.plan_command(&fx.athlete, &["Bruno"]).await;
    assert!(own.contains(GOAL_RACE), "the athlete's own plan: {own}");
    assert!(
        !own.contains("📋"),
        "no coach header on one's own plan: {own}"
    );
}
