// ABOUTME: carnet#754 — every provider call is admitted against the budget of the OAuth app that signs it, before it is sent
// ABOUTME: A tenant's own app registered with a daily limit is refused past it; the server's app keeps its own windows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A tenant OAuth app's `rate_limit_per_day` was stored for fourteen months
//! and never refused a request (carnet#754). These tests drive the path every
//! activity tool takes — `AuthService::create_authenticated_provider` over the
//! server context's own registry, then the provider's read — against a local
//! Strava that counts what reaches it, so they prove the wiring, not the
//! arithmetic: the budget comes from the composition root, rides the
//! credentials, and stops the call before it is sent.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use secrecy::SecretString;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{ConnectionType, TenantId, TenantOAuthCredentials, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::protocol::AuthService;
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

/// The client id of the tenant's own Strava app.
const TENANT_APP: &str = "tenant-strava-app";

/// A live Strava access token: forty characters, the shape Strava issues.
const LIVE_ACCESS: &str = "0123456789abcdef0123456789abcdef01234567";

/// A Strava API answering every activity listing with none, counting them.
async fn counting_strava() -> (String, Arc<AtomicUsize>) {
    let served = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&served);
    let app = Router::new().route(
        "/athlete/activities",
        get(move || {
            let count = Arc::clone(&count);
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Json(json!([]))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), served)
}

/// A server context whose Strava API is the mock.
async fn context_serving(base: &str) -> Arc<ServerContext> {
    let mut config = common::test_server_config_with_api_base("strava", base);
    common::set_server_app(
        &mut config,
        "strava",
        "server-strava-app",
        "server-strava-secret",
    );
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    resources
}

/// An athlete with a live Strava token in a tenant of their own.
async fn connected_athlete(resources: &ServerContext, label: &str) -> (Uuid, TenantId) {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, _, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let repos = &resources.common.repos;
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "strava".to_owned(),
        LIVE_ACCESS.to_owned(),
        Some("live-refresh".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("activity:read_all".to_owned()),
    );
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    (user_id, tenant)
}

/// One activity listing through the path every activity tool takes.
async fn list_activities(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Result<(), AppError> {
    let provider = AuthService::new(Arc::<ServerContext>::clone(resources))
        .create_authenticated_provider("strava", user_id, Some(&tenant.to_string()))
        .await
        .unwrap_or_else(|e| panic!("the provider is built: {:?}", e.error));
    provider
        .get_activities_with_params(&ActivityQueryParams {
            limit: Some(5),
            offset: None,
            before: None,
            after: None,
        })
        .await
        .map(|_| ())
}

/// A tenant registered its own Strava app with a daily limit of two: two
/// listings reach Strava, the third is refused as rate-limited and never
/// sent. An athlete of another tenant, on the server's app, is untouched by
/// the tenant app's spent day.
#[tokio::test]
async fn a_tenant_apps_daily_limit_refuses_the_call_past_it_before_it_is_sent() {
    let (base, served) = counting_strava().await;
    let resources = context_serving(&base).await;
    let (tenant_athlete, tenant) = connected_athlete(&resources, "tenant-app").await;
    resources
        .common
        .repos
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id: tenant,
            provider: "strava".to_owned(),
            client_id: TENANT_APP.to_owned(),
            client_secret: SecretString::from("tenant-strava-secret"),
            redirect_uri: "https://tenant.example.test/api/oauth/callback/strava".to_owned(),
            scopes: vec!["activity:read_all".to_owned()],
            rate_limit_per_day: 2,
        })
        .await
        .unwrap();
    let (server_athlete, server_tenant) = connected_athlete(&resources, "server-app").await;

    for _ in 0..2 {
        list_activities(&resources, tenant_athlete, tenant)
            .await
            .expect("within the tenant app's day");
    }
    let refused = list_activities(&resources, tenant_athlete, tenant).await;

    assert_eq!(
        refused.map_err(|e| e.code),
        Err(ErrorCode::ExternalRateLimited)
    );
    assert_eq!(
        served.load(Ordering::SeqCst),
        2,
        "the refused listing never reached Strava"
    );

    list_activities(&resources, server_athlete, server_tenant)
        .await
        .expect("the server's app keeps its own windows");
    assert_eq!(served.load(Ordering::SeqCst), 3);
}
