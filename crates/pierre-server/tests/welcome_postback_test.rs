// ABOUTME: A tapped welcome starter reaches the turn as its words or its command — resolved on the server, counted as a tap
// ABOUTME: Pins ex: and uc: postbacks, shown/tapped events and exposures, the origin on question_asked, and stale taps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#828.
//!
//! A welcome's starter used to post its own question as its postback, so a
//! tap arrived as the same text as typing it: nothing could say whether
//! suggestions are used at all. A starter now posts `ex:<slot>:<index>`, the
//! turn resolves it back into the question before anything reads the text,
//! and the tap is counted — `suggestion.shown` when the welcome lands,
//! `suggestion.tapped` and `origin=use_case` when it is pressed.
//!
//! Catalogue starters ranked from the athlete's state take the slots ahead of
//! the examples: `uc:<slot>:<id>` resolves to the starter's prompt or its
//! slash command, and the athlete's taps and ignored showings retire starters.
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
use pierre_core::models::{TenantId, AGENT_WELCOME_FINISH_REASON, COMMAND_FINISH_REASON};
use pierre_database::seed_models::SeedAgent;
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

/// The postbacks the welcome of `conv` offers, slot by slot.
async fn offered(fx: &Fixture, conv: &str) -> Vec<String> {
    messages(fx, conv)
        .await
        .iter()
        .find(|m| m.finish_reason.as_deref() == Some(AGENT_WELCOME_FINISH_REASON))
        .expect("the agent opens the thread")
        .actions
        .as_ref()
        .expect("the welcome carries its starters")
        .actions
        .iter()
        .map(|a| a.value.clone())
        .collect()
}

/// The catalogue ids among `postbacks`, sorted: a thread rotates them.
fn ranked(postbacks: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = postbacks
        .iter()
        .enumerate()
        .filter_map(|(slot, value)| {
            value
                .strip_prefix(&format!("uc:{slot}:"))
                .map(str::to_owned)
        })
        .collect();
    ids.sort();
    ids
}

fn string(fx: &Fixture, key: &str) -> String {
    fx.resources
        .mcp
        .messaging_strings_registry
        .get(key, &fx.locale)
}

fn origins(events: &CapturedEvents, name: &str) -> Vec<String> {
    named(events, name)
        .iter()
        .map(|e| e.field("origin").to_owned())
        .collect()
}

/// The defect and its fix in one thread: the welcome offers its starters as
/// postbacks — for an athlete with nothing connected, "Connect my watch" and
/// the nutrition starter that needs no data, then the agent's first example —
/// a tap reaches the transcript and the model as the question, and the tap is
/// counted as one.
#[tokio::test]
async fn a_tapped_starter_is_its_question_and_counts_as_a_tap() {
    let fx = setup("tap@test.com").await;
    let (events, _guard) = capture_notify();
    let conv = create_conversation(&fx, Some(&fx.agent_id)).await;

    let values = offered(&fx, &conv).await;
    assert_eq!(ranked(&values), ["connect", "fuel"]);
    assert_eq!(values[2], "ex:2:0", "the example fills the slot left");

    let shown = named(&events, "suggestion.shown");
    assert_eq!(shown.len(), 3, "one shown event per starter");
    for (slot, event) in shown.iter().enumerate() {
        assert_eq!(event.field("position"), slot.to_string());
        assert_eq!(event.field("surface"), "agent_welcome");
        assert_eq!(event.field("channel"), "web_chat");
        let expected_source = if slot < 2 { "state" } else { "static" };
        assert_eq!(event.field("source"), expected_source);
    }
    assert_eq!(shown[2].field("use_case"), "ex:0");

    let before = fx.provider.calls_so_far();
    let turn = send(&fx, &conv, &json!({ "content": "ex:2:0" })).await;
    assert_eq!(turn.user_message.content, SAMPLES[0]);

    let rows = messages(&fx, &conv).await;
    assert!(
        rows.iter()
            .any(|m| m.role == "user" && m.content == SAMPLES[0]),
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
            .any(|r| r.conversation.contains(SAMPLES[0])),
        "the model answers the question, not the postback"
    );

    let tapped = only(&events, "suggestion.tapped");
    assert_eq!(tapped.field("use_case"), "ex:0");
    assert_eq!(tapped.field("position"), "2");
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

/// A ranked prompt starter sends its prompt, in the athlete's language, as
/// the athlete's own words; the tap retires a `once` starter, so the next
/// welcome offers something else.
#[tokio::test]
async fn a_prompt_starter_is_its_prompt_and_one_tap_retires_it() {
    let fx = setup("uc-prompt@test.com").await;
    let (events, _guard) = capture_notify();
    let conv = create_conversation(&fx, Some(&fx.agent_id)).await;
    let slot = offered(&fx, &conv)
        .await
        .iter()
        .position(|v| v.ends_with(":fuel"))
        .expect("fuel is offered to an athlete with no data");
    let prompt = string(&fx, "use_cases.fuel.prompt");

    let before = fx.provider.calls_so_far();
    let turn = send(&fx, &conv, &json!({ "content": format!("uc:{slot}:fuel") })).await;

    assert_eq!(turn.user_message.content, prompt);
    assert!(
        fx.provider
            .requests_since(before)
            .iter()
            .any(|r| r.conversation.contains(&prompt)),
        "the model answers the prompt, not the postback"
    );
    let tapped = only(&events, "suggestion.tapped");
    assert_eq!(tapped.field("use_case"), "fuel");
    assert_eq!(tapped.field("source"), "state");
    assert_eq!(tapped.field("position"), slot.to_string());
    assert_eq!(origins(&events, "chat.question_asked"), ["use_case"]);

    let exposures = fx
        .resources
        .common
        .repos
        .use_case_exposures
        .use_case_exposures(fx.tenant_id, fx.user_id)
        .await
        .unwrap();
    let fuel = exposures.iter().find(|e| e.use_case_id == "fuel").unwrap();
    assert!(fuel.tapped_at.is_some(), "the tap is remembered");
    assert_eq!(fuel.shown_count, 0, "a tap restarts the count");
    let connect = exposures
        .iter()
        .find(|e| e.use_case_id == "connect")
        .unwrap();
    assert_eq!(connect.shown_count, 1, "the welcome counted what it showed");

    let next = create_conversation(&fx, Some(&fx.agent_id)).await;
    assert!(
        !offered(&fx, &next)
            .await
            .iter()
            .any(|v| v.ends_with(":fuel")),
        "a once-only starter is not offered after its tap"
    );
}

/// A ranked command starter runs its command, which the athlete's bubble
/// shows — teaching it — and an id the catalogue no longer lists is a stale
/// suggestion like any other.
#[tokio::test]
async fn a_command_starter_runs_its_command_and_an_unknown_one_is_told() {
    let fx = setup("uc-command@test.com").await;
    let (events, _guard) = capture_notify();
    let conv = create_conversation(&fx, None).await;

    let before = fx.provider.calls_so_far();
    let turn = send(&fx, &conv, &json!({ "content": "uc:0:can_see" })).await;
    assert_eq!(turn.user_message.content, "/status");
    assert_eq!(
        turn.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON),
        "the command answers"
    );
    assert_eq!(fx.provider.calls_so_far(), before, "no model ran");
    let tapped = only(&events, "suggestion.tapped");
    assert_eq!(tapped.field("use_case"), "can_see");
    assert_eq!(tapped.field("source"), "state");

    let turn = send(&fx, &conv, &json!({ "content": "uc:1:no_such_starter" })).await;
    assert_eq!(
        turn.assistant.message.content,
        string(&fx, KEY_USE_CASES_UNAVAILABLE)
    );
    assert_eq!(turn.user_message.content, "");
    assert_eq!(named(&events, "suggestion.tapped").len(), 1);
}

