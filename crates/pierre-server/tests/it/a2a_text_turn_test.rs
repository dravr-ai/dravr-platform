// ABOUTME: An A2A text message runs one coaching turn, under the A2A transport, the token's grant and the athlete's caps
// ABOUTME: Drives the real A2A router and pipeline with a scripted model, reading what the model was actually sent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1368: the coaching text skill.
//!
//! Provider terms come first. The first-party chat pipeline can read what
//! Nolio, WHOOP and the rest keep on Dravr's own surfaces, and a text turn
//! answers an external agent, so the test that matters reads every request the
//! model received. Each leak test holds a control: the same seed through a
//! first-party turn reaches the model, so a seed that never reached it cannot
//! make the A2A assertion pass.

#![cfg(all(feature = "client-chat", feature = "client-messaging"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use futures_util::stream;
use pierre_a2a::coaching::CoachingTurns;
use pierre_a2a::ClientRegistrationRequest;
use pierre_auth::auth::DelegatedGrant;
use pierre_chat_pipeline::{
    execute, CommandPersistence, InputSource, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_config::constants::usage_quotas::DEFAULT_MAX_ACTIVE_CONVERSATIONS;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_core::models::a2a::user_task_key;
use pierre_core::models::{AddMessageParams, ConversationTurnId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::USER_EXPORT_TABLES;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::a2a_coaching::PipelineCoachingTurns;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::ProviderBundle;
use pierre_routes_a2a::{A2ARoutes, A2ARoutesState};
use pierre_runtime_context::default_admin_config;
use pierre_services::usage_counter::UsageCounterService;
use pierre_test_support::account::delete_account;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::helpers::recording_llm::{RecordedRequest, RecordingProvider};
use crate::transport_gate_e2e_test::{
    athlete_with_relay, relay_factory, sessions, write_fact, RelayDescriptor, Rider, RELAY,
};

/// Long enough, and plain enough, that the local detector settles the language
/// and the model's first call is the turn's own.
const QUESTION: &str = "Looking back at the whole of this past week of my training, how did my \
                        rides go and what should I change for the next block of work?";
/// The coach's scripted answer: nothing a verifier would hold it to.
const ANSWER: &str = "Keep the easy days easy and add one longer ride next week.";
/// The name of an activity only the first-party relay holds.
const RELAY_ACTIVITY: &str = "Garmin Hills";
/// A fact stamped as derived from first-party-only data.
const WITHHELD_FACT: &str = "prefers-the-withheld-sentinel-climb";
/// A fact derived from nothing first-party-only.
const OPEN_FACT: &str = "prefers-the-open-sentinel-climb";
/// A tool call for the relay's activities, as the text tool loop reads one.
const READ_RELAY: &str = r#"Let me look. <tool_call>{"name":"get_activities","arguments":{"provider":"nolio","limit":10,"mode":"summary"}}</tool_call>"#;
/// A write the default grant does not cover.
const SET_GOAL: &str = r#"Saving it. <tool_call>{"name":"set_goal","arguments":{"goal_type":"distance","target_value":40,"timeframe":"week"}}</tool_call>"#;
/// A coach-family tool, which D4 keeps off every external transport.
const READ_ROSTER: &str =
    r#"Checking. <tool_call>{"name":"get_roster_overview","arguments":{}}</tool_call>"#;

/// The grant an agent holds when it registered asking for nothing.
const READ_ONLY: &[&str] = &["fitness:read", "profile:read"];
/// A grant that also writes fitness data.
const READ_WRITE: &[&str] = &["fitness:read", "fitness:write", "profile:read"];

const PATIENCE: Duration = Duration::from_secs(10);
const SETTLE: Duration = Duration::from_millis(500);

struct Fixture {
    resources: Arc<ServerContext>,
    model: Arc<RecordingProvider>,
    coaching: Arc<PipelineCoachingTurns>,
    app: Router,
}

fn router(resources: &Arc<ServerContext>, coaching: &Arc<PipelineCoachingTurns>) -> Router {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let seam: Arc<dyn CoachingTurns> = coaching.clone();
    A2ARoutes::routes(A2ARoutesState {
        ctx: resources.clone(),
        client_manager: resources.a2a.a2a_client_manager.clone(),
        auth_middleware: resources.auth.auth_middleware.clone(),
        tool_runtime,
        coaching: Some(seam),
    })
}

/// The server, with `model` answering and the first-party relay registered.
async fn server_over(model: Arc<dyn LlmProvider>) -> Arc<ServerContext> {
    common::init_server_config();
    common::init_test_http_clients();
    let base = common::create_test_server_resources_with_chat_provider(model)
        .await
        .unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(RelayDescriptor),
        relay_factory,
    ));
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    Arc::new(context)
}

async fn fixture() -> Fixture {
    let model = Arc::new(RecordingProvider::answering(ANSWER));
    let resources = server_over(model.clone()).await;
    let coaching = Arc::new(PipelineCoachingTurns::new(resources.clone()));
    let app = router(&resources, &coaching);
    Fixture {
        resources,
        model,
        coaching,
        app,
    }
}

/// A delegated grant holding exactly `scopes`, as the OAuth server mints one.
fn grant(resources: &ServerContext, rider: &Rider, scopes: &[&str]) -> String {
    let scopes: Vec<String> = scopes.iter().map(|s| (*s).to_owned()).collect();
    let token = resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: "copilot_studio_test",
                scopes: &scopes,
                audience: None,
            },
            &rider.user_id,
            &[],
            Some(rider.tenant.to_string()),
        )
        .unwrap();
    format!("Bearer {token}")
}

