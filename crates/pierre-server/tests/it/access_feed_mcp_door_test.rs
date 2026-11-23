// ABOUTME: Every outside call through the real /mcp door is one feed row whatever its credential: an API key, an MCP token
// ABOUTME: A third-party OAuth app's token never drives a REST chat turn: it is refused before one starts

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1543. The REST layer declares a request carrying an API key or a
//! personal MCP token `api_key` before any route runs, and `/mcp` cannot
//! relabel an external declaration. The call's usage row is named by the door
//! it came in by, so an API key's or MCP token's tool call over `/mcp` is one
//! `mcp_tool` row with transport `mcp_http`, listed in the athlete's feed, and
//! never also a request of the key's day. A third-party application's OAuth
//! token is refused by every REST route (`GrantPolicy::DirectOnly`), the chat
//! route included, so it can never run a turn there as Dravr's own web app.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::auth::DelegatedGrant;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::models::{CreateUserMcpTokenRequest, OutsideReadKind, TenantId};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::access_feed::own_access_feed;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;

/// A tool that reads the athlete's own account and needs no provider.
const TOOL: &str = "get_connection_status";

struct Athlete {
    res: Arc<ServerContext>,
    user: Uuid,
    tenant: TenantId,
}

async fn athlete() -> Athlete {
    common::init_server_config();
    common::init_test_http_clients();
    let res = common::create_test_server_resources().await.unwrap();
    let email = format!("mcp-door-{}@access-feed.test", Uuid::new_v4());
    let (user, _, tenant) =
        common::create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    Athlete { res, user, tenant }
}

/// `tools/call` of [`TOOL`] through the server's whole HTTP app, presenting
/// `credential`, answered with a result.
async fn call_over_mcp(a: &Athlete, credential: &str) {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {credential}"))
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": TOOL, "arguments": {} }
            })
            .to_string(),
        ))
        .unwrap();
    let response = ProviderToolRouter::build_http_app(&a.res)
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "test setup bug: the /mcp call was not answered"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8_lossy(&bytes);
    assert!(
        body.contains("\"result\"") && !body.contains("\"isError\":true"),
        "test setup bug: the tool answers over /mcp: {body}"
    );
}

/// Every direct-tool-call row of the athlete: subject, credential, client,
/// transport, outcome.
async fn tool_rows(
    a: &Athlete,
) -> Vec<(
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
    const SELECT: &str = "SELECT subject_user_id, credential_kind, client_id, transport, outcome \
         FROM llm_usage WHERE call_type = 'mcp_tool' AND user_id = $1 AND tenant_id = $2";
    let user = a.user.to_string();
    let tenant = a.tenant.to_string();
    macro_rules! rows {
        ($pool:expr) => {
            sqlx::query(SELECT)
                .bind(&user)
                .bind(&tenant)
                .fetch_all($pool)
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)))
                .collect()
        };
    }
    match a.res.agent.database.backend() {
        DatabaseBackend::SQLite(db) => rows!(db.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => rows!(db.pool()),
    }
}

