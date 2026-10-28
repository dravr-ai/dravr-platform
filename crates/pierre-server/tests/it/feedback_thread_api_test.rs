// ABOUTME: HTTP tests for GET/POST /api/me/feedback — the athlete's in-app feedback thread — and the feedback.recorded event it shares with the thumbs
// ABOUTME: Stored and read back oldest first, bounded, private to its athlete, only ever about a reply they rated, and each write tells the team (carnet#1235)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Value};

use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only};
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::{AddMessageParams, TenantId, UpsertMessageFeedbackParams};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_mcp_server::routes::feedback::FeedbackRoutes;
use uuid::Uuid;

/// A thread with nothing in it, as `thread` reads one.
const NO_MESSAGES: [Value; 0] = [];

/// The event every kind of feedback emits.
const FEEDBACK_RECORDED: &str = "feedback.recorded";

struct Athlete {
    user_id: Uuid,
    tenant_id: TenantId,
    token: String,
}

struct Setup {
    resources: Arc<ServerContext>,
    router: axum::Router,
}

/// An agent reply in a conversation its athlete holds with that agent, by id.
struct AgentReply {
    conversation: String,
    message: String,
    agent: String,
}

impl AgentReply {
    /// The `about` a thread message carries to point at this reply.
    fn about(&self) -> Value {
        json!({ "conversation_id": self.conversation, "message_id": self.message })
    }
}

async fn setup() -> Setup {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let router = FeedbackRoutes::routes(Arc::clone(&resources));
    Setup { resources, router }
}

async fn athlete(setup: &Setup, email: &str) -> Athlete {
    let (user_id, user, tenant_id) =
        create_test_user_with_plan(&setup.resources.agent.database, email, "starter")
            .await
            .expect("test user");
    let token = format!(
        "Bearer {}",
        generate_test_token(&setup.resources, &user).await
    );
    Athlete {
        user_id,
        tenant_id,
        token,
    }
}

/// A conversation `owner` holds with an agent of their tenant, with one
/// reply from it, optionally rated down.
async fn agent_reply(setup: &Setup, owner: &Athlete, rated: bool) -> AgentReply {
    let repos = &setup.resources.common.repos;
    let owner_id = owner.user_id.to_string();
    let agent_id = repos
        .agents
        .create_system_agent(
            owner.user_id,
            owner.tenant_id,
            &CreateSystemAgentRequest {
                title: "Volume coach".to_owned(),
                description: None,
                system_prompt: "You coach weekly volume.".to_owned(),
                category: AgentCategory::Recovery,
                tags: vec![],
                visibility: AgentVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    let conversation = repos
        .chat
        .create_conversation(
            &owner_id,
            owner.tenant_id,
            "Weekly volume",
            "gemini-1.5-flash",
            Some(&agent_id),
            None,
        )
        .await
        .unwrap();
    let message = repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: owner.tenant_id,
            conversation_id: &conversation.id,
            user_id: &owner_id,
            role: "assistant",
            content: "You rode 9 h 40 this week.",
            token_count: Some(10),
            finish_reason: Some("stop"),
            prompt_tokens: Some(5),
            model: Some("gemini-1.5-flash"),
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
        })
        .await
        .unwrap();
    if rated {
        repos
            .chat
            .upsert_message_feedback(&UpsertMessageFeedbackParams {
                tenant_id: owner.tenant_id,
                conversation_id: &conversation.id,
                message_id: &message.id,
                user_id: &owner_id,
                rating: "down",
                comment: Some("Tuesday is counted twice"),
            })
            .await
            .unwrap();
    }
    AgentReply {
        conversation: conversation.id,
        message: message.id,
        agent: agent_id,
    }
}

async fn post(setup: &Setup, athlete: &Athlete, body: &Value) -> (StatusCode, Value) {
    let response = AxumTestRequest::post("/api/me/feedback")
        .header("Authorization", &athlete.token)
        .json(body)
        .send(setup.router.clone())
        .await;
    let status = response.status_code();
    let json = if status.is_success() {
        response.json::<Value>()
    } else {
        Value::Null
    };
    (status, json)
}

async fn thread(setup: &Setup, athlete: &Athlete) -> Vec<Value> {
    let body: Value = AxumTestRequest::get("/api/me/feedback")
        .header("Authorization", &athlete.token)
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::OK)
        .json();
    body["messages"].as_array().cloned().unwrap_or_default()
}