async fn rpc_with_status(
    app: &Router,
    authorization: &str,
    method: &str,
    params: Value,
) -> (StatusCode, Option<String>, Value) {
    let request = Request::post("/a2a/jsonrpc")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("A2A-Version", "1.0")
        .header("authorization", authorization)
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, retry_after, serde_json::from_slice(&bytes).unwrap())
}

async fn rpc(app: &Router, authorization: &str, method: &str, params: Value) -> Value {
    rpc_with_status(app, authorization, method, params).await.2
}

fn text_message(text: &str) -> Value {
    json!({"message": {
        "messageId": Uuid::new_v4().to_string(),
        "role": "ROLE_USER",
        "parts": [{"text": text}],
    }})
}

fn text_message_in(context_id: &str, text: &str) -> Value {
    let mut params = text_message(text);
    params["message"]["contextId"] = json!(context_id);
    params
}

fn task_reply(task: &Value) -> &str {
    task["status"]["message"]["parts"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no reply text on {task}"))
}

/// Everything every model call was sent, system prompts and tool results alike.
fn everything_sent(model: &RecordingProvider) -> String {
    model
        .requests_since(0)
        .into_iter()
        .map(|request| format!("{}\n{}", request.system, request.conversation))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the first-party control turn feeds the model: a web turn over the same
/// server and the same seed.
async fn first_party_turn(fx: &Fixture, rider: &Rider) {
    let conversation = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &rider.user_id.to_string(),
            rider.tenant,
            "Control",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap();
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    });
    let ctx = fx.resources.chat_pipeline_context();
    let served = execute(
        &ctx,
        TurnRequest {
            conversation_id: conversation.id,
            user_id: rider.user_id,
            conversation_tenant_id: rider.tenant,
            tool_tenant_id: rider.tenant,
            content: QUESTION.to_owned(),
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quote: None,
            channel_type: "web",
            is_direct_message: true,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            transport: Transport::WebApp,
            grant: OAuthScope::self_grant(),
            hooks: PipelineHooks::none(),
        },
        &profile,
    )
    .await
    .unwrap();
    assert!(matches!(served, ServedTurn::Pipeline(_)));
}

/// The athlete's counted messages today, once the turn's accounting settles.
async fn daily_messages(resources: &ServerContext, rider: &Rider) -> i64 {
    let counters = resources.common.repos.usage_counters.clone();
    let service = UsageCounterService::new(counters.as_ref(), default_admin_config());
    let mut latest = 0;
    for _ in 0..50 {
        latest = service
            .get_current(
                &rider.tenant.to_string(),
                &rider.user_id.to_string(),
                "daily_messages",
            )
            .await
            .unwrap();
        if latest > 0 {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    latest
}

/// The athlete's rows of `table`, as the export reads them.
async fn exported_rows(resources: &ServerContext, rider: &Rider, table: &str) -> Vec<Value> {
    let table = USER_EXPORT_TABLES
        .iter()
        .find(|candidate| candidate.name == table)
        .expect("a table the export reads");
    resources
        .common
        .repos
        .user_export
        .export_page(table, rider.user_id, "")
        .await
        .unwrap()
        .rows
}

/// Seed what a model must not see over A2A, and what it may.
async fn seed_withheld(fx: &Fixture, rider: &Rider) {
    let activities = sessions();
    fx.resources
        .common
        .repos
        .activity_cache
        .upsert_activities(rider.user_id, &rider.tenant, RELAY, &activities)
        .await
        .unwrap();
    write_fact(
        &fx.resources,
        rider,
        WITHHELD_FACT,
        TransportPolicy::FirstPartyOnly,
    )
    .await;
    write_fact(
        &fx.resources,
        rider,
        OPEN_FACT,
        TransportPolicy::AnyTransport,
    )
    .await;
}

// ============================================================================
// Requirement 1: provider terms
// ============================================================================

#[tokio::test]
async fn a_text_turn_feeds_the_model_nothing_a_providers_terms_keep_first_party() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    seed_withheld(&fx, &rider).await;

    fx.model.answer_next_with(READ_RELAY);
    let authorization = grant(&fx.resources, &rider, READ_ONLY);
    let sent = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    let task = &sent["result"]["task"];
    assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED", "{sent}");
    assert_eq!(task_reply(task), ANSWER);

    let over_a2a = everything_sent(&fx.model);
    assert!(
        !over_a2a.contains(RELAY_ACTIVITY),
        "the relay's activity reached the model over A2A"
    );
    assert!(
        !over_a2a.contains(WITHHELD_FACT),
        "a fact derived from first-party-only data reached the model over A2A"
    );
    assert!(
        over_a2a.contains(OPEN_FACT),
        "the prompt's facts must still run: an empty memory proves nothing"
    );
    assert!(
        over_a2a.contains(QUESTION),
        "the turn must really have reached the model"
    );

    // The control: the same seed and the same script reach the model on the
    // athlete's own surface, so the assertions above are the transport's doing.
    let before = fx.model.calls_so_far();
    fx.model.answer_next_with(READ_RELAY);
    first_party_turn(&fx, &rider).await;
    let own_surface = fx
        .model
        .requests_since(before)
        .into_iter()
        .map(|request| format!("{}\n{}", request.system, request.conversation))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        own_surface.contains(RELAY_ACTIVITY),
        "control: a first-party turn reads the relay's activity"
    );
    assert!(
        own_surface.contains(WITHHELD_FACT),
        "control: a first-party turn reads the stamped fact"
    );
}

