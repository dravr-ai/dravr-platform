// ABOUTME: A Garmin API athlete end to end: PKCE connect at the dev fixture, pushes over HTTP, reads through the tools
// ABOUTME: Then the final sleep replaces the tentative one, a withdrawn permission purges, a deregistration ends it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin API end-to-end suite (carnet#1155).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
//
// Garmin is played by the dev fixture's stand-in (`pierre_dev_fixture::garmin`),
// the code the local stack runs: its consent page, token exchange and
// `user/id` complete the connect, and its emitter posts the synthetic push
// templates (`tests/fixtures/garmin/`) to the webhook served on a real port.
// The athlete's data is then read through the same tools the chat calls.
#![cfg(all(feature = "health-sync", feature = "provider-garmin"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_config::environment::ProviderEndpointOverride;
use pierre_core::models::TenantId;
use pierre_database::repositories::InboxClaim;
use pierre_dev_fixture::garmin::{
    self, GarminFixture, GarminFixtureConfig, PushTarget, DATA_PUSHES, GARMIN_USER_PREFIX,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::webhooks::WebhookRoutes;
use pierre_mcp_server::tools::implementations::endurance_intervals::ExtractActivityStreamsTool;
use pierre_providers::backend_resolver;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::implementations::stored_data::{
    GetHealthSnapshotsTool, GetRecoveryMetricsTool, GetSleepSessionsTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use reqwest::header::LOCATION;
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use url::Url;
use uuid::Uuid;

/// The webhook path token the server and the fixture share.
const PATH_TOKEN: &str = "garmin-e2e-path-token-5d1f0c7b93a2";

/// Dravr's Garmin client id, as the fixture names it in Garmin's header.
const CLIENT_ID: &str = "garmin-e2e-client";

/// The ride the templates push: summary, details with samples, two laps.
const RIDE_ID: &str = "81000000001";

/// The run the templates push: summary only.
const RUN_ID: &str = "81000000002";

/// Samples in the ride's `activityDetails` template.
const RIDE_SAMPLES: usize = 12;

/// A server whose Garmin consent, token exchange and API reach `garmin`, and
/// whose webhook routes listen on a real port the fixture pushes to.
struct Stack {
    resources: Arc<ServerContext>,
    fixture: GarminFixture,
}

async fn stack() -> Stack {
    common::init_server_config();
    common::init_test_http_clients();

    // The fixture's push target is the webhook's port, known only once the
    // server is up, and the server's Garmin endpoints are the fixture's:
    // bind both listeners first, then build each side.
    let fixture_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture_base = format!("http://{}", fixture_listener.local_addr().unwrap());
    let webhook_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let webhook_base = format!("http://{}", webhook_listener.local_addr().unwrap());

    let mut config = common::test_server_config();
    common::set_server_app(&mut config, "garmin", CLIENT_ID, "garmin-e2e-secret");
    common::set_provider_endpoints(
        &mut config,
        "garmin",
        ProviderEndpointOverride {
            auth_url: Some(format!("{fixture_base}/oauth2Confirm")),
            token_url: Some(format!("{fixture_base}/di-oauth2-service/oauth/token")),
            api_base_url: Some(fixture_base.clone()),
            revoke_url: Some(format!("{fixture_base}/user/registration")),
            ..ProviderEndpointOverride::default()
        },
    );
    config.webhooks.garmin_token = Some(SecretString::from(PATH_TOKEN));
    config.external_services.garmin_api.base_url = format!("{fixture_base}/wellness-api/rest");
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();

    let fixture = GarminFixture::try_new(GarminFixtureConfig {
        push: Some(PushTarget {
            url: format!("{webhook_base}/webhooks/garmin/{PATH_TOKEN}"),
            client_id: CLIENT_ID.to_owned(),
        }),
        files_base: format!("{fixture_base}/wellness-api/rest"),
        // The test emits after the connect returns, never on a timer.
        auto_push_after: None,
    })
    .unwrap();
    let fixture_app = garmin::router(fixture.clone());
    tokio::spawn(async move {
        axum::serve(fixture_listener, fixture_app).await.unwrap();
    });
    let webhook_app = WebhookRoutes::routes(Arc::clone(&resources));
    tokio::spawn(async move {
        axum::serve(webhook_listener, webhook_app).await.unwrap();
    });
    Stack { resources, fixture }
}

/// A new athlete of `resources` and the tenant they own.
async fn athlete(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, _user, tenant_id) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("garmin-e2e-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    (user_id, tenant_id)
}

/// Connect the athlete's Garmin through the fixture as a browser would:
/// Dravr's authorization URL, the consent page's redirect, the callback.
/// Returns the Garmin user id the connection captured.
async fn connect(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) -> String {
    let service = OAuthService::new(resources.data(), resources.common.config.clone());
    let authorization = service
        .get_auth_url(user_id, tenant_id, "garmin", AuthUrlOptions::default())
        .await
        .expect("a Garmin authorization URL");

    let browser = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .unwrap();
    let consent = browser
        .get(&authorization.authorization_url)
        .send()
        .await
        .unwrap();
    assert_eq!(consent.status(), StatusCode::FOUND);
    let location = consent
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(Url::parse)
        .expect("the consent page redirects")
        .unwrap();
    let param = |key: &str| {
        location
            .query_pairs()
            .find(|(name, _)| name == key)
            .map_or_else(
                || panic!("the redirect carries {key}"),
                |(_, value)| value.into_owned(),
            )
    };
    assert_eq!(param("state"), authorization.state, "the state comes back");

    service
        .handle_callback(&param("code"), &param("state"), "garmin")
        .await
        .expect("the code exchange at the fixture succeeds");

    let token = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "garmin")
        .await
        .unwrap()
        .expect("the connect stored a garmin token");
    let garmin_user = token
        .provider_user_id
        .expect("the connect captured the Garmin user id");
    assert!(garmin_user.starts_with(GARMIN_USER_PREFIX), "{garmin_user}");
    garmin_user
}

/// Emit the templates `names` for `garmin_user` and wait for their ingest.
async fn emit(stack: &Stack, garmin_user: &str, names: &[&str]) {
    let sent = stack.fixture.emit(garmin_user, names).await.unwrap();
    for (name, status) in &sent {
        assert_eq!(*status, 200, "the {name} push is accepted");
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while !stack.resources.common.turns.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the push ingest did not finish within 15s"
        );
        sleep(Duration::from_millis(25)).await;
    }
}

