// ABOUTME: A tapped welcome starter reaches the turn as its question — resolved on the server and counted as a tap
// ABOUTME: Pins ex: postbacks, suggestion.shown/.tapped, the origin on question_asked/answer_delivered, and a stale tap

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#828, P0.
//!
//! A welcome's starter used to post its own question as its postback, so a
//! tap arrived as the same text as typing it: nothing could say whether
//! suggestions are used at all. A starter now posts `ex:<slot>:<index>`, the
//! turn resolves it back into the question before anything reads the text,
//! and the tap is counted — `suggestion.shown` when the welcome lands,
//! `suggestion.tapped` and `origin=use_case` when it is pressed.
//!
//! These drive the app's own routes with a model that records what it was
//! sent, so they assert what the athlete's transcript holds and what the model
//! actually read, not the source that builds them.

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only, CapturedEvents};
use helpers::recording_llm::RecordingProvider;
use pierre_chat_pipeline::agent_welcome::{post_agent_welcome, WelcomeTarget};
use pierre_chat_pipeline::SurfaceId;
use pierre_contremaitre::messaging_strings::KEY_USE_CASES_UNAVAILABLE;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::{TenantId, AGENT_WELCOME_FINISH_REASON};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatRoutes, ConversationResponse, MessageResponse, MessagesListResponse, TurnResponse,
};
use pierre_services::locale::resolve_user_locale;

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
    agent_id: String,
    locale: String,
}

/// An athlete with no provider connected, and a nutrition agent whose
/// Example Inputs are [`SAMPLES`].
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
    let repos = &resources.common.repos;
    let agent_id = repos
        .agents
        .create_system_agent(
            user_id,
            tenant_id,
            &CreateSystemAgentRequest {
                title: "Fuelling Agent".to_owned(),
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
        .to_string();
    let locale = resolve_user_locale(repos.users.as_ref(), user_id).await;
    Fixture {
        resources,
        provider,
        user_id,
        tenant_id,
        auth,
        agent_id,
        locale,
    }
}

async fn create_conversation(fx: &Fixture, agent_id: Option<&str>) -> String {
    let body = agent_id.map_or_else(|| json!({}), |id| json!({ "agent_id": id }));
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.auth)
        .json(&body)
        .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    resp.json::<ConversationResponse>().id
}

async fn messages(fx: &Fixture, conv: &str) -> Vec<MessageResponse> {
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{conv}/messages"))
        .header("authorization", &fx.auth)
        .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json::<MessagesListResponse>().messages
}

async fn send(fx: &Fixture, conv: &str, body: &Value) -> TurnResponse {
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv}/messages"))
        .header("authorization", &fx.auth)
        .json(body)
        .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{body}");
    resp.json()
}

fn origins(events: &CapturedEvents, name: &str) -> Vec<String> {
    named(events, name)
        .iter()
        .map(|e| e.field("origin").to_owned())
        .collect()
}

/// The defect and its fix in one thread: the welcome offers its starters as
/// postbacks, a tap reaches the transcript and the model as the question, and
/// the tap is counted as one.
#[tokio::test]
async fn a_tapped_starter_is_its_question_and_counts_as_a_tap() {
    let fx = setup("tap@test.com").await;
    let (events, _guard) = capture_notify();
    let conv = create_conversation(&fx, Some(&fx.agent_id)).await;

    let rows = messages(&fx, &conv).await;
    let welcome = rows
        .iter()
        .find(|m| m.finish_reason.as_deref() == Some(AGENT_WELCOME_FINISH_REASON))
        .expect("the agent opens the thread");
    let values: Vec<&str> = welcome
        .actions
        .as_ref()
        .expect("the welcome carries its starters")
        .actions
        .iter()
        .map(|a| a.value.as_str())
        .collect();
    assert_eq!(values, ["ex:0:0", "ex:1:1", "ex:2:2"]);

    let shown = named(&events, "suggestion.shown");
    assert_eq!(shown.len(), 3, "one shown event per starter");
    for (slot, event) in shown.iter().enumerate() {
        assert_eq!(event.field("use_case"), format!("ex:{slot}"));
        assert_eq!(event.field("position"), slot.to_string());
        assert_eq!(event.field("source"), "static");
        assert_eq!(event.field("surface"), "agent_welcome");
        assert_eq!(event.field("channel"), "web_chat");
    }

    let before = fx.provider.calls_so_far();
    let turn = send(&fx, &conv, &json!({ "content": "ex:1:1" })).await;
    assert_eq!(turn.user_message.content, SAMPLES[1]);

    let rows = messages(&fx, &conv).await;
    assert!(
        rows.iter()
            .any(|m| m.role == "user" && m.content == SAMPLES[1]),
        "the transcript holds the question the athlete tapped"
    );
    assert!(
        rows.iter().all(|m| !m.content.starts_with("ex:")),
        "no postback ever reaches the transcript"
    );
    assert!(
        fx.provider
            .requests_since(before)
            .iter()
            .any(|r| r.conversation.contains(SAMPLES[1])),
        "the model answers the question, not the postback"
    );

    let tapped = only(&events, "suggestion.tapped");
    assert_eq!(tapped.field("use_case"), "ex:1");
    assert_eq!(tapped.field("position"), "1");
    assert_eq!(tapped.field("source"), "static");
    assert_eq!(tapped.field("channel"), "web_chat");
    assert_eq!(origins(&events, "chat.question_asked"), ["use_case"]);
    let delivered = only(&events, "chat.answer_delivered");
    assert_eq!(delivered.field("origin"), "use_case");
    assert_eq!(
        delivered.field("grounded"),
        "false",
        "no provider is connected, so no activities were in front of the model"
    );
    assert_eq!(delivered.field("account_age_hours"), "0");
}