#[tokio::test]
async fn a_follow_up_replays_nothing_a_first_party_row_derived_from() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let first = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    let context_id = first["result"]["task"]["contextId"]
        .as_str()
        .unwrap()
        .to_owned();

    // A row in the thread stamped first-party-only, as a platform post that read
    // the relay would have stamped it.
    let stamped = "a-first-party-derived-row-in-the-thread";
    fx.resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: rider.tenant,
            conversation_id: &context_id,
            user_id: &rider.user_id.to_string(),
            role: "assistant",
            content: stamped,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::FirstPartyOnly,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap();

    let before = fx.model.calls_so_far();
    let follow_up = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message_in(
            &context_id,
            "And what about tomorrow, given all of that training?",
        ),
    )
    .await;
    assert_eq!(
        follow_up["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED",
        "{follow_up}"
    );
    let sent: String = fx
        .model
        .requests_since(before)
        .into_iter()
        .map(|request| request.conversation)
        .collect();
    assert!(
        !sent.contains(stamped),
        "a first-party row replayed over A2A"
    );
}

// ============================================================================
// Requirement 2: scope
// ============================================================================

#[tokio::test]
async fn a_read_only_grant_is_offered_no_write_and_refused_one_it_names() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;

    fx.model.answer_next_with(SET_GOAL);
    let authorization = grant(&fx.resources, &rider, READ_ONLY);
    let sent = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    assert_eq!(
        sent["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED",
        "{sent}"
    );
    let requests = fx.model.requests_since(0);
    assert!(
        !requests[0].system.contains("set_goal"),
        "a read-only turn's tool index must not name a write"
    );
    let tool_round = requests.get(1).expect("the tool round's follow-up call");
    assert!(
        tool_round.conversation.contains("fitness:write"),
        "the executor must refuse the write naming the missing scope: {}",
        tool_round.conversation
    );

    // The control: the same turn under a grant that writes offers it.
    let before = fx.model.calls_so_far();
    let writer = grant(&fx.resources, &rider, READ_WRITE);
    rpc(&fx.app, &writer, "SendMessage", text_message(QUESTION)).await;
    assert!(
        fx.model.requests_since(before)[0]
            .system
            .contains("set_goal"),
        "control: a grant that writes is offered the write"
    );
}

#[tokio::test]
async fn a_grant_missing_a_read_the_prompt_needs_is_refused_before_a_task_opens() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, &["fitness:read"]);

    let refused = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    let message = refused["error"]["message"].as_str().expect("a refusal");
    assert!(message.contains("profile:read"), "{message}");
    assert_eq!(
        fx.model.calls_so_far(),
        0,
        "no model call for a refused grant"
    );
    let listed = rpc(&fx.app, &authorization, "ListTasks", json!({})).await;
    assert_eq!(
        listed["result"]["tasks"],
        json!([]),
        "no task for a refusal"
    );
}

#[tokio::test]
async fn a_client_credentials_token_acts_for_no_athlete_and_is_refused() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let credentials = fx
        .resources
        .a2a
        .a2a_client_manager
        .register_client(
            ClientRegistrationRequest {
                name: "b2b".into(),
                description: "a machine client".into(),
                capabilities: vec!["fitness-data-analysis".into()],
                redirect_uris: vec![],
                contact_email: "b2b@example.com".to_owned(),
            },
            rider.user_id,
        )
        .await
        .unwrap();
    let token = fx
        .resources
        .auth
        .auth_manager
        .generate_client_credentials_token(
            &fx.resources.auth.jwks_manager,
            &credentials.client_id,
            &["fitness:read".to_owned(), "profile:read".to_owned()],
            None,
            None,
        )
        .unwrap();

    let refused = rpc(
        &fx.app,
        &format!("Bearer {token}"),
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    let message = refused["error"]["message"].as_str().expect("a refusal");
    assert!(message.contains("acts for no athlete"), "{message}");
    assert_eq!(fx.model.calls_so_far(), 0);

    // And the client they registered cannot read what the coach told them.
    let athlete = grant(&fx.resources, &rider, READ_ONLY);
    let sent = rpc(&fx.app, &athlete, "SendMessage", text_message(QUESTION)).await;
    let id = sent["result"]["task"]["id"]
        .as_str()
        .expect("the athlete's task");
    let read = rpc(
        &fx.app,
        &format!("Bearer {token}"),
        "GetTask",
        json!({"id": id}),
    )
    .await;
    assert_eq!(
        read["error"]["data"][0]["reason"], "TASK_NOT_FOUND",
        "{read}"
    );
    let own = rpc(&fx.app, &athlete, "GetTask", json!({"id": id})).await;
    assert_eq!(task_reply(&own["result"]), ANSWER);
}

