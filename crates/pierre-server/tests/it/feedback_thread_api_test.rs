// ABOUTME: HTTP tests for /api/me/feedback — the athlete's in-app feedback thread, its unread team replies — and the feedback.recorded event
// ABOUTME: Stored oldest first, bounded, private, about a reply they rated; each write tells the team, whose replies stay unread until opened (carnet#1235, #1392)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::Utc;
use serde_json::{json, Value};

use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only};
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::{AddMessageParams, TenantId, UpsertMessageFeedbackParams};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{FeedbackWriter, NewFeedbackMessage};
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
            about_user_id: None,
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
    assert_eq!(
        created["message"]["author"], "athlete",
        "bug: what the athlete sends is theirs"
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
    let (status, created) = post(
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
        (
            "feedback_id",
            created["message"]["id"].as_str().unwrap().to_owned(),
        ),
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

    let rating = setup
        .resources
        .common
        .repos
        .chat
        .get_conversation_feedback(&reply.conversation, &a.user_id.to_string(), a.tenant_id)
        .await
        .unwrap()
        .pop()
        .expect("the rating row");
    let event = only(&events, FEEDBACK_RECORDED);
    for (field, expected) in [
        ("user_id", a.user_id.to_string()),
        ("tenant_id", a.tenant_id.to_string()),
        ("feedback_id", rating.id),
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

async fn unread(setup: &Setup, athlete: &Athlete) -> i64 {
    let body: Value = AxumTestRequest::get("/api/me/feedback/unread")
        .header("Authorization", &athlete.token)
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::OK)
        .json();
    body["unread"].as_i64().expect("an unread count")
}

/// A reply the team sent from Slack, stored as the reply flow stores it.
async fn team_reply(setup: &Setup, athlete: &Athlete, body: &str) {
    setup
        .resources
        .common
        .repos
        .feedback_messages
        .insert_feedback_message(&NewFeedbackMessage {
            tenant_id: athlete.tenant_id,
            user_id: athlete.user_id,
            writer: FeedbackWriter::Team {
                slack_user_id: "U0PHIL",
            },
            body,
            screen: None,
            app_version: None,
            locale: None,
            about: None,
            image: None,
            at: Utc::now(),
        })
        .await
        .expect("team reply");
}

/// A team reply reads back as the team's, waits unread until the athlete
/// opens the thread, and is no one else's to see or to clear.
#[tokio::test]
async fn a_team_reply_is_the_teams_and_unread_until_the_thread_is_opened() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-reply-a@example.com").await;
    let b = athlete(&setup, "feedback-reply-b@example.com").await;

    let (status, _) = post(&setup, &a, &json!({ "body": "The map is slow." })).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: the athlete's message is accepted"
    );
    assert_eq!(
        unread(&setup, &a).await,
        0,
        "bug: their own message is never unread"
    );

    team_reply(&setup, &a, "Fixed in tonight's build.").await;
    team_reply(&setup, &a, "It should load in under a second now.").await;

    let messages = thread(&setup, &a).await;
    let authors: Vec<&str> = messages
        .iter()
        .map(|m| m["author"].as_str().unwrap())
        .collect();
    assert_eq!(
        authors,
        ["athlete", "team", "team"],
        "bug: the team's replies read back as the team's, in order"
    );
    assert_eq!(unread(&setup, &a).await, 2, "bug: both replies wait unread");
    assert_eq!(
        unread(&setup, &b).await,
        0,
        "bug: another athlete has nothing waiting"
    );

    // Another athlete opening their own thread clears nothing of this one.
    let cleared: Value = AxumTestRequest::post("/api/me/feedback/read")
        .header("Authorization", &b.token)
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        cleared["marked"], 0,
        "bug: their thread had nothing to clear"
    );
    assert_eq!(
        unread(&setup, &a).await,
        2,
        "bug: someone else cannot read them for the athlete"
    );

    let marked: Value = AxumTestRequest::post("/api/me/feedback/read")
        .header("Authorization", &a.token)
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        marked["marked"], 2,
        "bug: opening the thread reads every waiting reply"
    );
    assert_eq!(
        unread(&setup, &a).await,
        0,
        "bug: nothing waits once opened"
    );

    team_reply(&setup, &a, "One more thing.").await;
    assert_eq!(
        unread(&setup, &a).await,
        1,
        "bug: a later reply waits on its own"
    );
}

/// The unread count and the read mark need a signed-in athlete.
#[tokio::test]
async fn unread_replies_require_a_session() {
    let setup = setup().await;
    let count = AxumTestRequest::get("/api/me/feedback/unread")
        .send(setup.router.clone())
        .await
        .status_code();
    let mark = AxumTestRequest::post("/api/me/feedback/read")
        .send(setup.router.clone())
        .await
        .status_code();
    assert_eq!(
        (count, mark),
        (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED),
        "bug: anonymous calls are refused"
    );
}

/// The smallest bytes a PNG's own header marks as one.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot a real picture, but a PNG by its header";

