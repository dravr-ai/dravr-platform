// ABOUTME: GET /api/usage/status reads the caller's real tier, like GET /api/users/me/quota
// ABOUTME: A Professional athlete sees Professional caps on both endpoints, never the Starter ones
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_core::models::UserTier;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::Value;
use tower::ServiceExt;

/// `GET path` as the bearer, returning the JSON body.
async fn get_json(resources: &Arc<ServerContext>, path: &str, token: &str) -> Value {
    let response = ProviderToolRouter::build_http_app(resources)
        .oneshot(
            Request::get(path)
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// The `limit` `/api/users/me/quota` reports for one counter.
fn quota_limit(quota: &Value, counter_type: &str) -> i64 {
    quota["counters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["counter_type"] == counter_type)
        .unwrap_or_else(|| panic!("no {counter_type} counter in {quota}"))["limit"]
        .as_i64()
        .unwrap()
}

/// The usage banner and the quota page read the same caps: a Professional
/// athlete was shown 50 messages / 500k tokens on `/api/usage/status`
/// (Starter's) while `/api/users/me/quota` said 500 / 5M.
#[tokio::test]
async fn usage_status_and_my_quota_agree_on_a_professional_users_limits() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        "pro-usage@example.com",
        "professional",
    )
    .await
    .unwrap();
    let user = resources
        .common
        .repos
        .users
        .set_tier(user_id, UserTier::Professional)
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .unwrap();

    let status = get_json(&resources, "/api/usage/status", &token).await;
    let quota = get_json(&resources, "/api/users/me/quota", &token).await;

    assert_eq!(quota["tier"], "professional");
    assert_eq!(quota_limit(&quota, "daily_messages"), 500);
    assert_eq!(quota_limit(&quota, "daily_tokens"), 5_000_000);
    assert_eq!(quota_limit(&quota, "daily_tool_calls"), 2_000);

    assert_eq!(status["daily"]["messages"]["limit"], 500);
    assert_eq!(status["daily"]["tokens"]["limit"], 5_000_000);
    assert_eq!(status["daily"]["tool_calls"]["limit"], 2_000);
}