#[tokio::test]
async fn a_read_only_grant_teaches_the_athletes_memory_nothing() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let extraction = fx
        .resources
        .chat_pipeline_context()
        .memory_extraction_prompt;
    let is_extraction = |request: &RecordedRequest| request.system.starts_with(&extraction);

    let reader = grant(&fx.resources, &rider, READ_ONLY);
    rpc(&fx.app, &reader, "SendMessage", text_message(QUESTION)).await;
    sleep(SETTLE).await;
    assert!(
        !fx.model.requests_since(0).iter().any(is_extraction),
        "a third party's read-only grant must not write to the athlete's memory"
    );

    // The control: a grant that writes learns as the athlete's own turn does.
    let before = fx.model.calls_so_far();
    let writer = grant(&fx.resources, &rider, READ_WRITE);
    rpc(&fx.app, &writer, "SendMessage", text_message(QUESTION)).await;
    assert!(
        fx.model
            .request_matching(before, PATIENCE, is_extraction)
            .await
            .is_some(),
        "control: a grant that writes runs the extraction"
    );
}

#[tokio::test]
async fn the_coach_family_stays_off_a_text_turn() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;

    fx.model.answer_next_with(READ_ROSTER);
    let authorization = grant(&fx.resources, &rider, READ_WRITE);
    rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    let requests = fx.model.requests_since(0);
    assert!(
        !requests[0].system.contains("get_roster_overview"),
        "D4: the turn's tool index must not name a coach tool"
    );
    assert!(
        requests[1]
            .conversation
            .contains("not available over this interface"),
        "D4: the executor must refuse the coach tool: {}",
        requests[1].conversation
    );
}

// ============================================================================
// Requirement 3: cost and capacity
// ============================================================================

#[tokio::test]
async fn no_free_slot_refuses_at_once_with_a_retryable_error() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let held = fx.coaching.try_reserve().expect("the instance's one slot");
    let (status, retry_after, refused) = timeout(
        Duration::from_secs(5),
        rpc_with_status(
            &fx.app,
            &authorization,
            "SendMessage",
            text_message(QUESTION),
        ),
    )
    .await
    .expect("a refusal never waits for a slot");
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{refused}");
    assert_eq!(retry_after.as_deref(), Some("30"));
    assert_eq!(fx.model.calls_so_far(), 0, "no spend on a refusal");
    let listed = rpc(&fx.app, &authorization, "ListTasks", json!({})).await;
    assert_eq!(
        listed["result"]["tasks"],
        json!([]),
        "no task for a refusal"
    );

    // The slot comes back and the same request runs.
    drop(held);
    let served = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    assert_eq!(
        served["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED",
        "{served}"
    );
    assert!(
        fx.coaching.try_reserve().is_some(),
        "a finished turn gives its slot back"
    );
}

#[tokio::test]
async fn a_text_turn_spends_the_athletes_own_daily_message_count() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    assert_eq!(
        daily_messages(&fx.resources, &rider).await,
        1,
        "one turn, one message on the athlete's counter"
    );
}

#[tokio::test]
async fn an_athlete_past_their_cap_is_refused_before_a_task_opens_or_a_model_runs() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    // Far past any tier's hard limit, burst included.
    let counters = fx.resources.common.repos.usage_counters.clone();
    UsageCounterService::new(counters.as_ref(), default_admin_config())
        .increment(
            &rider.tenant.to_string(),
            &rider.user_id.to_string(),
            "daily_messages",
            1_000_000,
        )
        .await
        .unwrap();

    let (status, retry_after, refused) = rpc_with_status(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{refused}");
    assert!(retry_after.is_some(), "a spent cap says when to come back");
    assert_eq!(
        fx.model.calls_so_far(),
        0,
        "a public caller spends no model call"
    );
    let listed = rpc(&fx.app, &authorization, "ListTasks", json!({})).await;
    assert_eq!(
        listed["result"]["tasks"],
        json!([]),
        "no task for a refusal"
    );
    assert!(
        fx.coaching.try_reserve().is_some(),
        "a refused turn gives its slot back"
    );
}

// ============================================================================
// Requirements 4 and 6: the real pipeline, and how a turn ends
// ============================================================================

/// A model that holds every call until told to answer.
struct GatedModel {
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
}

impl GatedModel {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
        }
    }
}

