// ABOUTME: What a call under a preset consent grant reaches over /mcp and A2A: streams, tier, writes, routes, budgets
// ABOUTME: A preset client always calls under its ceiling; plain grants, pmcp_, API keys and A2A clients are unchanged

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The grant ceiling (owl phase 5, slice 3 items 6 and 8)
//!
//! A token minted under a preset grant is offered and served only what the
//! preset reads, through the real HTTP composition: `/mcp`'s `tools/list` and
//! `tools/call`, and A2A's `SendMessage` and extended card. The data window
//! (item 7) has its own suite, `oauth2_grant_window_test`.

use crate::a2a_client_credentials_token_test::{
    register_client as register_cc_client, request_token as request_cc_token,
};
use crate::common;
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::consent_form::with_grant_lines;
use crate::helpers::stand_in_provider::{standing_in, with_registry, Account};
use crate::oauth2_grant_details_test::{
    access_detail, approve_with, code_of, constraints_detail, exchange, forget_standing,
    live_grants, register, tokens, world, Client, World,
};

use std::collections::BTreeSet;
use std::sync::Arc;

use dravr_tronc::dates::{Elapsed, Timestamp};
use jsonwebtoken::dangerous::insecure_decode;
use pierre_auth::auth::{Claims, DelegatedGrant};
use pierre_core::constants::oauth_providers::WAHOO;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, CreateUserMcpTokenRequest, OAuthClientGrant,
    SportType, TenantId, UserOAuthToken,
};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::routes::oauth_grants::OAuthGrantsRoutes;
use pierre_providers::core::{FitnessProvider, ProviderConfig};
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use uuid::Uuid;

// ============================================================================
// The world: an athlete with a Strava history, and a client under a preset
// ============================================================================

/// A placed ride `days_ago`.
fn ride(id: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id,
        format!("Ride {id}"),
        SportType::Ride,
        Timestamp::now() - Elapsed::days(days_ago),
        3_600,
        WAHOO,
    )
    .distance_meters(30_000.0)
    .start_latitude(45.501)
    .start_longitude(-73.567)
    .build()
}

/// Two sessions inside any preset's window, placed, and one long before it.
fn history() -> Vec<Activity> {
    vec![ride("recent-1", 1), ride("recent-2", 2), ride("old-1", 200)]
}

fn wahoo_account(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(Account {
        config,
        provider: WAHOO,
        activities: history(),
    })
}

