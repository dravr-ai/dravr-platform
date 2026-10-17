// ABOUTME: GET/POST/DELETE /api/chat/conversations/{id}/subject-plan — a coach's draft of their thread's athlete's plan over HTTP
// ABOUTME: Pins the read's shape, the 409 conflict body the clients parse, 404 no_draft, and every gate refusal by its reason

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1069. The Publish and Discard buttons on the plan card of a
//! coach's thread about one athlete (carnet#1070) call these routes. The
//! clients parse their bodies strictly (`packages/shared-types/src/
//! coach-plan.ts`), so each shape is asserted key by key, and every refusal
//! by the code a client branches on: `error_code` first, then
//! `details.reason`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Datelike, Days, NaiveDate, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{Tenant, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plans::{
    GetTrainingPlanTool, SaveTrainingPlanTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

const SUBJECT_NAME: &str = "Mira Subject";
const COACH_WORKOUT: &str = "coach-drafted tempo";

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

/// A group owned by `owner` and coached by `coach`, the subject a
/// consenting member with a live season, and the coach's thread about them.
struct Fixture {
    res: Arc<ServerContext>,
    owner: Person,
    coach: Person,
    subject: Person,
    group: CoachingGroup,
    thread: String,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@subject-plan.test", Uuid::new_v4());
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
    let today = Utc::now().date_naive();
    let this = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    this + Days::new(7 * weeks)
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn week(start: NaiveDate, workout: &str) -> Value {
    let days: Vec<Value> = (0..7)
        .map(|offset| {
            json!({
                "date": ymd(start + Days::new(offset)),
                "sport": "run",
                "workout": workout,
                "duration_min": 40,
                "intensity": "Z2",
            })
        })
        .collect();
    json!({ "week_start": ymd(start), "focus": workout, "days": days })
}

async fn call<T: McpTool<dyn ToolRuntime>>(
    res: &Arc<ServerContext>,
    who: &Person,
    tool: &T,
    conversation: Option<&str>,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = Arc::clone(res) as Arc<dyn ToolRuntime>;
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
        let subject = person(&res, "subject", SUBJECT_NAME).await;
        let repos = &res.common.repos;
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
                    name: "Tempo Squad".to_owned(),
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
        assert!(repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
            .await
            .unwrap());
        join(repos, &group, &subject).await;
        let resp = AxumTestRequest::post(&format!("/api/chat/athletes/{}/thread", subject.id))
            .header("authorization", &coach.auth)
            .json(&json!({ "group_id": group.id }))
            .send(ChatRoutes::routes(Arc::clone(&res)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let thread = resp.json::<Value>()["id"].as_str().unwrap().to_owned();
        let fx = Self {
            res,
            owner,
            coach,
            subject,
            group,
            thread,
        };
        // The athlete's own season: this week and next.
        let saved = call(
            &fx.res,
            &fx.subject,
            &SaveTrainingPlanTool,
            None,
            json!({
                "outline": {
                    "goal_race": { "name": "Athlete Race", "date": "2027-05-16", "discipline": "trail", "priority": "A" },
                    "strategy": "the athlete's season",
                },
                "weeks": [week(monday(0), "athlete run"), week(monday(1), "athlete run")],
            }),
        )
        .await;
        assert!(saved.get("error").is_none(), "{saved}");
        fx
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    async fn request(&self, request: AxumTestRequest, auth: &str) -> (StatusCode, Value) {
        let resp = request
            .header("authorization", auth)
            .send(ChatRoutes::routes(Arc::clone(&self.res)))
            .await;
        let status = resp.status_code();
        let body = if status == StatusCode::NO_CONTENT {
            Value::Null
        } else {
            resp.json()
        };
        (status, body)
    }

    async fn get_plan(&self, conversation: &str, auth: &str) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::get(&format!(
                "/api/chat/conversations/{conversation}/subject-plan"
            )),
            auth,
        )
        .await
    }

    async fn publish(&self, overwrite: bool) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::post(&format!(
                "/api/chat/conversations/{}/subject-plan/publish",
                self.thread
            ))
            .json(&json!({ "overwrite_athlete_changes": overwrite })),
            &self.coach.auth,
        )
        .await
    }

    async fn discard(&self) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::delete(&format!(
                "/api/chat/conversations/{}/subject-plan/draft",
                self.thread
            )),
            &self.coach.auth,
        )
        .await
    }

    /// The coach drafts this week in their thread.
    async fn coach_drafts(&self) {
        let saved = call(
            &self.res,
            &self.coach,
            &SaveTrainingPlanTool,
            Some(&self.thread),
            json!({ "weeks": [week(monday(0), COACH_WORKOUT)] }),
        )
        .await;
        assert_eq!(saved["draft"], true, "{saved}");
    }

    /// The athlete's own agent re-saves this week.
    async fn athlete_changes_this_week(&self) {
        let saved = call(
            &self.res,
            &self.subject,
            &SaveTrainingPlanTool,
            None,
            json!({ "weeks": [week(monday(0), "athlete changed run")] }),
        )
        .await;
        assert!(saved.get("error").is_none(), "{saved}");
    }
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
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// The `details.reason` of a refused route call.
fn reason(body: &Value) -> &str {
    body["details"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal naming its reason, got {body}"))
}

#[tokio::test]
async fn the_read_shows_the_live_plan_and_the_draft_in_the_home_shape() {
    let fx = Fixture::new().await;

    // No draft: the live plan, nothing else.
    let (status, body) = fx.get_plan(&fx.thread, &fx.coach.auth).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["athlete"],
        json!({ "user_id": fx.subject.id, "display_name": SUBJECT_NAME })
    );
    assert_eq!(body["today"], ymd(Utc::now().date_naive()));
    assert_eq!(body["live"]["goal_race"]["name"], "Athlete Race", "{body}");
    assert!(body["live"]["weeks"]
        .as_array()
        .is_some_and(|w| !w.is_empty()));
    assert!(
        body["draft"].is_null() && body["draft_state"].is_null(),
        "{body}"
    );

    fx.coach_drafts().await;
    let (status, body) = fx.get_plan(&fx.thread, &fx.coach.auth).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body["live"].to_string().contains(COACH_WORKOUT), "{body}");
    let draft = &body["draft"];
    assert_eq!(draft["goal_race"]["name"], "Athlete Race", "{body}");
    let weeks = draft["weeks"].as_array().unwrap();
    assert_eq!(weeks[0]["week_start"], ymd(monday(0)));
    assert_eq!(weeks[0]["days"][0]["workout"], COACH_WORKOUT);
    assert_eq!(weeks[1]["days"][0]["workout"], "athlete run");
    let state = &body["draft_state"];
    assert_eq!(state["conflicts"], json!([]), "{body}");
    assert_eq!(state["season_relaid"], false);
    assert!(state["updated_at"]
        .as_str()
        .is_some_and(|at| chrono::DateTime::parse_from_rfc3339(at).is_ok()));

    // The card's draft state and the read's come from one serialiser.
    let read = call(
        &fx.res,
        &fx.coach,
        &GetTrainingPlanTool,
        Some(&fx.thread),
        json!({}),
    )
    .await;
    assert_eq!(read["draft"]["state"], *state, "{read}");
}

