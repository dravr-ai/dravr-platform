// ABOUTME: The athlete's "who read my data" feed: one usage row per outside client's top-level call, none for Dravr's own
// ABOUTME: Coach reads collapse per UTC day, support reads name no client, deletion scrubs the subject, tenants never leak

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1543. An MCP client or an A2A agent calling a tool directly writes
//! one `llm_usage` row (`call_type = 'mcp_tool'`) naming whose data it read,
//! the client and credential it came through, the transport and the admin of
//! a support session. The athlete's feed (`own_access_feed`,
//! `GET /api/me/access-feed`) lists those rows reading their own data, and
//! their coach's `coach_data_reads` per UTC day. The suite drives the
//! dispatch chokepoint the way the `/mcp` and A2A doors do: the executor is
//! built inside the caller's `CALLER_CREDENTIAL` (and support session) scope,
//! which it reads when it is built.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::models::{
    CallOutcome, ConversationTurnId, DelegatedRead, InsertLlmUsage, OAuth2Client, OAuthClientGrant,
    OutsideReadKind, Tenant, TenantId,
};
use pierre_core::permissions::impersonation::{CallerCredential, CredentialKind};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{CoachReadClass, NewCoachDataRead};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::access_feed::{own_access_feed, AccessFeed};
use pierre_test_support::account::delete_account;
use pierre_tool_runtime::context::SUPPORT_SESSION;
use pierre_tool_runtime::protocols::{UniversalResponse, UniversalToolExecutor};
use pierre_tool_runtime::support_session::CALLER_CREDENTIAL;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

use crate::coach_athlete_reads_test::{
    fixture, ok, person, Fixture, Person, ATHLETE_STREAM_RUN, UPLOAD,
};
use crate::common;
use crate::helpers::impersonation::support_grant;

/// The athlete's roster name, as a coach names them with `athlete=`.
const MEMBER: &str = "Mira";

/// The OAuth client the athlete connected, as it registered.
const OAUTH_CLIENT_ID: &str = "access-feed-client";
const OAUTH_CLIENT_NAME: &str = "Trail Companion";
const OAUTH_CLIENT_HOST: &str = "companion.example.com";

/// A row's tool, transport, credential and client.
type Served = (String, Option<String>, Option<String>, Option<String>);

/// The usage row's `call_type` of a direct tool call.
const DIRECT_TOOL_CALL: &str = "mcp_tool";

/// One stored direct-tool-call usage row, every column the feed reads.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageRow {
    tenant_id: String,
    user_id: String,
    tool: String,
    subject_user_id: Option<String>,
    client_id: Option<String>,
    credential: Option<String>,
    transport: Option<String>,
    impersonator_user_id: Option<String>,
    outcome: Option<String>,
}

