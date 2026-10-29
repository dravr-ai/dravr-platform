// ABOUTME: The messages list tells a reloaded client the caller's turn is still running, from the rows on file
// ABOUTME: Set for the caller's own unanswered question on the newest window; null once answered, stopped, too old, older windows, or another's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A turn runs detached from the request that started it, and its reply
//! streams only on that request's body (carnet#1193). A client that reloads
//! mid-turn reads the transcript back from `GET …/messages`, which carries
//! `awaiting_reply` so the client knows a reply is coming: the caller's own
//! newest question, while it has neither a reply nor a stop and is younger
//! than the turn watchdog.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use crate::common;
use crate::helpers;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::Router;
use chrono::DateTime;
use pierre_chat_pipeline::turn_stop::question_blocks;
use pierre_core::llm::LlmProvider;
use pierre_core::models::{AddMessageParams, TenantId, User};
use pierre_core::transport::TransportPolicy;
use pierre_database::database::MessageRecord;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use serde_json::{json, Value};
use tokio::time::sleep;

use common::{
    create_test_server_resources_with_chat_provider,
    create_test_server_resources_with_chat_provider_and_config, create_test_tenant,
    test_server_config,
};
use helpers::axum_test::AxumTestRequest;
use helpers::drained_turn::{wait_for_a_tracked_turn, wait_for_turns_to_finish, HangingProvider};

const QUESTION: &str = "construis-moi un plan pour le marathon";

/// Long enough for the turn's one-second stop poll to fire a few times over.
const STOP_BUDGET: Duration = Duration::from_secs(15);

struct Fixture {
    resources: Arc<ServerContext>,
    router: Router,
    user: User,
    token: String,
    conversation_id: String,
}

async fn fixture_over(resources: Arc<ServerContext>, email: &str) -> Fixture {
    let (user, token) = create_test_tenant(&resources, email)
        .await
        .expect("seed user + tenant");
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let created = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({"title": "plan", "model": "gemini-1.5-flash"}))
        .send(router.clone())
        .await;
    assert_eq!(created.status_code(), StatusCode::CREATED);
    let conversation: Value = created.json();
    let conversation_id = conversation["id"].as_str().expect("id").to_owned();
    Fixture {
        resources,
        router,
        user,
        token,
        conversation_id,
    }
}

/// A conversation whose turns park on a provider that never answers.
async fn fixture(email: &str) -> Fixture {
    let provider: Arc<dyn LlmProvider> = Arc::new(HangingProvider);
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .expect("resources");
    fixture_over(resources, email).await
}

impl Fixture {
    async fn tenant(&self) -> TenantId {
        self.resources
            .common
            .repos
            .tenants
            .list_for_user(self.user.id)
            .await
            .unwrap()[0]
            .id
    }

    /// `GET …/messages` as the holder of `token`, optionally before a row.
    async fn list(&self, token: &str, before: Option<&str>) -> Value {
        let path = before.map_or_else(
            || format!("/api/chat/conversations/{}/messages", self.conversation_id),
            |id| {
                format!(
                    "/api/chat/conversations/{}/messages?before={id}",
                    self.conversation_id
                )
            },
        );
        let response = AxumTestRequest::get(&path)
            .header("authorization", &format!("Bearer {token}"))
            .send(self.router.clone())
            .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        let body: Value = response.json();
        assert!(
            body.as_object()
                .is_some_and(|o| o.contains_key("awaiting_reply")),
            "awaiting_reply is always on the wire, null when nothing is awaited: {body}"
        );
        body
    }

    /// Store a row the way a turn does: a question names its author.
    async fn seed(&self, author: &str, role: &str, content: &str) -> MessageRecord {
        let marker = (role == "user")
            .then(|| question_blocks(author, None))
            .flatten();
        self.resources
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: self.tenant().await,
                conversation_id: &self.conversation_id,
                user_id: author,
                role,
                content,
                token_count: None,
                finish_reason: (role == "assistant").then_some("stop"),
                prompt_tokens: None,
                model: None,
                content_blocks: marker.as_deref(),
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: Some("web"),
            })
            .await
            .unwrap()
    }

    async fn ask_and_hang_up(&self) {
        let response = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{}/messages",
            self.conversation_id
        ))
        .header("authorization", &format!("Bearer {}", self.token))
        .header("accept", "text/event-stream")
        .json(&json!({"content": QUESTION}))
        .send_sse(self.router.clone())
        .await;
        assert_eq!(response.status_code(), StatusCode::OK, "the stream opened");
        assert!(
            wait_for_a_tracked_turn(&self.resources).await,
            "the turn is running, detached from the dropped stream"
        );
        for _ in 0..150 {
            let body = self.list(&self.token, None).await;
            if body["messages"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|m| m["role"] == "user"))
            {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
        panic!("the turn never stored its question");
    }

    async fn stop(&self) {
        let response = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{}/stop",
            self.conversation_id
        ))
        .header("authorization", &format!("Bearer {}", self.token))
        .send(self.router.clone())
        .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.json::<Value>(), json!({"stopped": true}));
        assert!(
            wait_for_turns_to_finish(&self.resources, STOP_BUDGET).await,
            "the stopped turn ends"
        );
    }
}