#[async_trait]
impl LlmProvider for GatedModel {
    fn name(&self) -> &'static str {
        "gated_mock"
    }
    fn display_name(&self) -> &'static str {
        "Gated Mock LLM"
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
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(ChatResponse {
            content: ANSWER.to_owned(),
            model: "mock-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let response = self.complete(request).await?;
        Ok(Box::pin(stream::iter(vec![Ok(StreamChunk {
            delta: response.content,
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        })])))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

async fn gated_fixture() -> (Fixture, Arc<GatedModel>) {
    let gated = Arc::new(GatedModel::new());
    let resources = server_over(gated.clone()).await;
    let coaching = Arc::new(PipelineCoachingTurns::new(resources.clone()));
    let app = router(&resources, &coaching);
    (
        Fixture {
            resources,
            model: Arc::new(RecordingProvider::answering(ANSWER)),
            coaching,
            app,
        },
        gated,
    )
}

/// The task, once it reaches `state`.
async fn wait_for_state(app: &Router, authorization: &str, id: &str, state: &str) -> Value {
    for _ in 0..200 {
        let got = rpc(app, authorization, "GetTask", json!({"id": id})).await;
        if got["result"]["status"]["state"] == state {
            return got["result"].clone();
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("task {id} never reached {state}");
}

#[tokio::test]
async fn return_immediately_answers_with_the_task_and_get_task_reports_the_reply() {
    let (fx, gated) = gated_fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let mut params = text_message(QUESTION);
    params["configuration"] = json!({"returnImmediately": true});
    let sent = rpc(&fx.app, &authorization, "SendMessage", params).await;
    let id = sent["result"]["task"]["id"]
        .as_str()
        .expect("a task")
        .to_owned();

    gated.entered.notified().await;
    let running = rpc(&fx.app, &authorization, "GetTask", json!({"id": id})).await;
    assert_eq!(running["result"]["status"]["state"], "TASK_STATE_WORKING");

    gated.release.notify_one();
    let done = wait_for_state(&fx.app, &authorization, &id, "TASK_STATE_COMPLETED").await;
    assert_eq!(task_reply(&done), ANSWER);
    assert_eq!(done["artifacts"][0]["parts"][0]["text"], ANSWER);
}

#[tokio::test]
async fn a_client_that_drops_a_blocking_call_loses_neither_the_turn_nor_the_reply() {
    let (fx, gated) = gated_fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let blocking = {
        let (app, authorization) = (fx.app.clone(), authorization.clone());
        tokio::spawn(async move {
            rpc(&app, &authorization, "SendMessage", text_message(QUESTION)).await;
        })
    };
    gated.entered.notified().await;
    blocking.abort();
    assert!(blocking.await.unwrap_err().is_cancelled());

    gated.release.notify_one();
    let listed = rpc(&fx.app, &authorization, "ListTasks", json!({})).await;
    let id = listed["result"]["tasks"][0]["id"]
        .as_str()
        .expect("the task")
        .to_owned();
    let done = wait_for_state(&fx.app, &authorization, &id, "TASK_STATE_COMPLETED").await;
    assert_eq!(task_reply(&done), ANSWER);
}

#[tokio::test]
async fn cancel_stops_the_turn_gives_its_slot_back_and_stands() {
    let (fx, gated) = gated_fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let mut params = text_message(QUESTION);
    params["configuration"] = json!({"returnImmediately": true});
    let sent = rpc(&fx.app, &authorization, "SendMessage", params).await;
    let id = sent["result"]["task"]["id"].as_str().unwrap().to_owned();
    gated.entered.notified().await;
    assert!(
        fx.coaching.try_reserve().is_none(),
        "the turn holds the slot"
    );

    rpc(&fx.app, &authorization, "CancelTask", json!({"id": id})).await;

    for _ in 0..200 {
        if fx.coaching.try_reserve().is_some() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        fx.coaching.try_reserve().is_some(),
        "a canceled turn stops and gives its slot back"
    );
    let after = rpc(&fx.app, &authorization, "GetTask", json!({"id": id})).await;
    assert_eq!(after["result"]["status"]["state"], "TASK_STATE_CANCELED");
}

#[tokio::test]
async fn a_message_naming_a_running_task_does_not_end_it() {
    let (fx, gated) = gated_fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let mut params = text_message(QUESTION);
    params["configuration"] = json!({"returnImmediately": true});
    let sent = rpc(&fx.app, &authorization, "SendMessage", params).await;
    let id = sent["result"]["task"]["id"].as_str().unwrap().to_owned();
    gated.entered.notified().await;

    let mut second = text_message("And what about tomorrow then?");
    second["message"]["taskId"] = json!(id);
    let refused = rpc(&fx.app, &authorization, "SendMessage", second).await;
    assert!(refused["error"].is_object(), "{refused}");
    let running = rpc(&fx.app, &authorization, "GetTask", json!({"id": id})).await;
    assert_eq!(running["result"]["status"]["state"], "TASK_STATE_WORKING");

    gated.release.notify_one();
    wait_for_state(&fx.app, &authorization, &id, "TASK_STATE_COMPLETED").await;
}

/// A model whose every call fails, naming its internals.
struct BrokenModel;

const MODEL_SECRET: &str = "upstream-copilot-tier-secret-detail";

#[async_trait]
impl LlmProvider for BrokenModel {
    fn name(&self) -> &'static str {
        "broken_mock"
    }
    fn display_name(&self) -> &'static str {
        "Broken Mock LLM"
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
        Err(AppError::internal(MODEL_SECRET))
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::internal(MODEL_SECRET))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

#[tokio::test]
async fn a_failed_turn_fails_its_task_saying_nothing_of_why_and_closes_the_question() {
    let resources = server_over(Arc::new(BrokenModel)).await;
    let coaching = Arc::new(PipelineCoachingTurns::new(resources.clone()));
    let app = router(&resources, &coaching);
    let rider = athlete_with_relay(&resources).await;
    let authorization = grant(&resources, &rider, READ_ONLY);

    let sent = rpc(&app, &authorization, "SendMessage", text_message(QUESTION)).await;
    let task = &sent["result"]["task"];
    assert_eq!(task["status"]["state"], "TASK_STATE_FAILED", "{sent}");
    let wire = sent.to_string();
    assert!(!wire.contains(MODEL_SECRET), "{wire}");
    assert!(!wire.contains("broken_mock"), "{wire}");

    let rows = resources
        .common
        .repos
        .chat
        .get_messages(
            task["contextId"].as_str().unwrap(),
            &rider.user_id.to_string(),
            rider.tenant,
        )
        .await
        .unwrap();
    assert_eq!(
        rows.last().map(|row| row.role.as_str()),
        Some("assistant"),
        "the question must not stand open on the athlete's thread"
    );
}

// ============================================================================
// Requirement 5: context
// ============================================================================

#[tokio::test]
async fn a_follow_up_continues_the_conversation_its_context_names() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let first = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    let context_id = first["result"]["task"]["contextId"]
        .as_str()
        .unwrap()
        .to_owned();
    let conversation = fx
        .resources
        .common
        .repos
        .chat
        .get_conversation(&context_id, &rider.user_id.to_string(), rider.tenant)
        .await
        .unwrap()
        .expect("the contextId is the athlete's conversation");
    assert_eq!(conversation.channel_type, "a2a");

    let before = fx.model.calls_so_far();
    let second = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message_in(
            &context_id,
            "Tell me more about the longer ride you mentioned here.",
        ),
    )
    .await;
    assert_eq!(second["result"]["task"]["contextId"], context_id.as_str());
    assert!(
        fx.model.requests_since(before)[0]
            .conversation
            .contains(QUESTION),
        "a follow-up reads the turns before it"
    );
}

#[tokio::test]
async fn a_context_id_that_cannot_be_theirs_is_refused_and_reads_alike() {
    let fx = fixture().await;
    let owner = athlete_with_relay(&fx.resources).await;
    let stranger = athlete_with_relay(&fx.resources).await;
    let owner_grant = grant(&fx.resources, &owner, READ_ONLY);
    let stranger_grant = grant(&fx.resources, &stranger, READ_ONLY);

    let first = rpc(&fx.app, &owner_grant, "SendMessage", text_message(QUESTION)).await;
    let owner_context = first["result"]["task"]["contextId"]
        .as_str()
        .unwrap()
        .to_owned();

    // Another athlete's conversation.
    let before = fx.model.calls_so_far();
    let foreign = rpc(
        &fx.app,
        &stranger_grant,
        "SendMessage",
        text_message_in(&owner_context, QUESTION),
    )
    .await;
    let foreign_refusal = foreign["error"]["message"]
        .as_str()
        .expect("a refusal")
        .to_owned();
    assert_eq!(fx.model.calls_so_far(), before, "nothing ran on it");

    // The athlete's own thread outside this surface.
    let web = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &owner.user_id.to_string(),
            owner.tenant,
            "Own thread",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap();
    let own_thread = rpc(
        &fx.app,
        &owner_grant,
        "SendMessage",
        text_message_in(&web.id, QUESTION),
    )
    .await;
    assert_eq!(own_thread["error"]["message"], foreign_refusal.as_str());

    // No id at all.
    let malformed = rpc(
        &fx.app,
        &owner_grant,
        "SendMessage",
        text_message_in("not-an-id", QUESTION),
    )
    .await;
    assert_eq!(malformed["error"]["message"], foreign_refusal.as_str());
}

#[tokio::test]
async fn a_client_chosen_context_id_opens_a_conversation_it_then_continues() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);
    let chosen = Uuid::new_v4().to_string();

    let first = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message_in(&chosen, QUESTION),
    )
    .await;
    assert_eq!(
        first["result"]["task"]["contextId"],
        chosen.as_str(),
        "{first}"
    );
    let second = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message_in(&chosen, QUESTION),
    )
    .await;
    assert_eq!(
        second["result"]["task"]["contextId"],
        chosen.as_str(),
        "{second}"
    );
    let rows = fx
        .resources
        .common
        .repos
        .chat
        .get_messages(&chosen, &rider.user_id.to_string(), rider.tenant)
        .await
        .unwrap();
    assert_eq!(rows.iter().filter(|row| row.role == "user").count(), 2);
}