/// Run `tool` for the athlete with `args`; its structured payload.
async fn call(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    tool: &dyn McpTool<dyn ToolRuntime>,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer");
    let response = tool.execute(&runtime, &ctx, args).await;
    response.structured_content.expect("a structured payload")
}

/// A stored-data window covering the templates' days.
fn window() -> Value {
    json!({
        "start": (Utc::now() - ChronoDuration::days(7)).to_rfc3339(),
        "end": (Utc::now() + ChronoDuration::days(1)).to_rfc3339(),
    })
}

/// The ids in a `get_activities` payload.
fn activity_ids(payload: &Value) -> Vec<String> {
    payload["activities"]
        .as_array()
        .unwrap_or_else(|| panic!("activities in {payload}"))
        .iter()
        .filter_map(|a| a["id"].as_str().map(str::to_owned))
        .collect()
}

/// The records of a stored-data payload's `key` list.
fn records<'a>(payload: &'a Value, key: &str) -> &'a [Value] {
    payload[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} in {payload}"))
}

/// Find `field` in a stored-data entry, which carries its record either
/// flattened or under `record`.
fn field<'a>(entry: &'a Value, field: &str) -> &'a Value {
    entry
        .get(field)
        .or_else(|| entry.get("record").and_then(|r| r.get(field)))
        .unwrap_or(&Value::Null)
}