fn screenshot(media_type: &str, bytes: &[u8]) -> Value {
    json!({ "media_type": media_type, "data": STANDARD.encode(bytes) })
}

/// A screenshot is kept with its message, served back to its athlete as the
/// type its bytes mark, and to no one else; it may come without words, and
/// the team hears that it came.
#[tokio::test]
async fn a_screenshot_is_kept_with_its_message_and_served_only_to_its_athlete() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-image-a@example.com").await;
    let b = athlete(&setup, "feedback-image-b@example.com").await;

    let (events, guard) = capture_notify();
    let (status, created) = post(
        &setup,
        &a,
        &json!({ "image": screenshot("image/png", PNG) }),
    )
    .await;
    drop(guard);
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: a screenshot alone is a message"
    );
    assert_eq!(
        created["message"]["has_image"], true,
        "bug: the stored message says it carries a screenshot"
    );
    assert_eq!(created["message"]["body"], "", "bug: no words came with it");
    assert_eq!(
        only(&events, FEEDBACK_RECORDED).field("has_image"),
        "true",
        "bug: the team hears a screenshot came"
    );

    let id = created["message"]["id"].as_str().unwrap().to_owned();
    let path = format!("/api/me/feedback/{id}/image");
    let response = AxumTestRequest::get(&path)
        .header("Authorization", &a.token)
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        response.header("content-type"),
        Some("image/png"),
        "bug: the screenshot is served as the type its bytes mark"
    );
    assert_eq!(
        response.header("x-content-type-options"),
        Some("nosniff"),
        "bug: a browser must not sniff an athlete's upload into another type"
    );
    assert_eq!(
        response.bytes().as_slice(),
        PNG,
        "bug: the bytes come back as sent"
    );

    let stranger = AxumTestRequest::get(&path)
        .header("Authorization", &b.token)
        .send(setup.router.clone())
        .await
        .status_code();
    assert_eq!(
        stranger,
        StatusCode::NOT_FOUND,
        "bug: another athlete never reads it"
    );
    assert_eq!(
        thread(&setup, &a).await[0]["has_image"],
        true,
        "bug: the thread says which message carries one"
    );
}

/// A screenshot whose bytes are not the type declared, or not an image the
/// thread takes, is refused and nothing is stored; a text message has none.
#[tokio::test]
async fn a_screenshot_that_is_not_what_it_says_is_refused() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-image-refused@example.com").await;
    for image in [
        screenshot("image/gif", PNG),
        screenshot("image/png", b"%PDF-1.7 not an image"),
        screenshot(
            "image/svg+xml",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
        ),
        json!({ "media_type": "image/png", "data": "not base64!" }),
    ] {
        let (status, _) = post(&setup, &a, &json!({ "body": "Look", "image": image })).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "bug: {image} must be refused"
        );
    }
    assert_eq!(
        thread(&setup, &a).await,
        NO_MESSAGES,
        "bug: a refused screenshot stores nothing"
    );

    let (status, created) = post(&setup, &a, &json!({ "body": "Words only." })).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bug: a text message is accepted after the refusals"
    );
    assert_eq!(
        created["message"]["has_image"], false,
        "bug: a text message carries no screenshot"
    );
    let id = created["message"]["id"].as_str().unwrap();
    let none = AxumTestRequest::get(&format!("/api/me/feedback/{id}/image"))
        .header("Authorization", &a.token)
        .send(setup.router.clone())
        .await
        .status_code();
    assert_eq!(
        none,
        StatusCode::NOT_FOUND,
        "bug: a text message serves no screenshot"
    );
}

/// Messages written in one millisecond read back in the order they were sent:
/// the thread breaks a time tie by id, and ids are time-ordered.
#[tokio::test]
async fn messages_written_in_one_millisecond_read_back_in_the_order_they_were_sent() {
    let setup = setup().await;
    let a = athlete(&setup, "feedback-same-instant@example.com").await;
    let at = Utc::now();
    let team = FeedbackWriter::Team {
        slack_user_id: "U0PHIL",
    };
    let sent = [
        (FeedbackWriter::Athlete, "one"),
        (team, "two"),
        (FeedbackWriter::Athlete, "three"),
        (team, "four"),
        (FeedbackWriter::Athlete, "five"),
    ];
    for (writer, body) in sent {
        setup
            .resources
            .common
            .repos
            .feedback_messages
            .insert_feedback_message(&NewFeedbackMessage {
                tenant_id: a.tenant_id,
                user_id: a.user_id,
                writer,
                body,
                screen: None,
                app_version: None,
                locale: None,
                about: None,
                image: None,
                at,
            })
            .await
            .expect("message stored");
    }

    let bodies: Vec<String> = thread(&setup, &a)
        .await
        .iter()
        .map(|m| m["body"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        bodies,
        ["one", "two", "three", "four", "five"],
        "bug: messages sharing a millisecond must keep the order they were sent"
    );
}
