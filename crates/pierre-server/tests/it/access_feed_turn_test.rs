// ABOUTME: An outside caller driving Dravr's agent (A2A, the athlete's API key) is one agent-turn row of the owner feed
// ABOUTME: An API key's REST requests are counted per day and key from api_key_usage; its /mcp calls are app reads instead

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1543, Q6. A coaching turn an A2A agent or the athlete's API key
//! drives writes one `turn_summary` usage row naming the athlete, the
//! credential and the transport; the feed lists it as an `agent_turn` app
//! read with the tools the agent ran. A first-party turn writes none. Every
//! REST request an API key makes is already an `api_key_usage` row; the feed
//! counts the successful ones per UTC day and key, leaving out the key's
//! `/mcp` calls, which are listed per call as app reads.

#![cfg(all(feature = "client-chat", feature = "client-messaging"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_a2a::coaching::CoachingTurns;
use pierre_auth::auth::DelegatedGrant;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_chat_pipeline::{
    execute, CommandPersistence, InputSource, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{
    ApiKeyUsage, ConversationTurnId, OutsideReadKind, TenantId, AGENT_TURN_ROUTE,
};
use pierre_core::permissions::impersonation::{CallerCredential, CredentialKind};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::a2a_coaching::PipelineCoachingTurns;
use pierre_routes_a2a::{A2ARoutes, A2ARoutesState};
use pierre_routes_admin::LlmConsumptionRoutes;
use pierre_services::access_feed::own_access_feed;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::helpers::recording_llm::RecordingProvider;

/// The agent's answer once it has read.
const ANSWER: &str = "Your connections look fine; keep riding.";
/// The tool the agent calls first, which needs no provider connected.
const READ: &str =
    r#"Checking. <tool_call>{"name":"get_connection_status","arguments":{}}</tool_call>"#;
/// The tool [`READ`] calls.
const READ_TOOL: &str = "get_connection_status";
/// Long and plain enough that the local detector settles the language.
const QUESTION: &str = "Could you look at which of my training accounts are connected right now \
                        and tell me whether anything needs fixing before my next block?";
/// The OAuth client an A2A agent's grant was issued to.
const A2A_CLIENT: &str = "copilot_studio_test";

struct Fixture {
    res: Arc<ServerContext>,
    model: Arc<RecordingProvider>,
    user: Uuid,
    tenant: TenantId,
    conversation: String,
}

async fn fixture() -> Fixture {
    common::init_server_config();
    common::init_test_http_clients();
    let model = Arc::new(RecordingProvider::answering(ANSWER));
    let res = common::create_test_server_resources_with_chat_provider(model.clone())
        .await
        .unwrap();
    let email = format!("turn-feed-{}@access-feed.test", Uuid::new_v4());
    let (user, _, tenant) =
        common::create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let conversation = res
        .common
        .repos
        .chat
        .create_conversation(
            &user.to_string(),
            tenant,
            "Connections",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    Fixture {
        res,
        model,
        user,
        tenant,
        conversation,
    }
}

/// One stored turn-summary row's feed columns.
#[derive(Debug, PartialEq, Eq)]
struct SummaryRow {
    user_id: String,
    subject_user_id: Option<String>,
    client_id: Option<String>,
    credential: Option<String>,
    transport: Option<String>,
    outcome: Option<String>,
    tools_called: String,
    tool_calls_count: i64,
    total_tokens: i64,
}

/// The tools a row's `tools_called` names. The turn's own list: the tools the
/// agent called, and the athlete's activities when the platform prefetched
/// them into the prompt.
fn ran(tools_called: &str) -> Vec<String> {
    serde_json::from_str(tools_called).unwrap()
}

/// Every turn-summary row of the fixture's user.
async fn summary_rows(f: &Fixture) -> Vec<SummaryRow> {
    const SELECT: &str = "SELECT user_id, subject_user_id, client_id, credential_kind, transport, \
         outcome, tools_called, tool_calls_count, total_tokens FROM llm_usage \
         WHERE call_type = 'turn_summary' AND user_id = $1";
    let user = f.user.to_string();
    macro_rules! rows {
        ($pool:expr) => {
            sqlx::query(SELECT)
                .bind(&user)
                .fetch_all($pool)
                .await
                .unwrap()
                .iter()
                .map(|row| SummaryRow {
                    user_id: row.get(0),
                    subject_user_id: row.get(1),
                    client_id: row.get(2),
                    credential: row.get(3),
                    transport: row.get(4),
                    outcome: row.get(5),
                    tools_called: row.get(6),
                    tool_calls_count: row.get(7),
                    total_tokens: row.get(8),
                })
                .collect::<Vec<SummaryRow>>()
        };
    }
    match f.res.agent.database.backend() {
        DatabaseBackend::SQLite(db) => rows!(db.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => rows!(db.pool()),
    }
}

/// Run one turn of the athlete's question over `transport`, asked through
/// `caller`, the agent reading [`READ_TOOL`] first.
async fn turn_over(f: &Fixture, transport: Transport, caller: Option<CallerCredential>) {
    f.model.answer_next_with(READ);
    let served = run_turn(f, transport, caller, ConversationTurnId::new())
        .await
        .unwrap();
    assert!(
        matches!(served, ServedTurn::Pipeline(_)),
        "test setup bug: the question reaches the agent"
    );
}

/// Run one turn `turn_id` of the athlete's question over `transport`, asked
/// through `caller`, as the model is scripted.
async fn run_turn(
    f: &Fixture,
    transport: Transport,
    caller: Option<CallerCredential>,
    turn_id: ConversationTurnId,
) -> AppResult<ServedTurn> {
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    });
    execute(
        &f.res.chat_pipeline_context(),
        TurnRequest {
            conversation_id: f.conversation.clone(),
            user_id: f.user,
            conversation_tenant_id: f.tenant,
            tool_tenant_id: f.tenant,
            content: QUESTION.to_owned(),
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            turn_id,
            ambient_context: None,
            quote: None,
            channel_type: "web",
            is_direct_message: true,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            transport,
            grant: OAuthScope::self_grant(),
            caller,
            hooks: PipelineHooks::none(),
        },
        &profile,
    )
    .await
}

/// `f`'s user's feed over REST, through their session cookie.
async fn feed_over_rest(f: &Fixture) -> Value {
    let user = f
        .res
        .common
        .repos
        .users
        .get_global(f.user)
        .await
        .unwrap()
        .unwrap();
    let token = common::generate_test_token(&f.res, &user).await;
    let response = ProviderToolRouter::build_http_app(&f.res)
        .oneshot(
            Request::get("/api/me/access-feed")
                .header("cookie", format!("{}={token}", auth_cookie_name()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "bug: the owner's own feed was not served"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ── Q6: a turn an outside caller drove ───────────────────────────────────

#[tokio::test]
async fn an_api_key_chat_message_is_one_agent_turn_row_and_no_request_of_its_day() {
    let f = fixture().await;
    let key = common::issue_test_api_key(&f.res.agent.database, f.user, "Training script")
        .await
        .unwrap();
    f.model.answer_next_with(READ);
    let response = ProviderToolRouter::build_http_app(&f.res)
        .oneshot(
            Request::post(format!(
                "/api/chat/conversations/{}/messages",
                f.conversation
            ))
            .header("authorization", key.as_str())
            .header("content-type", "application/json")
            .body(Body::from(json!({ "content": QUESTION }).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "test setup bug: the athlete's API key may post a chat message"
    );

    // The key then reads the thread back: a REST request starting no turn.
    let read_back = ProviderToolRouter::build_http_app(&f.res)
        .oneshot(
            Request::get(format!(
                "/api/chat/conversations/{}/messages",
                f.conversation
            ))
            .header("authorization", key.as_str())
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        read_back.status(),
        StatusCode::OK,
        "test setup bug: the athlete's API key may read the thread"
    );

    let user = f.user.to_string();
    let rows = summary_rows(&f).await;
    assert_eq!(rows.len(), 1, "bug: one turn, one summary row: {rows:?}");
    let tools = ran(&rows[0].tools_called);
    assert_eq!(
        rows[0].tool_calls_count,
        i64::try_from(tools.len()).unwrap(),
        "bug: the row's count is every tool its list names, the prefetch included: {rows:?}"
    );
    assert_eq!(
        SummaryRow {
            tools_called: String::new(),
            ..rows.into_iter().next().unwrap()
        },
        SummaryRow {
            user_id: user.clone(),
            subject_user_id: Some(user),
            client_id: None,
            credential: Some("api_key".to_owned()),
            transport: Some("api_key".to_owned()),
            outcome: Some("ok".to_owned()),
            tools_called: String::new(),
            tool_calls_count: i64::try_from(tools.len()).unwrap(),
            total_tokens: 0,
        },
        "bug: an API-key chat turn writes one summary row naming the athlete, the key's \
         credential and transport, with no tokens of its own"
    );
    assert!(
        tools.iter().any(|tool| tool == READ_TOOL),
        "bug: the row names the tool the agent called: {tools:?}"
    );

    let body = feed_over_rest(&f).await;
    let reads = body["app_reads"].as_array().expect("an app_reads array");
    assert_eq!(reads.len(), 1, "bug: the turn is listed once: {body}");
    assert_eq!(reads[0]["kind"], "agent_turn", "{body}");
    assert_eq!(
        reads[0]["tool"],
        Value::Null,
        "bug: an agent turn names no tool, and never the model behind it: {body}"
    );
    assert_eq!(
        reads[0]["tools"],
        json!(tools),
        "bug: the feed lists the turn's tools as its row holds them: {body}"
    );
    assert_eq!(reads[0]["credential"], "api_key", "{body}");
    assert_eq!(reads[0]["transport"], "api_key", "{body}");
    assert!(
        !body.to_string().contains("mock-model"),
        "bug: the model's name reaches the athlete: {body}"
    );

    // The message is the agent-turn row, listed once: the day counts only the
    // key's other request, the read-back.
    let days = body["api_key_days"]
        .as_array()
        .expect("an api_key_days array");
    assert_eq!(days.len(), 1, "bug: one key, one day: {body}");
    assert_eq!(days[0]["key_name"], "Training script", "{body}");
    assert_eq!(
        days[0]["requests"], 1,
        "bug: a message starting an agent turn is its app read, not also a request of the day: \
         {body}"
    );
    assert_eq!(
        days[0]["day"],
        Timestamp::now().utc_date().to_string(),
        "{body}"
    );
}

#[tokio::test]
async fn an_a2a_agents_turn_is_one_agent_turn_row_naming_its_client() {
    let f = fixture().await;
    turn_over(
        &f,
        Transport::A2a,
        Some(CallerCredential {
            kind: CredentialKind::OAuthGrant,
            client_id: Some(A2A_CLIENT.to_owned()),
            binding: None,
        }),
    )
    .await;

    let rows = summary_rows(&f).await;
    assert_eq!(rows.len(), 1, "bug: one turn, one summary row: {rows:?}");
    assert_eq!(rows[0].client_id.as_deref(), Some(A2A_CLIENT), "{rows:?}");
    assert_eq!(
        rows[0].credential.as_deref(),
        Some("oauth_grant"),
        "{rows:?}"
    );
    assert_eq!(rows[0].transport.as_deref(), Some("a2a"), "{rows:?}");
    assert_eq!(
        rows[0].tool_calls_count,
        i64::try_from(ran(&rows[0].tools_called).len()).unwrap(),
        "bug: the count agrees with the list: {rows:?}"
    );

    let feed = own_access_feed(
        &f.res.common.repos,
        f.user,
        Timestamp::now() - Elapsed::days(30),
    )
    .await
    .unwrap();
    assert_eq!(feed.outside_reads.len(), 1, "{feed:?}");
    let read = &feed.outside_reads[0];
    assert_eq!(read.kind, OutsideReadKind::AgentTurn, "{read:?}");
    assert_eq!(read.tool, None, "{read:?}");
    assert!(
        read.tools.iter().any(|tool| tool == READ_TOOL),
        "bug: the turn lists the tool the agent called: {read:?}"
    );
    assert_eq!(read.client_id.as_deref(), Some(A2A_CLIENT), "{read:?}");
}

/// A slash command the athlete's API key sends through the chat route reads
/// their account to answer: it is one agent-turn row naming no tool, and,
/// like a message, no request of the key's day.
#[tokio::test]
async fn an_api_key_slash_command_is_one_agent_turn_row_and_no_request_of_its_day() {
    let f = fixture().await;
    let key = common::issue_test_api_key(&f.res.agent.database, f.user, "Status script")
        .await
        .unwrap();
    let calls_before = f.model.calls_so_far();
    let response = ProviderToolRouter::build_http_app(&f.res)
        .oneshot(
            Request::post(format!(
                "/api/chat/conversations/{}/messages",
                f.conversation
            ))
            .header("authorization", key.as_str())
            .header("content-type", "application/json")
            .body(Body::from(json!({ "content": "/status" }).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "test setup bug: the athlete's API key may send a command"
    );
    assert_eq!(
        f.model.calls_so_far(),
        calls_before,
        "test setup bug: a command answers with no model call"
    );

    let user = f.user.to_string();
    assert_eq!(
        summary_rows(&f).await,
        vec![SummaryRow {
            user_id: user.clone(),
            subject_user_id: Some(user),
            client_id: None,
            credential: Some("api_key".to_owned()),
            transport: Some("api_key".to_owned()),
            outcome: Some("ok".to_owned()),
            tools_called: "[]".to_owned(),
            tool_calls_count: 0,
            total_tokens: 0,
        }],
        "bug: a command the key sent is one turn row, its read listed somewhere"
    );

    let body = feed_over_rest(&f).await;
    let reads = body["app_reads"].as_array().expect("an app_reads array");
    assert_eq!(reads.len(), 1, "bug: the command is listed once: {body}");
    assert_eq!(reads[0]["kind"], "agent_turn", "{body}");
    assert_eq!(reads[0]["tools"], json!([]), "{body}");
    assert_eq!(
        body["api_key_days"],
        json!([]),
        "bug: the command is its turn row, not also a request of the day: {body}"
    );
}

/// The athlete's A2A agent sends a text message through the real A2A door,
/// holding a grant the athlete authorized for [`A2A_CLIENT`]: the door's
/// credential reaches the turn's row.
#[tokio::test]
async fn an_a2a_text_message_through_the_door_names_the_agents_client_on_its_row() {
    let f = fixture().await;
    let coaching = Arc::new(PipelineCoachingTurns::new(f.res.clone()));
    let tool_runtime: Arc<dyn ToolRuntime> = f.res.clone();
    let seam: Arc<dyn CoachingTurns> = coaching;
    let app = A2ARoutes::routes(A2ARoutesState {
        ctx: f.res.clone(),
        client_manager: f.res.a2a.a2a_client_manager.clone(),
        auth_middleware: f.res.auth.auth_middleware.clone(),
        tool_runtime,
        coaching: Some(seam),
    });
    let scopes = vec!["fitness:read".to_owned(), "profile:read".to_owned()];
    let token = f
        .res
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &f.res.auth.jwks_manager,
            &DelegatedGrant {
                client_id: A2A_CLIENT,
                scopes: &scopes,
                audience: None,
            },
            &f.user,
            Some(f.tenant.to_string()),
        )
        .unwrap();
    f.model.answer_next_with(READ);
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "SendMessage", "params": {
        "message": {
            "messageId": Uuid::new_v4().to_string(),
            "role": "ROLE_USER",
            "parts": [{"text": QUESTION}],
        },
    }});
    let response = app
        .oneshot(
            Request::post("/a2a/jsonrpc")
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .header("A2A-Version", "1.0")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let sent: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        sent["result"]["task"]["id"].is_string(),
        "test setup bug: the agent's text message opens a task: {sent}"
    );

    let rows = summary_rows(&f).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: one A2A turn, one summary row: {rows:?}"
    );
    assert_eq!(
        (
            rows[0].client_id.as_deref(),
            rows[0].credential.as_deref(),
            rows[0].transport.as_deref(),
            rows[0].outcome.as_deref(),
        ),
        (
            Some(A2A_CLIENT),
            Some("oauth_grant"),
            Some("a2a"),
            Some("ok")
        ),
        "bug: the door's credential names the agent's client on the turn's row: {rows:?}"
    );
}

#[tokio::test]
async fn a_first_party_turn_writes_no_summary_row_and_no_feed_entry() {
    let f = fixture().await;
    turn_over(&f, Transport::WebApp, None).await;
    turn_over(&f, Transport::MobileApp, None).await;

    assert_eq!(
        summary_rows(&f).await,
        Vec::<SummaryRow>::new(),
        "bug: the athlete's own app writes no outside caller's turn"
    );
    let feed = own_access_feed(
        &f.res.common.repos,
        f.user,
        Timestamp::now() - Elapsed::days(30),
    )
    .await
    .unwrap();
    assert!(feed.outside_reads.is_empty(), "{feed:?}");
}

// ── An API key's REST requests per day ───────────────────────────────────

/// Record one `api_key_usage` row of `key` at `at`, as the request-budget
/// layer writes one.
async fn key_request(f: &Fixture, key: &str, at: Timestamp, endpoint: &str, status: u16) {
    f.res
        .common
        .repos
        .usage
        .record_api_key(&ApiKeyUsage {
            id: None,
            api_key_id: key.to_owned(),
            timestamp: at,
            tool_name: endpoint.to_owned(),
            response_time_ms: Some(3),
            status_code: status,
            error_message: None,
            request_size_bytes: None,
            response_size_bytes: None,
            ip_address: None,
            user_agent: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn an_api_keys_successful_rest_requests_are_counted_per_day_and_key_and_its_mcp_calls_are_not(
) {
    let f = fixture().await;
    let db = &f.res.agent.database;
    let script = common::create_and_store_test_api_key(db, f.user, "Script")
        .await
        .unwrap();
    let sheet = common::create_and_store_test_api_key(db, f.user, "Sheet")
        .await
        .unwrap();
    let other_user = common::create_test_user_with_plan(
        db,
        &format!("other-{}@access-feed.test", Uuid::new_v4()),
        "professional",
    )
    .await
    .unwrap()
    .0;
    let theirs = common::create_and_store_test_api_key(db, other_user, "Not yours")
        .await
        .unwrap();

    let now = Timestamp::now();
    let two_days_ago = now - Elapsed::days(2);
    key_request(&f, &script.id, now, "GET /api/activities", 200).await;
    key_request(&f, &script.id, now, "GET /api/athlete/profile", 204).await;
    key_request(&f, &script.id, now, "GET /api/activities", 404).await;
    key_request(&f, &script.id, now, "GET /api/activities", 500).await;
    // `/mcp` names the tool or method it served; the route when none.
    key_request(&f, &script.id, now, "get_activities", 200).await;
    key_request(&f, &script.id, now, "tools/list", 200).await;
    key_request(&f, &script.id, now, "POST /mcp", 200).await;
    key_request(&f, &script.id, now, "GET /mcp/tools", 200).await;
    // A chat message starts an agent turn: its app read, not a request here.
    key_request(
        &f,
        &script.id,
        now,
        &format!("POST {AGENT_TURN_ROUTE}"),
        200,
    )
    .await;
    key_request(&f, &script.id, two_days_ago, "GET /api/activities", 200).await;
    key_request(&f, &sheet.id, now, "GET /api/activities", 200).await;
    key_request(&f, &theirs.id, now, "GET /api/activities", 200).await;
    key_request(
        &f,
        &script.id,
        now - Elapsed::days(40),
        "GET /api/activities",
        200,
    )
    .await;

    let feed = own_access_feed(&f.res.common.repos, f.user, now - Elapsed::days(30))
        .await
        .unwrap();
    let days: Vec<(String, String, u32)> = feed
        .api_key_days
        .iter()
        .map(|day| (day.day.to_string(), day.key_name.clone(), day.requests))
        .collect();
    let today = now.utc_date().to_string();
    assert_eq!(
        days,
        vec![
            (today.clone(), "Script".to_owned(), 2),
            (today, "Sheet".to_owned(), 1),
            (two_days_ago.utc_date().to_string(), "Script".to_owned(), 1),
        ],
        "bug: only the athlete's keys' 2xx REST requests of the last 30 days count, per day \
         and key, newest day first; a key's /mcp calls are its app reads"
    );
    assert_eq!(
        feed.api_key_days[0].key_prefix, script.key_prefix,
        "bug: a key's day names the prefix it is recognised by"
    );

    let body = feed_over_rest(&f).await;
    let listed = body["api_key_days"]
        .as_array()
        .expect("an api_key_days array");
    assert_eq!(listed.len(), 3, "{body}");
    let mut keys: Vec<&str> = listed[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["day", "key_id", "key_name", "key_prefix", "requests"],
        "{body}"
    );
    assert_eq!(listed[0]["key_id"], script.id.as_str(), "{body}");
}

// ── A turn that fails ─────────────────────────────────────────────────────

/// An API-key turn whose model fails after the agent ran a tool: its one
/// summary row says `error` and names the tool, the feed lists only what
/// answered, and the admin per-turn view shows the turn as it ended.
#[tokio::test]
async fn an_api_key_turn_that_fails_writes_one_error_row_naming_what_it_ran() {
    let f = fixture().await;
    f.model.answer_next_with(READ);
    f.model
        .fail_once_queue_is_empty("the model went down mid-turn");
    let turn = ConversationTurnId::new();
    let served = run_turn(
        &f,
        Transport::ApiKey,
        Some(CallerCredential {
            kind: CredentialKind::ApiKey,
            client_id: None,
            binding: None,
        }),
        turn,
    )
    .await;
    assert!(
        served.is_err(),
        "test setup bug: the turn fails once the model does"
    );

    let rows = summary_rows(&f).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: a failed turn still writes its one row: {rows:?}"
    );
    let tools = ran(&rows[0].tools_called);
    assert_eq!(
        (
            rows[0].outcome.as_deref(),
            tools.clone(),
            rows[0].tool_calls_count
        ),
        (Some("error"), vec![READ_TOOL.to_owned()], 1),
        "bug: the row says the turn failed, naming the tool it ran first: {rows:?}"
    );

    let feed = own_access_feed(
        &f.res.common.repos,
        f.user,
        Timestamp::now() - Elapsed::days(30),
    )
    .await
    .unwrap();
    assert!(
        feed.outside_reads.is_empty(),
        "bug: the feed lists only reads that answered: {feed:?}"
    );

    let user = f
        .res
        .common
        .repos
        .users
        .get_global(f.user)
        .await
        .unwrap()
        .unwrap();
    let token = common::generate_test_token(&f.res, &user).await;
    let response = LlmConsumptionRoutes::routes(f.res.clone())
        .oneshot(
            Request::get(format!("/internal/conversation-turn/{turn}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let view: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        view["outcome"], "error",
        "bug: the per-turn view says how it ended: {view}"
    );
    assert!(
        view["tools_called"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|tool| tool == READ_TOOL)),
        "bug: the per-turn view names the tool the failed turn ran: {view}"
    );
    assert!(
        view["first_call_at"]
            .as_str()
            .is_some_and(|at| !at.is_empty()),
        "bug: the turn is dated: {view}"
    );
}
