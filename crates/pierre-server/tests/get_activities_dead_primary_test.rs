// ABOUTME: An auth-dead elected provider must not blank a turn the athlete's other connections can answer
// ABOUTME: The reconnect prompt accompanies the served window as a caveat instead of replacing it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An athlete with years of Strava history and a WHOOP connection added later
//! whose token expired got ONLY the canned "reconnect WHOOP" sentence on every
//! activity-touching turn: `resolve_most_recent` elected the newest connection,
//! `get_activities` returned `provider_auth_required` from it, and the merge
//! that exists for the 2026-08-22 multi-provider incident sat below that return,
//! unreachable.
//!
//! A multi-source aggregator serves what it holds and prompts for the dead
//! source alongside. These tests pin both halves: the sibling's real rows reach
//! the athlete, and the dead provider is still named for reconnection.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType};
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};

use crate::common::{create_test_server_resources, create_test_user};

/// The elected provider's token is dead, but the athlete's other connection
/// still holds their rides: the window is served from it and the dead provider
/// rides along as a reconnect caveat.
#[tokio::test]
async fn a_dead_primary_serves_the_window_its_healthy_sibling_holds() {
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

    // Strava first: a healthy connection whose durable cache holds real rides.
    // Its live fetch fails (no OAuth token in the test tenant), so the cache is
    // what answers — exactly the degraded path a provider blip takes.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let long_ride = ActivityBuilder::new(
        "strava-ride-1".to_owned(),
        "Sortie longue".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(2),
        7_200,
        "strava".to_owned(),
    )
    .distance_meters(200_000.0)
    .build();
    let tempo_run = ActivityBuilder::new(
        "strava-run-1".to_owned(),
        "Tempo".to_owned(),
        SportType::Run,
        Utc::now() - Duration::days(4),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(14_000.0)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &[long_ride, tempo_run])
        .await
        .unwrap();

    // WHOOP second, so it is elected primary — and it has no token at all, so
    // authenticating it fails auth-shaped.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    // No `provider` argument: the athlete asked about their training, not about
    // one device.
    let response = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await;

    let payload = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        payload.get("error").is_none(),
        "a dead primary must not blank a turn the sibling can answer, got: {payload}"
    );

    let activities = payload
        .get("activities")
        .and_then(Value::as_array)
        .expect("activities array present");
    let ids: Vec<&str> = activities
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        ids,
        vec!["strava-ride-1", "strava-run-1"],
        "the healthy connection's rides must be served, newest first"
    );
    let served_distance = activities[0]
        .get("distance_meters")
        .and_then(Value::as_f64)
        .expect("the served ride keeps its distance");
    assert!(
        (served_distance - 200_000.0).abs() < 1.0,
        "the served rows are the sibling's real records, not a placeholder; got \
         {served_distance} m"
    );

    // The answer names its real source, not the provider that could not answer.
    assert_eq!(
        payload.get("provider").and_then(Value::as_str),
        Some("strava"),
        "the window must be attributed to the connection that produced it"
    );

    // And the athlete is still told which provider to reconnect.
    let caveat = payload
        .get("reconnect_required")
        .expect("the dead provider must still be surfaced for reconnection");
    assert_eq!(
        caveat.get("provider").and_then(Value::as_str),
        Some("whoop"),
        "the caveat names the provider whose connection died"
    );
    assert!(
        caveat
            .get("note")
            .and_then(Value::as_str)
            .is_some_and(|note| note.contains("whoop")),
        "the note tells the model to ask for a whoop reconnect, got: {caveat}"
    );
}

/// An explicit `provider` argument pins the ask to one source. A dead pinned
/// provider is the whole answer: serving another connection's rides would answer
/// a question the athlete did not ask — the same rule the merge path follows.
#[tokio::test]
async fn a_pinned_dead_provider_is_never_substituted_by_a_sibling() {
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
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let long_ride = ActivityBuilder::new(
        "strava-ride-1".to_owned(),
        "Sortie longue".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(2),
        7_200,
        "strava".to_owned(),
    )
    .distance_meters(200_000.0)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &[long_ride])
        .await
        .unwrap();
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(
            &runtime,
            &ctx,
            json!({ "provider": "whoop", "limit": 10, "mode": "summary" }),
        )
        .await;

    let payload = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        payload.get("activities").is_none(),
        "a pinned provider's dead connection must not be answered with another \
         provider's rides, got: {payload}"
    );
    assert_eq!(
        payload.get("error_code").and_then(Value::as_str),
        Some("provider_auth_required"),
        "the pinned ask becomes the reconnect signal, got: {payload}"
    );
}

/// When the dead provider is the athlete's ONLY source, the turn is still the
/// reconnect signal — nothing can be served, and a background backfill against
/// a dead session would loop the athlete forever.
#[tokio::test]
async fn a_dead_only_connection_still_surfaces_the_reconnect_signal() {
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
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await;

    assert!(
        response.is_error,
        "a total blackout is still a refusal, not an empty answer"
    );
    let payload = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        payload.get("activities").is_none(),
        "no connection can serve, so there is nothing to answer with: {payload}"
    );
    assert_eq!(
        payload.get("error_code").and_then(Value::as_str),
        Some("provider_auth_required"),
        "the turn must still hand auth_recovery its signal, got: {payload}"
    );
    assert_eq!(
        payload.get("provider").and_then(Value::as_str),
        Some("whoop"),
        "auth_recovery mints the link for the provider named here"
    );
}
