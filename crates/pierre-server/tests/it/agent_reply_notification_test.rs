// ABOUTME: Drives real in-app turns in an agent's thread over HTTP and reads the notifications they raise
// ABOUTME: Names the agent, opens the thread; raised only for a reply left unread, cleared once the thread is read
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
//!
//! It is raised only for a reply the athlete did not see: a question asked
//! from the open thread is answered on screen, and a "message from your agent"
//! row and push for that answer would only fill the inbox. The notification
//! waits [`UNSEEN_REPLY_GRACE`] and is raised only while the reply is still
//! unread, and reading the thread marks read every notification that opens it.

#![cfg(feature = "client-notifications")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;
use pierre_auth::api_keys::{ApiKey, ApiKeyManager, ApiKeyTier};
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
use pierre_notifications::triggers::UNSEEN_REPLY_GRACE;
use pierre_notifications::TenantId as CommereTenantId;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::{sleep, Instant};
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
    // The notification waits out the grace before it is raised.
    let deadline = Instant::now() + UNSEEN_REPLY_GRACE + SETTLE;
    loop {
        let rows = coach_rows(resources, user_id, tenant_id).await;
        if !rows.is_empty() || Instant::now() >= deadline {
            return rows;
        }
        sleep(Duration::from_millis(100)).await;
    }
}

/// Time past [`UNSEEN_REPLY_GRACE`] for the check and its dispatch to land.
const SETTLE: Duration = Duration::from_secs(3);

