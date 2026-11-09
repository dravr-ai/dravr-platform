// ABOUTME: Drives a real in-app turn in an agent's thread over HTTP and reads the notification it raised
// ABOUTME: The notification names the agent that answered and opens that thread — never the thread's own title
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An agent's reply in the in-app chat raises a "message from your agent"
//! notification. It named the conversation instead of the agent — "Chat Jun 8
//! 8:36 AM sent you a message" — because it read the thread's title, which is
//! whatever the thread was created under: a typed title, the group's name, or
//! the dated stamp a thread gets before an agent is bound. The turn's envelope
//! now says which agent the reply spoke as, and the notification names it.
//!
//! It names that agent the way the recipient's chat does: the sentence around
//! the name renders in their stored language, so the name reads from the same
//! one, through the `agent_translations` overlay.

#![cfg(feature = "client-notifications")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{ConnectionType, TenantId, User};
use pierre_database::seed_models::SeedAgentTranslation;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_notifications::models::Notification;
use pierre_notifications::TenantId as CommereTenantId;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

/// The model's answer; its words do not matter here.
const REPLY: &str = "Caffeine lowers perceived effort, so a steady pace feels easier.";

/// A general question: it reads none of the athlete's data, so the model's
/// reply is what the turn serves.
const GENERAL_QUESTION: &str = "How does caffeine affect endurance?";

/// The thread's title — the dated stamp from the screenshot this pins.
const THREAD_TITLE: &str = "Chat Jun 8 8:36 AM";

/// The agent bound to the thread, who writes the reply.
const AGENT_TITLE: &str = "Marathon Agent";

/// A catalogue agent's canonical title, as `agents.title` stores it.
const CANONICAL_TITLE: &str = "Road Running Agent";

/// The same agent in French, as `agent_translations` overlays it.
const FRENCH_TITLE: &str = "Agent Course sur Route";

/// Answers every call with [`REPLY`].
struct FixedReplyProvider;