/// Every direct-tool-call usage row the coach or the athlete owns, under any
/// tenant, sorted by tool.
async fn direct_rows(f: &Fixture) -> Vec<UsageRow> {
    const SELECT: &str = "SELECT tenant_id, user_id, model, subject_user_id, client_id, \
         credential_kind, transport, impersonator_user_id, outcome FROM llm_usage \
         WHERE call_type = $1 AND (user_id = $2 OR user_id = $3)";
    let coach = f.coach.id.to_string();
    let athlete = f.athlete.id.to_string();
    macro_rules! rows {
        ($pool:expr) => {
            sqlx::query(SELECT)
                .bind(DIRECT_TOOL_CALL)
                .bind(&coach)
                .bind(&athlete)
                .fetch_all($pool)
                .await
                .unwrap()
                .iter()
                .map(|row| UsageRow {
                    tenant_id: row.get(0),
                    user_id: row.get(1),
                    tool: row.get(2),
                    subject_user_id: row.get(3),
                    client_id: row.get(4),
                    credential: row.get(5),
                    transport: row.get(6),
                    impersonator_user_id: row.get(7),
                    outcome: row.get(8),
                })
                .collect::<Vec<UsageRow>>()
        };
    }
    let mut rows = match f.res.agent.database.backend() {
        DatabaseBackend::SQLite(db) => rows!(db.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => rows!(db.pool()),
    };
    rows.sort_by(|a, b| a.tool.cmp(&b.tool).then(a.user_id.cmp(&b.user_id)));
    rows
}

/// The credential an OAuth client's access token carries at the `/mcp` door.
pub(crate) fn oauth_credential() -> CallerCredential {
    CallerCredential {
        kind: CredentialKind::OAuthGrant,
        client_id: Some(OAUTH_CLIENT_ID.to_owned()),
        binding: None,
    }
}

/// Register the athlete's OAuth client, so the feed can name it.
async fn register_oauth_client(f: &Fixture) {
    f.repos()
        .oauth2_server
        .store_client(&OAuth2Client {
            id: Uuid::new_v4().to_string(),
            client_id: OAUTH_CLIENT_ID.to_owned(),
            client_secret_hash: "unused".to_owned(),
            redirect_uris: vec![format!("https://{OAUTH_CLIENT_HOST}/callback")],
            grant_types: vec!["authorization_code".to_owned()],
            response_types: vec!["code".to_owned()],
            client_name: Some(OAUTH_CLIENT_NAME.to_owned()),
            client_uri: None,
            scope: None,
            created_at: Timestamp::now(),
            expires_at: None,
        })
        .await
        .unwrap();
}

/// `caller`'s call of `tool` through an outside door over `transport`,
/// presenting `credential`: the executor is built inside the credential's
/// scope, as the `/mcp` dispatcher and the A2A executor build theirs.
pub(crate) async fn outside_call(
    f: &Fixture,
    credential: Option<CallerCredential>,
    transport: Transport,
    caller: Person,
    tool: &str,
    params: Value,
) -> UniversalResponse {
    CALLER_CREDENTIAL
        .scope(credential, async {
            let executor = UniversalToolExecutor::new(f.res.clone())
                .with_scopes(OAuthScope::self_grant())
                .with_transport(transport)
                .with_caller_writing_reply();
            f.call(&executor, caller, tool, params).await
        })
        .await
}

/// `who`'s feed of the last 30 days.
pub(crate) async fn feed_of(f: &Fixture, who: Person) -> AccessFeed {
    own_access_feed(f.repos(), who.id, Timestamp::now() - Elapsed::days(30))
        .await
        .unwrap()
}

/// A GET of `/api/me/access-feed`, presenting `header` when given, with its
/// status and body.
async fn get_feed(res: &Arc<ServerContext>, header: Option<(&str, String)>) -> (StatusCode, Value) {
    let app = ProviderToolRouter::build_http_app(res);
    let mut request = Request::get("/api/me/access-feed");
    if let Some((name, value)) = header {
        request = request.header(name, value);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// `who`'s web session cookie, as the browser sends it.
async fn session_cookie(f: &Fixture, who: Person) -> (&'static str, String) {
    let user = f
        .repos()
        .users
        .get_global(who.id)
        .await
        .unwrap()
        .expect("the person has an account");
    let token = common::generate_test_token(&f.res, &user).await;
    ("cookie", format!("{}={token}", auth_cookie_name()))
}

/// `who`'s feed over REST, through their session cookie.
pub(crate) async fn feed_over_rest(f: &Fixture, who: Person) -> Value {
    let cookie = session_cookie(f, who).await;
    let (status, body) = get_feed(&f.res, Some(cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// A direct-tool-call row written straight to the store under `tenant`, as
/// the executor writes one for an OAuth client's read of `user`'s own data.
async fn insert_own_read(f: &Fixture, tenant: &str, user: Uuid, tool: &str) {
    let user_id = user.to_string();
    let tools_called = serde_json::to_string(&[tool]).unwrap();
    let usage = InsertLlmUsage::for_direct_tool_call(
        tenant,
        &user_id,
        ConversationTurnId::new(),
        tool,
        &tools_called,
        5,
    );
    let read = DelegatedRead {
        subject_user_id: Some(&user_id),
        client_id: Some(OAUTH_CLIENT_ID),
        credential: Some(CredentialKind::OAuthGrant.as_str()),
        transport: Transport::McpHttp.as_str(),
        impersonator_user_id: None,
        outcome: CallOutcome::Ok,
    };
    f.repos()
        .llm_usage
        .insert_delegated_usage(&usage, &read)
        .await
        .unwrap();
}

/// The sorted keys of a JSON object.
fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

// ── An outside client's call writes one row ─────────────────────────────

#[tokio::test]
async fn an_oauth_clients_mcp_read_of_the_athletes_data_is_one_row_listed_once_with_its_client() {
    let f = fixture().await;
    register_oauth_client(&f).await;
    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.athlete,
            "get_activities",
            json!({}),
        )
        .await,
        "get_activities over MCP",
    );

    let athlete = f.athlete.id.to_string();
    assert_eq!(
        direct_rows(&f).await,
        vec![UsageRow {
            tenant_id: f.athlete.tenant.to_string(),
            user_id: athlete.clone(),
            tool: "get_activities".to_owned(),
            subject_user_id: Some(athlete),
            client_id: Some(OAUTH_CLIENT_ID.to_owned()),
            credential: Some("oauth_grant".to_owned()),
            transport: Some("mcp_http".to_owned()),
            impersonator_user_id: None,
            outcome: Some("ok".to_owned()),
        }],
        "bug: one outside call writes one row naming the athlete, the client, the credential \
         and the transport"
    );

    let feed = feed_of(&f, f.athlete).await;
    assert_eq!(
        feed.outside_reads.len(),
        1,
        "bug: the call is listed once: {feed:?}"
    );
    let read = &feed.outside_reads[0];
    assert_eq!(
        (read.kind, read.tool.as_deref()),
        (OutsideReadKind::ToolCall, Some("get_activities")),
        "bug: the row is the client's call of another tool"
    );
    assert_eq!(read.tools, vec!["get_activities".to_owned()], "{read:?}");
    assert_eq!(
        read.client_id.as_deref(),
        Some(OAUTH_CLIENT_ID),
        "bug: the row names another client"
    );
    assert!(
        feed.coach_days.is_empty(),
        "bug: an outside client's read is no coach's: {feed:?}"
    );

    let body = feed_over_rest(&f, f.athlete).await;
    let reads = body["app_reads"].as_array().expect("an app_reads array");
    assert_eq!(reads.len(), 1, "bug: the route lists the call once: {body}");
    assert_eq!(reads[0]["tool"], "get_activities", "{body}");
    assert_eq!(reads[0]["client_id"], OAUTH_CLIENT_ID, "{body}");
    assert_eq!(
        reads[0]["client_name"], OAUTH_CLIENT_NAME,
        "bug: the route names the client: {body}"
    );
    assert_eq!(
        reads[0]["client_host"], OAUTH_CLIENT_HOST,
        "bug: and where it is: {body}"
    );
    assert_eq!(reads[0]["credential"], "oauth_grant", "{body}");
    assert_eq!(reads[0]["kind"], "tool_call", "{body}");
    assert_eq!(reads[0]["tools"], json!(["get_activities"]), "{body}");
    assert_eq!(reads[0]["transport"], "mcp_http", "{body}");
    assert_eq!(reads[0]["support_read"], false, "{body}");
}

// carnet#1543 Q7: a call the tool answered with an error is still the
// client's call, and admin counts include it; it served nothing, so the
// athlete's feed does not list it.
#[tokio::test]
async fn a_failed_outside_call_writes_one_error_row_and_is_no_feed_entry() {
    let f = fixture().await;
    // The coach's own call naming no member of their group: the tool runs
    // and answers with an error, reading no one.
    let response = outside_call(
        &f,
        Some(oauth_credential()),
        Transport::McpHttp,
        f.coach,
        "get_group_member_activities",
        json!({ "member": "Nobody In Any Group" }),
    )
    .await;
    assert!(
        !response.success,
        "test setup bug: a member no group holds answers with an error: {:?}",
        response.result
    );

    let coach = f.coach.id.to_string();
    assert_eq!(
        direct_rows(&f).await,
        vec![UsageRow {
            tenant_id: f.coach.tenant.to_string(),
            user_id: coach.clone(),
            tool: "get_group_member_activities".to_owned(),
            subject_user_id: Some(coach),
            client_id: Some(OAUTH_CLIENT_ID.to_owned()),
            credential: Some("oauth_grant".to_owned()),
            transport: Some("mcp_http".to_owned()),
            impersonator_user_id: None,
            outcome: Some("error".to_owned()),
        }],
        "bug: a failed outside call writes its one row, marked an error"
    );
    let feed = feed_of(&f, f.coach).await;
    assert!(
        feed.outside_reads.is_empty(),
        "bug: a call that failed served nothing, so the feed lists none: {feed:?}"
    );
}

#[tokio::test]
async fn a_tool_that_dispatches_another_writes_one_row_for_the_clients_call() {
    let f = fixture().await;
    // analyze_activity reads through get_activity_intelligence: a nested
    // dispatch, part of the one call the client made.
    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.athlete,
            "analyze_activity",
            json!({ "activity_id": ATHLETE_STREAM_RUN, "provider": UPLOAD }),
        )
        .await,
        "analyze_activity over MCP",
    );
    let rows = direct_rows(&f).await;
    assert_eq!(
        rows.iter().map(|row| row.tool.as_str()).collect::<Vec<_>>(),
        vec!["analyze_activity"],
        "bug: the nested call writes no row of its own: {rows:?}"
    );
    assert_eq!(
        feed_of(&f, f.athlete).await.outside_reads.len(),
        1,
        "bug: the feed lists the client's one call"
    );
}

#[tokio::test]
async fn an_a2a_read_writes_one_row_over_a2a_for_a_user_token_and_a_client_credentials_one() {
    let f = fixture().await;
    let registration = pierre_a2a::ClientRegistrationRequest {
        name: "Recovery Agent".to_owned(),
        description: "access feed test agent".to_owned(),
        capabilities: vec!["fitness-data-analysis".to_owned()],
        redirect_uris: vec![],
        contact_email: "agent@access-feed.test".to_owned(),
    };
    let a2a_client = f
        .res
        .a2a
        .a2a_client_manager
        .register_client(registration, f.athlete.id)
        .await
        .unwrap()
        .client_id;

    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::A2a,
            f.athlete,
            "get_activities",
            json!({}),
        )
        .await,
        "get_activities over A2A with a user token",
    );
    ok(
        outside_call(
            &f,
            Some(CallerCredential {
                kind: CredentialKind::A2aClientCredentials,
                client_id: Some(a2a_client.clone()),
                binding: None,
            }),
            Transport::A2a,
            f.athlete,
            "get_training_history",
            json!({}),
        )
        .await,
        "get_training_history over A2A with a client-credentials token",
    );

    let served: Vec<Served> = direct_rows(&f)
        .await
        .into_iter()
        .map(|row| (row.tool, row.transport, row.credential, row.client_id))
        .collect();
    assert_eq!(
        served,
        vec![
            (
                "get_activities".to_owned(),
                Some("a2a".to_owned()),
                Some("oauth_grant".to_owned()),
                Some(OAUTH_CLIENT_ID.to_owned()),
            ),
            (
                "get_training_history".to_owned(),
                Some("a2a".to_owned()),
                Some("a2a_client_credentials".to_owned()),
                Some(a2a_client.clone()),
            ),
        ],
        "bug: each A2A call writes one row over a2a, naming its credential and client"
    );

    let body = feed_over_rest(&f, f.athlete).await;
    let reads = body["app_reads"].as_array().expect("an app_reads array");
    assert_eq!(reads.len(), 2, "bug: both calls are listed once: {body}");
    let agent = reads
        .iter()
        .find(|read| read["tool"] == "get_training_history")
        .unwrap_or_else(|| panic!("the client-credentials read is listed: {body}"));
    assert_eq!(
        agent["client_name"], "Recovery Agent",
        "bug: an A2A client is named by its registration: {body}"
    );
    assert_eq!(agent["transport"], "a2a", "{body}");
}