/// An athlete whose Wahoo account, stood in, holds [`history`], warm in the
/// cache too.
async fn reading_world() -> World {
    let mut world = world().await;
    world.resources = with_registry(&world.resources, standing_in(WAHOO, wahoo_account));
    let repos = &world.resources.common.repos;
    let tenant = TenantId::from_uuid(world.tenant_id.parse().unwrap());
    let token = UserOAuthToken::new(
        world.user.id,
        world.tenant_id.clone(),
        WAHOO.to_owned(),
        "wahoo-access".to_owned(),
        Some("wahoo-refresh".to_owned()),
        Some(Timestamp::now() + Elapsed::hours(6)),
        None,
    );
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(world.user.id, tenant, WAHOO, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(world.user.id, &tenant, WAHOO, &history())
        .await
        .unwrap();
    world
}

/// The athlete approves `client` for `grant` on the picker, or for the
/// details the request carries: its access token.
async fn connect(
    world: &World,
    client: &Client,
    extra: &[(&str, &str)],
    grant: Option<&str>,
) -> String {
    let approved = approve_with(world, client, extra, grant).await;
    let code = code_of(&approved);
    tokens(&exchange(world, client, &code).await).0
}

/// A training-coach grant narrowed to `budgets`.
fn training_coach_with(world: &World, budgets: &Value) -> String {
    json!([
        access_detail(world, &json!({})),
        constraints_detail(&json!({ "budgets": budgets })),
    ])
    .to_string()
}

/// A fresh client, approved for the picker's `preset`: the client and token.
async fn under_preset(world: &World, preset: &str) -> (Client, String) {
    with_grant_lines(&world.resources.mcp.messaging_strings_registry);
    let client = register(world).await;
    let token = connect(world, &client, &[], Some(preset)).await;
    (client, token)
}

// ============================================================================
// /mcp and A2A
// ============================================================================

/// One JSON-RPC request to `/mcp` under `bearer`: its status and body.
async fn mcp(world: &World, bearer: &str, method: &str, params: &Value) -> (u16, Value) {
    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {bearer}"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await;
    let status = response.status();
    let body = serde_json::from_str(&response.body_text()).unwrap_or(Value::Null);
    (status, body)
}

/// The tools `/mcp` lists under `bearer`.
async fn listed(world: &World, bearer: &str) -> BTreeSet<String> {
    let (status, body) = mcp(world, bearer, "tools/list", &json!({})).await;
    assert_eq!(status, 200, "{body}");
    body["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("a listing: {body}"))
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

/// `tools/call` of `tool` with `arguments` under `bearer`: the JSON-RPC body.
async fn call(world: &World, bearer: &str, tool: &str, arguments: &Value) -> Value {
    let (status, body) = mcp(
        world,
        bearer,
        "tools/call",
        &json!({"name": tool, "arguments": arguments}),
    )
    .await;
    assert_eq!(status, 200, "{tool}: {body}");
    body
}

/// The structured result of a `tools/call` answer.
fn structured(body: &Value) -> &Value {
    &body["result"]["structuredContent"]
}

/// `SendMessage` addressing `tool` over A2A under `bearer`: the envelope.
async fn a2a_call(world: &World, bearer: &str, tool: &str, parameters: &Value) -> Value {
    AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("authorization", &format!("Bearer {bearer}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "SendMessage",
            "params": {"message": {
                "messageId": format!("m-{tool}-{}", Uuid::new_v4()),
                "role": "ROLE_USER",
                "parts": [{"data": {"tool_name": tool, "parameters": parameters}}],
            }},
            "id": 1,
        }))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await
        .json()
}

/// The skills A2A's extended card offers under `bearer`.
async fn card_skills(world: &World, bearer: &str) -> BTreeSet<String> {
    let card: Value = AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("authorization", &format!("Bearer {bearer}"))
        .json(&json!({"jsonrpc": "2.0", "method": "GetExtendedAgentCard", "id": 1}))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await
        .json();
    card["result"]["skills"]
        .as_array()
        .unwrap_or_else(|| panic!("a card: {card}"))
        .iter()
        .map(|skill| skill["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The `error_code` an in-band refusal carries, wherever the answer put it.
fn error_code(body: &Value) -> Option<String> {
    let text = body.to_string();
    [
        "tier_exceeds_grant",
        "tier_t3_remote",
        "stream_not_granted",
        "outside_grant_window",
        "budget_exhausted",
    ]
    .into_iter()
    .find(|code| text.contains(code))
    .map(str::to_owned)
}

// ============================================================================
// A training-coach grant
// ============================================================================

#[tokio::test]
async fn a_training_coach_grant_is_offered_and_served_its_preset_alone() {
    let world = reading_world().await;
    with_grant_lines(&world.resources.mcp.messaging_strings_registry);
    let client = register(&world).await;
    // Asked with the write scope too: the preset withholds every write.
    let token = connect(
        &world,
        &client,
        &[("scope", "fitness:read fitness:write profile:read")],
        Some("training_coach"),
    )
    .await;

    let offered = listed(&world, &token).await;
    for kept in [
        "get_activities",
        "get_training_history",
        "verify_claim",
        "get_sleep_sessions",
    ] {
        assert!(
            offered.contains(kept),
            "{kept} is in the preset: {offered:?}"
        );
    }
    for withheld in [
        "get_health_records",
        "recall_user_memory",
        "export_routes",
        "analyze_weather_impact",
        "name_place",
        "get_heart_rate_series",
        "get_stats",
        "set_goal",
        "save_training_plan",
        "connect_provider",
    ] {
        assert!(
            !offered.contains(withheld),
            "bug: a training-coach grant was offered {withheld}"
        );
    }
    assert_eq!(
        card_skills(&world, &token)
            .await
            .intersection(&BTreeSet::from(["get_health_records".to_owned()]))
            .count(),
        0,
        "the A2A card offers what the listing offers"
    );

    // Sessions served without their place.
    let answer = call(&world, &token, "get_activities", &json!({})).await;
    let served = answer.to_string();
    assert!(served.contains("recent-1"), "{answer}");
    assert!(
        !served.contains("45.501") && !served.contains("-73.567"),
        "bug: a grant without routes read where the athlete rode: {answer}"
    );

    // Health records, a T2 read: refused on the grant's tier.
    let records = call(&world, &token, "get_health_records", &json!({})).await;
    assert_eq!(
        error_code(&records).as_deref(),
        Some("tier_exceeds_grant"),
        "{records}"
    );
    // The agent's memory, a T3 read: never to a remote client.
    let memory = call(&world, &token, "recall_user_memory", &json!({})).await;
    assert_eq!(
        error_code(&memory).as_deref(),
        Some("tier_t3_remote"),
        "{memory}"
    );

    // A write the token's scope covers: refused on that scope, which the
    // preset withholds.
    let claims = insecure_decode::<Claims>(&token).unwrap().claims;
    assert!(claims.scope.contains("fitness:write"), "{}", claims.scope);
    let (_, write) = mcp(
        &world,
        &token,
        "tools/call",
        &json!({"name": "set_goal", "arguments": {
            "goal_type": "distance", "target_value": 100.0, "timeframe": "month"
        }}),
    )
    .await;
    assert!(
        write.to_string().contains("fitness:write"),
        "bug: a preset grant wrote: {write}"
    );
}

#[tokio::test]
async fn a_sleep_grant_reads_no_session() {
    let world = reading_world().await;
    let (_, token) = under_preset(&world, "sleep_coach").await;
    let offered = listed(&world, &token).await;
    assert!(offered.contains("get_sleep_sessions"), "{offered:?}");
    assert!(!offered.contains("get_activities"), "bug: {offered:?}");
    let refused = call(&world, &token, "get_activities", &json!({})).await;
    assert_eq!(
        error_code(&refused).as_deref(),
        Some("stream_not_granted"),
        "{refused}"
    );
    assert!(!refused.to_string().contains("recent-1"), "{refused}");
}

// ============================================================================
// Connection metadata: what a client is told to call first
// ============================================================================

/// The server's own instructions tell a client to call
/// `get_connection_status` before reading anything. It and the two other
/// tools that read which sources are connected and how fresh each is read no
/// stream's row, so every preset is offered and served them.
#[tokio::test]
async fn a_preset_grant_is_offered_and_served_the_connection_metadata() {
    let world = reading_world().await;
    for preset in ["training_coach", "sleep_coach"] {
        let (_, token) = under_preset(&world, preset).await;
        let offered = listed(&world, &token).await;
        for tool in [
            "get_connection_status",
            "list_data_sources",
            "get_data_freshness",
        ] {
            assert!(
                offered.contains(tool),
                "bug: {preset} withheld {tool}: {offered:?}"
            );
            let answer = call(&world, &token, tool, &json!({})).await;
            assert_eq!(error_code(&answer), None, "{preset}: {tool}: {answer}");
            assert!(
                answer["result"]["isError"] != json!(true),
                "{preset}: {tool} was refused: {answer}"
            );
        }
        let status = call(&world, &token, "get_connection_status", &json!({})).await;
        assert!(
            status.to_string().contains(WAHOO),
            "{preset}: the connected source is named: {status}"
        );
    }
}

// ============================================================================
// Connected apps: the athlete sees a preset grant's terms
// ============================================================================

/// `GET /api/me/oauth-grants` as the signed-in athlete.
async fn connected_apps(world: &World) -> Vec<Value> {
    let session = common::generate_test_token(&world.resources, &world.user).await;
    let listed: Value = AxumTestRequest::get("/api/me/oauth-grants")
        .header("Authorization", &format!("Bearer {session}"))
        .send(OAuthGrantsRoutes::routes(Arc::clone(&world.resources)))
        .await
        .json();
    listed["grants"]
        .as_array()
        .unwrap_or_else(|| panic!("a listing: {listed}"))
        .clone()
}

/// A preset grant lists its preset, its tier and when it expires; a plain
/// grant lists none of them; and a grant whose stored document does not
/// read lists without them rather than failing the athlete's whole list,
/// which is where they revoke from.
#[tokio::test]
async fn connected_apps_show_a_preset_grants_preset_tier_and_expiry() {
    let world = world().await;
    let (client, _) = under_preset(&world, "sleep_coach").await;
    let stored = live_grants(&world)
        .await
        .into_iter()
        .find(|grant| grant.client_id == client.id)
        .unwrap();
    let expires_at = stored.expires_at.expect("a preset grant expires");
    let plain = register(&world).await;
    let unreadable = register(&world).await;
    for (other, details) in [
        (&plain, None),
        (&unreadable, Some("{\"not\": \"details\"}")),
    ] {
        world
            .resources
            .common
            .repos
            .oauth2_server
            .store_client_grant(&OAuthClientGrant {
                id: Uuid::new_v4().to_string(),
                user_id: world.user.id.to_string(),
                tenant_id: world.tenant_id.clone(),
                client_id: other.id.clone(),
                scope: "fitness:read".to_owned(),
                granted_at: Timestamp::now(),
                revoked_at: None,
                consent_version: None,
                consent_locale: None,
                authorization_details: details.map(str::to_owned),
                expires_at: None,
            })
            .await
            .unwrap();
    }

    let apps = connected_apps(&world).await;
    let of = |client: &Client| {
        apps.iter()
            .find(|app| app["client_id"] == client.id.as_str())
            .unwrap_or_else(|| panic!("{} is listed: {apps:?}", client.id))
            .clone()
    };
    let sleep = of(&client);
    assert_eq!(sleep["preset"], "sleep_coach", "{sleep}");
    assert_eq!(sleep["max_tier"], "T1", "{sleep}");
    assert_eq!(
        sleep["expires_at"],
        expires_at.to_rfc3339().as_str(),
        "{sleep}"
    );
    for app in [of(&plain), of(&unreadable)] {
        assert_eq!(app["preset"], Value::Null, "{app}");
        assert_eq!(app["max_tier"], Value::Null, "{app}");
        assert_eq!(app["expires_at"], Value::Null, "{app}");
    }
}

// ============================================================================
// Fail-closed: a preset client never calls without its ceiling
// ============================================================================

/// A token for `client` under `token`'s own scope, minted without the
/// `client_grant_id` claim, as a missed stamp would leave it.
fn without_grant_claim(world: &World, client: &Client, token: &str) -> String {
    let claims = insecure_decode::<Claims>(token).unwrap().claims;
    let scopes: Vec<String> = claims.scope.split(' ').map(str::to_owned).collect();
    world
        .resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &world.resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: &client.id,
                scopes: &scopes,
                audience: None,
            },
            &world.user.id,
            &[],
            Some(world.tenant_id.clone()),
        )
        .unwrap()
}

#[tokio::test]
async fn a_preset_client_never_calls_without_its_ceiling() {
    let world = reading_world().await;
    let (client, token) = under_preset(&world, "sleep_coach").await;

    // A token that lost its claim is held to the client's live grant.
    let claimless = without_grant_claim(&world, &client, &token);
    let offered = listed(&world, &claimless).await;
    assert!(
        !offered.contains("get_activities"),
        "bug: a claimless token under a preset grant read as a plain grant: {offered:?}"
    );
    let refused = call(&world, &claimless, "get_activities", &json!({})).await;
    assert_eq!(error_code(&refused).as_deref(), Some("stream_not_granted"));

    // A call converted into a task keeps the ceiling across its spawn.
    let refused = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "get_activities", "arguments": {}, "task": {"ttl": 60_000}},
        }))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await
        .body_text();
    assert!(
        !refused.contains("recent-1"),
        "bug: a task read past its grant: {refused}"
    );

    // And over A2A, where the task runs detached.
    let envelope = a2a_call(&world, &token, "get_activities", &json!({})).await;
    assert_eq!(
        error_code(&envelope).as_deref(),
        Some("stream_not_granted"),
        "{envelope}"
    );
    assert!(!envelope.to_string().contains("recent-1"), "{envelope}");
}

