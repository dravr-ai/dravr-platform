// ABOUTME: Tests create_conversation_slot and reactivate_for_turn — the cap the REST create, /reset and turns share
// ABOUTME: Pins the cap refusal, archived threads freeing a slot /api/usage agrees on, a turn reclaiming one, the fallback
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use common::{
    create_test_server_resources, create_test_server_resources_with_llm, create_test_user_with_plan,
};
use futures_util::stream;
use helpers::notify_capture::capture_notify;
use pierre_config::constants::usage_quotas::{
    DEFAULT_MAX_ACTIVE_CONVERSATIONS, UNLIMITED_CONVERSATIONS,
};
use pierre_core::errors::{AppError, AppResult, ErrorCode, RETRY_AFTER_SECS_DETAIL};
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::{NewConversation, PendingGuardianAction};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
use pierre_services::conversation_forge::{
    create_conversation_slot, max_active_conversations, reactivate_for_turn, SlotQuota,
    MAX_ACTIVE_CONVERSATIONS_KEY,
};
use pierre_test_support::db::create_concurrent_test_db;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// Test double for the admin catalogue: answers one pinned value for the
/// conversation cap, or fails the read outright, so the helper's fallback
/// can be exercised without a catalogue row. Real catalogue resolution is
/// covered by `admin_config_scopes_test`.
struct PinnedCap(Result<Option<i64>, ()>);

