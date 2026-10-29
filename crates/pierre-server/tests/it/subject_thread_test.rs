// ABOUTME: A group's human coach's thread bound to one athlete — opened by id through the coach gates, reopened, never personal
// ABOUTME: Pins every route refusal, the cap exemption, the list fields, and the plan and calendar tools defaulting to the subject
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1070. `POST /api/chat/athletes/{athlete_user_id}/thread` opens the
//! coach's one thread about an athlete they coach in a group, and every plan
//! and calendar tool called in it acts for that athlete with no `athlete`
//! named, re-checking the coach gates on each call.
//!
//! Every positive tool assertion reads back whose plan was answered — the
//! athlete's season, never the coach's own, which is seeded beside it — and
//! every refusal is asserted by the reason a client or the model relays.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use secrecy::SecretString;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_commands::reset::ResetHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{AddMessageParams, Tenant, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{
    NewConversation, PlanOutlineInput, Reactivation, SavePlanBundleParams, SlotClaim,
};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, ConversationListResponse};
use pierre_memory::training_plans::{GoalRace, RacePriority};
use pierre_services::personal_thread::find_personal_thread;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plans::{
    GetTrainingPlanTool, SaveTrainingPlanTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

const SUBJECT_NAME: &str = "Mira Subject";
const OTHER_NAME: &str = "Oscar Other";
const COACH_NAME: &str = "Casey Coach";
const SUBJECT_RACE: &str = "Subject Season Race";
const COACH_RACE: &str = "Coach Own Race";

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    email: String,
}

/// A group `owner` owns and `coach` coaches, with two consenting members:
/// the subject and another coached athlete.
struct Fixture {
    res: Arc<ServerContext>,
    /// The group's owner, who is not its coach.
    owner_auth: String,
    coach: Person,
    coach_auth: String,
    subject: Person,
    other: Person,
    group: CoachingGroup,
}

