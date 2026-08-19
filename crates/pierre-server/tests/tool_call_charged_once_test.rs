// ABOUTME: One real tool dispatch charges the athlete's tool budget exactly once
// ABOUTME: A refused call costs nothing, and Copilot's own builtins never reach the meter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::uninlined_format_args)]

//! `daily_tool_calls` used to be written from three places: the `/mcp` handler
//! (per loopback call), the chat turn's end, and the messaging turn's end. On a
//! Copilot ACP turn all of them fired — the loopback charged each real tool, and
//! the turn charged again for its whole ACP-reported count, which includes
//! Copilot's own `noop` and shell calls. One `get_activities` cost the athlete
//! at least 2 and typically 4. Worse than the overcharge: the loopback can be
//! blocked mid-turn on a budget the chat path inflated, so an athlete could be
//! cut off partway through an answer by quota they never consumed.
//!
//! The charge now belongs to `UniversalExecutor::execute_tool` — the one
//! chokepoint every transport passes through — so it is levied once per real
//! dispatch, and tools that are not ours never reach it.

mod common;
mod helpers;

use std::sync::Arc;

use chrono::Utc;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::mcp::McpRoutes;
use serde_json::json;
use uuid::Uuid;

/// Drive a real `tools/call` over the `/mcp` wire — the transport where the
/// duplicate charge lived. Calling the executor directly cannot see the bug:
/// the second charge was levied by the HTTP handler, not the executor.
async fn call_tool_over_mcp(resources: &Arc<ServerContext>, jwt: &str, tool: &str) -> u16 {
    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {jwt}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": {} }
        }))
        .send(McpRoutes::routes(resources.clone()))
        .await;
    response.status()
}

/// Read `daily_tool_calls` for the current period.
async fn charged(resources: &Arc<ServerContext>, tenant: TenantId, user_id: Uuid) -> i64 {
    let period = Utc::now().format("%Y-%m-%d").to_string();
    resources
        .common
        .repos
        .usage_counters
        .get_counter(
            &tenant.to_string(),
            &user_id.to_string(),
            "daily_tool_calls",
            &period,
        )
        .await
        .expect("counter reads")
        .value
}

#[tokio::test]
async fn one_dispatch_over_mcp_charges_exactly_once() {
    let resources = create_test_server_resources().await.unwrap();

    // A user owning its own tenant, with a JWT the /mcp endpoint accepts.
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();
    let mut user = User::new(
        "charge_once@example.com".to_owned(),
        password_hash,
        Some("Charge Once".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant,
            name: "Charge Once Tenant".to_owned(),
            slug: format!("charge-once-{tenant}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant)
        .await
        .unwrap();

    // A synthetic provider so the onboarding gate lets the tool actually run —
    // a refused call is correctly uncharged, which would make this test pass
    // for the wrong reason.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            "synthetic",
            &ConnectionType::Synthetic,
            None,
        )
        .await
        .unwrap();

    let jwt = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .expect("jwt mints");

    assert_eq!(
        charged(&resources, tenant, user_id).await,
        0,
        "fixture precondition: nothing charged yet"
    );

    let status = call_tool_over_mcp(&resources, &jwt, "list_fitness_configs").await;
    assert_eq!(status, 200, "an authenticated tools/call is served in-band");

    assert_eq!(
        charged(&resources, tenant, user_id).await,
        1,
        "ONE tool call must cost ONE. The /mcp handler and the turn's end both \
         charged before this, so a single get_activities on an ACP turn cost the \
         athlete at least 2 and typically 4"
    );
}
