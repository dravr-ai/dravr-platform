// ABOUTME: A successful provider serve must stamp provider_connections.last_used_at
// ABOUTME: Without that write the resolver's documented ordering collapses to "newest connection wins"

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `resolve_most_recent` orders on `last_used_at` ahead of `connected_at`, and
//! `ProviderConnection::last_used_at` documents itself as "most recent time this
//! provider actually served data". Nothing in production wrote the column, so it
//! was NULL for every row, `NULLS LAST` demoted nothing, and the election
//! collapsed to the newest connection forever — a documented contract that was
//! false wherever it mattered.
//!
//! This drives the real chat tool against a live provider fetch and pins both
//! halves: the column is written by the serve, and the resolver then elects the
//! backend the athlete actually used over a connection added later.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::Arc;

use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::ConnectionType;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use serial_test::serial;

use crate::common::{create_test_server_resources, create_test_user};
use crate::helpers::sciotte_mock::{seed_sciotte_session, spawn_mock_scraper};

/// The ride the mock scraper serves, proving the fetch really happened.
const SCRAPER_RIDE_ID: &str = "15551234567";

#[tokio::test]
#[serial]
async fn a_served_window_stamps_last_used_and_wins_the_next_election() {
    let scraper_url = spawn_mock_scraper().await;
    env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
    // The remote client is both-or-neither: a URL with no audience disables it.
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");

    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::Manual, None)
        .await
        .unwrap();
    seed_sciotte_session(&resources, user_id, tenant).await;

    let connections = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap();
    assert_eq!(connections.len(), 1, "one connection is registered");
    assert!(
        connections[0].last_used_at.is_none(),
        "a freshly registered connection has never served"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 5, "mode": "summary" }))
        .await;

    let payload = response
        .structured_content
        .expect("tool result carries structured content");
    let ids: Vec<&str> = payload
        .get("activities")
        .and_then(Value::as_array)
        .expect("activities array present")
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        ids,
        vec![SCRAPER_RIDE_ID],
        "the serve must be a real provider fetch, not a cache miss"
    );

    let connections = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap();
    let served = connections
        .iter()
        .find(|c| c.provider == "sciotte")
        .expect("the sciotte connection still exists");
    assert!(
        served.last_used_at.is_some(),
        "a successful serve must stamp last_used_at — the column the resolver \
         orders on cannot stay NULL in production"
    );

    // A connection added AFTER the serve has the newer `connected_at` and would
    // win under the collapsed ordering. It must not: the athlete just trained on
    // sciotte, and that is what `last_used_at` is for.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let elected = resources
        .common
        .repos
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant))
        .await
        .unwrap()
        .expect("the user has connections");
    assert_eq!(
        elected.provider, "sciotte",
        "the connection that actually served must outrank one merely added later"
    );
}