/// The athlete's feed over REST, through their session cookie.
async fn feed_over_rest(a: &Athlete) -> Value {
    let user = a
        .res
        .common
        .repos
        .users
        .get_global(a.user)
        .await
        .unwrap()
        .unwrap();
    let token = common::generate_test_token(&a.res, &user).await;
    let response = ProviderToolRouter::build_http_app(&a.res)
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

/// The call is one row naming the athlete, `credential` and the `/mcp` door,
/// listed once in their feed.
async fn assert_one_listed_mcp_read(a: &Athlete, credential: &str) {
    let athlete = a.user.to_string();
    assert_eq!(
        tool_rows(a).await,
        vec![(
            Some(athlete),
            Some(credential.to_owned()),
            None,
            Some("mcp_http".to_owned()),
            Some("ok".to_owned()),
        )],
        "bug: a {credential} tool call over /mcp is one row, named by the door it came in by"
    );
    let feed = own_access_feed(
        &a.res.common.repos,
        a.user,
        Timestamp::now() - Elapsed::days(30),
    )
    .await
    .unwrap();
    assert_eq!(feed.outside_reads.len(), 1, "{feed:?}");
    let read = &feed.outside_reads[0];
    assert_eq!(read.kind, OutsideReadKind::ToolCall, "{read:?}");
    assert_eq!(read.tool.as_deref(), Some(TOOL), "{read:?}");
    assert_eq!(read.credential.as_deref(), Some(credential), "{read:?}");

    let body = feed_over_rest(a).await;
    let reads = body["app_reads"].as_array().expect("an app_reads array");
    assert_eq!(reads.len(), 1, "{body}");
    assert_eq!(reads[0]["transport"], "mcp_http", "{body}");
    assert_eq!(reads[0]["client_id"], Value::Null, "{body}");
    assert_eq!(
        body["api_key_days"],
        json!([]),
        "bug: a /mcp call is its app read, never also a request of the key's day: {body}"
    );
}

#[tokio::test]
async fn an_api_keys_tool_call_over_mcp_is_one_feed_row_and_no_request_of_its_day() {
    let a = athlete().await;
    let key = common::issue_test_api_key(&a.res.agent.database, a.user, "Desktop client")
        .await
        .unwrap();
    call_over_mcp(&a, &key).await;
    assert_one_listed_mcp_read(&a, "api_key").await;
}

#[tokio::test]
async fn a_personal_mcp_tokens_tool_call_over_mcp_is_one_feed_row() {
    let a = athlete().await;
    let created = a
        .res
        .common
        .repos
        .user_mcp_tokens
        .create_token(
            a.user,
            &CreateUserMcpTokenRequest {
                name: "Claude Desktop".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap();
    call_over_mcp(&a, created.token_value.expose_secret()).await;
    assert_one_listed_mcp_read(&a, "mcp_token").await;
}

/// The question asked: a third-party application's OAuth token on the REST
/// chat route must not run as Dravr's own web app. It runs as nothing: the
/// route refuses a delegated grant before a turn opens, so no turn is served,
/// no model is asked and no row is written.
#[tokio::test]
async fn a_third_party_apps_oauth_token_is_refused_by_the_chat_route_before_any_turn() {
    let a = athlete().await;
    let conversation = a
        .res
        .common
        .repos
        .chat
        .create_conversation(
            &a.user.to_string(),
            a.tenant,
            "Plan",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    let scopes = vec!["fitness:read".to_owned(), "profile:read".to_owned()];
    let token = a
        .res
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &a.res.auth.jwks_manager,
            &DelegatedGrant {
                client_id: "third-party-coach-app",
                scopes: &scopes,
                audience: None,
            },
            &a.user,
            &[],
            Some(a.tenant.to_string()),
        )
        .unwrap();
    let response = ProviderToolRouter::build_http_app(&a.res)
        .oneshot(
            Request::post(format!("/api/chat/conversations/{conversation}/messages"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "content": "How did my week of training go?" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "bug: a third-party application's token drove a REST chat turn"
    );
    let messages = a
        .res
        .common
        .repos
        .chat
        .get_messages(&conversation, &a.user.to_string(), a.tenant)
        .await
        .unwrap();
    assert!(
        messages.is_empty(),
        "bug: the refused message reached the thread: {messages:?}"
    );
    let feed = own_access_feed(
        &a.res.common.repos,
        a.user,
        Timestamp::now() - Elapsed::days(30),
    )
    .await
    .unwrap();
    assert!(feed.outside_reads.is_empty(), "{feed:?}");
}

/// `GET path` through the whole HTTP app with `authorization`; the status and
/// body.
async fn get_with(a: &Athlete, path: &str, authorization: &str) -> (StatusCode, String) {
    let response = ProviderToolRouter::build_http_app(&a.res)
        .oneshot(
            Request::get(path)
                .header("authorization", authorization)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The feed is the athlete's own screen, as their health records are: every
/// outside credential (their API key, a personal MCP token, an application's
/// delegated grant) is refused with the 403 the health-records route answers
/// it with, and only their session is served.
#[tokio::test]
async fn the_feed_is_refused_to_every_outside_credential_as_health_records_are() {
    let a = athlete().await;
    let api_key = common::issue_test_api_key(&a.res.agent.database, a.user, "Script")
        .await
        .unwrap();
    let mcp_token = a
        .res
        .common
        .repos
        .user_mcp_tokens
        .create_token(
            a.user,
            &CreateUserMcpTokenRequest {
                name: "Claude Desktop".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap()
        .token_value
        .expose_secret()
        .to_owned();
    let scopes = vec!["fitness:read".to_owned(), "profile:read".to_owned()];
    let grant = a
        .res
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &a.res.auth.jwks_manager,
            &DelegatedGrant {
                client_id: "third-party-coach-app",
                scopes: &scopes,
                audience: None,
            },
            &a.user,
            &[],
            Some(a.tenant.to_string()),
        )
        .unwrap();

    for (what, credential) in [
        ("an API key", api_key.clone()),
        ("a personal MCP token", format!("Bearer {mcp_token}")),
        ("a delegated grant", format!("Bearer {grant}")),
    ] {
        let (feed, body) = get_with(&a, "/api/me/access-feed", &credential).await;
        let (records, _) = get_with(&a, "/api/me/health-records?kind=labs", &credential).await;
        assert_eq!(
            feed,
            StatusCode::FORBIDDEN,
            "bug: the feed was served to {what}: {body}"
        );
        assert_eq!(
            feed, records,
            "bug: {what} meets the feed as it meets health records"
        );
    }
    let (_, body) = get_with(&a, "/api/me/access-feed", &api_key).await;
    assert!(
        body.contains("not served to outside apps"),
        "bug: an API key reads the same refusal health records give: {body}"
    );

    let feed = feed_over_rest(&a).await;
    assert_eq!(
        feed["days"], 30,
        "bug: the athlete's session is served: {feed}"
    );
}