#[tokio::test]
async fn the_thread_is_in_the_athletes_export_and_gone_with_their_account() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let sent = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(QUESTION),
    )
    .await;
    let task = sent["result"]["task"].clone();
    let context_id = task["contextId"].as_str().unwrap().to_owned();
    let task_id = task["id"].as_str().unwrap().to_owned();
    let repos = fx.resources.common.repos.clone();

    let exported = exported_rows(&fx.resources, &rider, "chat_conversations").await;
    assert!(
        exported.iter().any(|row| row["id"] == context_id.as_str()),
        "the A2A thread must be in the export: {exported:?}"
    );

    // The task carries the reply too, keyed to no client: only the account
    // delete's own predicate reaches it.
    let stored = repos
        .a2a
        .get_task(&task_id)
        .await
        .unwrap()
        .expect("the task row");
    assert_eq!(stored.client_id, user_task_key(rider.user_id));

    delete_account(&repos, rider.user_id, None).await.unwrap();
    assert!(
        repos
            .chat
            .get_conversation(&context_id, &rider.user_id.to_string(), rider.tenant)
            .await
            .unwrap()
            .is_none(),
        "the thread goes with the account"
    );
    assert!(
        repos.a2a.get_task(&task_id).await.unwrap().is_none(),
        "the task holding the reply goes with the account"
    );
}

/// The task a text message opened, by its id, and the context it ran in.
async fn sent_task(app: &Router, authorization: &str, params: Value) -> (String, String) {
    let sent = rpc(app, authorization, "SendMessage", params).await;
    let task = &sent["result"]["task"];
    let id = task["id"].as_str().map(str::to_owned);
    let context = task["contextId"].as_str().map(str::to_owned);
    (
        id.unwrap_or_else(|| panic!("a task: {sent}")),
        context.unwrap_or_else(|| panic!("a context: {sent}")),
    )
}