// ── A coach's read is the coach's day, never an app read ─────────────────

#[tokio::test]
async fn a_coachs_mcp_read_of_the_athlete_is_one_coach_day_and_no_app_read_in_either_feed() {
    let f = fixture().await;
    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.coach,
            "get_activities",
            json!({ "athlete": MEMBER }),
        )
        .await,
        "the coach's get_activities over MCP",
    );

    let rows = direct_rows(&f).await;
    assert_eq!(rows.len(), 1, "bug: one call, one usage row: {rows:?}");
    assert_eq!(
        rows[0].user_id,
        f.coach.id.to_string(),
        "bug: the row is the coach's"
    );
    assert_eq!(
        rows[0].subject_user_id,
        Some(f.athlete.id.to_string()),
        "bug: and names the athlete whose data it read"
    );

    let athlete = feed_of(&f, f.athlete).await;
    assert!(
        athlete.outside_reads.is_empty(),
        "bug: the coach's call is no app read of the athlete's: {athlete:?}"
    );
    assert_eq!(
        athlete.coach_days.len(),
        1,
        "bug: it is one coach day: {athlete:?}"
    );
    let day = &athlete.coach_days[0];
    assert_eq!(
        day.coach,
        Some(f.coach.id),
        "bug: the day names another coach"
    );
    assert_eq!(day.group, f.group, "bug: the day names another group");
    assert_eq!(
        day.class,
        CoachReadClass::Training,
        "bug: an activity read counts as training data"
    );
    assert_eq!((day.reads, day.views), (1, 0), "bug: one read: {day:?}");
    assert!(
        day.served_to_assistant,
        "bug: a read served over MCP is the coach's assistant's: {day:?}"
    );
    assert!(!day.support_read, "{day:?}");

    let coach = feed_of(&f, f.coach).await;
    assert!(
        coach.outside_reads.is_empty(),
        "bug: a read of the athlete is no app read of the coach's own data: {coach:?}"
    );
    assert!(coach.coach_days.is_empty(), "{coach:?}");
}