/// A postback that no longer resolves — past the agent's examples, or in a
/// thread with no agent — is answered with a notice: no model call, nothing
/// written, and the athlete never sees the postback.
#[tokio::test]
async fn a_stale_starter_is_told_and_never_reaches_the_model() {
    let fx = setup("stale@test.com").await;
    let (events, _guard) = capture_notify();
    let with_agent = create_conversation(&fx, Some(&fx.agent_id)).await;
    let without_agent = create_conversation(&fx, None).await;
    let notice = fx
        .resources
        .mcp
        .messaging_strings_registry
        .get(KEY_USE_CASES_UNAVAILABLE, &fx.locale);

    for (conv, postback) in [(&with_agent, "ex:0:7"), (&without_agent, "ex:0:0")] {
        let before = fx.provider.calls_so_far();
        let turn = send(&fx, conv, &json!({ "content": postback })).await;
        assert_eq!(turn.assistant.message.content, notice, "{postback}");
        assert_eq!(
            turn.user_message.content, "",
            "the client keeps the label it showed"
        );
        assert_eq!(fx.provider.calls_so_far(), before, "no model ran");
        assert!(
            messages(&fx, conv)
                .await
                .iter()
                .all(|m| !m.content.starts_with("ex:") && m.content != notice),
            "nothing about a stale tap is written to the transcript"
        );
    }
    assert!(named(&events, "suggestion.tapped").is_empty());
    assert!(named(&events, "chat.question_asked").is_empty());
}

/// The client says how the athlete produced a message; only the server may
/// call one a resolved suggestion.
#[tokio::test]
async fn the_client_says_how_the_message_was_produced() {
    let fx = setup("origin@test.com").await;
    let (events, _guard) = capture_notify();
    let conv = create_conversation(&fx, None).await;

    send(&fx, &conv, &json!({ "content": "Bonjour" })).await;
    send(
        &fx,
        &conv,
        &json!({ "content": "Analyse ma sortie", "origin": "draft" }),
    )
    .await;
    send(
        &fx,
        &conv,
        &json!({ "content": "Comment était mon allure ?", "origin": "chip" }),
    )
    .await;
    assert_eq!(
        origins(&events, "chat.question_asked"),
        ["typed", "draft", "chip"]
    );
    assert_eq!(
        origins(&events, "chat.answer_delivered"),
        ["typed", "draft", "chip"]
    );

    let claimed = AxumTestRequest::post(&format!("/api/chat/conversations/{conv}/messages"))
        .header("authorization", &fx.auth)
        .json(&json!({ "content": "Bonjour", "origin": "use_case" }))
        .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
        .await;
    assert_eq!(
        claimed.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a client cannot claim a resolved suggestion"
    );
}

/// A messaging channel receives the welcome as text, where nothing can be
/// tapped: its starters are not counted as shown suggestions.
#[tokio::test]
async fn a_text_welcome_shows_no_suggestion() {
    let fx = setup("text-welcome@test.com").await;
    let conv = create_conversation(&fx, None).await;
    let repos = &fx.resources.common.repos;
    repos
        .chat
        .set_conversation_agent_id(&conv, Some(&fx.agent_id), fx.tenant_id)
        .await
        .unwrap();
    let conversation = repos
        .chat
        .get_conversation(&conv, &fx.user_id.to_string(), fx.tenant_id)
        .await
        .unwrap()
        .unwrap();

    let (events, _guard) = capture_notify();
    let user_id = fx.user_id.to_string();
    let posted = post_agent_welcome(
        repos,
        &fx.resources.mcp.messaging_strings_registry,
        WelcomeTarget {
            conversation: &conversation,
            user_id: &user_id,
            conversation_tenant_id: fx.tenant_id,
            agent_id: &fx.agent_id,
            agent_tenant_id: fx.tenant_id,
            locale: &fx.locale,
            surface: SurfaceId::Telegram,
        },
    )
    .await
    .unwrap()
    .expect("the agent welcomes the thread");

    assert!(
        posted.channel_text.contains(SAMPLES[0]),
        "the starters ride the text"
    );
    assert!(named(&events, "suggestion.shown").is_empty());
}