#[async_trait]
impl LlmProvider for FixedReplyProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn display_name(&self) -> &'static str {
        "Fixed reply LLM (tests)"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &'static str {
        "mock-model"
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: REPLY.to_owned(),
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(40, 12, 52)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: REPLY.to_owned(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// The notifications the athlete holds in the `coach` category, once the
/// fire-and-forget dispatch has landed.
async fn agent_notifications(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<Notification> {
    let service = resources
        .common
        .notification_service
        .as_ref()
        .expect("the server boots with its notification service");
    for _ in 0..40 {
        let (rows, _, _) = service
            .list_notifications(
                user_id,
                CommereTenantId(tenant_id.as_uuid()),
                10,
                0,
                Some("coach"),
                false,
            )
            .await
            .unwrap();
        if !rows.is_empty() {
            return rows;
        }
        sleep(Duration::from_millis(50)).await;
    }
    Vec::new()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reply_notification_names_the_agent_not_the_thread_title() {
    let resources: Arc<ServerContext> =
        common::create_test_server_resources_with_llm(Arc::new(FixedReplyProvider))
            .await
            .expect("server resources");
    let app = ChatRoutes::routes(Arc::clone(&resources));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let base_url = format!("http://{addr}");

    let (user, token) =
        common::create_test_tenant_with_provider(&resources, "agent-reply-notice@example.com")
            .await
            .expect("create user + token + provider");
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id;
    let agent = resources
        .common
        .repos
        .agents
        .create(
            user.id,
            tenant_id,
            &CreateAgentRequest {
                title: AGENT_TITLE.to_owned(),
                description: Some("Writes the reply the notification reports".to_owned()),
                system_prompt: "You answer endurance questions briefly.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap();

    let client = Client::builder().no_gzip().build().expect("client");
    let created: Value = client
        .post(format!("{base_url}/api/chat/conversations"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "title": THREAD_TITLE,
            "model": "mock-model",
            "agent_id": agent.id.to_string(),
        }))
        .send()
        .await
        .expect("create conversation")
        .json()
        .await
        .expect("conversation json");
    let conversation_id = created["id"].as_str().expect("conversation id").to_owned();
    assert_eq!(
        created["title"], THREAD_TITLE,
        "the typed title names the thread"
    );

    let response = client
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "content": GENERAL_QUESTION }))
        .send()
        .await
        .expect("POST send_message");
    assert!(response.status().is_success(), "{}", response.status());
    let turn: Value = response.json().await.expect("turn json");
    assert_eq!(turn["assistant"]["message"]["content"], REPLY, "{turn}");

    let rows = agent_notifications(&resources, user.id, tenant_id).await;
    assert_eq!(rows.len(), 1, "one reply, one notification: {rows:?}");
    let notice = &rows[0];
    assert!(
        notice.body.contains(AGENT_TITLE),
        "the notification names the agent that answered: {}",
        notice.body
    );
    assert!(
        !notice.body.contains(THREAD_TITLE),
        "the thread's title is not who sent the message: {}",
        notice.body
    );
    let data = notice.data.as_ref().expect("the notice routes somewhere");
    assert_eq!(data.get("screen").and_then(Value::as_str), Some("coach"));
    assert_eq!(
        data.get("id").and_then(Value::as_str),
        Some(conversation_id.as_str()),
        "the tap opens this thread"
    );
}

/// The chat routes on a live listener, and their base URL.
async fn serve_chat(resources: &Arc<ServerContext>) -> String {
    let app = ChatRoutes::routes(Arc::clone(resources));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

/// An agent `owner` authored under the canonical title, with a French overlay.
async fn translated_agent(resources: &ServerContext, owner: Uuid, tenant_id: TenantId) -> String {
    let agent = resources
        .common
        .repos
        .agents
        .create(
            owner,
            tenant_id,
            &CreateAgentRequest {
                title: CANONICAL_TITLE.to_owned(),
                description: Some("Writes the reply the notification reports".to_owned()),
                system_prompt: "You answer endurance questions briefly.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap();
    let agent_id = agent.id.to_string();
    resources
        .common
        .repos
        .seeder
        .seed_upsert_agent_translation(&SeedAgentTranslation {
            agent_id: agent_id.clone(),
            locale: "fr".to_owned(),
            title: Some(FRENCH_TITLE.to_owned()),
            description: None,
            purpose: None,
            instructions: None,
            source_sha: None,
            tags: None,
            sample_prompts: None,
        })
        .await
        .unwrap();
    agent_id
}

/// Open a thread with `thread` as its creation body and send it one turn.
async fn turn_in_new_thread(base_url: &str, token: &str, thread: Value) {
    let response = Client::new()
        .post(format!("{base_url}/api/chat/conversations"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&thread)
        .send()
        .await
        .expect("create conversation");
    assert!(response.status().is_success(), "{}", response.status());
    let created: Value = response.json().await.expect("conversation json");
    send_turn(
        base_url,
        token,
        created["id"].as_str().expect("conversation id"),
    )
    .await;
}

/// Send one turn to an existing thread.
async fn send_turn(base_url: &str, token: &str, conversation_id: &str) {
    let response = Client::builder()
        .no_gzip()
        .build()
        .expect("client")
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "content": GENERAL_QUESTION }))
        .send()
        .await
        .expect("POST send_message");
    assert!(response.status().is_success(), "{}", response.status());
    let turn: Value = response.json().await.expect("turn json");
    assert_eq!(turn["assistant"]["message"]["content"], REPLY, "{turn}");
}

async fn tenant_of(resources: &ServerContext, user: &User) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id
}

/// A French athlete writing in English still reads the agent as their chat
/// names it: the name follows the stored language the sentence around it
/// renders in, not the language of this one message.
#[tokio::test(flavor = "multi_thread")]
async fn the_notification_names_the_agent_in_the_recipients_language() {
    let resources: Arc<ServerContext> =
        common::create_test_server_resources_with_llm(Arc::new(FixedReplyProvider))
            .await
            .expect("server resources");
    let base_url = serve_chat(&resources).await;
    let (user, token) =
        common::create_test_tenant_with_provider(&resources, "agent-notice-fr@example.com")
            .await
            .expect("create user + token + provider");
    let tenant_id = tenant_of(&resources, &user).await;
    let agent_id = translated_agent(&resources, user.id, tenant_id).await;

    turn_in_new_thread(
        &base_url,
        &token,
        json!({ "title": THREAD_TITLE, "model": "mock-model", "agent_id": agent_id }),
    )
    .await;

    let rows = agent_notifications(&resources, user.id, tenant_id).await;
    assert_eq!(rows.len(), 1, "one reply, one notification: {rows:?}");
    assert_eq!(
        rows[0].body,
        format!("{FRENCH_TITLE} t'a envoyé un message"),
        "the French athlete reads the title their chat shows"
    );
}

/// A language the agent has no overlay in reads the canonical title.
#[tokio::test(flavor = "multi_thread")]
async fn a_language_without_an_overlay_reads_the_canonical_title() {
    let resources: Arc<ServerContext> =
        common::create_test_server_resources_with_llm(Arc::new(FixedReplyProvider))
            .await
            .expect("server resources");
    let base_url = serve_chat(&resources).await;
    let (user, token) =
        common::create_test_tenant_with_provider(&resources, "agent-notice-en@example.com")
            .await
            .expect("create user + token + provider");
    let tenant_id = tenant_of(&resources, &user).await;
    let agent_id = translated_agent(&resources, user.id, tenant_id).await;
    resources
        .common
        .repos
        .users
        .update_locale(user.id, "en")
        .await
        .unwrap();

    turn_in_new_thread(
        &base_url,
        &token,
        json!({ "title": THREAD_TITLE, "model": "mock-model", "agent_id": agent_id }),
    )
    .await;

    let rows = agent_notifications(&resources, user.id, tenant_id).await;
    assert_eq!(rows.len(), 1, "one reply, one notification: {rows:?}");
    assert_eq!(
        rows[0].body,
        format!("{CANONICAL_TITLE} sent you a message")
    );
}

/// A group's agent belongs to whoever set the group up. The member it answers
/// was never assigned it, and the notification still names it in the member's
/// language, not the owner's.
#[tokio::test(flavor = "multi_thread")]
async fn a_group_member_reads_the_groups_agent_in_their_own_language() {
    let resources: Arc<ServerContext> =
        common::create_test_server_resources_with_llm(Arc::new(FixedReplyProvider))
            .await
            .expect("server resources");
    let base_url = serve_chat(&resources).await;
    let (owner, _) =
        common::create_test_tenant_with_provider(&resources, "agent-notice-owner@example.com")
            .await
            .expect("create owner");
    let (member, _) =
        common::create_test_tenant_with_provider(&resources, "agent-notice-member@example.com")
            .await
            .expect("create member");
    let repos = &resources.common.repos;
    repos.users.update_locale(owner.id, "en").await.unwrap();
    let group_tenant = tenant_of(&resources, &owner).await;
    // The member acts inside the group's tenant: a real `tenant_users` row, as
    // authentication requires, and a provider the onboarding gate reads there.
    repos
        .users
        .update_tenant_id(member.id, group_tenant)
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            member.id,
            group_tenant,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    let member_token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &member,
            &resources.auth.jwks_manager,
            Some(group_tenant.to_string()),
        )
        .unwrap();
    let agent_id = translated_agent(&resources, owner.id, group_tenant).await;

    let group_id = Uuid::new_v4();
    let now = Utc::now();
    let groups = &resources.common.repos.groups;
    groups
        .create_group(
            group_tenant,
            &CoachingGroup {
                id: group_id,
                tenant_id: group_tenant.to_string(),
                name: "Saturday long run".to_owned(),
                description: None,
                agent_id: agent_id.clone(),
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::All,
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    for (user_id, role) in [(owner.id, GroupRole::Owner), (member.id, GroupRole::Member)] {
        groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: group_tenant.to_string(),
                role,
                peer_sharing_consent: false,
                coach_sharing_consent: false,
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

    // The member's row of the room, bound to the group's agent the way the
    // room's forge binds it.
    let conversation = repos
        .chat
        .create_conversation(
            &member.id.to_string(),
            group_tenant,
            "Squad room",
            "mock-model",
            Some(&agent_id),
            Some(&group_id.to_string()),
        )
        .await
        .unwrap();
    send_turn(&base_url, &member_token, &conversation.id).await;

    let rows = agent_notifications(&resources, member.id, group_tenant).await;
    assert_eq!(rows.len(), 1, "one reply, one notification: {rows:?}");
    assert_eq!(
        rows[0].body,
        format!("{FRENCH_TITLE} t'a envoyé un message"),
        "the member reads their own language's title"
    );
}