/// A starter shown in three welcomes without a tap is not offered a fourth
/// time: the next setup starter takes its slot.
#[tokio::test]
async fn three_untapped_showings_retire_a_starter() {
    let fx = setup("uc-ignored@test.com").await;
    for _ in 0..3 {
        let conv = create_conversation(&fx, Some(&fx.agent_id)).await;
        assert_eq!(ranked(&offered(&fx, &conv).await), ["connect", "fuel"]);
    }

    let fourth = create_conversation(&fx, Some(&fx.agent_id)).await;
    let values = offered(&fx, &fourth).await;
    assert_eq!(
        ranked(&values),
        ["about_me"],
        "connect and fuel were ignored three times; the next setup starter leads"
    );
    assert_eq!(&values[1..], ["ex:1:0", "ex:2:1"]);
}

/// An agent that needs activity data keeps its examples from an athlete with
/// nothing connected: "Analyse my last ride" is never offered to them.
#[tokio::test]
async fn examples_wait_for_the_agents_prerequisites() {
    let fx = setup("uc-prereq@test.com").await;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    fx.resources
        .common
        .repos
        .seeder
        .seed_insert_agent(&SeedAgent {
            id: id.clone(),
            user_id: fx.user_id,
            tenant_id: fx.tenant_id,
            title: "Ride Analyst".to_owned(),
            description: "Reads your rides.".to_owned(),
            system_prompt: "You analyse rides.".to_owned(),
            category: "analysis".to_owned(),
            tags_json: "[]".to_owned(),
            sample_prompts_json: r#"["Analyse my last ride"]"#.to_owned(),
            token_count: 10,
            visibility: "tenant".to_owned(),
            slug: "ride-analyst".to_owned(),
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            prerequisites_json: r#"{"providers":["strava"]}"#.to_owned(),
            source_file: None,
            content_hash: None,
            startup_query: None,
            data_requirements: None,
            visuals: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    let conv = create_conversation(&fx, Some(&id)).await;
    let values = offered(&fx, &conv).await;
    assert_eq!(ranked(&values), ["connect"]);
    assert!(
        values.iter().all(|v| !v.starts_with("ex:")),
        "no example of an agent the athlete cannot use yet: {values:?}"
    );
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
    assert!(
        posted
            .channel_text
            .contains(&string(&fx, "use_cases.connect.prompt")),
        "a prompt starter is listed as its prompt"
    );
    assert!(
        !posted.channel_text.contains("uc:") && !posted.channel_text.contains("ex:"),
        "no postback leaks into the text: {}",
        posted.channel_text
    );
    assert!(named(&events, "suggestion.shown").is_empty());
    assert!(
        fx.resources
            .common
            .repos
            .use_case_exposures
            .use_case_exposures(fx.tenant_id, fx.user_id)
            .await
            .unwrap()
            .is_empty(),
        "nothing was shown that could be tapped"
    );
}