/// A message is stored trimmed, answered with what a later read returns,
/// and the thread reads back oldest first.
#[tokio::test]
async fn messages_are_stored_and_read_back_oldest_first() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-oldest@example.com").await;
    assert_eq!(
        thread(&setup, &a).await,
        NO_MESSAGES,
        "a new thread is empty"
    );

    let (status, created) = post(
        &setup,
        &a,
        &json!({
            "body": "  The week view skips my indoor ride.  ",
            "screen": "home",
            "app_version": "web 2026.10.28",
            "locale": "en",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: a message with context is accepted"
    );
    assert_eq!(
        created["message"]["body"], "The week view skips my indoor ride.",
        "bug: the stored message is the trimmed text"
    );
    assert!(
        created["message"]["about"].is_null(),
        "bug: a message opened from the menu is about no reply"
    );

    let (status, _) = post(&setup, &a, &json!({ "body": "And the map is slow." })).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: a bare message is accepted"
    );

    let messages = thread(&setup, &a).await;
    let bodies: Vec<&str> = messages
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert_eq!(
        bodies,
        [
            "The week view skips my indoor ride.",
            "And the map is slow."
        ],
        "bug: the thread reads back oldest first"
    );
    assert_eq!(
        messages[0]["id"], created["message"]["id"],
        "bug: the 201 names the row a later read returns"
    );
    assert!(
        messages[0]["created_at"].is_string(),
        "bug: every message carries when it was sent"
    );
}

/// A message exactly at the bound is accepted whole; an empty one and one a
/// character past the bound are refused, and nothing is stored for either.
#[tokio::test]
async fn a_message_at_the_bound_is_kept_and_one_past_it_is_refused() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-bounds@example.com").await;
    for body in [
        json!({ "body": "   " }),
        json!({ "body": "x".repeat(4001) }),
    ] {
        let (status, _) = post(&setup, &a, &body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "bug: an empty or oversized message is refused"
        );
    }
    assert_eq!(
        thread(&setup, &a).await,
        NO_MESSAGES,
        "bug: a refused message stores nothing"
    );

    let longest = "x".repeat(4000);
    let (status, created) = post(&setup, &a, &json!({ "body": longest })).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: a message exactly at the bound is accepted"
    );
    assert_eq!(
        created["message"]["body"], longest,
        "bug: a message at the bound is stored whole"
    );
}

/// Opened from a thumbs-down, a message carries the reply it is about, and
/// the thread reads that link back.
#[tokio::test]
async fn a_reply_the_athlete_rated_is_attached() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-about@example.com").await;
    let reply = agent_reply(&setup, &a, true).await;

    let about = reply.about();
    let (status, created) = post(
        &setup,
        &a,
        &json!({ "body": "Strava and Garmin both sent Tuesday.", "about": about }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: a message about a rated reply is accepted"
    );
    assert_eq!(
        created["message"]["about"], about,
        "bug: the 201 names the reply"
    );
    assert_eq!(
        thread(&setup, &a).await[0]["about"],
        about,
        "bug: the thread reads the reply back"
    );
}

/// A reply the athlete never rated, and a reply in another athlete's
/// conversation, are not something a thread message can point at.
#[tokio::test]
async fn a_reply_the_athlete_did_not_rate_is_refused() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-unrated@example.com").await;
    let b = athlete(&setup, "feedback-stranger@example.com").await;

    let own_unrated = agent_reply(&setup, &a, false).await;
    let their_rated = agent_reply(&setup, &b, true).await;

    for reply in [own_unrated, their_rated] {
        let (status, _) = post(
            &setup,
            &a,
            &json!({ "body": "About that reply.", "about": reply.about() }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "bug: only a reply the caller rated can be attached"
        );
    }
    assert_eq!(thread(&setup, &a).await, NO_MESSAGES, "nothing was stored");
}

/// One athlete's thread is theirs alone.
#[tokio::test]
async fn another_athletes_thread_is_not_visible() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-private-a@example.com").await;
    let b = athlete(&setup, "feedback-private-b@example.com").await;

    let (status, _) = post(&setup, &a, &json!({ "body": "Only for the team." })).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: the first athlete's message is accepted"
    );

    assert_eq!(
        thread(&setup, &a).await.len(),
        1,
        "bug: the author reads their message back"
    );
    assert_eq!(
        thread(&setup, &b).await,
        NO_MESSAGES,
        "bug: another athlete never reads it"
    );
}

/// The thread needs a signed-in athlete.
#[tokio::test]
async fn the_thread_requires_a_session() {
    let setup = setup().await;
    let status = AxumTestRequest::get("/api/me/feedback")
        .send(setup.router.clone())
        .await
        .status_code();
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "bug: an anonymous read is refused"
    );
}

