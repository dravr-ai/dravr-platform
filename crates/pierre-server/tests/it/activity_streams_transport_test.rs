// ABOUTME: Pins that extract_activity_streams hands the raw GPS track only to Dravr's own surfaces, never over /mcp, A2A or an API key
// ABOUTME: The other streams travel everywhere; the track is every recorded point, door to door

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The streams of an activity carry `gps_coordinates`, the position of every
//! sample: the athlete's front door at both ends. In the app the track becomes
//! the route map the athlete sees. Over an external transport it would leave
//! Dravr as the provider recorded it, so the tool withholds it there and keeps
//! every other stream.

use crate::common::{create_test_server_resources, create_test_user_with_plan};

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use pierre_auth::api_keys::{ApiKey, ApiKeyManager, ApiKeyTier};
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType, TenantId, TimeSeriesData};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::{ActivityStreamsWrite, StreamOrigin};
use pierre_mcp_server::context::ServerContext;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const TOOL: &str = "extract_activity_streams";

/// The provider key an athlete's uploaded file is held under.
const UPLOAD: &str = "upload";

const RUN: &str = "upload-hill-loop";

/// The latitude of the run's first sample, as written into the stream.
const FIRST_POINT: &str = "38.512345";

struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
}

fn samples() -> TimeSeriesData {
    TimeSeriesData {
        timestamps: vec![0, 1, 2],
        heart_rate: Some(vec![Some(120), Some(131), Some(140)]),
        power: None,
        cadence: None,
        speed: None,
        altitude: Some(vec![Some(610.0), Some(611.5), Some(613.0)]),
        temperature: None,
        gps_coordinates: Some(vec![
            Some((38.512_345, -120.212_345)),
            Some((38.512_399, -120.212_401)),
            None,
        ]),
        distance: None,
    }
}

/// An athlete with one uploaded run whose samples carry its track.
async fn athlete_with_a_tracked_run(resources: &ServerContext) -> Athlete {
    let email = format!("streams-{}@example.com", Uuid::new_v4());
    let (user_id, _user, tenant) =
        create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let repos = resources.agent.database.repositories();
    let run = ActivityBuilder::new(
        RUN.to_owned(),
        "Hill loop".to_owned(),
        SportType::Run,
        Utc::now() - Duration::days(1),
        3_000,
        UPLOAD.to_owned(),
    )
    .distance_meters(8_000.0)
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, UPLOAD, &[run])
        .await
        .unwrap();
    repos
        .activity_streams
        .upsert_activity_streams(&ActivityStreamsWrite {
            tenant_id: &tenant,
            user_id,
            provider: UPLOAD,
            activity_id: RUN,
            origin: StreamOrigin::Upload,
            source_ref: None,
            streams: &samples(),
            synced_at: Utc::now(),
        })
        .await
        .unwrap();
    // The per-activity reads look for a connection to read through; the
    // uploaded run, found by its id, answers through it with no token.
    repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    Athlete { user_id, tenant }
}

/// The tool's payload for `athlete`, on an executor bound to `transport`
/// (`None`: an entry point that declared nothing).
async fn streams_over(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    transport: Option<Transport>,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = UniversalExecutor::new(runtime).with_scopes(OAuthScope::self_grant());
    let executor = match transport {
        Some(served) => executor.with_transport(served),
        None => executor,
    };
    let response = executor
        .execute_tool(UniversalRequest {
            tool_name: TOOL.to_owned(),
            parameters: json!({ "activity_id": RUN }),
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool answers");
    assert!(response.success, "{transport:?}: {:?}", response.error);
    response.result.expect("a payload")
}

#[tokio::test]
async fn the_app_reads_the_track_and_no_external_transport_does() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete_with_a_tracked_run(&resources).await;

    for transport in [
        Transport::WebApp,
        Transport::MobileApp,
        Transport::Messaging,
    ] {
        let payload = streams_over(&resources, &athlete, Some(transport)).await;
        let track = &payload["streams"]["gps_coordinates"];
        assert_eq!(
            track[0][0], 38.512_345,
            "{transport:?} draws the map: {payload}"
        );
        assert_eq!(track.as_array().map(Vec::len), Some(3), "{payload}");
    }

    for transport in [
        Some(Transport::McpHttp),
        Some(Transport::A2a),
        Some(Transport::ApiKey),
        None,
    ] {
        let payload = streams_over(&resources, &athlete, transport).await;
        let text = payload.to_string();
        assert!(
            !text.contains("gps_coordinates"),
            "{transport:?} received the raw track: {text}"
        );
        assert!(
            !text.contains(FIRST_POINT),
            "{transport:?}: a track point leaked: {text}"
        );
        assert_eq!(payload["sample_count"], 3, "{payload}");
        assert_eq!(payload["streams"]["heart_rate"][2], 140, "{payload}");
        assert_eq!(payload["streams"]["altitude"][0], 610.0, "{payload}");
    }
}

/// A stored API key for `user_id`, and the full key to present.
async fn api_key(resources: &Arc<ServerContext>, user_id: Uuid) -> String {
    let data = ApiKeyManager::new().generate_api_key(false);
    let key = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: "streams transport key".to_owned(),
        key_prefix: data.key_prefix,
        key_hash: data.key_hash,
        description: None,
        tier: ApiKeyTier::Starter,
        rate_limit_requests: 1_000,
        rate_limit_window_seconds: 3_600,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    resources.common.repos.api_keys.create(&key).await.unwrap();
    data.full_key
}

#[tokio::test]
async fn post_mcp_returns_the_streams_without_the_track() {
    let resources = create_test_server_resources().await.unwrap();
    let athlete = athlete_with_a_tracked_run(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let key = api_key(&resources, athlete.user_id).await;

    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {key}"))
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": TOOL, "arguments": { "activity_id": RUN } }
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains("\"result\""), "{body}");
    assert!(
        body.contains("heart_rate"),
        "the other streams are served: {body}"
    );
    assert!(body.contains("sample_count"), "{body}");
    assert!(
        !body.contains("gps_coordinates"),
        "/mcp received the raw track: {body}"
    );
    assert!(
        !body.contains(FIRST_POINT),
        "a track point leaked over /mcp: {body}"
    );
}