#[tokio::test]
async fn a_group_view_counts_as_a_view_and_a_tool_read_as_a_read_on_one_day_row() {
    let f = fixture().await;
    let recorded = f
        .repos()
        .coach_data_reads
        .record_daily_coach_read(&NewCoachDataRead {
            tenant: f.athlete.tenant,
            athlete: f.athlete.id,
            coach: f.coach.id,
            group: f.group,
            tool: "group_stats",
            class: CoachReadClass::Training,
            read_at: Timestamp::now(),
            transport: None,
            impersonator: None,
        })
        .await
        .unwrap();
    assert!(recorded, "test setup bug: the day's first view is written");
    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.coach,
            "get_activities",
            json!({ "athlete": MEMBER }),
        )
        .await,
        "the coach's get_activities over MCP",
    );

    let feed = feed_of(&f, f.athlete).await;
    assert_eq!(
        feed.coach_days.len(),
        1,
        "bug: the same day, group and class collapse to one row: {feed:?}"
    );
    let day = &feed.coach_days[0];
    assert_eq!(
        (day.reads, day.views),
        (1, 1),
        "bug: the tool read counts under reads, the group view under views: {day:?}"
    );
    assert_eq!(
        day.day,
        Timestamp::now().utc_date(),
        "bug: today's reads are counted under another day"
    );

    let body = feed_over_rest(&f, f.athlete).await;
    let days = body["coach_days"].as_array().expect("a coach_days array");
    assert_eq!(days.len(), 1, "{body}");
    assert_eq!(days[0]["reads"], 1, "{body}");
    assert_eq!(days[0]["views"], 1, "{body}");
}