/// Each thread message tells the team: the text as a one-line preview with
/// its length, and the reply and agent it is about when it has one.
#[tokio::test]
async fn a_thread_message_tells_the_team() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-event@example.com").await;
    let reply = agent_reply(&setup, &a, true).await;

    let (events, guard) = capture_notify();
    let (status, _) = post(
        &setup,
        &a,
        &json!({ "body": "Strava and\n\nGarmin both sent Tuesday.", "about": reply.about() }),
    )
    .await;
    drop(guard);
    assert_eq!(status, StatusCode::CREATED, "bug: the message is accepted");

    let event = only(&events, FEEDBACK_RECORDED);
    for (field, expected) in [
        ("user_id", a.user_id.to_string()),
        ("tenant_id", a.tenant_id.to_string()),
        ("source", "thread".to_owned()),
        ("surface", "app".to_owned()),
        ("rating", "none".to_owned()),
        ("conversation_id", reply.conversation.clone()),
        ("message_id", reply.message.clone()),
        ("agent_id", reply.agent.clone()),
        ("comment_len", "37".to_owned()),
        (
            "comment_preview",
            "Strava and Garmin both sent Tuesday.".to_owned(),
        ),
    ] {
        assert_eq!(
            event.field(field),
            expected,
            "bug: feedback.recorded carries the wrong {field}"
        );
    }
}

/// A message about no reply names none, and a long one reaches the team
/// cut to the preview bound while its full length is still reported.
#[tokio::test]
async fn a_long_message_reaches_the_team_as_a_bounded_preview() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-event-long@example.com").await;

    let (events, guard) = capture_notify();
    let (status, _) = post(&setup, &a, &json!({ "body": "x".repeat(4000) })).await;
    drop(guard);
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: a message at the bound is accepted"
    );

    let event = only(&events, FEEDBACK_RECORDED);
    assert_eq!(
        event.field("conversation_id"),
        "none",
        "bug: a message about no reply names no conversation"
    );
    assert_eq!(
        event.field("agent_id"),
        "none",
        "bug: a message about no reply names no agent"
    );
    assert_eq!(
        event.field("comment_len"),
        "4000",
        "bug: the event reports the whole message's length"
    );
    assert_eq!(
        event.field("comment_preview"),
        format!("{}…", "x".repeat(280)),
        "bug: the event carries the text cut to the preview bound, never whole"
    );
}

/// A refused message tells no one.
#[tokio::test]
async fn a_refused_message_tells_no_one() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-event-refused@example.com").await;
    let unrated = agent_reply(&setup, &a, false).await;

    let (events, guard) = capture_notify();
    let (empty, _) = post(&setup, &a, &json!({ "body": "  " })).await;
    let (unowned, _) = post(
        &setup,
        &a,
        &json!({ "body": "About that reply.", "about": unrated.about() }),
    )
    .await;
    drop(guard);
    assert_eq!(
        (empty, unowned),
        (StatusCode::BAD_REQUEST, StatusCode::NOT_FOUND),
        "bug: both messages are refused"
    );
    assert!(
        named(&events, FEEDBACK_RECORDED).is_empty(),
        "bug: a refused message must not reach the team"
    );
}

/// A thumbs-down with its reason, given through the chat route, tells the
/// team the rating, the reason and the agent that wrote the reply.
#[tokio::test]
async fn a_thumbs_down_tells_the_team() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-event-thumbs@example.com").await;
    let reply = agent_reply(&setup, &a, false).await;
    let chat = ChatRoutes::routes(Arc::clone(&setup.resources));

    let (events, guard) = capture_notify();
    let status = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/messages/{}/feedback",
        reply.conversation, reply.message
    ))
    .header("Authorization", &a.token)
    .json(&json!({ "rating": "down", "comment": "  Tuesday is counted twice " }))
    .send(chat)
    .await
    .status_code();
    drop(guard);
    assert_eq!(status, StatusCode::OK, "bug: the thumbs-down is stored");

    let event = only(&events, FEEDBACK_RECORDED);
    for (field, expected) in [
        ("user_id", a.user_id.to_string()),
        ("tenant_id", a.tenant_id.to_string()),
        ("source", "thumbs".to_owned()),
        ("surface", "app".to_owned()),
        ("rating", "down".to_owned()),
        ("conversation_id", reply.conversation.clone()),
        ("message_id", reply.message.clone()),
        ("agent_id", reply.agent.clone()),
        ("comment_len", "24".to_owned()),
        ("comment_preview", "Tuesday is counted twice".to_owned()),
    ] {
        assert_eq!(
            event.field(field),
            expected,
            "bug: feedback.recorded carries the wrong {field}"
        );
    }
}
