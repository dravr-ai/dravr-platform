// ABOUTME: A personal thread created without an agent is bound to the athlete's own agent and opens on its welcome
// ABOUTME: Pins the bind, the welcome whose starters resolve, the athlete-speaks-first create, and the unbound room

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#829.
//!
//! `POST /api/chat/conversations` welcomed only a thread created with an
//! `agent_id`. Home's "+", the empty state's links and the mobile composer all
//! create without one, so the athlete landed in a thread that opened on
//! nothing — and a welcome starter shown there could never resolve, because a
//! suggestion postback is resolved against the thread's agent.
//!
//! A personal thread created without an agent is now bound the way the
//! messaging DM forge binds one — the athlete's selected agent, else their own
//! thread's default — and that agent opens it, unless the thread was created
//! to carry the athlete's first line at once. These drive the create route
//! and the turn route with a model that records what it was sent.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::recording_llm::RecordingProvider;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{TenantId, AGENT_WELCOME_FINISH_REASON};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatRoutes, ConversationResponse, MessageResponse, MessagesListResponse, TurnResponse,
};

const SAMPLES: [&str; 3] = [
    "What should I eat before a 6am run?",
    "How do I carb load for a marathon?",
    "How many gels during a marathon?",
];
const REPLY: &str = "Start two days out, with rice and pasta at every meal.";

struct Fixture {
    resources: Arc<ServerContext>,
    provider: Arc<RecordingProvider>,
    user_id: Uuid,
    tenant_id: TenantId,
    auth: String,
}

async fn setup(email: &str) -> Fixture {
    let provider = Arc::new(RecordingProvider::answering(REPLY));
    let resources = create_test_server_resources_with_chat_provider(provider.clone())
        .await
        .unwrap();
    let (user_id, user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&resources, &user).await);
    Fixture {
        resources,
        provider,
        user_id,
        tenant_id,
        auth,
    }
}

/// A system agent of the athlete's tenant whose Example Inputs are [`SAMPLES`].
async fn system_agent(fx: &Fixture, title: &str) -> String {
    fx.resources
        .common
        .repos
        .agents
        .create_system_agent(
            fx.user_id,
            fx.tenant_id,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: Some("Fuelling specialist for endurance athletes.".to_owned()),
                system_prompt: "You are a fuelling specialist.".to_owned(),
                category: AgentCategory::Nutrition,
                tags: vec![],
                sample_prompts: SAMPLES.iter().map(|s| (*s).to_owned()).collect(),
                visibility: AgentVisibility::Tenant,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

async fn create(fx: &Fixture, body: &Value) -> ConversationResponse {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.auth)
        .json(body)
        .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED, "{body}");
    resp.json()
}

async fn messages(fx: &Fixture, conv: &str) -> Vec<MessageResponse> {
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{conv}/messages"))
        .header("authorization", &fx.auth)
        .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json::<MessagesListResponse>().messages
}

async fn send(fx: &Fixture, conv: &str, content: &str) -> TurnResponse {
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv}/messages"))
        .header("authorization", &fx.auth)
        .json(&json!({ "content": content }))
        .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{content}");
    resp.json()
}

/// The thread's agent, as stored.
async fn bound_agent(fx: &Fixture, conv: &str) -> Option<String> {
    fx.resources
        .common
        .repos
        .chat
        .get_conversation(conv, &fx.user_id.to_string(), fx.tenant_id)
        .await
        .unwrap()
        .expect("the thread exists")
        .agent_id
}