/// The reload case itself: the coach hangs up a running turn, reads the
/// thread back, and is told which question a reply is coming for. Once the
/// turn is stopped, nothing is awaited.
#[tokio::test]
async fn a_running_turn_is_awaited_until_it_is_stopped() {
    let fx = fixture("awaiting-running@test.local").await;
    fx.ask_and_hang_up().await;

    let body = fx.list(&fx.token, None).await;
    let question = &body["messages"][0];
    assert_eq!(question["role"], "user");
    assert_eq!(
        body["awaiting_reply"],
        json!({
            "question_id": question["id"],
            "asked_at": question["created_at"],
        }),
        "the newest window names the running turn's question"
    );
    let asked_at = body["awaiting_reply"]["asked_at"].as_str().unwrap();
    assert!(
        DateTime::parse_from_rfc3339(asked_at).is_ok(),
        "asked_at is RFC 3339: {asked_at}"
    );

    // An older window is history: it says nothing about a running turn.
    let older = fx
        .list(&fx.token, Some(question["id"].as_str().unwrap()))
        .await;
    assert_eq!(older["awaiting_reply"], Value::Null);

    fx.stop().await;
    let body = fx.list(&fx.token, None).await;
    assert_eq!(
        body["awaiting_reply"],
        Value::Null,
        "a stopped question awaits nothing"
    );
}

/// A question its reply answered is not awaited, and the window size does
/// not matter: the newest window of one row still sees the open question.
#[tokio::test]
async fn an_answered_question_is_not_awaited() {
    let fx = fixture("awaiting-answered@test.local").await;
    let me = fx.user.id.to_string();
    let body = fx.list(&fx.token, None).await;
    assert_eq!(body["awaiting_reply"], Value::Null, "an empty thread");

    let question = fx.seed(&me, "user", QUESTION).await;
    fx.seed(&me, "tool_call", "get_activities").await;
    let narrow = AxumTestRequest::get(&format!(
        "/api/chat/conversations/{}/messages?limit=1",
        fx.conversation_id
    ))
    .header("authorization", &format!("Bearer {}", fx.token))
    .send(fx.router.clone())
    .await
    .json::<Value>();
    assert_eq!(narrow["messages"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        narrow["awaiting_reply"]["question_id"],
        question.id.as_str(),
        "a window that does not reach the question still reports it"
    );

    fx.seed(&me, "assistant", "Voici ton plan.").await;
    let body = fx.list(&fx.token, None).await;
    assert_eq!(body["awaiting_reply"], Value::Null);
}

/// Past the turn watchdog, an unanswered question is a turn that died without
/// writing a row: reporting it would keep the client waiting on nothing.
#[tokio::test]
async fn a_question_older_than_the_turn_watchdog_is_not_awaited() {
    let mut config = test_server_config();
    config.timeouts.turn_watchdog_secs = Some(2);
    let resources = create_test_server_resources_with_chat_provider_and_config(
        Arc::new(HangingProvider),
        config,
    )
    .await
    .unwrap();
    let fx = fixture_over(resources, "awaiting-old@test.local").await;
    let question = fx.seed(&fx.user.id.to_string(), "user", QUESTION).await;

    let body = fx.list(&fx.token, None).await;
    assert_eq!(
        body["awaiting_reply"]["question_id"],
        question.id.as_str(),
        "inside the watchdog the question is awaited"
    );

    sleep(Duration::from_secs(3)).await;
    let body = fx.list(&fx.token, None).await;
    assert_eq!(
        body["awaiting_reply"],
        Value::Null,
        "past the watchdog it is not"
    );
}

/// In a shared conversation each participant waits only on their own
/// question: another member's running turn is not the caller's.
#[tokio::test]
async fn another_participants_question_is_not_the_callers() {
    let fx = fixture("awaiting-owner@test.local").await;
    let tenant = fx.tenant().await;
    let (member, _) = create_test_tenant(&fx.resources, "awaiting-member@test.local")
        .await
        .expect("seed the member");
    fx.resources
        .common
        .repos
        .users
        .update_tenant_id(member.id, tenant)
        .await
        .unwrap();
    let member_token = fx
        .resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &member,
            &fx.resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap();
    let added = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/participants",
        fx.conversation_id
    ))
    .header("authorization", &format!("Bearer {}", fx.token))
    .json(&json!({"user_id": member.id.to_string()}))
    .send(fx.router.clone())
    .await;
    assert_eq!(added.status_code(), StatusCode::CREATED);

    let question = fx.seed(&member.id.to_string(), "user", QUESTION).await;

    let owner_view = fx.list(&fx.token, None).await;
    assert_eq!(
        owner_view["messages"].as_array().map(Vec::len),
        Some(1),
        "the owner reads the member's question"
    );
    assert_eq!(
        owner_view["awaiting_reply"],
        Value::Null,
        "but awaits nothing: the question is not theirs"
    );
    let member_view = fx.list(&member_token, None).await;
    assert_eq!(
        member_view["awaiting_reply"]["question_id"],
        question.id.as_str(),
        "the member who asked is the one waiting"
    );
}