// ── Dravr's own surfaces write nothing ───────────────────────────────────

#[tokio::test]
async fn a_first_party_call_writes_no_row() {
    let f = fixture().await;
    ok(
        f.call(&f.own_executor(), f.athlete, "get_activities", json!({}))
            .await,
        "get_activities in the athlete's own chat",
    );
    // The door's credential is in scope, yet the executor serves the web app
    // and writes no reply of its own.
    let web = CALLER_CREDENTIAL
        .scope(Some(oauth_credential()), async {
            let executor = UniversalToolExecutor::new(f.res.clone())
                .with_scopes(OAuthScope::self_grant())
                .with_transport(Transport::WebApp);
            f.call(&executor, f.athlete, "get_training_history", json!({}))
                .await
        })
        .await;
    ok(web, "get_training_history on the web app");

    assert_eq!(
        direct_rows(&f).await,
        Vec::<UsageRow>::new(),
        "bug: Dravr's own surfaces write no direct-tool-call row"
    );
    assert_eq!(
        feed_of(&f, f.athlete).await,
        AccessFeed::default(),
        "bug: Dravr's own surfaces' reads are listed as someone else's"
    );
}

// ── Support sessions ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_support_sessions_call_is_listed_as_supports_with_no_client_named() {
    let f = fixture().await;
    register_oauth_client(&f).await;
    let support = support_grant(&f.res, f.athlete.id).await;
    let admin = support.impersonator_id;
    let served = SUPPORT_SESSION
        .scope(
            Some(support),
            outside_call(
                &f,
                Some(oauth_credential()),
                Transport::McpHttp,
                f.athlete,
                "get_activities",
                json!({}),
            ),
        )
        .await;
    ok(served, "get_activities in the admin's support session");

    let rows = direct_rows(&f).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].impersonator_user_id,
        Some(admin.to_string()),
        "bug: the row names the admin"
    );
    assert_eq!(
        rows[0].subject_user_id,
        Some(f.athlete.id.to_string()),
        "bug: a support read names the athlete it acted as"
    );

    let body = feed_over_rest(&f, f.athlete).await;
    let reads = body["app_reads"].as_array().expect("an app_reads array");
    assert_eq!(reads.len(), 1, "{body}");
    assert_eq!(
        reads[0]["support_read"], true,
        "bug: the call is listed as support's: {body}"
    );
    assert_eq!(
        reads[0]["client_name"],
        Value::Null,
        "bug: the client is the admin's, never named: {body}"
    );
    assert_eq!(reads[0]["client_id"], Value::Null, "{body}");
    assert_eq!(reads[0]["client_host"], Value::Null, "{body}");
    assert!(
        !body.to_string().contains(&admin.to_string()),
        "bug: the feed never names the admin: {body}"
    );
}