/// The defect and its fix in one thread: created with no agent, the thread is
/// bound to the athlete's selected agent, opens on that agent's welcome, and
/// a tapped starter resolves into its question and reaches the model.
#[tokio::test]
async fn a_thread_created_without_an_agent_opens_on_the_selected_agents_welcome() {
    let fx = setup("unbound-selected@test.com").await;
    // The tenant's own default is another agent: the selection wins over it.
    system_agent(&fx, "Generalist Agent").await;
    let selected = system_agent(&fx, "Fuelling Agent").await;
    fx.resources
        .common
        .repos
        .tenants
        .set_selected_agent(fx.tenant_id, fx.user_id, Some(&selected))
        .await
        .unwrap();

    let conv = create(&fx, &json!({})).await;

    assert_eq!(conv.agent_id.as_deref(), Some(selected.as_str()));
    assert_eq!(
        bound_agent(&fx, &conv.id).await.as_deref(),
        Some(selected.as_str())
    );
    let rows = messages(&fx, &conv.id).await;
    assert_eq!(rows.len(), 1, "the thread opens on exactly one row");
    let welcome = &rows[0];
    assert_eq!(
        welcome.finish_reason.as_deref(),
        Some(AGENT_WELCOME_FINISH_REASON)
    );
    assert!(
        welcome.content.contains("Fuelling Agent"),
        "{}",
        welcome.content
    );
    let starters: Vec<(&str, &str)> = welcome
        .actions
        .as_ref()
        .expect("the welcome carries its starters")
        .actions
        .iter()
        .map(|a| (a.label.as_str(), a.value.as_str()))
        .collect();
    // Ranked from the athlete's state first (carnet#828) — nothing is
    // connected, so "Connect my watch" and the nutrition starter that needs
    // no data, in the thread's rotation — then the agent's first example in
    // the slot they leave.
    assert_eq!(starters.len(), 3, "{starters:?}");
    let mut ranked: Vec<&str> = starters[..2]
        .iter()
        .enumerate()
        .map(|(slot, (_, value))| {
            value
                .strip_prefix(&format!("uc:{slot}:"))
                .expect("the ranked starters lead")
        })
        .collect();
    ranked.sort_unstable();
    assert_eq!(ranked, ["connect", "fuel"]);
    assert_eq!(starters[2], (SAMPLES[0], "ex:2:0"));

    let before = fx.provider.calls_so_far();
    let turn = send(&fx, &conv.id, "ex:2:0").await;

    assert_eq!(
        turn.user_message.content, SAMPLES[0],
        "the starter resolves against the agent the thread was bound to"
    );
    assert!(
        fx.provider
            .requests_since(before)
            .iter()
            .any(|r| r.conversation.contains(SAMPLES[0])),
        "the model answers the question, not the postback"
    );
}

/// With nothing selected, the thread is bound to the athlete's own-thread
/// default — the tenant's athlete-facing system agent — as a messaging DM is.
#[tokio::test]
async fn with_no_selection_the_thread_is_bound_to_the_own_thread_default() {
    let fx = setup("unbound-default@test.com").await;
    let default = system_agent(&fx, "Fuelling Agent").await;

    let conv = create(&fx, &json!({})).await;

    assert_eq!(conv.agent_id.as_deref(), Some(default.as_str()));
    let rows = messages(&fx, &conv.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].finish_reason.as_deref(),
        Some(AGENT_WELCOME_FINISH_REASON)
    );
}

/// A thread created to carry the athlete's first line is bound like any
/// other, but opens empty: the line goes in first, and the agent's first
/// reply introduces it rather than a welcome landing beside the answer.
#[tokio::test]
async fn a_thread_the_athlete_speaks_first_in_is_bound_but_not_welcomed() {
    let fx = setup("unbound-first@test.com").await;
    let default = system_agent(&fx, "Fuelling Agent").await;

    let conv = create(&fx, &json!({ "athlete_speaks_first": true })).await;

    assert_eq!(conv.agent_id.as_deref(), Some(default.as_str()));
    assert!(messages(&fx, &conv.id).await.is_empty());

    let turn = send(&fx, &conv.id, "How do I carb load for a marathon?").await;
    assert_eq!(
        turn.user_message.content,
        "How do I carb load for a marathon?"
    );
    let rows = messages(&fx, &conv.id).await;
    assert!(
        rows.iter()
            .all(|m| m.finish_reason.as_deref() != Some(AGENT_WELCOME_FINISH_REASON)),
        "no welcome lands in a thread the athlete opened by speaking"
    );
    assert_eq!(rows.first().map(|m| m.role.as_str()), Some("user"));
}

/// A room created without an agent stays unbound: who answers there is the
/// group's agent, never the creator's own selection.
#[tokio::test]
async fn a_room_created_without_an_agent_stays_unbound() {
    let fx = setup("unbound-room@test.com").await;
    let group_agent = system_agent(&fx, "Squad Agent").await;
    let selected = system_agent(&fx, "Fuelling Agent").await;
    let repos = &fx.resources.common.repos;
    repos
        .tenants
        .set_selected_agent(fx.tenant_id, fx.user_id, Some(&selected))
        .await
        .unwrap();
    let now = Utc::now();
    let group_id = Uuid::new_v4();
    repos
        .groups
        .create_group(
            fx.tenant_id,
            &CoachingGroup {
                id: group_id,
                tenant_id: fx.tenant_id.to_string(),
                name: "Marathon Squad".to_owned(),
                description: None,
                agent_id: group_agent,
                owner_id: fx.user_id,
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
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id: fx.user_id,
            tenant_id: fx.tenant_id.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: false,
            coach_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();

    let conv = create(&fx, &json!({ "group_id": group_id.to_string() })).await;

    assert_eq!(conv.agent_id, None);
    assert_eq!(bound_agent(&fx, &conv.id).await, None);
    assert!(messages(&fx, &conv.id).await.is_empty());
}