/// The task row keeps its own copy of the reply, so a deleted thread that left
/// it behind would keep the words the athlete just asked to be rid of.
#[tokio::test]
async fn deleting_a_thread_purges_its_task_rows_and_only_those() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);
    let (deleted_task, deleted_thread) = sent_task(
        &fx.app,
        &authorization,
        text_message_in(&Uuid::new_v4().to_string(), QUESTION),
    )
    .await;
    let (kept_task, _) = sent_task(
        &fx.app,
        &authorization,
        text_message_in(&Uuid::new_v4().to_string(), QUESTION),
    )
    .await;

    // Another athlete's tool task may carry the same contextId: a tool message
    // names no thread, and a task has no tenant to scope it by.
    let stranger = athlete_with_relay(&fx.resources).await;
    let stranger_authorization = grant(&fx.resources, &stranger, READ_ONLY);
    let mut tool_message = text_message_in(&deleted_thread, "get my profile");
    tool_message["message"]["parts"] = json!([
        {"data": {"tool_name": "get_connection_status", "parameters": {}}}
    ]);
    let (strangers_task, strangers_context) =
        sent_task(&fx.app, &stranger_authorization, tool_message).await;
    assert_eq!(strangers_context, deleted_thread);

    let repos = fx.resources.common.repos.clone();
    let owner = rider.user_id.to_string();

    // A delete that takes no thread takes no task.
    assert!(!repos
        .chat
        .delete_conversation(&deleted_thread, &stranger.user_id.to_string(), rider.tenant)
        .await
        .unwrap());
    assert!(repos.a2a.get_task(&deleted_task).await.unwrap().is_some());

    assert!(repos
        .chat
        .delete_conversation(&deleted_thread, &owner, rider.tenant)
        .await
        .unwrap());

    assert!(
        repos.a2a.get_task(&deleted_task).await.unwrap().is_none(),
        "the task holding the reply goes with its thread"
    );
    let read = rpc(
        &fx.app,
        &authorization,
        "GetTask",
        json!({"id": deleted_task}),
    )
    .await;
    assert!(read["error"].is_object(), "{read}");
    assert!(
        repos.a2a.get_task(&kept_task).await.unwrap().is_some(),
        "another thread's task stays"
    );
    assert!(
        repos.a2a.get_task(&strangers_task).await.unwrap().is_some(),
        "another athlete's task under the same contextId stays"
    );
}

/// A task row is the athlete's words and an agent's answer, kept until they
/// delete it, so it answers an access request like the thread it runs in.
#[tokio::test]
async fn a_task_is_in_its_athletes_export_and_no_one_elses() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);
    let (task_id, _) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;
    let stranger = athlete_with_relay(&fx.resources).await;

    let rows = exported_rows(&fx.resources, &rider, "a2a_tasks").await;
    let row = rows
        .iter()
        .find(|row| row["task_id"] == task_id.as_str())
        .unwrap_or_else(|| panic!("the task is in the export: {rows:?}"));
    assert!(row["history"].to_string().contains(QUESTION), "{row}");
    assert!(row.get("session_token").is_none(), "{row}");
    let strangers = exported_rows(&fx.resources, &stranger, "a2a_tasks").await;
    assert_eq!(strangers, Vec::<Value>::new());
}

/// A client that never threads is one conversation, however much it asks.
#[tokio::test]
async fn messages_naming_no_context_roll_on_in_one_thread_holding_one_slot() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);
    let owner = rider.user_id.to_string();
    let repos = fx.resources.common.repos.clone();
    let before = repos
        .chat
        .count_conversations(&owner, rider.tenant)
        .await
        .unwrap();

    let (_, first) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;
    let (_, second) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;
    let (_, third) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;

    assert_eq!(first, second);
    assert_eq!(second, third);
    assert_eq!(
        repos
            .chat
            .count_conversations(&owner, rider.tenant)
            .await
            .unwrap(),
        before + 1,
        "three messages hold one of the athlete's conversation slots"
    );
    let stored = repos
        .chat
        .get_messages(&first, &owner, rider.tenant)
        .await
        .unwrap();
    assert_eq!(stored.iter().filter(|row| row.role == "user").count(), 3);
}

/// The rolling thread is this athlete's, on this surface, and still live.
#[tokio::test]
async fn the_rolling_thread_is_never_another_surfaces_another_athletes_or_a_retired_one() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);
    let owner = rider.user_id.to_string();
    let repos = fx.resources.common.repos.clone();

    let app_thread = repos
        .chat
        .create_conversation(&owner, rider.tenant, "In the app", "m", None, None)
        .await
        .unwrap();
    let (_, rolling) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;
    assert_ne!(
        rolling, app_thread.id,
        "an app thread is not this surface's"
    );

    // A newer thread in the app does not take the client off its own.
    repos
        .chat
        .create_conversation(&owner, rider.tenant, "Newer", "m", None, None)
        .await
        .unwrap();
    let (_, again) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;
    assert_eq!(again, rolling);

    let stranger = athlete_with_relay(&fx.resources).await;
    let stranger_authorization = grant(&fx.resources, &stranger, READ_ONLY);
    let (_, theirs) = sent_task(&fx.app, &stranger_authorization, text_message(QUESTION)).await;
    assert_ne!(theirs, rolling, "a thread is one athlete's");

    assert!(repos
        .chat
        .archive_conversation(&rolling, &owner, rider.tenant)
        .await
        .unwrap());
    let (_, fresh) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;
    assert_ne!(fresh, rolling, "a retired thread stays retired");
}