#[tokio::test]
async fn a_conflicted_publish_is_a_409_in_the_shape_the_clients_parse() {
    let fx = Fixture::new().await;
    fx.coach_drafts().await;
    fx.athlete_changes_this_week().await;

    let (status, body) = fx.publish(false).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error_code"], "plan_publish_conflict");
    assert_eq!(reason(&body), "plan_publish_conflict");
    assert_eq!(
        body["conflicts"],
        json!([{ "week_start": ymd(monday(0)), "reason": "athlete_changed_week" }])
    );
    assert_eq!(body["season_relaid"], false);
    // Nothing moved: the draft is still there to publish.
    let (_, read) = fx.get_plan(&fx.thread, &fx.coach.auth).await;
    assert_eq!(read["draft_state"]["draft_id"], body["draft_id"], "{read}");
    assert!(!read["live"].to_string().contains(COACH_WORKOUT));

    let (status, body) = fx.publish(true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["athlete_notified", "plan_id", "superseded_plan_id"]
    );
    assert!(body["superseded_plan_id"].is_null(), "{body}");
    // The athlete is told whenever the server sends notices at all
    // (`coach_plan_notice_test` reads the notice back).
    assert_eq!(
        body["athlete_notified"],
        cfg!(feature = "client-notifications"),
        "{body}"
    );
    let live = fx
        .repos()
        .training_plans
        .get_active_plan(&fx.subject.tenant.to_string(), &fx.subject.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(body["plan_id"], live.id.as_str());
    let (_, read) = fx.get_plan(&fx.thread, &fx.coach.auth).await;
    assert!(read["live"].to_string().contains(COACH_WORKOUT), "{read}");
    assert!(read["draft"].is_null(), "{read}");

    // Published: there is no draft left to publish or discard.
    let (status, body) = fx.publish(false).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(reason(&body), "no_draft");
    let (status, body) = fx.discard().await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(reason(&body), "no_draft");
}

#[tokio::test]
async fn discard_is_a_204_and_leaves_the_live_plan() {
    let fx = Fixture::new().await;
    fx.coach_drafts().await;

    let (status, body) = fx.discard().await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, read) = fx.get_plan(&fx.thread, &fx.coach.auth).await;
    assert!(
        read["draft"].is_null() && read["draft_state"].is_null(),
        "{read}"
    );
    assert!(!read["live"].to_string().contains(COACH_WORKOUT));
    let (status, body) = fx.discard().await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(reason(&body), "no_draft");
}