#[tokio::test]
async fn a_garmin_api_athlete_connects_is_pushed_to_and_reads_through_the_tools() {
    let stack = stack().await;
    let resources = &stack.resources;
    let (user_id, tenant_id) = athlete(resources).await;

    let garmin_user = connect(resources, user_id, tenant_id).await;
    let auth = resources.common.repos.auth_repos();
    assert_eq!(
        backend_resolver::resolve_backend(&auth, user_id, Some(tenant_id), "garmin").await,
        "garmin",
        "the athlete's Garmin reads go to the API connection"
    );

    emit(
        &stack,
        &garmin_user,
        &["activities", "activityDetails", "sleeps", "dailies"],
    )
    .await;

    // The activity list, named and unnamed: both pushed activities, with the
    // sports their Garmin types map to.
    for args in [
        json!({ "provider": "garmin", "limit": 10, "mode": "summary" }),
        json!({ "limit": 10, "mode": "summary" }),
    ] {
        let list = call(resources, user_id, tenant_id, &GetActivitiesTool, args).await;
        assert!(list.get("error").is_none(), "{list}");
        let mut ids = activity_ids(&list);
        ids.sort();
        assert_eq!(ids, [RIDE_ID, RUN_ID], "{list}");
        let sports: Vec<String> = list["activities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["sport_type"].to_string().to_lowercase())
            .collect();
        assert!(sports.iter().any(|s| s.contains("ride")), "{list}");
        assert!(sports.iter().any(|s| s.contains("run")), "{list}");
    }

    // The ride's detail with the samples its activityDetails carried.
    let streams = call(
        resources,
        user_id,
        tenant_id,
        &ExtractActivityStreamsTool,
        json!({ "activity_id": RIDE_ID }),
    )
    .await;
    assert!(streams.get("error").is_none(), "{streams}");
    assert_eq!(streams["sample_count"], RIDE_SAMPLES, "{streams}");
    let heart_rate = streams["streams"]["heart_rate"]
        .as_array()
        .unwrap_or_else(|| panic!("heart rate in {streams}"));
    assert_eq!(heart_rate.len(), RIDE_SAMPLES, "{streams}");
    assert_eq!(heart_rate[0], 118, "{streams}");
    let laps = resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("garmin"),
            Utc::now() - ChronoDuration::days(7),
            Utc::now(),
            10,
        )
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.id() == RIDE_ID)
        .and_then(|a| a.laps().map(Vec::len));
    assert_eq!(laps, Some(2), "the details' laps stand beside the ride");

    // Sleep and recovery, from the stores the pushes filled.
    let sleeps = call(
        resources,
        user_id,
        tenant_id,
        &GetSleepSessionsTool,
        window(),
    )
    .await;
    let sessions = records(&sleeps, "sessions");
    assert_eq!(sessions.len(), 1, "{sleeps}");
    assert_eq!(field(&sessions[0], "sleep_score"), 64, "{sleeps}");
    let recovery = call(
        resources,
        user_id,
        tenant_id,
        &GetRecoveryMetricsTool,
        window(),
    )
    .await;
    let days = records(&recovery, "metrics");
    assert_eq!(days.len(), 1, "{recovery}");
    assert_eq!(field(&days[0], "resting_heart_rate"), 46, "{recovery}");

    // The tentative sleep is replaced by its final record.
    emit(&stack, &garmin_user, &["sleeps.final"]).await;
    let sleeps = call(
        resources,
        user_id,
        tenant_id,
        &GetSleepSessionsTool,
        window(),
    )
    .await;
    let sessions = records(&sleeps, "sessions");
    assert_eq!(sessions.len(), 1, "the final record replaces it: {sleeps}");
    assert_eq!(field(&sessions[0], "sleep_score"), 83, "{sleeps}");
    assert_eq!(
        field(&sessions[0], "total_sleep_seconds"),
        26_100,
        "{sleeps}"
    );

    // HEALTH_EXPORT withdrawn: health purged, activities kept.
    emit(&stack, &garmin_user, &["userPermissionsChange"]).await;
    let recovery = call(
        resources,
        user_id,
        tenant_id,
        &GetRecoveryMetricsTool,
        window(),
    )
    .await;
    assert!(records(&recovery, "metrics").is_empty(), "{recovery}");
    let sleeps = call(
        resources,
        user_id,
        tenant_id,
        &GetSleepSessionsTool,
        window(),
    )
    .await;
    assert!(records(&sleeps, "sessions").is_empty(), "{sleeps}");
    let list = call(
        resources,
        user_id,
        tenant_id,
        &GetActivitiesTool,
        json!({ "provider": "garmin", "limit": 10, "mode": "summary" }),
    )
    .await;
    assert_eq!(activity_ids(&list).len(), 2, "{list}");

    // The athlete removes Dravr in Garmin Connect: connection and data go.
    emit(&stack, &garmin_user, &["deregistrations"]).await;
    assert!(resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "garmin")
        .await
        .unwrap()
        .is_none());
    assert!(resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("garmin"),
            Utc::now() - ChronoDuration::days(30),
            Utc::now() + ChronoDuration::days(1),
            10,
        )
        .await
        .unwrap()
        .is_empty());
    assert!(resources
        .common
        .repos
        .activity_streams
        .get_activity_streams(&tenant_id, user_id, "garmin", RIDE_ID)
        .await
        .unwrap()
        .is_none());
    let purges: Vec<String> = resources
        .common
        .repos
        .provider_data
        .list_provider_data_purges("garmin", 10)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.reason)
        .collect();
    assert_eq!(purges, ["provider_revoked", "permission_withdrawn"]);
}

