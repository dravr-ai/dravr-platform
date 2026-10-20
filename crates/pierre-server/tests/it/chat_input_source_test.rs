// ABOUTME: A tapped next step reaches the turn as its words or its command, counted as a next step, never as typing
// ABOUTME: Pins nx: taps, origin=next_step, the Next time line, command replies' steps, a first analysis, and client claims

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#830.
//!
//! A next step is what a reply hands the athlete as it ends — a guided walk's
//! wrap-up, a command's answer — posted as `nx:<slot>:<id>`. The turn
//! resolves it into the catalogue entry's command or prompt, and the tap is
//! counted apart from a typed question and from a welcome starter: the share
//! of an athlete's turns that were taps is read from
//! `chat.question_asked.origin`, so a next step reporting itself as `typed`
//! would hide the very taps it exists to count. A tap that ran a command ends
//! on one line teaching it. Like a resolved starter, a next step is the
//! server's finding: a client may not claim one.
//!
//! These drive the app's own routes with a model that records what it was
//! sent, so they assert what the athlete's transcript holds and what the
//! model read.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use pierre_core::models::{
    ActivityBuilder, GuidedFlow, OnboardingState, SportType, TenantId, COMMAND_FINISH_REASON,
};
use serde_json::{json, Value};
use uuid::Uuid;

use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only, CapturedEvents};
use helpers::recording_llm::RecordingProvider;
use pierre_contremaitre::messaging_strings::KEY_USE_CASES_NEXT_TIME;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatRoutes, MessageResponse, MessagesListResponse, ReplyBlockResponse, TurnResponse,
};
use pierre_services::locale::resolve_user_locale;

const REPLY: &str = "Your next two weeks build on the long run you did on Sunday.";

struct Fixture {
    resources: Arc<ServerContext>,
    provider: Arc<RecordingProvider>,
    user_id: Uuid,
    tenant_id: TenantId,
    auth: String,
    conversation_id: String,
    locale: String,
}

/// An athlete with nothing connected and nothing told, alone in a thread of
/// their own.
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
    let conversation_id = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant_id,
            "Next steps",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), user_id).await;
    Fixture {
        resources,
        provider,
        user_id,
        tenant_id,
        auth,
        conversation_id,
        locale,
    }
}

async fn send(fx: &Fixture, body: &Value) -> TurnResponse {
    let resp = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation_id
    ))
    .header("authorization", &fx.auth)
    .json(body)
    .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
    .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{body}");
    resp.json()
}

async fn messages(fx: &Fixture) -> Vec<MessageResponse> {
    let resp = AxumTestRequest::get(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation_id
    ))
    .header("authorization", &fx.auth)
    .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
    .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json::<MessagesListResponse>().messages
}

fn string(fx: &Fixture, key: &str) -> String {
    fx.resources
        .mcp
        .messaging_strings_registry
        .get(key, &fx.locale)
}

/// The line a reply ends on when a tap ran `command`.
fn next_time(fx: &Fixture, command: &str) -> String {
    let line = fx.resources.mcp.messaging_strings_registry.render(
        KEY_USE_CASES_NEXT_TIME,
        &fx.locale,
        &[command],
    );
    assert!(
        line.contains(command),
        "the pinned strings carry the Next time line: {line:?}"
    );
    line
}

/// The postbacks the live reply's buttons send, in order.
fn offered(turn: &TurnResponse) -> Vec<String> {
    turn.assistant
        .blocks
        .iter()
        .filter_map(|block| match block {
            ReplyBlockResponse::Actions { actions, .. } => Some(actions),
            _ => None,
        })
        .flatten()
        .map(|action| action.value.clone())
        .collect()
}

fn origins(events: &CapturedEvents, name: &str) -> Vec<String> {
    named(events, name)
        .iter()
        .map(|event| event.field("origin").to_owned())
        .collect()
}