/// The `coach` notifications the athlete holds right now, read or not.
async fn coach_rows(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<Notification> {
    resources
        .common
        .notification_service
        .as_ref()
        .expect("the server boots with its notification service")
        .list_notifications(
            user_id,
            CommereTenantId(tenant_id.as_uuid()),
            10,
            0,
            Some("coach"),
            false,
        )
        .await
        .unwrap()
        .0
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

/// Open a thread with `thread` as its creation body and send it one turn,
/// returning the thread's id.
async fn turn_in_new_thread(base_url: &str, token: &str, thread: Value) -> String {
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
    let conversation_id = created["id"].as_str().expect("conversation id").to_owned();
    send_turn(base_url, token, &conversation_id).await;
    conversation_id
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

/// An athlete who is looking at the thread: the reply lands on screen, their
/// app advances the read marker, and nothing is raised — no inbox row and no
/// push for an answer they are reading.
#[tokio::test(flavor = "multi_thread")]
async fn a_reply_read_within_the_grace_raises_nothing() {
    let resources: Arc<ServerContext> =
        common::create_test_server_resources_with_llm(Arc::new(FixedReplyProvider))
            .await
            .expect("server resources");
    let base_url = serve_chat(&resources).await;
    let (user, token) =
        common::create_test_tenant_with_provider(&resources, "agent-notice-seen@example.com")
            .await
            .expect("create user + token + provider");
    let tenant_id = tenant_of(&resources, &user).await;
    let agent_id = translated_agent(&resources, user.id, tenant_id).await;

    let conversation_id = turn_in_new_thread(
        &base_url,
        &token,
        json!({ "title": THREAD_TITLE, "model": "mock-model", "agent_id": agent_id }),
    )
    .await;
    let marked = Client::new()
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/read"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .expect("POST read");
    assert_eq!(marked.status().as_u16(), 204, "the marker advances");

    sleep(UNSEEN_REPLY_GRACE + SETTLE).await;
    let rows = coach_rows(&resources, user.id, tenant_id).await;
    assert!(
        rows.is_empty(),
        "a reply already read raises nothing: {rows:?}"
    );
}

/// Reading a thread marks read the notifications that open it, and only
/// those: another thread's reply stays unread in the inbox.
#[tokio::test(flavor = "multi_thread")]
async fn reading_the_thread_marks_its_notifications_read() {
    let resources: Arc<ServerContext> =
        common::create_test_server_resources_with_llm(Arc::new(FixedReplyProvider))
            .await
            .expect("server resources");
    let base_url = serve_chat(&resources).await;
    let (user, token) =
        common::create_test_tenant_with_provider(&resources, "agent-notice-clear@example.com")
            .await
            .expect("create user + token + provider");
    let tenant_id = tenant_of(&resources, &user).await;
    let agent_id = translated_agent(&resources, user.id, tenant_id).await;
    let thread = json!({ "title": THREAD_TITLE, "model": "mock-model", "agent_id": agent_id });

    let read_thread = turn_in_new_thread(&base_url, &token, thread.clone()).await;
    let other_thread = turn_in_new_thread(&base_url, &token, thread).await;
    let deadline = Instant::now() + UNSEEN_REPLY_GRACE + SETTLE;
    let mut rows = coach_rows(&resources, user.id, tenant_id).await;
    while rows.len() < 2 && Instant::now() < deadline {
        sleep(Duration::from_millis(100)).await;
        rows = coach_rows(&resources, user.id, tenant_id).await;
    }
    assert_eq!(
        rows.len(),
        2,
        "two replies nobody read, two notifications: {rows:?}"
    );
    assert!(
        rows.iter().all(|row| row.read_at.is_none()),
        "nothing has read either thread yet: {rows:?}"
    );

    let opened = Client::new()
        .get(format!(
            "{base_url}/api/chat/conversations/{read_thread}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .expect("GET messages");
    assert!(opened.status().is_success(), "{}", opened.status());

    let opens = |row: &Notification, id: &str| {
        row.data
            .as_ref()
            .and_then(|data| data.get("id"))
            .and_then(Value::as_str)
            == Some(id)
    };
    let rows = coach_rows(&resources, user.id, tenant_id).await;
    let read = rows
        .iter()
        .find(|row| opens(row, &read_thread))
        .expect("the read thread's row");
    let other = rows
        .iter()
        .find(|row| opens(row, &other_thread))
        .expect("the other thread's row");
    assert!(
        read.read_at.is_some(),
        "opening the thread reads its notification"
    );
    assert!(
        other.read_at.is_none(),
        "another thread's notification stays unread"
    );
}

/// A stored API key for `user_id`, and the full key a caller presents.
async fn stored_api_key(resources: &ServerContext, user_id: Uuid) -> String {
    let data = ApiKeyManager::new().generate_api_key(false);
    let key = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: "script".to_owned(),
        key_prefix: data.key_prefix,
        key_hash: data.key_hash,
        description: None,
        tier: ApiKeyTier::Starter,
        rate_limit_requests: 1_000,
        rate_limit_window_seconds: 3_600,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    resources.common.repos.api_keys.create(&key).await.unwrap();
    data.full_key
}

/// A caller holding one of the athlete's API keys reads the reply in its own
/// response and never moves the read marker: it is not the athlete away from
/// the app, and raises nothing.
#[tokio::test(flavor = "multi_thread")]
async fn an_api_key_turn_raises_nothing() {
    let resources: Arc<ServerContext> =
        common::create_test_server_resources_with_llm(Arc::new(FixedReplyProvider))
            .await
            .expect("server resources");
    let base_url = serve_chat(&resources).await;
    let (user, token) =
        common::create_test_tenant_with_provider(&resources, "agent-notice-key@example.com")
            .await
            .expect("create user + token + provider");
    let tenant_id = tenant_of(&resources, &user).await;
    let agent_id = translated_agent(&resources, user.id, tenant_id).await;
    let api_key = stored_api_key(&resources, user.id).await;

    let created: Value = Client::new()
        .post(format!("{base_url}/api/chat/conversations"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "title": THREAD_TITLE, "model": "mock-model", "agent_id": agent_id }))
        .send()
        .await
        .expect("create conversation")
        .json()
        .await
        .expect("conversation json");
    let conversation_id = created["id"].as_str().expect("id");
    // An API key is presented as the whole header, not as a bearer token.
    let response = Client::new()
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, api_key)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "content": GENERAL_QUESTION }))
        .send()
        .await
        .expect("POST send_message");
    assert!(response.status().is_success(), "{}", response.status());
    let turn: Value = response.json().await.expect("turn json");
    assert_eq!(turn["assistant"]["message"]["content"], REPLY, "{turn}");

    sleep(UNSEEN_REPLY_GRACE + SETTLE).await;
    let rows = coach_rows(&resources, user.id, tenant_id).await;
    assert!(rows.is_empty(), "an API-key turn raises nothing: {rows:?}");
}