#[tokio::test]
async fn only_the_coachs_own_thread_about_an_athlete_answers() {
    let fx = Fixture::new().await;
    fx.coach_drafts().await;

    // Another user — the group's owner here — gets nothing of it.
    let (status, body) = fx.get_plan(&fx.thread, &fx.owner.auth).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // An unknown conversation.
    let (status, body) = fx
        .get_plan(&Uuid::new_v4().to_string(), &fx.coach.auth)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // The coach's own thread about no athlete.
    let plain = fx
        .repos()
        .chat
        .create_conversation(
            &fx.coach.id.to_string(),
            fx.coach.tenant,
            "plain",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    for (status, body) in [
        fx.get_plan(&plain, &fx.coach.auth).await,
        fx.request(
            AxumTestRequest::post(&format!(
                "/api/chat/conversations/{plain}/subject-plan/publish"
            ))
            .json(&json!({ "overwrite_athlete_changes": false })),
            &fx.coach.auth,
        )
        .await,
        fx.request(
            AxumTestRequest::delete(&format!(
                "/api/chat/conversations/{plain}/subject-plan/draft"
            )),
            &fx.coach.auth,
        )
        .await,
    ] {
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(reason(&body), "no_subject");
    }
}

#[tokio::test]
async fn every_gate_runs_again_on_each_request_with_its_reason() {
    let fx = Fixture::new().await;
    fx.coach_drafts().await;
    let group = fx.group.id.to_string();

    // The athlete stops sharing with the coach: each route refuses, and the
    // draft is neither published nor thrown away.
    assert!(fx
        .repos()
        .groups
        .update_coach_sharing_consent(&group, fx.subject.id, false)
        .await
        .unwrap());
    for (status, body) in [
        fx.get_plan(&fx.thread, &fx.coach.auth).await,
        fx.publish(true).await,
        fx.discard().await,
    ] {
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(reason(&body), "coach_sharing_revoked");
    }
    assert!(fx
        .repos()
        .groups
        .update_coach_sharing_consent(&group, fx.subject.id, true)
        .await
        .unwrap());
    let (_, read) = fx.get_plan(&fx.thread, &fx.coach.auth).await;
    assert!(
        read["draft_state"].is_object(),
        "the draft survived: {read}"
    );

    // An athlete in two tenants has no one place for their plan.
    let now = Utc::now();
    let second = TenantId::generate();
    fx.repos()
        .tenants
        .create(&Tenant {
            id: second,
            name: "Second home".to_owned(),
            slug: format!("second-{second}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: fx.subject.id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let (status, body) = fx.publish(true).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(reason(&body), "several_tenants");
    assert!(body.get("error_code").is_none(), "{body}");

    // A member who left.
    assert!(fx
        .repos()
        .groups
        .remove_member(&group, fx.subject.id)
        .await
        .unwrap());
    let (status, body) = fx.get_plan(&fx.thread, &fx.coach.auth).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(reason(&body), "not_a_member");

    // A coach detached from the group.
    assert!(fx
        .repos()
        .groups
        .set_group_coach_user(&group, None, fx.owner.tenant)
        .await
        .unwrap());
    let (status, body) = fx.discard().await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(reason(&body), "not_group_coach");
}
