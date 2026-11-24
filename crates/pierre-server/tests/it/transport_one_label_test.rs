// ABOUTME: One call through the real /mcp door carries one transport label on every record it writes, whatever its credential
// ABOUTME: An API key's call is mcp_http on its usage row, the admin trail and the athlete's coach-read record alike

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The REST layer declares a request carrying an API key `api_key` before any
//! route runs; `/mcp` then declares `mcp_http`. Both are external, so the
//! inner door names the call (`Transport::narrowed_by`) and every record the
//! call writes reads `mcp_http`: its `llm_usage` row, its admin-trail row and
//! the coach-read row on the athlete's record, whose day then reads as served
//! to the coach's assistant. What data the call is served is unchanged:
//! every external transport withholds the same.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::transport::Transport;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::RecordedTransport;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::access_feed::own_access_feed;
use serde_json::{json, Value};
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

use crate::admin_tool_audit_test::{admin, trail};
use crate::coach_athlete_reads_test::fixture;
use crate::common;

/// The coach's roster name for the athlete.
const MEMBER: &str = "Mira";

/// `tools/call` of `tool` with `arguments`, posted to the whole HTTP app with
/// `credential`; the JSON-RPC body.
async fn post_mcp(
    res: &Arc<ServerContext>,
    credential: &str,
    tool: &str,
    arguments: Value,
) -> String {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {credential}"))
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": arguments },
            })
            .to_string(),
        ))
        .unwrap();
    let response = ProviderToolRouter::build_http_app(res)
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "test setup bug: the /mcp call was not answered"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    assert!(
        body.contains("\"result\"") && !body.contains("\"isError\":true"),
        "test setup bug: {tool} answers over /mcp: {body}"
    );
    body
}

/// The transport and credential of `user`'s direct-tool-call rows.
async fn usage_labels(
    res: &Arc<ServerContext>,
    user: Uuid,
) -> Vec<(Option<String>, Option<String>)> {
    const SELECT: &str = "SELECT transport, credential_kind FROM llm_usage \
         WHERE call_type = 'mcp_tool' AND user_id = $1";
    let user = user.to_string();
    macro_rules! rows {
        ($pool:expr) => {
            sqlx::query(SELECT)
                .bind(&user)
                .fetch_all($pool)
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get(0), row.get(1)))
                .collect()
        };
    }
    match res.agent.database.backend() {
        DatabaseBackend::SQLite(db) => rows!(db.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => rows!(db.pool()),
    }
}

/// A coach's read of their athlete with the coach's API key over `/mcp`: the
/// usage row and the athlete's coach-read row both say `mcp_http`, and the
/// coach day reads as served to the coach's assistant. A coach's data read is
/// no admin-only tool, so it writes no admin-trail row (the next test pins
/// that record).
#[tokio::test]
async fn a_coachs_api_key_read_over_mcp_is_mcp_http_on_its_usage_and_coach_read_rows() {
    let f = fixture().await;
    let key = common::issue_test_api_key(&f.res.agent.database, f.coach.id, "Coach desktop")
        .await
        .unwrap();
    post_mcp(&f.res, &key, "get_activities", json!({ "athlete": MEMBER })).await;

    assert_eq!(
        usage_labels(&f.res, f.coach.id).await,
        vec![(Some("mcp_http".to_owned()), Some("api_key".to_owned()))],
        "bug: the usage row names the /mcp door and the key"
    );
    let reads = f
        .repos()
        .coach_data_reads
        .list_coach_reads_since(
            f.athlete.tenant,
            f.athlete.id,
            Timestamp::now() - Elapsed::days(1),
            100,
        )
        .await
        .unwrap();
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert_eq!(
        reads[0].transport,
        RecordedTransport::Served(Transport::McpHttp),
        "bug: the coach-read row names the same door as the usage row: {reads:?}"
    );
    assert!(
        trail(&f.res, f.coach.id).await.is_empty(),
        "a coach's data read is no admin-only call, and writes no trail row"
    );

    let feed = own_access_feed(
        f.repos(),
        f.athlete.id,
        Timestamp::now() - Elapsed::days(30),
    )
    .await
    .unwrap();
    assert_eq!(feed.coach_days.len(), 1, "{feed:?}");
    assert!(
        feed.coach_days[0].served_to_assistant,
        "bug: a read over /mcp reached the coach's assistant: {feed:?}"
    );
}

/// An admin's API key calling an admin-only tool over `/mcp`: its trail row
/// and its usage row both say `mcp_http`, the trail naming the key.
#[tokio::test]
async fn an_admin_api_key_tool_call_over_mcp_is_mcp_http_on_the_trail_and_usage_rows() {
    common::init_server_config();
    common::init_test_http_clients();
    let res = common::create_test_server_resources().await.unwrap();
    let (admin, _tenant) = admin(&res).await;
    let key = common::issue_test_api_key(&res.agent.database, admin, "Admin desktop")
        .await
        .unwrap();
    post_mcp(&res, &key, "admin_list_system_agents", json!({})).await;

    let rows = trail(&res, admin).await;
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert_eq!(
        (
            rows[0].metadata["transport"].as_str(),
            rows[0].metadata["credential_kind"].as_str(),
        ),
        (Some("mcp_http"), Some("api_key")),
        "bug: the trail names the /mcp door and the key: {rows:#?}"
    );
    assert_eq!(
        usage_labels(&res, admin).await,
        vec![(Some("mcp_http".to_owned()), Some("api_key".to_owned()))],
        "bug: the usage row names the same door as the trail"
    );
}