// ── Account deletion ─────────────────────────────────────────────────────

#[tokio::test]
async fn deleting_the_athlete_removes_their_rows_and_scrubs_them_from_the_coachs() {
    let f = fixture().await;
    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.athlete,
            "get_activities",
            json!({}),
        )
        .await,
        "the athlete's own get_activities over MCP",
    );
    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.coach,
            "get_training_history",
            json!({ "athlete": MEMBER }),
        )
        .await,
        "the coach's get_training_history over MCP",
    );
    assert_eq!(
        direct_rows(&f).await.len(),
        2,
        "test setup bug: one row each"
    );

    delete_account(f.repos(), f.athlete.id, None).await.unwrap();

    let coach = f.coach.id.to_string();
    assert_eq!(
        direct_rows(&f).await,
        vec![UsageRow {
            tenant_id: f.coach.tenant.to_string(),
            user_id: coach,
            tool: "get_training_history".to_owned(),
            subject_user_id: None,
            client_id: Some(OAUTH_CLIENT_ID.to_owned()),
            credential: Some("oauth_grant".to_owned()),
            transport: Some("mcp_http".to_owned()),
            impersonator_user_id: None,
            outcome: Some("ok".to_owned()),
        }],
        "bug: the athlete's own row goes; the coach's stays, no longer naming them"
    );
}

// ── Tenants ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_read_under_the_athletes_second_tenant_appears_and_another_tenants_never_does() {
    let f = fixture().await;
    let now = Timestamp::now();
    let second = TenantId::generate();
    f.repos()
        .tenants
        .create(&Tenant {
            id: second,
            name: "Second home".to_owned(),
            slug: format!("second-{second}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: f.athlete.id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    insert_own_read(
        &f,
        &f.athlete.tenant.to_string(),
        f.athlete.id,
        "get_activities",
    )
    .await;
    insert_own_read(
        &f,
        &second.to_string(),
        f.athlete.id,
        "get_training_history",
    )
    .await;
    // The same user id under a tenant the athlete does not belong to.
    insert_own_read(
        &f,
        &TenantId::generate().to_string(),
        f.athlete.id,
        "get_sleep_sessions",
    )
    .await;

    let mut listed: Vec<String> = feed_of(&f, f.athlete)
        .await
        .outside_reads
        .into_iter()
        .filter_map(|read| read.tool)
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        vec![
            "get_activities".to_owned(),
            "get_training_history".to_owned()
        ],
        "bug: the feed reads every tenant the athlete belongs to, and no other"
    );

    let body = feed_over_rest(&f, f.athlete).await;
    let mut tools: Vec<&str> = body["app_reads"]
        .as_array()
        .expect("an app_reads array")
        .iter()
        .filter_map(|read| read["tool"].as_str())
        .collect();
    tools.sort_unstable();
    assert_eq!(
        tools,
        vec!["get_activities", "get_training_history"],
        "bug: the route lists the same record: {body}"
    );
}