/// A verified user named `name`, in a tenant of their own, with a bearer
/// token for the chat routes.
async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> (Person, String) {
    let email = format!("{label}-{}@subject-thread.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    let token = generate_test_token(res, &user).await;
    (Person { id, tenant, email }, format!("Bearer {token}"))
}

async fn join(repos: &RepositoryRegistry, group: &CoachingGroup, who: &Person, shares: bool) {
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
            coach_sharing_consent: shares,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

impl Fixture {
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let (owner, owner_auth) = person(&res, "owner", "Olive Owner").await;
        let (coach, coach_auth) = person(&res, "coach", COACH_NAME).await;
        let (subject, _) = person(&res, "subject", SUBJECT_NAME).await;
        let (other, _) = person(&res, "other", OTHER_NAME).await;
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
        // Redeeming a coach invite attaches the coach through this write.
        assert!(repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
            .await
            .unwrap());
        join(repos, &group, &subject, true).await;
        join(repos, &group, &other, true).await;
        Self {
            res,
            owner_auth,
            coach,
            coach_auth,
            subject,
            other,
            group,
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    fn router(&self) -> axum::Router {
        ChatRoutes::routes(Arc::clone(&self.res))
    }

    /// `POST /api/chat/athletes/{athlete}/thread` as `auth`, naming `group`.
    async fn open_as(&self, auth: &str, athlete: Uuid, group: Uuid) -> (StatusCode, Value) {
        let resp = AxumTestRequest::post(&format!("/api/chat/athletes/{athlete}/thread"))
            .header("authorization", auth)
            .json(&json!({ "group_id": group }))
            .send(self.router())
            .await;
        (resp.status_code(), resp.json())
    }

    /// The coach opens their thread about the subject; its id.
    async fn open_subject(&self) -> String {
        let (status, body) = self
            .open_as(&self.coach_auth, self.subject.id, self.group.id)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["id"].as_str().unwrap().to_owned()
    }

    /// `who`'s active season with goal race `race`, laid by no agent.
    async fn season(&self, who: &Person, race: &str) {
        let goal = GoalRace {
            name: race.to_owned(),
            date: "2027-05-16".to_owned(),
            discipline: "trail".to_owned(),
            priority: RacePriority::A,
        };
        self.repos()
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &who.tenant.to_string(),
                user_id: &who.id.to_string(),
                author: PlanAuthor::none(),
                goal_fact_id: None,
                replace_season: false,
                outline: Some(PlanOutlineInput {
                    goal_race: &goal,
                    races: Some(&[]),
                    strategy: "steady base",
                    flavour: None,
                    season_start: None,
                    season_end: None,
                    phases: &[],
                    source_conversation_id: None,
                }),
                weeks: &[],
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();
    }

    /// Call `tool` as the coach, in their own tenant, inside `conversation`.
    async fn coach_calls<T: McpTool<dyn ToolRuntime>>(
        &self,
        tool: &T,
        conversation: &str,
        args: Value,
    ) -> Value {
        let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&self.res);
        let ctx = ToolContext::new()
            .with_user(self.coach.id.to_string())
            .with_tenant(self.coach.tenant.to_string())
            .with_auth_method("jwt_bearer");
        CONVERSATION_ID
            .scope(Some(conversation.to_owned()), async {
                tool.execute(&runtime, &ctx, args)
                    .await
                    .structured_content
                    .expect("tool result carries structured content")
            })
            .await
    }
}

/// The `details.reason` of a refused route call.
fn reason(body: &Value) -> &str {
    body["details"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal naming its reason, got {body}"))
}

fn error_text(payload: &Value) -> &str {
    payload["error"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal, got {payload}"))
}

// ════════════════════════════════════════════════════════════════════════
// Opening the thread
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn the_coach_opens_one_thread_per_athlete_and_reopening_returns_it_unarchived() {
    let fx = Fixture::new().await;

    let (status, opened) = fx.open_as(&fx.coach_auth, fx.subject.id, fx.group.id).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let id = opened["id"].as_str().unwrap().to_owned();
    assert_eq!(opened["title"], SUBJECT_NAME, "{opened}");
    assert_eq!(opened["subject_user_id"], fx.subject.id.to_string());
    assert_eq!(opened["subject_group_id"], fx.group.id.to_string());
    assert_eq!(opened["subject_display_name"], SUBJECT_NAME);
    // Bound to the group's agent, so the plan is built with its package.
    assert_eq!(opened["agent_id"], fx.group.agent_id.as_str());
    assert_eq!(opened["group_id"], Value::Null, "never a room: {opened}");
    assert_eq!(opened["personal"], false, "{opened}");

    let (status, again) = fx.open_as(&fx.coach_auth, fx.subject.id, fx.group.id).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["id"], id.as_str(), "one thread per coach per athlete");

    let coach = fx.coach.id.to_string();
    let chat = &fx.repos().chat;
    assert!(chat
        .archive_conversation(&id, &coach, fx.coach.tenant)
        .await
        .unwrap());
    assert!(chat
        .is_conversation_archived(&id, &coach, fx.coach.tenant)
        .await
        .unwrap());
    let (status, reopened) = fx.open_as(&fx.coach_auth, fx.subject.id, fx.group.id).await;
    assert_eq!(status, StatusCode::OK, "{reopened}");
    assert_eq!(reopened["id"], id.as_str());
    assert!(
        !chat
            .is_conversation_archived(&id, &coach, fx.coach.tenant)
            .await
            .unwrap(),
        "reopening the thread brings it back to the active set"
    );

    // Reading the thread back names its athlete too.
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{id}"))
        .header("authorization", &fx.coach_auth)
        .send(fx.router())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let read: Value = resp.json();
    assert_eq!(read["subject_user_id"], fx.subject.id.to_string());
    assert_eq!(read["subject_display_name"], SUBJECT_NAME, "{read}");

    // The other athlete gets a thread of their own.
    let (status, other) = fx.open_as(&fx.coach_auth, fx.other.id, fx.group.id).await;
    assert_eq!(status, StatusCode::OK, "{other}");
    assert_ne!(other["id"], id.as_str());
    assert_eq!(other["title"], OTHER_NAME);
}

#[tokio::test]
async fn every_gate_refuses_with_its_reason_and_opens_nothing() {
    let fx = Fixture::new().await;

    // The group's owner is not its coach.
    let (status, body) = fx.open_as(&fx.owner_auth, fx.subject.id, fx.group.id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(reason(&body), "not_group_coach");
    // Nor is anyone the coach of a group that does not exist.
    let (status, body) = fx
        .open_as(&fx.coach_auth, fx.subject.id, Uuid::new_v4())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(reason(&body), "not_group_coach");

    // A coach is never their own athlete.
    let (status, body) = fx.open_as(&fx.coach_auth, fx.coach.id, fx.group.id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(reason(&body), "not_a_member");

    // A member who left.
    assert!(fx
        .repos()
        .groups
        .remove_member(&fx.group.id.to_string(), fx.other.id)
        .await
        .unwrap());
    let (status, body) = fx.open_as(&fx.coach_auth, fx.other.id, fx.group.id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(reason(&body), "not_a_member");

    // An athlete who stopped sharing with the coach.
    assert!(fx
        .repos()
        .groups
        .update_coach_sharing_consent(&fx.group.id.to_string(), fx.subject.id, false)
        .await
        .unwrap());
    let (status, body) = fx.open_as(&fx.coach_auth, fx.subject.id, fx.group.id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(reason(&body), "coach_sharing_revoked");
    assert!(
        body["message"].as_str().unwrap().contains(SUBJECT_NAME),
        "past the membership gate the refusal names the athlete: {body}"
    );

    // An athlete in two tenants has no one place for their plan.
    let (third, _) = person(&fx.res, "third", "Theo Twotenants").await;
    join(fx.repos(), &fx.group, &third, true).await;
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
            owner_user_id: third.id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let (status, body) = fx.open_as(&fx.coach_auth, third.id, fx.group.id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(reason(&body), "several_tenants");

    let opened = fx
        .repos()
        .chat
        .count_participating_conversations(
            &fx.coach.id.to_string(),
            fx.coach.tenant,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap();
    assert_eq!(opened, 0, "no refusal opened a thread");
}

// ════════════════════════════════════════════════════════════════════════
// What the thread is not
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a_subject_thread_is_never_the_coachs_personal_thread_nor_a_room() {
    let fx = Fixture::new().await;
    let id = fx.open_subject().await;
    let coach = fx.coach.id.to_string();
    let chat = &fx.repos().chat;

    // The coach's only thread is about the athlete, so there is nothing to
    // adopt as theirs.
    let personal = find_personal_thread(fx.repos(), &coach, fx.coach.tenant)
        .await
        .unwrap();
    assert!(personal.is_none(), "adopted {personal:?}");
    let resp = AxumTestRequest::get("/api/chat/personal")
        .header("authorization", &fx.coach_auth)
        .send(fx.router())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: Value = resp.json();
    assert_eq!(body["conversation"], Value::Null, "{body}");

    // Nor can it be designated, or made a room.
    assert!(!chat
        .designate_personal_conversation(&id, &coach, fx.coach.tenant)
        .await
        .unwrap());
    assert!(!chat
        .set_conversation_group_id(&id, Some(&fx.group.id.to_string()), fx.coach.tenant)
        .await
        .unwrap());
    let stored = chat
        .get_conversation(&id, &coach, fx.coach.tenant)
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.personal);
    assert_eq!(stored.group_id, None);
    assert_eq!(
        stored.subject().map(|s| (s.athlete_user_id, s.group_id)),
        Some((fx.subject.id, fx.group.id))
    );

    // Nobody else is added to read the athlete's data.
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{id}/participants"))
        .header("authorization", &fx.coach_auth)
        .json(&json!({ "user_id": fx.coach.id.to_string() }))
        .send(fx.router())
        .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_subject_thread_holds_no_slot_of_the_conversation_cap() {
    let fx = Fixture::new().await;
    let id = fx.open_subject().await;
    let coach = fx.coach.id.to_string();
    let chat = &fx.repos().chat;

    assert_eq!(
        chat.count_conversations(&coach, fx.coach.tenant)
            .await
            .unwrap(),
        0
    );
    // A cap of one still lets the coach open a thread of their own.
    let claimed = chat
        .create_conversation_within_cap(
            &NewConversation {
                user_id: &coach,
                tenant_id: fx.coach.tenant,
                title: "own thread",
                model: "gemini-2.0-flash",
                agent_id: None,
                group_id: None,
            },
            Some(1),
        )
        .await
        .unwrap();
    assert!(matches!(claimed, SlotClaim::Claimed(_)), "{claimed:?}");

    // At that cap, an archived athlete thread still comes back: it takes no
    // slot once active.
    assert!(chat
        .archive_conversation(&id, &coach, fx.coach.tenant)
        .await
        .unwrap());
    let back = chat
        .reactivate_conversation(&id, &coach, fx.coach.tenant, Some(1))
        .await
        .unwrap();
    assert_eq!(back, Reactivation::Reactivated);
}

#[tokio::test]
async fn the_list_names_the_athlete_a_thread_is_about() {
    let fx = Fixture::new().await;
    let id = fx.open_subject().await;
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

    let resp = AxumTestRequest::get("/api/chat/conversations")
        .header("authorization", &fx.coach_auth)
        .send(fx.router())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let page: ConversationListResponse = resp.json();
    let row = |wanted: &str| {
        page.conversations
            .iter()
            .find(|row| row.id == wanted)
            .unwrap_or_else(|| panic!("{wanted} is listed"))
    };
    let subject = row(&id);
    assert_eq!(
        subject.subject_user_id.as_deref(),
        Some(fx.subject.id.to_string().as_str())
    );
    assert_eq!(
        subject.subject_group_id.as_deref(),
        Some(fx.group.id.to_string().as_str())
    );
    assert_eq!(subject.subject_display_name.as_deref(), Some(SUBJECT_NAME));
    let plain = row(&plain);
    assert_eq!(plain.subject_user_id, None);
    assert_eq!(plain.subject_group_id, None);
    assert_eq!(plain.subject_display_name, None);
}

#[tokio::test]
async fn reset_in_a_subject_thread_opens_a_topic_and_keeps_the_binding() {
    let fx = Fixture::new().await;
    let id = fx.open_subject().await;
    let coach = fx.coach.id.to_string();
    for (role, content) in [
        ("user", "Build her base block"),
        ("assistant", "Here is a base block"),
    ] {
        fx.repos()
            .chat
            .add_message(&AddMessageParams {
                tenant_id: fx.coach.tenant,
                conversation_id: &id,
                user_id: &coach,
                role,
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
            })
            .await
            .unwrap();
    }

    let ctx = PlatformCommandContext {
        user_id: fx.coach.id,
        tenant_id: fx.coach.tenant,
        channel_type: "web".to_owned(),
        args: vec![],
        raw_text: "/reset".to_owned(),
        ctx: Arc::<ServerContext>::clone(&fx.res),
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: false,
        conversation_id: Some(id.clone()),
        conversation_tenant_id: fx.coach.tenant,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(&fx.res),
    };
    ResetHandler.execute(&ctx).await.unwrap();

    assert_eq!(ctx.rotation.taken(), None, "the coach stays on the thread");
    let divider = ctx.rotation.opened_topic().expect("a topic opened");
    assert_eq!(
        fx.repos()
            .chat
            .latest_divider_id(&id, fx.coach.tenant)
            .await
            .unwrap()
            .as_deref(),
        Some(divider)
    );
    let thread = fx
        .repos()
        .chat
        .get_conversation(&id, &coach, fx.coach.tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(thread.archived_at, None);
    assert_eq!(thread.subject_user_id, Some(fx.subject.id));
    assert_eq!(
        fx.repos()
            .chat
            .count_participating_conversations(
                &coach,
                fx.coach.tenant,
                TransportPolicy::FirstPartyOnly
            )
            .await
            .unwrap(),
        1,
        "no fresh thread bound to no one was forged"
    );
}

// ════════════════════════════════════════════════════════════════════════
// The tools in the thread
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn the_plan_tools_in_a_subject_thread_read_the_subjects_plan() {
    let fx = Fixture::new().await;
    fx.season(&fx.subject, SUBJECT_RACE).await;
    fx.season(&fx.coach, COACH_RACE).await;
    let id = fx.open_subject().await;

    for args in [json!({}), json!({ "athlete": "mira" })] {
        let fetched = fx.coach_calls(&GetTrainingPlanTool, &id, args).await;
        assert!(fetched.get("error").is_none(), "{fetched}");
        assert_eq!(fetched["athlete"], SUBJECT_NAME, "{fetched}");
        assert_eq!(
            fetched["plan"]["goal_race"]["name"], SUBJECT_RACE,
            "the subject's season, never the coach's own: {fetched}"
        );
    }
}

#[tokio::test]
async fn naming_anyone_else_in_a_subject_thread_is_refused() {
    let fx = Fixture::new().await;
    fx.season(&fx.other, "Other Race").await;
    let id = fx.open_subject().await;

    let refused = fx
        .coach_calls(&GetTrainingPlanTool, &id, json!({ "athlete": "oscar" }))
        .await;
    let text = error_text(&refused);
    assert!(
        text.contains(&format!(
            "This thread works on {SUBJECT_NAME}'s plan — open {OTHER_NAME}'s own thread from \
             Group info"
        )),
        "{text}"
    );

    // A write naming the other athlete lands nowhere.
    let saved = fx
        .coach_calls(
            &SaveTrainingPlanTool,
            &id,
            json!({
                "athlete": "oscar",
                "outline": {
                    "goal_race": { "name": "Overwritten", "date": "2027-06-01", "discipline": "trail", "priority": "A" },
                    "strategy": "should never land"
                },
            }),
        )
        .await;
    assert!(error_text(&saved).contains("own thread from Group info"));
    let other = fx
        .repos()
        .training_plans
        .get_active_plan(&fx.other.tenant.to_string(), &fx.other.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(other.goal_race.name, "Other Race");

    // The coach's own plan is never acted on here.
    let refused = fx
        .coach_calls(&GetTrainingPlanTool, &id, json!({ "athlete": "casey" }))
        .await;
    assert!(
        error_text(&refused).contains("act on your own plan from your own conversation"),
        "{refused}"
    );

    // A name that is nobody's.
    let refused = fx
        .coach_calls(&GetTrainingPlanTool, &id, json!({ "athlete": "zelda" }))
        .await;
    assert!(
        error_text(&refused).contains("does not name them"),
        "{refused}"
    );
}

#[tokio::test]
async fn the_gates_run_again_on_every_call_in_the_thread() {
    let fx = Fixture::new().await;
    fx.season(&fx.subject, SUBJECT_RACE).await;
    let id = fx.open_subject().await;

    let fetched = fx.coach_calls(&GetTrainingPlanTool, &id, json!({})).await;
    assert_eq!(fetched["athlete"], SUBJECT_NAME, "{fetched}");

    assert!(fx
        .repos()
        .groups
        .update_coach_sharing_consent(&fx.group.id.to_string(), fx.subject.id, false)
        .await
        .unwrap());
    let refused = fx.coach_calls(&GetTrainingPlanTool, &id, json!({})).await;
    assert!(
        error_text(&refused).contains("has stopped sharing their data with you"),
        "{refused}"
    );

    assert!(fx
        .repos()
        .groups
        .remove_member(&fx.group.id.to_string(), fx.subject.id)
        .await
        .unwrap());
    let refused = fx.coach_calls(&GetTrainingPlanTool, &id, json!({})).await;
    assert!(
        error_text(&refused).contains("no longer a member of the group you coach them in"),
        "{refused}"
    );
}

/// The calendar tools resolve the thread's athlete exactly as a named one:
/// a push without `athlete` goes through the subject's link, and over an
/// external transport it is refused as `athlete=` would be.
#[cfg(all(feature = "provider-intervals-icu", feature = "tools-data"))]
#[tokio::test]
async fn the_calendar_tools_in_a_subject_thread_act_for_the_subject() {
    use pierre_core::constants::oauth::providers::INTERVALS_ICU;
    use pierre_core::models::{
        ConnectionType, DelegatedConnection, RosterAthlete, UserOAuthToken, API_KEY_TOKEN_TYPE,
    };
    use pierre_core::transport::Transport;
    use pierre_providers::ai_scope::serve_over;
    use pierre_tool_runtime::implementations::planned_workouts::GetPlannedWorkoutsTool;
    use pierre_tool_runtime::implementations::training_plan_push::PushTrainingPlanTool;

    let fx = Fixture::new().await;
    let id = fx.open_subject().await;
    let repos = fx.repos();

    // With no link yet, the planned read names the subject it resolved.
    let planned = serve_over(
        Transport::WebApp,
        fx.coach_calls(&GetPlannedWorkoutsTool, &id, json!({})),
    )
    .await;
    assert_eq!(planned["reason"], "not_linked", "{planned}");
    assert!(
        error_text(&planned).starts_with(&format!("{SUBJECT_NAME} has no confirmed link")),
        "{planned}"
    );

    // The coach's own Intervals.icu key, and a confirmed link to the subject
    // that does not allow calendar writes.
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: fx.coach.id,
            tenant_id: fx.coach.tenant.to_string(),
            provider: INTERVALS_ICU.to_owned(),
            access_token: SecretString::from("coach-key"),
            refresh_token: None,
            token_type: API_KEY_TOKEN_TYPE.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: Some("i100".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            fx.coach.id,
            fx.coach.tenant,
            INTERVALS_ICU,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    let proposed = repos
        .delegated_connections
        .propose(&DelegatedConnection::propose(
            INTERVALS_ICU.to_owned(),
            fx.group.id,
            fx.coach.id,
            fx.coach.tenant,
            fx.subject.id,
            RosterAthlete::new("i201", None, Some(fx.subject.email.clone())),
        ))
        .await
        .unwrap()
        .unwrap();
    let link = repos
        .delegated_connections
        .confirm(proposed.id, fx.subject.id, fx.subject.tenant, now)
        .await
        .unwrap()
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            fx.subject.id,
            fx.subject.tenant,
            INTERVALS_ICU,
            &ConnectionType::Delegated,
            Some(&json!({ "delegated_connection_id": link.id }).to_string()),
        )
        .await
        .unwrap();

    let pushed = serve_over(
        Transport::WebApp,
        fx.coach_calls(&PushTrainingPlanTool, &id, json!({})),
    )
    .await;
    assert_eq!(
        pushed["reason"], "no_calendar_write_consent",
        "the push went through the subject's link: {pushed}"
    );

    // Over an external transport the thread's athlete is another person's
    // calendar exactly as a named one is.
    let external = serve_over(
        Transport::McpHttp,
        fx.coach_calls(&PushTrainingPlanTool, &id, json!({})),
    )
    .await;
    assert_eq!(external["reason"], "external_transport", "{external}");
}
