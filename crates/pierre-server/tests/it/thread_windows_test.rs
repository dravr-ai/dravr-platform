// ABOUTME: Integration tests for reading a thread in windows and listing its topics over the chat routes (carnet#840)
// ABOUTME: Pins the newest window, the cursor back, has_more, the per-row channel, and the topics each divider opens

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's personal thread never rotates, so it is read in windows from
//! the newest back, and History lists its topics.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{
    AddMessageParams, MessageRecord, Tenant, TenantId, User, UserStatus, COMMAND_FINISH_REASON,
};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, MessagesListResponse};
use pierre_services::personal_thread::{resolve_personal_thread, PersonalForge};
use serde_json::{json, Value};
use tokio::task::spawn_blocking;
use uuid::Uuid;

/// An Active athlete with their own tenant and a bearer token for the chat
/// routes.
async fn seed_athlete(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId, String) {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();
    let mut user = User::new(email.to_owned(), password_hash, Some("Windows".to_owned()));
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Windows Tenant".to_owned(),
        slug: format!("windows-{tenant_id}"),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    (user_id, tenant_id, format!("Bearer {token}"))
}

/// What one row of a test thread is.
#[derive(Clone, Copy)]
struct Row<'a> {
    role: &'a str,
    content: &'a str,
    channel: Option<&'a str>,
    finish_reason: Option<&'a str>,
}

impl<'a> Row<'a> {
    const fn user(content: &'a str) -> Self {
        Self {
            role: "user",
            content,
            channel: None,
            finish_reason: None,
        }
    }

    const fn assistant(content: &'a str) -> Self {
        Self {
            role: "assistant",
            ..Self::user(content)
        }
    }

    const fn over(self, channel: &'a str) -> Self {
        Self {
            channel: Some(channel),
            ..self
        }
    }

    const fn command(self) -> Self {
        Self {
            finish_reason: Some(COMMAND_FINISH_REASON),
            ..self
        }
    }
}

/// One row written into `conversation`, as a turn on its channel writes it.
async fn add_row(
    resources: &Arc<ServerContext>,
    conversation: &str,
    (user_id, tenant_id): (Uuid, TenantId),
    row: Row<'_>,
) -> MessageRecord {
    resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: conversation,
            user_id: &user_id.to_string(),
            role: row.role,
            content: row.content,
            token_count: None,
            finish_reason: row.finish_reason,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: row.channel,
        })
        .await
        .unwrap()
}

/// The athlete's personal thread, forged for the test.
async fn personal_thread(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> String {
    resolve_personal_thread(
        &resources.common.repos,
        &user_id.to_string(),
        tenant_id,
        PersonalForge {
            title_fallback: "Windows",
            guided_flow: false,
        },
    )
    .await
    .unwrap()
    .conversation
    .id
}

/// `GET …/messages` with `query`, as the athlete.
async fn read_window(
    resources: &Arc<ServerContext>,
    auth: &str,
    conversation: &str,
    query: &str,
) -> MessagesListResponse {
    let resp = AxumTestRequest::get(&format!(
        "/api/chat/conversations/{conversation}/messages{query}"
    ))
    .header("authorization", auth)
    .send(ChatRoutes::routes(Arc::clone(resources)))
    .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json()
}

fn contents(window: &MessagesListResponse) -> Vec<String> {
    window.messages.iter().map(|m| m.content.clone()).collect()
}

#[tokio::test]
async fn a_thread_is_read_in_windows_from_the_newest_back() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "windows@example.com").await;
    let thread = personal_thread(&resources, user_id, tenant_id).await;
    let owner = (user_id, tenant_id);
    for i in 1..=5 {
        add_row(&resources, &thread, owner, Row::user(&format!("m{i}"))).await;
    }

    // With no window named, the newest window holds the whole short thread.
    let whole = read_window(&resources, &auth, &thread, "").await;
    assert_eq!(contents(&whole), ["m1", "m2", "m3", "m4", "m5"]);
    assert!(
        !whole.has_more,
        "nothing remains before a thread read whole"
    );

    // The newest two, oldest first, with older rows remaining.
    let newest = read_window(&resources, &auth, &thread, "?limit=2").await;
    assert_eq!(contents(&newest), ["m4", "m5"]);
    assert!(newest.has_more);

    // The window before the oldest row held.
    let before = &newest.messages[0].id;
    let middle = read_window(
        &resources,
        &auth,
        &thread,
        &format!("?limit=2&before={before}"),
    )
    .await;
    assert_eq!(contents(&middle), ["m2", "m3"]);
    assert!(middle.has_more);

    let before = &middle.messages[0].id;
    let oldest = read_window(
        &resources,
        &auth,
        &thread,
        &format!("?limit=2&before={before}"),
    )
    .await;
    assert_eq!(contents(&oldest), ["m1"]);
    assert!(
        !oldest.has_more,
        "the first row of the thread ends the read back"
    );

    // A cursor that names no row of the thread reads an empty window.
    let stranger = Uuid::new_v4();
    let none = read_window(&resources, &auth, &thread, &format!("?before={stranger}")).await;
    assert!(none.messages.is_empty());
    assert!(!none.has_more);
}