// ============================================================================
// An approval leaves the client one live grant
// ============================================================================

#[tokio::test]
async fn a_preset_approval_revokes_the_clients_older_grant_of_another_scope() {
    let world = world().await;
    with_grant_lines(&world.resources.mcp.messaging_strings_registry);
    let client = register(&world).await;
    world
        .resources
        .common
        .repos
        .oauth2_server
        .store_client_grant(&OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: world.user.id.to_string(),
            tenant_id: world.tenant_id.clone(),
            client_id: client.id.clone(),
            scope: "fitness:read".to_owned(),
            granted_at: Timestamp::now(),
            revoked_at: None,
            consent_version: None,
            consent_locale: None,
            authorization_details: None,
            expires_at: None,
        })
        .await
        .unwrap();
    connect(&world, &client, &[], Some("training_coach")).await;
    let held: Vec<OAuthClientGrant> = live_grants(&world)
        .await
        .into_iter()
        .filter(|grant| grant.client_id == client.id)
        .collect();
    assert_eq!(
        held.len(),
        1,
        "bug: an older plain grant stayed live: {held:?}"
    );
    assert!(held[0].authorization_details.is_some());
}

// ============================================================================
// Budgets
// ============================================================================

#[tokio::test]
async fn a_grant_spent_for_the_hour_is_answered_429_with_retry_after() {
    let world = world().await;
    with_grant_lines(&world.resources.mcp.messaging_strings_registry);
    let client = register(&world).await;
    let details = training_coach_with(&world, &json!({"requests_per_hour": 2}));
    let token = connect(
        &world,
        &client,
        &[("authorization_details", &details)],
        None,
    )
    .await;
    for _ in 0..2 {
        let (status, body) = mcp(&world, &token, "tools/list", &json!({})).await;
        assert_eq!(status, 200, "{body}");
    }
    let spent = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))
        .send(ProviderToolRouter::build_http_app(&world.resources))
        .await;
    assert_eq!(spent.status(), 429, "{}", spent.body_text());
    let retry_after: i64 = spent.header("retry-after").unwrap().parse().unwrap();
    assert!(retry_after > 0 && retry_after <= 3_600, "{retry_after}");
    assert_eq!(
        spent.header("x-ratelimit-limit"),
        Some("2"),
        "the tighter budget"
    );
}