/// A tapped step that stands for a question is the athlete's question, in
/// their words, and is counted as a next step on both analytics events and
/// on the tap itself. It ran no command, so it teaches none.
#[tokio::test]
async fn a_tapped_next_step_is_its_question_and_counts_as_a_next_step() {
    let fx = setup("next-step-question@test.com").await;
    let prompt = string(&fx, "use_cases.connect.prompt");
    let (events, _guard) = capture_notify();

    let before = fx.provider.calls_so_far();
    let turn = send(&fx, &json!({ "content": "nx:1:connect" })).await;

    assert_eq!(turn.user_message.content, prompt);
    assert!(
        fx.provider
            .requests_since(before)
            .iter()
            .any(|request| request.conversation.contains(&prompt)),
        "the model answers the prompt, not the postback"
    );
    assert!(
        messages(&fx)
            .await
            .iter()
            .all(|row| !row.content.starts_with("nx:")),
        "no postback reaches the transcript"
    );
    assert_eq!(origins(&events, "chat.question_asked"), ["next_step"]);
    assert_eq!(
        origins(&events, "chat.answer_delivered"),
        ["next_step"],
        "the answer is attributed to the same tap as the question"
    );
    let tapped = only(&events, "suggestion.tapped");
    assert_eq!(tapped.field("use_case"), "connect");
    assert_eq!(tapped.field("source"), "next_step");
    assert_eq!(tapped.field("surface"), "next_step");
    assert_eq!(tapped.field("position"), "1");
    assert_eq!(tapped.field("channel"), "web_chat");
    assert!(
        offered(&turn).is_empty(),
        "an ordinary coaching answer offers no step"
    );
}