// ── The route ────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_route_refuses_no_auth_and_answers_the_athletes_session_with_the_feed() {
    let f = fixture().await;
    let (status, body) = get_feed(&f.res, None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "bug: the feed needs a session: {body}"
    );

    register_oauth_client(&f).await;
    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.athlete,
            "get_activities",
            json!({}),
        )
        .await,
        "the athlete's get_activities over MCP",
    );
    ok(
        outside_call(
            &f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.coach,
            "get_activities",
            json!({ "athlete": MEMBER }),
        )
        .await,
        "the coach's get_activities over MCP",
    );

    let body = feed_over_rest(&f, f.athlete).await;
    assert_eq!(
        keys(&body),
        vec![
            "api_key_days",
            "app_reads",
            "coach_days",
            "days",
            "peer_days"
        ],
        "{body}"
    );
    assert_eq!(body["days"], 30, "{body}");

    let reads = body["app_reads"].as_array().expect("an app_reads array");
    assert_eq!(reads.len(), 1, "{body}");
    assert_eq!(
        keys(&reads[0]),
        vec![
            "client_host",
            "client_id",
            "client_name",
            "credential",
            "kind",
            "read_at",
            "support_read",
            "tool",
            "tools",
            "transport",
        ],
        "{body}"
    );
    let read_at = reads[0]["read_at"].as_str().expect("an RFC 3339 read_at");
    assert!(
        Timestamp::parse_from_rfc3339(read_at).is_ok(),
        "bug: read_at is RFC 3339: {read_at}"
    );

    let days = body["coach_days"].as_array().expect("a coach_days array");
    assert_eq!(days.len(), 1, "{body}");
    assert_eq!(
        keys(&days[0]),
        vec![
            "coach_name",
            "data_class",
            "day",
            "group_id",
            "group_name",
            "reads",
            "served_to_assistant",
            "support_read",
            "views",
        ],
        "{body}"
    );
    assert_eq!(days[0]["coach_name"], "Cole Coach", "{body}");
    assert_eq!(days[0]["group_id"], f.group.to_string(), "{body}");
    assert_eq!(days[0]["group_name"], "Reads Squad", "{body}");
    assert_eq!(days[0]["data_class"], "training", "{body}");
    assert_eq!(
        days[0]["day"],
        Timestamp::now().utc_date().to_string(),
        "{body}"
    );
    assert_eq!(days[0]["reads"], 1, "{body}");
    assert_eq!(days[0]["views"], 0, "{body}");
    assert_eq!(days[0]["served_to_assistant"], true, "{body}");
    assert_eq!(days[0]["support_read"], false, "{body}");
}

// ── Grants and deleted accounts ──────────────────────────────────────────

/// Store a grant of `scope` from the athlete to the feed's OAuth client.
async fn grant_client(f: &Fixture, scope: &str) -> String {
    let id = Uuid::new_v4().to_string();
    f.repos()
        .oauth2_server
        .store_client_grant(&OAuthClientGrant {
            id: id.clone(),
            user_id: f.athlete.id.to_string(),
            tenant_id: f.athlete.tenant.to_string(),
            client_id: OAUTH_CLIENT_ID.to_owned(),
            scope: scope.to_owned(),
            granted_at: Timestamp::now(),
            revoked_at: None,
            consent_version: None,
            consent_locale: None,
            authorization_details: None,
            expires_at: None,
        })
        .await
        .unwrap();
    id
}

/// The athlete's own `tool` call through the feed's OAuth client over MCP.
async fn athlete_reads(f: &Fixture, tool: &str) {
    ok(
        outside_call(
            f,
            Some(oauth_credential()),
            Transport::McpHttp,
            f.athlete,
            tool,
            json!({}),
        )
        .await,
        tool,
    );
}

/// The tools of the athlete's app reads over REST, sorted, and the clients
/// they name.
async fn listed_app_reads(f: &Fixture) -> (Vec<String>, Vec<Value>) {
    let body = feed_over_rest(f, f.athlete).await;
    let reads = body["app_reads"].as_array().expect("an app_reads array");
    let mut tools: Vec<String> = reads
        .iter()
        .map(|read| read["tool"].as_str().unwrap_or_default().to_owned())
        .collect();
    tools.sort();
    let clients = reads
        .iter()
        .map(|read| read["client_name"].clone())
        .collect();
    (tools, clients)
}