#[tokio::test]
async fn a_grant_spent_for_the_day_answers_budget_exhausted_over_mcp_and_a2a() {
    let world = reading_world().await;
    with_grant_lines(&world.resources.mcp.messaging_strings_registry);
    let details = training_coach_with(&world, &json!({"rows_per_day": 1}));

    let over_mcp = register(&world).await;
    let token = connect(
        &world,
        &over_mcp,
        &[("authorization_details", &details)],
        None,
    )
    .await;
    let first = call(&world, &token, "get_activities", &json!({})).await;
    assert!(
        first.to_string().contains("recent-1"),
        "admitted on the day's counts: {first}"
    );
    let spent = call(&world, &token, "get_activities", &json!({})).await;
    assert_eq!(
        error_code(&spent).as_deref(),
        Some("budget_exhausted"),
        "{spent}"
    );
    assert!(
        structured(&spent)["retry_after_secs"]
            .as_i64()
            .is_some_and(|secs| secs > 0)
            || spent.to_string().contains("retry_after_secs"),
        "{spent}"
    );

    let over_a2a = register(&world).await;
    let token = connect(
        &world,
        &over_a2a,
        &[("authorization_details", &details)],
        None,
    )
    .await;
    let first = a2a_call(&world, &token, "get_activities", &json!({})).await;
    assert!(first.to_string().contains("recent-1"), "{first}");
    let spent = a2a_call(&world, &token, "get_activities", &json!({})).await;
    assert_eq!(
        error_code(&spent).as_deref(),
        Some("budget_exhausted"),
        "{spent}"
    );
    assert!(spent.to_string().contains("retry_after_secs"), "{spent}");
}