#[async_trait]
impl AdminConfigLookup for PinnedCap {
    async fn get_value(
        &self,
        key: &str,
        _scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<Value>> {
        assert_eq!(key, MAX_ACTIVE_CONVERSATIONS_KEY);
        match self.0 {
            Ok(cap) => Ok(cap.map(Value::from)),
            Err(()) => Err(AppError::internal("catalogue unreachable")),
        }
    }

    /// The pin is the only row this double knows, so it is its override too.
    async fn get_override_value(
        &self,
        key: &str,
        scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<Value>> {
        self.get_value(key, scope).await
    }
}

async fn athlete_with_threads(n: usize) -> (Arc<ServerContext>, String, TenantId) {
    athlete_with_threads_on(create_test_server_resources().await.unwrap(), n).await
}

/// A fresh athlete owning `n` active threads on `resources`.
async fn athlete_with_threads_on(
    resources: Arc<ServerContext>,
    n: usize,
) -> (Arc<ServerContext>, String, TenantId) {
    let (user_id, _user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        &format!("quota-{}@dravr.test", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    let user = user_id.to_string();
    for i in 0..n {
        resources
            .common
            .repos
            .chat
            .create_conversation(&user, tenant_id, &format!("Thread {i}"), "m", None, None)
            .await
            .unwrap();
    }
    (resources, user, tenant_id)
}

#[tokio::test]
async fn the_cap_refuses_the_thread_that_would_exceed_it() {
    let (resources, user, tenant) = athlete_with_threads(2).await;
    let repos = &resources.common.repos;
    let cap = PinnedCap(Ok(Some(2)));

    let err =
        create_conversation_slot(repos, SlotQuota::Capped(Some(&cap)), &thread(&user, tenant))
            .await
            .expect_err("two owned threads against a cap of two must refuse a third");
    assert_eq!(
        repos.chat.count_conversations(&user, tenant).await.unwrap(),
        2,
        "a refused create writes nothing"
    );
    assert_eq!(err.code, ErrorCode::QuotaExceeded);
    // A fixed cap: waiting does not lift it, so it names no retry window.
    assert_eq!(err.retry_after_secs(), None);
    let details = err.details.expect("a quota refusal carries its numbers");
    assert_eq!(details["limit_type"], "max_active_conversations");
    assert_eq!(details["current"], 2);
    assert_eq!(details["limit"], 2);
    assert!(details.get(RETRY_AFTER_SECS_DETAIL).is_none());
    // A new thread refused is not an archived one: no archive reason.
    assert!(details.get("reason").is_none());
}

/// The REST create answers the cap with a 429 that carries no `Retry-After`,
/// through the application the server serves.
#[tokio::test]
async fn the_cap_refusal_over_http_names_no_retry_window() {
    let threads = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();
    let (resources, user, tenant) = athlete_with_threads(threads).await;
    let token = bearer(&resources, &user, tenant).await;

    let response = ProviderToolRouter::build_http_app(&resources)
        .oneshot(
            Request::post("/api/chat/conversations")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        response.headers().get(RETRY_AFTER).is_none(),
        "a fixed cap carries no Retry-After"
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], "QuotaExceeded");
    assert_eq!(body["details"]["limit_type"], "max_active_conversations");
    assert_eq!(body["details"]["limit"], DEFAULT_MAX_ACTIVE_CONVERSATIONS);
    assert!(body["details"].get(RETRY_AFTER_SECS_DETAIL).is_none());
}

#[tokio::test]
async fn one_below_the_cap_is_allowed() {
    let (resources, user, tenant) = athlete_with_threads(1).await;
    let repos = &resources.common.repos;
    let created = create_conversation_slot(
        repos,
        SlotQuota::Capped(Some(&PinnedCap(Ok(Some(2))))),
        &thread(&user, tenant),
    )
    .await
    .expect("one owned thread against a cap of two leaves room for one more");
    assert_eq!(created.title, "Capped thread");
    assert_eq!(
        repos.chat.count_conversations(&user, tenant).await.unwrap(),
        2
    );
}

/// The platform's own forge — a messaging session repaired — is exempt: it
/// creates past the cap rather than leave a channel dead.
#[tokio::test]
async fn an_exempt_create_ignores_the_cap() {
    let threads = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();
    let (resources, user, tenant) = athlete_with_threads(threads).await;
    let repos = &resources.common.repos;
    create_conversation_slot(repos, SlotQuota::Exempt, &thread(&user, tenant))
        .await
        .expect("an exempt create never refuses");
    assert_eq!(
        repos.chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS + 1
    );
}

/// The row every direct create in this file writes.
fn thread(user: &str, tenant: TenantId) -> NewConversation<'_> {
    NewConversation {
        user_id: user,
        tenant_id: tenant,
        title: "Capped thread",
        model: "m",
        agent_id: None,
        group_id: None,
    }
}

#[tokio::test]
async fn zero_lifts_the_cap() {
    // More threads than the registered default allows, so this passes only
    // because the unlimited value skips the count — not because the count
    // happens to fit.
    let extra = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap() + 2;
    let (resources, user, tenant) = athlete_with_threads(extra).await;
    let cap = PinnedCap(Ok(Some(UNLIMITED_CONVERSATIONS)));
    assert_eq!(
        max_active_conversations(&cap, &user, tenant).await,
        UNLIMITED_CONVERSATIONS
    );
    create_conversation_slot(
        &resources.common.repos,
        SlotQuota::Capped(Some(&cap)),
        &thread(&user, tenant),
    )
    .await
    .expect("an unlimited account is never refused");
}

#[tokio::test]
async fn a_failed_or_unregistered_lookup_falls_back_to_the_registered_default() {
    let (_, user, tenant) = athlete_with_threads(0).await;
    assert_eq!(
        max_active_conversations(&PinnedCap(Err(())), &user, tenant).await,
        DEFAULT_MAX_ACTIVE_CONVERSATIONS,
        "a catalogue read error must degrade to the default, never to no cap"
    );
    assert_eq!(
        max_active_conversations(&PinnedCap(Ok(None)), &user, tenant).await,
        DEFAULT_MAX_ACTIVE_CONVERSATIONS,
        "an unregistered key must degrade to the default"
    );
}

/// A bearer token for `user` scoped to `tenant`.
async fn bearer(resources: &Arc<ServerContext>, user: &str, tenant: TenantId) -> String {
    let athlete = resources
        .common
        .repos
        .users
        .get_global(Uuid::parse_str(user).unwrap())
        .await
        .unwrap()
        .unwrap();
    resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &athlete,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap()
}

/// `resources.conversations` from `GET /api/usage/status`.
async fn usage_conversations(resources: &Arc<ServerContext>, token: &str) -> i64 {
    let response = ProviderToolRouter::build_http_app(resources)
        .oneshot(
            Request::get("/api/usage/status")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body["resources"]["max_conversations"],
        DEFAULT_MAX_ACTIVE_CONVERSATIONS
    );
    body["resources"]["conversations"].as_i64().unwrap()
}

/// `POST /api/chat/conversations` — the "+" button — and its status.
async fn create_over_http(resources: &Arc<ServerContext>, token: &str) -> StatusCode {
    ProviderToolRouter::build_http_app(resources)
        .oneshot(
            Request::post("/api/chat/conversations")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// "Active" means not archived: at the cap, archiving one thread frees the
/// slot the next create takes, and `/api/usage/status` reports the same count
/// the quota checks at every step.
#[tokio::test]
async fn an_archived_thread_frees_its_slot_and_usage_reports_the_same_count() {
    let cap = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();
    let (resources, user, tenant) = athlete_with_threads(cap).await;
    let repos = &resources.common.repos;
    let token = bearer(&resources, &user, tenant).await;

    assert_eq!(
        usage_conversations(&resources, &token).await,
        DEFAULT_MAX_ACTIVE_CONVERSATIONS
    );
    assert_eq!(
        create_over_http(&resources, &token).await,
        StatusCode::TOO_MANY_REQUESTS,
        "fixture precondition: the athlete is at the cap"
    );

    let listed = repos
        .chat
        .list_conversations(&user, tenant, 50, 0)
        .await
        .unwrap();
    assert_eq!(listed.items.len(), cap);
    let retired = listed.items[0].id.clone();
    assert!(repos
        .chat
        .archive_conversation(&retired, &user, tenant)
        .await
        .unwrap());
    assert_eq!(
        repos.chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS - 1
    );
    assert_eq!(
        usage_conversations(&resources, &token).await,
        DEFAULT_MAX_ACTIVE_CONVERSATIONS - 1,
        "usage reports the active count, not every owned row"
    );

    assert_eq!(
        create_over_http(&resources, &token).await,
        StatusCode::CREATED,
        "the slot the archive freed is the one the create takes"
    );
    assert_eq!(
        usage_conversations(&resources, &token).await,
        DEFAULT_MAX_ACTIVE_CONVERSATIONS
    );
    assert_eq!(
        repos
            .chat
            .list_conversations(&user, tenant, 50, 0)
            .await
            .unwrap()
            .items
            .len(),
        cap + 1,
        "the archived thread is still listed"
    );
    let err = create_conversation_slot(repos, SlotQuota::Capped(None), &thread(&user, tenant))
        .await
        .expect_err("back at the cap, the next thread is refused");
    let details = err.details.expect("a quota refusal carries its numbers");
    assert_eq!(details["current"], DEFAULT_MAX_ACTIVE_CONVERSATIONS);
    assert_eq!(details["limit"], DEFAULT_MAX_ACTIVE_CONVERSATIONS);
}

/// One fixed reply and real token counts, so a coaching turn runs the real
/// pipeline end to end without a network model. The turn's own behaviour is
/// covered elsewhere; here only whether it was admitted matters.
struct ReplyingMock;

#[async_trait]
impl LlmProvider for ReplyingMock {
    fn name(&self) -> &'static str {
        "replying_mock"
    }
    fn display_name(&self) -> &'static str {
        "Replying Mock LLM (conversation cap)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }
    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: "Keep the volume and sleep more.".to_owned(),
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(25, 15, 40)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: String::new(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// `POST /api/chat/conversations/{id}/messages` — a turn typed into the app.
async fn post_turn(
    resources: &Arc<ServerContext>,
    token: &str,
    conversation_id: &str,
    content: &str,
) -> (StatusCode, Value) {
    let response = ProviderToolRouter::build_http_app(resources)
        .oneshot(
            Request::post(format!(
                "/api/chat/conversations/{conversation_id}/messages"
            ))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "content": content }).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// An athlete owning `cap` threads with the first one listed archived, so
/// they hold `cap - 1` active slots; returns the archived thread's id.
async fn athlete_with_one_archived() -> (Arc<ServerContext>, String, TenantId, String, String) {
    let resources = create_test_server_resources_with_llm(Arc::new(ReplyingMock))
        .await
        .unwrap();
    let cap = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();
    let (resources, user, tenant) = athlete_with_threads_on(resources, cap).await;
    let chat = &resources.common.repos.chat;
    let archived = chat
        .list_conversations(&user, tenant, 50, 0)
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    assert!(chat
        .archive_conversation(&archived, &user, tenant)
        .await
        .unwrap());
    assert_eq!(
        chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS - 1,
        "fixture precondition: one slot free"
    );
    let token = bearer(&resources, &user, tenant).await;
    (resources, user, tenant, archived, token)
}

/// Under the cap, a turn posted into an archived thread takes its slot back:
/// the thread is active again, the count goes up by one, and the turn runs.
#[tokio::test]
async fn a_turn_into_an_archived_thread_under_the_cap_reactivates_it() {
    let (resources, user, tenant, archived, token) = athlete_with_one_archived().await;
    let chat = &resources.common.repos.chat;

    let (status, body) = post_turn(&resources, &token, &archived, "How was my week?").await;
    assert_eq!(status, StatusCode::OK, "the turn is admitted: {body}");

    assert_eq!(
        chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS,
        "the reactivated thread holds a slot again"
    );
    assert_eq!(
        usage_conversations(&resources, &token).await,
        DEFAULT_MAX_ACTIVE_CONVERSATIONS
    );
    // Active, not merely counted: archiving it now frees a slot.
    assert!(chat
        .archive_conversation(&archived, &user, tenant)
        .await
        .unwrap());
    let messages = chat.get_messages(&archived, &user, tenant).await.unwrap();
    assert_eq!(
        messages.iter().filter(|m| m.role == "user").count(),
        1,
        "the athlete's turn is in the transcript"
    );
    assert!(messages.iter().any(|m| m.role == "assistant"));
}

/// At the cap, the same turn is refused with the conversation-limit error the
/// "+" button gets: nothing is written, and the thread stays archived.
#[tokio::test]
async fn a_turn_into_an_archived_thread_at_the_cap_is_refused_and_writes_nothing() {
    let (resources, user, tenant, archived, token) = athlete_with_one_archived().await;
    let chat = &resources.common.repos.chat;
    // Take the free slot, so the archived thread has none to come back to.
    assert_eq!(
        create_over_http(&resources, &token).await,
        StatusCode::CREATED
    );
    assert_eq!(
        chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS
    );

    // A coaching turn, and a command that carries the thread on (`/pillars`
    // opens a walk its next turns answer), each ask for the slot back.
    for content in ["How was my week?", "/pillars"] {
        let (status, body) = post_turn(&resources, &token, &archived, content).await;
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "{content:?} into an archived thread at the cap is refused: {body}"
        );
        assert_eq!(body["code"], "QuotaExceeded");
        assert_eq!(body["details"]["limit_type"], "max_active_conversations");
        assert_eq!(body["details"]["current"], DEFAULT_MAX_ACTIVE_CONVERSATIONS);
        assert_eq!(body["details"]["limit"], DEFAULT_MAX_ACTIVE_CONVERSATIONS);
        // Both clients word this refusal as "this conversation is archived"
        // from the reason, not as the "+" button's "delete one to start one".
        assert_eq!(body["details"]["reason"], "conversation_archived");
    }
    assert!(
        chat.get_messages(&archived, &user, tenant)
            .await
            .unwrap()
            .is_empty(),
        "a refused turn persists no message row"
    );
    assert!(
        chat.is_conversation_archived(&archived, &user, tenant)
            .await
            .unwrap(),
        "the refused thread stays archived"
    );

    // A command that only reads account state, and a typo, are answered in
    // place: the thread takes no slot for them.
    for content in ["/help", "/nosuchcommand"] {
        let (status, body) = post_turn(&resources, &token, &archived, content).await;
        assert_eq!(status, StatusCode::OK, "{content:?} is answered: {body}");
    }
    assert!(
        chat.is_conversation_archived(&archived, &user, tenant)
            .await
            .unwrap(),
        "a read-only command leaves the thread archived"
    );
    assert_eq!(
        chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS,
        "no command took a slot"
    );
}

/// Under the cap, only a command that carries the thread on reactivates it:
/// `/help` leaves it archived, `/pillars` takes the free slot.
#[tokio::test]
async fn only_a_command_that_resumes_the_thread_reactivates_it() {
    let (resources, user, tenant, archived, token) = athlete_with_one_archived().await;
    let chat = &resources.common.repos.chat;

    let (status, body) = post_turn(&resources, &token, &archived, "/help").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(chat
        .is_conversation_archived(&archived, &user, tenant)
        .await
        .unwrap());
    assert_eq!(
        chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS - 1
    );

    let (status, body) = post_turn(&resources, &token, &archived, "/pillars").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!chat
        .is_conversation_archived(&archived, &user, tenant)
        .await
        .unwrap());
    assert_eq!(
        chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS
    );
}

/// The list the clients render says which rows are archived, so "delete one"
/// can be aimed at a row that frees a slot.
#[tokio::test]
async fn the_conversation_list_marks_the_archived_thread() {
    let (resources, _user, _tenant, archived, token) = athlete_with_one_archived().await;
    let response = ProviderToolRouter::build_http_app(&resources)
        .oneshot(
            Request::get("/api/chat/conversations?limit=50")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let rows = body["conversations"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap()
    );
    let marked: Vec<&str> = rows
        .iter()
        .filter(|row| row["archived_at"].is_string())
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(marked, vec![archived.as_str()], "exactly the archived row");
    let stamp = rows
        .iter()
        .find(|row| row["id"] == archived.as_str())
        .unwrap()["archived_at"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        chrono::DateTime::parse_from_rfc3339(&stamp).is_ok(),
        "{stamp}"
    );
}

/// Concurrent slot decisions at the cap's edge, on a pool whose connections
/// really overlap in the engine: creates and reactivations race for the one
/// free slot. Exactly one wins; every other is the cap refusal — never a
/// database error, which is what a deferred `SQLite` transaction produced
/// when its snapshot went stale under a concurrent writer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_slot_decisions_at_the_edge_take_exactly_the_last_slot() {
    let database = create_concurrent_test_db().await.unwrap();
    let (user_id, _user, tenant) = create_test_user_with_plan(
        &database,
        &format!("race-{}@dravr.test", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    let user = user_id.to_string();
    let repos = Arc::clone(database.repositories());
    let cap = usize::try_from(DEFAULT_MAX_ACTIVE_CONVERSATIONS).unwrap();

    // Two archived threads, then enough active ones to leave one slot free.
    let mut archived = Vec::new();
    for _ in 0..2 {
        let record = repos
            .chat
            .create_conversation(&user, tenant, "Archived", "m", None, None)
            .await
            .unwrap();
        assert!(repos
            .chat
            .archive_conversation(&record.id, &user, tenant)
            .await
            .unwrap());
        archived.push(record.id);
    }
    for i in 0..cap - 1 {
        repos
            .chat
            .create_conversation(&user, tenant, &format!("Active {i}"), "m", None, None)
            .await
            .unwrap();
    }
    assert_eq!(
        repos.chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS - 1
    );

    // Every attempt is spawned before any is awaited, so they overlap.
    let mut attempts = Vec::new();
    for i in 0..12 {
        let repos = Arc::clone(&repos);
        let user = user.clone();
        let archived_id = archived[i % 2].clone();
        attempts.push(tokio::spawn(async move {
            if i % 3 == 0 {
                reactivate_for_turn(&repos, None, &user, tenant, &archived_id).await
            } else {
                create_conversation_slot(&repos, SlotQuota::Capped(None), &thread(&user, tenant))
                    .await
                    .map(|_| ())
            }
        }));
    }
    let mut won = 0;
    for attempt in attempts {
        match attempt.await.unwrap() {
            Ok(()) => won += 1,
            Err(e) => assert_eq!(
                e.code,
                ErrorCode::QuotaExceeded,
                "a losing decision is the cap refusal, never a database error: {e}"
            ),
        }
    }
    // A reactivation of a thread another attempt already reactivated is a
    // no-op success, so count the slots rather than the wins alone.
    assert!(won >= 1);
    assert_eq!(
        repos.chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS,
        "exactly the last slot was taken"
    );
}

/// The same refusal over the stream both clients read turns from: the
/// `failed` frame carries the status, code and details a JSON refusal body
/// would, so the client words it from `details.reason`.
#[tokio::test]
async fn a_streamed_turn_into_an_archived_thread_at_the_cap_fails_with_the_reason() {
    let (resources, user, tenant, archived, token) = athlete_with_one_archived().await;
    assert_eq!(
        create_over_http(&resources, &token).await,
        StatusCode::CREATED,
        "fixture: take the free slot"
    );

    let response = ProviderToolRouter::build_http_app(&resources)
        .oneshot(
            Request::post(format!("/api/chat/conversations/{archived}/messages"))
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "text/event-stream, application/json")
                .body(Body::from(
                    json!({ "content": "How was my week?" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "the stream opens");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    let failed = body
        .split("\n\n")
        .find(|frame| frame.lines().any(|line| line == "event: failed"))
        .unwrap_or_else(|| panic!("no failed frame in {body}"));
    let data = failed
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    let frame: Value = serde_json::from_str(data).unwrap();
    assert_eq!(frame["status"], 429);
    assert_eq!(frame["code"], "QuotaExceeded");
    assert_eq!(frame["details"]["limit_type"], "max_active_conversations");
    assert_eq!(frame["details"]["reason"], "conversation_archived");
    assert_eq!(frame["details"]["limit"], DEFAULT_MAX_ACTIVE_CONVERSATIONS);
    assert!(frame["error"].is_string());
    assert!(
        resources
            .common
            .repos
            .chat
            .get_messages(&archived, &user, tenant)
            .await
            .unwrap()
            .is_empty(),
        "the refused streamed turn persists nothing"
    );
}

/// A command that carries its thread on claims the slot when it resolves,
/// before its handler runs — so one that then finds nothing to do (an unknown
/// agent, a bad invite code, an expired confirmation) still leaves the thread
/// active, holding the free slot. Accepted: the athlete chose to act in that
/// thread, and nothing the handler might write can land before the claim.
#[tokio::test]
async fn a_resuming_command_claims_the_thread_even_when_it_then_does_nothing() {
    for command in ["/agent add @no-such-agent", "/confirm"] {
        let (resources, user, tenant, archived, token) = athlete_with_one_archived().await;
        let chat = &resources.common.repos.chat;
        let content = if command == "/confirm" {
            let expired = PendingGuardianAction {
                id: Uuid::new_v4().simple().to_string(),
                tenant_id: tenant.to_string(),
                user_id: user.clone(),
                conversation_id: Some(archived.clone()),
                tool_name: "disconnect_provider".to_owned(),
                arguments: json!({ "provider": "strava" }),
                deny_reason: "tainted_sink".to_owned(),
            };
            resources
                .common
                .repos
                .guardian_actions
                .create_pending_action(&expired, Utc::now() - Duration::minutes(1))
                .await
                .unwrap();
            format!("/confirm {}", expired.id)
        } else {
            command.to_owned()
        };

        let (status, body) = post_turn(&resources, &token, &archived, &content).await;
        assert_eq!(status, StatusCode::OK, "{content:?} is answered: {body}");
        assert!(
            !chat
                .is_conversation_archived(&archived, &user, tenant)
                .await
                .unwrap(),
            "{content:?} resolved to a resuming command, so the thread is active"
        );
        assert_eq!(
            chat.count_conversations(&user, tenant).await.unwrap(),
            DEFAULT_MAX_ACTIVE_CONVERSATIONS,
            "{content:?} took the free slot"
        );
    }
}

/// `/group join` never writes to the thread it is typed in — a member code
/// files a room beside it and a coach code adopts only an active thread — so
/// it claims no slot: typed into an archived thread, even at the cap, it is
/// answered and the thread stays archived.
#[tokio::test]
async fn group_join_leaves_an_archived_thread_archived() {
    let (resources, user, tenant, archived, token) = athlete_with_one_archived().await;
    assert_eq!(
        create_over_http(&resources, &token).await,
        StatusCode::CREATED,
        "the free slot is taken, so the athlete is at the cap"
    );
    let chat = &resources.common.repos.chat;

    let (status, body) = post_turn(&resources, &token, &archived, "/group join BADCODE").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the join is answered, not refused: {body}"
    );
    assert!(
        chat.is_conversation_archived(&archived, &user, tenant)
            .await
            .unwrap(),
        "a join claims no slot, so the thread stays archived"
    );
    assert_eq!(
        chat.count_conversations(&user, tenant).await.unwrap(),
        DEFAULT_MAX_ACTIVE_CONVERSATIONS
    );
}

/// A command refused at the cap is recorded like any other failed command:
/// one `messaging.command_executed` with `success = false`, the handler's
/// refusal being the command's own outcome.
#[tokio::test]
async fn a_command_refused_at_the_cap_is_recorded_as_a_failed_command() {
    let (resources, _user, _tenant, archived, token) = athlete_with_one_archived().await;
    assert_eq!(
        create_over_http(&resources, &token).await,
        StatusCode::CREATED,
        "fixture: take the free slot"
    );

    let (events, _guard) = capture_notify();
    let (status, body) = post_turn(&resources, &token, &archived, "/pillars").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");

    let executed: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.event == "messaging.command_executed")
        .cloned()
        .collect();
    assert_eq!(executed.len(), 1, "{executed:?}");
    assert_eq!(executed[0].field("command_name"), "pillars");
    assert_eq!(executed[0].field("success"), "false");
}