/// The cap the app answers to is not spent any further by a rolling client;
/// a client minting an id per message still answers to it.
#[tokio::test]
async fn at_the_cap_the_rolling_thread_still_answers_and_a_new_chosen_id_is_refused() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);
    let owner = rider.user_id.to_string();
    let repos = fx.resources.common.repos.clone();

    let (_, rolling) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;
    let held = repos
        .chat
        .count_conversations(&owner, rider.tenant)
        .await
        .unwrap();
    for i in held..DEFAULT_MAX_ACTIVE_CONVERSATIONS {
        repos
            .chat
            .create_conversation(&owner, rider.tenant, &format!("App {i}"), "m", None, None)
            .await
            .unwrap();
    }

    let (_, still) = sent_task(&fx.app, &authorization, text_message(QUESTION)).await;
    assert_eq!(still, rolling);

    let (status, retry_after, refused) = rpc_with_status(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message_in(&Uuid::new_v4().to_string(), QUESTION),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{refused}");
    assert!(retry_after.is_some(), "{refused}");
}

// ============================================================================
// What stays as it was
// ============================================================================

#[tokio::test]
async fn a_command_never_reaches_the_turns_dispatch() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let refused = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message("/logout"),
    )
    .await;
    let message = refused["error"]["message"].as_str().expect("a refusal");
    assert!(message.contains("Commands are not available"), "{message}");
    assert_eq!(fx.model.calls_so_far(), 0);
}

#[tokio::test]
async fn a_message_past_the_length_a_turn_reads_is_refused() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let refused = rpc(
        &fx.app,
        &authorization,
        "SendMessage",
        text_message(&"a".repeat(16_001)),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    assert_eq!(fx.model.calls_so_far(), 0);
}

#[tokio::test]
async fn a_message_with_neither_text_nor_a_tool_keeps_its_refusal() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let mut params = text_message("   ");
    params["message"]["parts"] = json!([{"text": "   "}, {"data": {"no_tool": true}}]);
    let refused = rpc(&fx.app, &authorization, "SendMessage", params).await;
    assert_eq!(refused["error"]["code"], -32005, "{refused}");
    assert_eq!(fx.model.calls_so_far(), 0);
}

#[tokio::test]
async fn a_data_part_naming_a_tool_still_runs_the_tool_and_no_model() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let mut params = text_message("get my profile");
    params["message"]["parts"] = json!([
        {"text": "get my profile"},
        {"data": {"tool_name": "get_connection_status", "parameters": {}}}
    ]);
    let sent = rpc(&fx.app, &authorization, "SendMessage", params).await;
    assert!(sent["result"]["task"]["id"].is_string(), "{sent}");
    assert_eq!(
        fx.model.calls_so_far(),
        0,
        "a tool intent spends no model call"
    );
    assert_eq!(
        sent["result"]["task"]["artifacts"][0]["name"],
        "get_connection_status result"
    );
}

#[tokio::test]
async fn the_streaming_send_carries_the_reply() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let request = Request::post("/a2a/jsonrpc")
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .header("A2A-Version", "1.0")
        .header("authorization", &authorization)
        .body(Body::from(
            json!({
                "jsonrpc": "2.0", "id": 1,
                "method": "SendStreamingMessage",
                "params": text_message(QUESTION),
            })
            .to_string(),
        ))
        .unwrap();
    let response = fx.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = timeout(PATIENCE, to_bytes(response.into_body(), usize::MAX))
        .await
        .expect("the stream closes when the task ends")
        .unwrap();
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains(ANSWER), "{body}");
    assert!(body.contains("TASK_STATE_COMPLETED"), "{body}");
}

/// A2A 1.0: a terminal task names its ids on every message, the agent speaks as
/// `ROLE_AGENT`, and the result is an artifact a client reads without knowing
/// this server.
#[tokio::test]
async fn the_reply_is_in_the_specs_wire_shape() {
    let fx = fixture().await;
    let rider = athlete_with_relay(&fx.resources).await;
    let authorization = grant(&fx.resources, &rider, READ_ONLY);

    let mut params = text_message(QUESTION);
    params["configuration"] = json!({"historyLength": 10});
    let sent = rpc(&fx.app, &authorization, "SendMessage", params).await;
    let task = &sent["result"]["task"];
    let (id, context) = (
        task["id"].as_str().unwrap(),
        task["contextId"].as_str().unwrap(),
    );

    let reply = &task["status"]["message"];
    assert_eq!(reply["role"], "ROLE_AGENT", "{sent}");
    assert_eq!(reply["taskId"], id);
    assert_eq!(reply["contextId"], context);
    assert!(reply["messageId"].is_string());

    let artifact = &task["artifacts"][0];
    assert!(artifact["artifactId"].is_string(), "{sent}");
    assert_eq!(artifact["name"], "Coach reply");
    assert_eq!(artifact["parts"][0]["text"], ANSWER);

    let history = task["history"].as_array().expect("the history asked for");
    assert_eq!(history.first().unwrap()["role"], "ROLE_USER");
    assert_eq!(history.first().unwrap()["contextId"], context);
    assert_eq!(history.last().unwrap()["role"], "ROLE_AGENT");
}