// ============================================================================
// Unchanged: a plain grant, a personal MCP token, an API key, an A2A client
// ============================================================================

#[tokio::test]
async fn a_plain_grant_reads_and_writes_as_before() {
    let world = reading_world().await;
    with_grant_lines(&world.resources.mcp.messaging_strings_registry);
    let client = register(&world).await;
    let token = connect(&world, &client, &[], Some("everything")).await;
    let offered = listed(&world, &token).await;
    for kept in [
        "get_stats",
        "export_routes",
        "get_heart_rate_series",
        "get_health_records",
    ] {
        assert!(offered.contains(kept), "{kept}: {offered:?}");
    }
    let answer = call(&world, &token, "get_activities", &json!({})).await;
    assert!(answer.to_string().contains("old-1"), "no window: {answer}");
    forget_standing(&world, &client).await;
}

#[tokio::test]
async fn a_personal_mcp_token_and_an_api_key_read_as_before() {
    let world = world().await;
    let repos = &world.resources.common.repos;
    let personal = repos
        .user_mcp_tokens
        .create_token(
            world.user.id,
            &CreateUserMcpTokenRequest {
                name: "Claude Desktop".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap()
        .token_value;
    let personal = personal.expose_secret().to_owned();
    let offered = listed(&world, &personal).await;
    assert!(
        offered.contains("get_stats") && offered.contains("set_goal"),
        "{offered:?}"
    );

    let key = common::issue_test_api_key(&world.resources.agent.database, world.user.id, "plain")
        .await
        .unwrap();
    let offered = listed(&world, &key).await;
    assert!(
        offered.contains("get_stats") && offered.contains("set_goal"),
        "{offered:?}"
    );
}

#[tokio::test]
async fn an_a2a_client_credentials_token_is_offered_as_before() {
    let world = world().await;
    let (_jwt, client_id, secret) = register_cc_client(
        &world.resources,
        &format!("ceiling-cc-{}@example.com", Uuid::new_v4()),
    )
    .await;
    let token: Value = request_cc_token(&world.resources, "/oauth2/token", &client_id, &secret)
        .await
        .json();
    let skills = card_skills(&world, token["access_token"].as_str().unwrap()).await;
    assert!(skills.contains("get_stats"), "{skills:?}");
}