/// Every data template the dev stack sends a new connection lands: the
/// webhook accepts it and the stores the reads use hold its readings.
#[tokio::test]
async fn every_data_push_template_is_accepted_and_lands() {
    let stack = stack().await;
    let resources = &stack.resources;
    let (user_id, tenant_id) = athlete(resources).await;
    let garmin_user = connect(resources, user_id, tenant_id).await;

    emit(&stack, &garmin_user, DATA_PUSHES).await;
    // The consent-free file notice is accepted too; its activity is short,
    // so its file is never fetched.
    emit(&stack, &garmin_user, &["activityFiles"]).await;

    let recovery = call(
        resources,
        user_id,
        tenant_id,
        &GetRecoveryMetricsTool,
        window(),
    )
    .await;
    let days = records(&recovery, "metrics");
    assert_eq!(days.len(), 1, "one recovery day: {recovery}");
    let day = &days[0];
    assert_eq!(field(day, "resting_heart_rate"), 46, "dailies: {recovery}");
    assert_eq!(field(day, "hrv_rmssd"), 61.0, "hrv: {recovery}");
    assert_eq!(field(day, "body_battery"), 77, "stressDetails: {recovery}");
    assert_eq!(
        field(day, "skin_temp_deviation"),
        -0.3,
        "skinTemp: {recovery}"
    );

    let body = call(
        resources,
        user_id,
        tenant_id,
        &GetHealthSnapshotsTool,
        window(),
    )
    .await;
    let snapshots = records(&body, "snapshots");
    assert_eq!(snapshots.len(), 1, "one body day: {body}");
    assert_eq!(field(&snapshots[0], "weight_kg"), 71.2, "bodyComps: {body}");
    assert_eq!(field(&snapshots[0], "vo2_max"), 55.0, "userMetrics: {body}");

    let sleeps = call(
        resources,
        user_id,
        tenant_id,
        &GetSleepSessionsTool,
        window(),
    )
    .await;
    assert_eq!(records(&sleeps, "sessions").len(), 1, "{sleeps}");

    let list = call(
        resources,
        user_id,
        tenant_id,
        &GetActivitiesTool,
        json!({ "provider": "garmin", "limit": 10, "mode": "summary" }),
    )
    .await;
    assert_eq!(activity_ids(&list).len(), 2, "{list}");

    // Nothing is left waiting or failed in the inbox.
    let inbox = resources
        .common
        .repos
        .garmin_push_inbox
        .claim_due_pushes(InboxClaim {
            now_ms: Utc::now().timestamp_millis() + 86_400_000,
            lease_ms: 60_000,
            max_attempts: 8,
            limit: 50,
        })
        .await
        .unwrap();
    assert!(
        inbox.is_empty(),
        "every push was processed: {:?}",
        inbox
            .iter()
            .map(|r| (&r.summary_type, &r.last_error))
            .collect::<Vec<_>>()
    );
}
