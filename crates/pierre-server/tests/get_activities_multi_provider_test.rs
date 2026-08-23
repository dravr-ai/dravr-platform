// ABOUTME: get_activities must see ALL of a multi-provider athlete's connections, not one
// ABOUTME: A distance-less watch record and its GPS twin merge and dedup to the GPS row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression guard for the 2026-08-22 live incident: an athlete with Strava
//! and WHOOP asked about "ma sortie d'aujourd'hui" and the coach described
//! WHOOP's distance-less, misclassified "run" — because
//! `resolve_provider_for_tool` picks ONE provider (the most recently used
//! connection resolved to WHOOP) and the 200km Strava ride was never
//! fetched. `get_activities` with no explicit `provider` argument must fold
//! in every other usable connection and deduplicate, so the GPS row wins
//! over its sensor-only twin and provider-exclusive sessions still appear.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType};
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};

use crate::common::{create_test_server_resources, create_test_user};
use crate::helpers::sciotte_mock::{seed_sciotte_session, spawn_mock_scraper};

/// The canned ride the mock scraper serves (the "GPS provider" side).
const SCRAPER_RIDE_ID: &str = "15551234567";
/// Its start instant; the sensor twin overlaps this window.
const SCRAPER_RIDE_START: &str = "2026-08-10T12:00:00Z";

#[tokio::test]
async fn a_no_provider_ask_merges_all_connections_and_keeps_the_gps_row() {
    let scraper_url = spawn_mock_scraper().await;
    env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
    // The remote client is both-or-neither: a URL with no audience disables
    // it, because unsigned requests are refused by the scraper rather than served.
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

    let ride_start: DateTime<Utc> = SCRAPER_RIDE_START.parse().unwrap();

    // WHOOP first: a distance-less sensor twin of the scraper's ride (same
    // window, misclassified sport) plus a WHOOP-only session the day before.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let sensor_twin = ActivityBuilder::new(
        "whoop-twin-1".to_owned(),
        "WHOOP Course".to_owned(),
        SportType::Run,
        ride_start + Duration::minutes(5),
        2_400,
        "whoop".to_owned(),
    )
    .build();
    let whoop_only = ActivityBuilder::new(
        "whoop-only-1".to_owned(),
        "WHOOP Workout".to_owned(),
        SportType::Workout,
        ride_start - Duration::days(1),
        3_600,
        "whoop".to_owned(),
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "whoop", &[sensor_twin, whoop_only])
        .await
        .unwrap();

    // sciotte second, so `resolve_most_recent` picks it as the primary — the
    // live-fetch path then serves the mock scraper's GPS ride.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::Manual, None)
        .await
        .unwrap();
    seed_sciotte_session(&resources, user_id, tenant).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    // No `provider` argument on purpose — the merge only runs then.
    let response = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await;

    let payload = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        payload.get("error").is_none(),
        "multi-provider ask must succeed, got: {payload}"
    );
    let activities = payload
        .get("activities")
        .and_then(Value::as_array)
        .expect("activities array present");
    let ids: Vec<&str> = activities
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();

    assert!(
        ids.contains(&SCRAPER_RIDE_ID),
        "the primary provider's GPS ride must be served, got ids: {ids:?}"
    );
    assert!(
        ids.contains(&"whoop-only-1"),
        "a session only the secondary provider recorded must be merged in, got ids: {ids:?}"
    );
    assert!(
        !ids.contains(&"whoop-twin-1"),
        "the sensor twin of the GPS ride must dedup away (pick_best keeps the \
         distance-bearing row), got ids: {ids:?}"
    );
    assert_eq!(
        activities.len(),
        2,
        "exactly the GPS ride and the WHOOP-only session survive, got: {ids:?}"
    );
}