/// A tapped step that stands for a command runs it — no model — and the
/// reply ends teaching the command, then hands over the command's own next
/// steps, live and on the row a reload reads.
#[tokio::test]
async fn a_tapped_command_step_runs_it_teaches_it_and_offers_the_next() {
    let fx = setup("next-step-command@test.com").await;
    let (events, _guard) = capture_notify();

    let before = fx.provider.calls_so_far();
    let turn = send(&fx, &json!({ "content": "nx:0:can_see" })).await;

    assert_eq!(turn.user_message.content, "/status");
    assert_eq!(
        turn.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    assert_eq!(fx.provider.calls_so_far(), before, "no model ran");
    let content = &turn.assistant.message.content;
    assert!(
        content.ends_with(&format!("\n\n{}", next_time(&fx, "/status"))),
        "the reply ends teaching the command it ran: {content}"
    );

    // Nothing connected and nothing told: connect a source, then tell the
    // coach about yourself.
    assert_eq!(offered(&turn), ["nx:0:connect", "nx:1:about_me"]);
    let tapped = only(&events, "suggestion.tapped");
    assert_eq!(tapped.field("use_case"), "can_see");
    assert_eq!(tapped.field("source"), "next_step");
    assert_eq!(tapped.field("surface"), "next_step");
    let shown = named(&events, "suggestion.shown");
    let shown: Vec<(&str, &str, &str, &str)> = shown
        .iter()
        .map(|event| {
            (
                event.field("use_case"),
                event.field("position"),
                event.field("source"),
                event.field("surface"),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("connect", "0", "next_step", "next_step"),
            ("about_me", "1", "next_step", "next_step"),
        ]
    );

    let rows = messages(&fx).await;
    let reply = rows
        .iter()
        .rfind(|row| row.role == "assistant")
        .expect("the command's reply is written");
    assert_eq!(&reply.content, content);
    let stored: Vec<&str> = reply
        .actions
        .as_ref()
        .expect("the steps are stored with the reply")
        .actions
        .iter()
        .map(|action| action.value.as_str())
        .collect();
    assert_eq!(stored, ["nx:0:connect", "nx:1:about_me"]);

    let tapped_at = fx
        .resources
        .common
        .repos
        .use_case_exposures
        .use_case_exposures(fx.tenant_id, fx.user_id)
        .await
        .unwrap()
        .into_iter()
        .find(|exposure| exposure.use_case_id == "can_see")
        .and_then(|exposure| exposure.tapped_at);
    assert!(tapped_at.is_some(), "a once entry tapped as a step is used");
}

/// A typed command already knows its own name: it is offered the same next
/// steps, and taught nothing.
#[tokio::test]
async fn a_typed_command_offers_its_steps_and_teaches_nothing() {
    let fx = setup("next-step-typed@test.com").await;
    let line = next_time(&fx, "/status");
    let (events, _guard) = capture_notify();

    let turn = send(&fx, &json!({ "content": "/status" })).await;

    assert!(
        !turn.assistant.message.content.contains(&line),
        "{}",
        turn.assistant.message.content
    );
    assert_eq!(offered(&turn), ["nx:0:connect", "nx:1:about_me"]);
    assert!(named(&events, "suggestion.tapped").is_empty());
}

/// A command that opens a walk hands the thread to the walk: its reply is
/// the walk's question, and no step competes with answering it.
#[tokio::test]
async fn a_command_in_a_thread_a_walk_owns_offers_no_step() {
    let fx = setup("next-step-walk@test.com").await;
    let walk = OnboardingState::start(Utc::now().to_rfc3339(), GuidedFlow::Pillars);
    fx.resources
        .common
        .repos
        .chat
        .set_conversation_onboarding_state(
            &fx.conversation_id,
            Some(&walk.to_column().unwrap()),
            fx.tenant_id,
        )
        .await
        .unwrap();

    let turn = send(&fx, &json!({ "content": "/status" })).await;

    assert_eq!(
        turn.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    assert!(offered(&turn).is_empty(), "{:?}", turn.assistant.blocks);
}

/// The athlete's first tapped workout analysis ends on the week-on-week
/// comparison, once there are two weeks to compare; a second analysis is an
/// ordinary coaching turn.
#[tokio::test]
async fn a_first_tapped_analysis_leads_to_the_weekly_comparison_once() {
    let fx = setup("next-step-analysis@test.com").await;
    for days_ago in [2, 9] {
        let activity = ActivityBuilder::new(
            format!("strava-{}", Uuid::new_v4()),
            "Morning ride".to_owned(),
            SportType::Ride,
            Utc::now() - Duration::days(days_ago),
            3_600,
            "strava".to_owned(),
        )
        .build();
        fx.resources
            .common
            .repos
            .activity_cache
            .upsert_activities(fx.user_id, &fx.tenant_id, "strava", &[activity])
            .await
            .unwrap();
    }

    let first = send(&fx, &json!({ "content": "uc:0:last_workout" })).await;
    assert_eq!(
        first.user_message.content,
        string(&fx, "use_cases.last_workout.prompt")
    );
    assert_eq!(offered(&first), ["nx:0:compare_week"]);
    let stored = messages(&fx)
        .await
        .into_iter()
        .rfind(|row| row.role == "assistant")
        .and_then(|row| row.actions)
        .expect("the step is stored with the answer");
    assert_eq!(stored.actions[0].value, "nx:0:compare_week");

    let second = send(&fx, &json!({ "content": "uc:0:last_workout" })).await;
    assert!(
        offered(&second).is_empty(),
        "only the first analysis leads on"
    );
}

#[tokio::test]
async fn a_client_cannot_claim_a_next_step() {
    let fx = setup("next-step-claim@test.com").await;
    let (events, _guard) = capture_notify();

    let claimed = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation_id
    ))
    .header("authorization", &fx.auth)
    .json(&json!({ "content": "/season", "origin": "next_step" }))
    .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
    .await;

    assert_eq!(
        claimed.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "only the server resolves a next step"
    );
    assert!(
        named(&events, "chat.question_asked").is_empty(),
        "a refused claim asks nothing"
    );
    let rows = fx
        .resources
        .common
        .repos
        .chat
        .get_messages(&fx.conversation_id, &fx.user_id.to_string(), fx.tenant_id)
        .await
        .unwrap();
    assert!(rows.is_empty(), "and writes nothing: {rows:?}");
}