#[tokio::test]
async fn a_client_holding_a_grant_per_scope_has_each_of_its_reads_listed_once() {
    let f = fixture().await;
    register_oauth_client(&f).await;
    grant_client(&f, "fitness:read").await;
    grant_client(&f, "profile:read").await;
    athlete_reads(&f, "get_activities").await;
    athlete_reads(&f, "get_training_history").await;

    let (tools, clients) = listed_app_reads(&f).await;
    assert_eq!(
        tools,
        vec![
            "get_activities".to_owned(),
            "get_training_history".to_owned()
        ],
        "bug: two grants of one client list each of its reads once, not once per grant"
    );
    assert!(
        clients.iter().all(|name| name == OAUTH_CLIENT_NAME),
        "bug: each read names the one client: {clients:?}"
    );
}

#[tokio::test]
async fn revoking_the_clients_grant_leaves_its_reads_in_the_feed() {
    let f = fixture().await;
    register_oauth_client(&f).await;
    let grant = grant_client(&f, "fitness:read").await;
    athlete_reads(&f, "get_activities").await;

    let athlete = f
        .repos()
        .users
        .get_global(f.athlete.id)
        .await
        .unwrap()
        .unwrap();
    let token = common::generate_test_token(&f.res, &athlete).await;
    let response = ProviderToolRouter::build_http_app(&f.res)
        .oneshot(
            Request::delete(format!("/api/me/oauth-grants/{grant}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "test setup bug: the athlete revokes the grant through Connected apps"
    );
    assert!(
        f.repos()
            .oauth2_server
            .list_client_grants(&f.athlete.id.to_string(), &f.athlete.tenant.to_string())
            .await
            .unwrap()
            .is_empty(),
        "test setup bug: the grant is revoked"
    );

    let (tools, clients) = listed_app_reads(&f).await;
    assert_eq!(
        tools,
        vec!["get_activities".to_owned()],
        "bug: what the client read before it lost access is still the athlete's to see"
    );
    assert_eq!(
        clients,
        vec![json!(OAUTH_CLIENT_NAME)],
        "bug: and names the client"
    );
}

/// A coach whose account is deleted: the reads they made stay on the
/// athlete's record, naming nobody. A coach still owning or coaching a group
/// cannot be deleted (the deletion's blockers ask them to hand it on first),
/// so the reader here is a former coach of the group who holds nothing.
#[tokio::test]
async fn a_deleted_coachs_day_rows_stay_in_the_feed_with_no_coach_named() {
    let f = fixture().await;
    let former = person(&f.res, "former-coach", "Fern Former").await;
    f.repos()
        .coach_data_reads
        .record_coach_read(&NewCoachDataRead {
            tenant: f.athlete.tenant,
            athlete: f.athlete.id,
            coach: former.id,
            group: f.group,
            tool: "get_activities",
            class: CoachReadClass::Training,
            read_at: Timestamp::now(),
            transport: None,
            impersonator: None,
        })
        .await
        .unwrap();
    let before = feed_of(&f, f.athlete).await;
    assert_eq!(
        before
            .coach_days
            .iter()
            .map(|day| day.coach)
            .collect::<Vec<_>>(),
        vec![Some(former.id)],
        "test setup bug: one day of the former coach's reads"
    );

    delete_account(f.repos(), former.id, None).await.unwrap();

    let feed = feed_of(&f, f.athlete).await;
    assert_eq!(
        feed.coach_days.len(),
        1,
        "bug: the coach's reads stay on the athlete's record once the coach is gone: {feed:?}"
    );
    let day = &feed.coach_days[0];
    assert_eq!(
        day.coach, None,
        "bug: the day names a deleted account: {day:?}"
    );
    assert_eq!((day.reads, day.views), (1, 0), "{day:?}");

    let body = feed_over_rest(&f, f.athlete).await;
    let days = body["coach_days"].as_array().expect("a coach_days array");
    assert_eq!(days.len(), 1, "{body}");
    assert_eq!(
        days[0]["coach_name"],
        Value::Null,
        "bug: a deleted coach is named: {body}"
    );
    assert_eq!(days[0]["group_name"], "Reads Squad", "{body}");
    assert!(
        !body.to_string().contains("Fern Former"),
        "bug: the deleted coach's name survives somewhere in the feed: {body}"
    );
}