#[tokio::test]
async fn each_row_names_the_channel_its_turn_came_over() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "channel@example.com").await;
    let thread = personal_thread(&resources, user_id, tenant_id).await;
    let owner = (user_id, tenant_id);
    add_row(
        &resources,
        &thread,
        owner,
        Row::user("from the app").over("web"),
    )
    .await;
    add_row(
        &resources,
        &thread,
        owner,
        Row::user("from telegram").over("telegram"),
    )
    .await;
    add_row(&resources, &thread, owner, Row::assistant("a post")).await;

    let window = read_window(&resources, &auth, &thread, "").await;
    let channels: Vec<Option<&str>> = window
        .messages
        .iter()
        .map(|m| m.channel_type.as_deref())
        .collect();
    assert_eq!(channels, [Some("web"), Some("telegram"), None]);
}

/// `GET …/topics` as the athlete, as JSON.
async fn topics(resources: &Arc<ServerContext>, auth: &str, conversation: &str) -> Vec<Value> {
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{conversation}/topics"))
        .header("authorization", auth)
        .send(ChatRoutes::routes(Arc::clone(resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: Value = resp.json();
    body["topics"].as_array().cloned().unwrap()
}

#[tokio::test]
async fn topics_are_titled_with_the_athletes_first_words_in_each() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "topics@example.com").await;
    let thread = personal_thread(&resources, user_id, tenant_id).await;
    let owner = (user_id, tenant_id);
    add_row(
        &resources,
        &thread,
        owner,
        Row::user("How was my **tempo** week?"),
    )
    .await;
    add_row(&resources, &thread, owner, Row::assistant("Solid.")).await;

    // "+" opens a topic: a divider, since the opening holds a turn.
    let resp = AxumTestRequest::post("/api/chat/personal/topics")
        .header("authorization", &auth)
        .json(&json!({}))
        .send(ChatRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    // A command in the new topic is account state, never its title.
    add_row(&resources, &thread, owner, Row::user("/status").command()).await;
    add_row(&resources, &thread, owner, Row::user("Plan my long run")).await;

    let listed = topics(&resources, &auth, &thread).await;
    assert_eq!(
        listed.len(),
        2,
        "one topic per divider, plus the opening: {listed:?}"
    );
    assert!(
        listed[0]["divider_id"].is_string(),
        "the newest topic opens on its divider"
    );
    assert_eq!(listed[0]["title"], "Plan my long run");
    assert!(
        listed[1].get("divider_id").is_none(),
        "the opening has no divider"
    );
    assert_eq!(
        listed[1]["title"], "How was my tempo week?",
        "a title is laid out as one plain line"
    );

    // A topic nobody has spoken in yet has no title.
    let resp = AxumTestRequest::post("/api/chat/personal/topics")
        .header("authorization", &auth)
        .json(&json!({}))
        .send(ChatRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let listed = topics(&resources, &auth, &thread).await;
    assert_eq!(listed.len(), 3, "{listed:?}");
    assert!(listed[0].get("title").is_none(), "{listed:?}");
}

#[tokio::test]
async fn a_stranger_reads_neither_windows_nor_topics() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, _) = seed_athlete(&resources, "owner@example.com").await;
    let (_, _, stranger) = seed_athlete(&resources, "stranger@example.com").await;
    let thread = personal_thread(&resources, user_id, tenant_id).await;
    let owner = (user_id, tenant_id);
    add_row(&resources, &thread, owner, Row::user("mine")).await;

    for path in ["messages", "topics"] {
        let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{thread}/{path}"))
            .header("authorization", &stranger)
            .send(ChatRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND, "{path}");
    }
}
