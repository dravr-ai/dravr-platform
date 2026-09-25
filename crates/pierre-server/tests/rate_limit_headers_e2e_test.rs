// ABOUTME: Drives the production HTTP app (build_http_app) to pin X-RateLimit-* and 429 Retry-After per credential
// ABOUTME: Covers API keys, JWTs, MCP, A2A, usage rows' real outcomes, CORS exposure and the messaging leak guard
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! End-to-end tests for the request-budget headers.
//!
//! Every request goes through [`ProviderToolRouter::build_http_app`], the
//! composition the server serves, so these tests pin the install point of the
//! header layer and not only the layer: a credential that authenticates on any
//! route reports its budget, a spent one is a 429 with `Retry-After` on every
//! transport (REST, MCP, A2A), and nothing is reported for a caller that never
//! authenticated or for a messaging vendor. An A2A client-credentials token
//! carries no budget at all; that gap is registered as registre#620 on
//! `A2AServer::resolve_client_principal`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_REQUEST_METHOD,
    AUTHORIZATION, CONTENT_TYPE, ORIGIN, WWW_AUTHENTICATE,
};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use futures_util::{stream, StreamExt, TryStreamExt};
use hmac::{Hmac, Mac};
use pierre_auth::api_keys::{ApiKey, ApiKeyManager, ApiKeyTier, ApiKeyUsage};
use pierre_config::environment::{CorsConfig, ServerConfig};
use pierre_core::models::usage::JwtUsage;
use pierre_core::models::{RequestLog, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::backends::{
    CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
};
use pierre_database::repositories::analytics::next_utc_month_start;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use sha2::Sha256;
use tower::ServiceExt;
use uuid::Uuid;

/// The window every test key is given, one hour.
const KEY_WINDOW_SECS: u32 = 3_600;

/// The browser origin the CORS test configures and sends.
const BROWSER_ORIGIN: &str = "https://app.dravr.ai";

/// An active user on `tier` who owns a tenant, with a JWT naming that tenant.
struct Athlete {
    user: User,
    token: String,
}

async fn athlete(resources: &Arc<ServerContext>, tier: UserTier, role: UserRole) -> Athlete {
    let mut user = User::new(
        format!("budget+{}@example.com", Uuid::new_v4()),
        "not-a-real-hash".to_owned(),
        Some("Budget Athlete".to_owned()),
    );
    user.tier = tier;
    user.role = role;
    user.is_admin = role.is_admin_or_higher();
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());

    let repos = &resources.common.repos;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("Budget tenant {tenant_id}"),
            slug: format!("budget-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
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
    Athlete { user, token }
}

/// A stored API key for `user_id` admitting `limit` calls per hour, and the
/// full key to present.
async fn api_key(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tier: ApiKeyTier,
    limit: u32,
    trial: bool,
) -> (ApiKey, String) {
    let data = ApiKeyManager::new().generate_api_key(trial);
    let api_key = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: format!("{tier:?} budget key"),
        key_prefix: data.key_prefix,
        key_hash: data.key_hash,
        description: None,
        tier,
        rate_limit_requests: limit,
        rate_limit_window_seconds: KEY_WINDOW_SECS,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    resources
        .common
        .repos
        .api_keys
        .create(&api_key)
        .await
        .unwrap();
    (api_key, data.full_key)
}

async fn seed_key_calls(
    resources: &Arc<ServerContext>,
    api_key_id: &str,
    count: u32,
    at: DateTime<Utc>,
) {
    for i in 0..count {
        resources
            .common
            .repos
            .usage
            .record_api_key(&ApiKeyUsage {
                id: None,
                api_key_id: api_key_id.to_owned(),
                timestamp: at,
                tool_name: format!("seeded_{i}"),
                response_time_ms: None,
                status_code: 200,
                error_message: None,
                request_size_bytes: None,
                response_size_bytes: None,
                ip_address: None,
                user_agent: None,
            })
            .await
            .unwrap();
    }
}

async fn key_calls(resources: &Arc<ServerContext>, api_key_id: &str) -> u32 {
    resources
        .common
        .repos
        .usage
        .get_api_key_window_usage(api_key_id, Utc::now() - Duration::days(1))
        .await
        .unwrap()
        .count
}

/// `count` month-to-date JWT requests for `user_id`, written the way
/// authentication writes them.
async fn seed_jwt_calls(resources: &Arc<ServerContext>, user_id: Uuid, count: u32) {
    /// Writes in flight; SQLite serialises them regardless.
    const WRITE_CONCURRENCY: usize = 8;
    let usage = Arc::clone(&resources.common.repos.usage);
    stream::iter(0..count)
        .map(|_| {
            let usage = Arc::clone(&usage);
            async move {
                usage
                    .record_jwt_usage(&JwtUsage {
                        id: None,
                        user_id,
                        timestamp: Utc::now(),
                        endpoint: "http:jwt".to_owned(),
                        method: "AUTH".to_owned(),
                        status_code: 200,
                        response_time_ms: None,
                        request_size_bytes: None,
                        response_size_bytes: None,
                        ip_address: None,
                        user_agent: None,
                    })
                    .await
            }
        })
        .buffer_unordered(WRITE_CONCURRENCY)
        .try_collect::<Vec<()>>()
        .await
        .unwrap();
}

/// What a response carried: status, headers and JSON body (`Null` when the
/// body is not JSON).
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    fn numeric_header(&self, name: &str) -> i64 {
        self.header(name)
            .unwrap_or_else(|| panic!("{name} missing, status {}", self.status))
            .parse()
            .unwrap()
    }

    fn assert_no_budget_headers(&self, context: &str) {
        for name in [
            "x-ratelimit-limit",
            "x-ratelimit-remaining",
            "x-ratelimit-reset",
            "retry-after",
        ] {
            assert!(
                self.headers.get(name).is_none(),
                "{context}: {name} must not be sent, got {:?}",
                self.headers.get(name)
            );
        }
    }
}

async fn send(app: &Router, request: Request<Body>) -> Answer {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer {
        status,
        headers,
        body,
    }
}

fn get_with(uri: &str, headers: &[(&str, &str)]) -> Request<Body> {
    let mut builder = Request::get(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::empty()).unwrap()
}

fn mcp_ping(bearer: Option<&str>) -> Request<Body> {
    let mut builder = Request::post("/mcp").header(CONTENT_TYPE, "application/json");
    if let Some(bearer) = bearer {
        builder = builder.header(AUTHORIZATION, format!("Bearer {bearer}"));
    }
    builder
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string(),
        ))
        .unwrap()
}

/// A JSON-RPC `method` on `POST /mcp` with `bearer`.
fn mcp_request(bearer: &str, method: &str, params: &Value) -> Request<Body> {
    Request::post("/mcp")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {bearer}"))
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string(),
        ))
        .unwrap()
}

/// A JSON-RPC `method` on the A2A binding, `POST /a2a/jsonrpc`, with `bearer`.
fn a2a_request(bearer: &str, method: &str) -> Request<Body> {
    Request::post("/a2a/jsonrpc")
        .header(CONTENT_TYPE, "application/json")
        .header("A2A-Version", "1.0")
        .header(AUTHORIZATION, format!("Bearer {bearer}"))
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {}}).to_string(),
        ))
        .unwrap()
}

/// Every usage row recorded for `api_key_id`.
async fn key_rows(resources: &Arc<ServerContext>, api_key_id: &str) -> Vec<RequestLog> {
    resources
        .common
        .repos
        .usage
        .get_request_logs(None, Some(api_key_id), None, None, None, None)
        .await
        .unwrap()
}

/// Take the JWT usage counter away, so authenticating a JWT fails on the
/// server side rather than on the credential.
async fn drop_jwt_usage(resources: &Arc<ServerContext>) {
    match resources.agent.database.as_ref() {
        Database::SQLite(sqlite) => {
            sqlx::query("DROP TABLE jwt_usage")
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(postgres) => {
            sqlx::query("DROP TABLE jwt_usage CASCADE")
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// Seconds between `a` and `b`, either way round.
fn seconds_apart(a: i64, b: i64) -> i64 {
    (a - b).abs()
}

#[tokio::test]
async fn test_api_key_request_returns_numeric_rate_limit_headers() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let owner = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let (key, full_key) = api_key(&resources, owner.user.id, ApiKeyTier::Starter, 5, false).await;
    let seeded_at = Utc::now().trunc_subsecs(0) - Duration::seconds(600);
    seed_key_calls(&resources, &key.id, 1, seeded_at).await;

    let first = send(
        &app,
        get_with("/api/usage/status", &[("authorization", &full_key)]),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(first.header("x-ratelimit-limit"), Some("5"));
    // One seeded call and this request: 5 - 2
    assert_eq!(first.header("x-ratelimit-remaining"), Some("3"));
    let expected_reset = (seeded_at + Duration::seconds(i64::from(KEY_WINDOW_SECS))).timestamp();
    assert!(
        seconds_apart(first.numeric_header("x-ratelimit-reset"), expected_reset) <= 2,
        "the window frees its first slot when the seeded call leaves it"
    );
    assert!(first.header("retry-after").is_none());
    assert_eq!(key_calls(&resources, &key.id).await, 2);

    let second = send(
        &app,
        get_with("/api/usage/status", &[("authorization", &full_key)]),
    )
    .await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.header("x-ratelimit-remaining"), Some("2"));
    assert_eq!(
        key_calls(&resources, &key.id).await,
        3,
        "each admitted request writes exactly one api_key_usage row"
    );
}

#[tokio::test]
async fn test_jwt_bearer_request_returns_numeric_rate_limit_headers() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let athlete = athlete(&resources, UserTier::Starter, UserRole::User).await;
    seed_jwt_calls(&resources, athlete.user.id, 3).await;

    let before = next_utc_month_start(Utc::now()).timestamp();
    let bearer = format!("Bearer {}", athlete.token);
    let answer = send(
        &app,
        get_with("/api/usage/status", &[("authorization", &bearer)]),
    )
    .await;
    let after = next_utc_month_start(Utc::now()).timestamp();

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.header("x-ratelimit-limit"), Some("10000"));
    assert_eq!(answer.header("x-ratelimit-remaining"), Some("9996"));
    let reset = answer.numeric_header("x-ratelimit-reset");
    assert!(
        reset == before || reset == after,
        "reset {reset} is the first instant of next month"
    );
    assert!(answer.header("retry-after").is_none());
}

#[tokio::test]
async fn test_jwt_cookie_request_returns_rate_limit_headers() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let athlete = athlete(&resources, UserTier::Starter, UserRole::User).await;
    seed_jwt_calls(&resources, athlete.user.id, 3).await;

    let before = next_utc_month_start(Utc::now()).timestamp();
    let cookie = format!("auth_token={}", athlete.token);
    let answer = send(&app, get_with("/api/usage/status", &[("cookie", &cookie)])).await;
    let after = next_utc_month_start(Utc::now()).timestamp();

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.header("x-ratelimit-limit"), Some("10000"));
    assert_eq!(answer.header("x-ratelimit-remaining"), Some("9996"));
    let reset = answer.numeric_header("x-ratelimit-reset");
    assert!(reset == before || reset == after);
}

#[tokio::test]
async fn test_unlimited_principal_gets_no_rate_limit_headers() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let enterprise = athlete(&resources, UserTier::Enterprise, UserRole::User).await;

    let bearer = format!("Bearer {}", enterprise.token);
    let by_jwt = send(
        &app,
        get_with("/api/usage/status", &[("authorization", &bearer)]),
    )
    .await;
    assert_eq!(by_jwt.status, StatusCode::OK, "{}", by_jwt.body);
    by_jwt.assert_no_budget_headers("Enterprise user");

    let (_, full_key) = api_key(
        &resources,
        enterprise.user.id,
        ApiKeyTier::Enterprise,
        1,
        false,
    )
    .await;
    let by_key = send(
        &app,
        get_with("/api/usage/status", &[("authorization", &full_key)]),
    )
    .await;
    assert_eq!(by_key.status, StatusCode::OK, "{}", by_key.body);
    by_key.assert_no_budget_headers("Enterprise key");
}

#[tokio::test]
async fn test_rate_limited_api_key_gets_429_with_retry_after_matching_body() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let owner = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let (key, full_key) = api_key(&resources, owner.user.id, ApiKeyTier::Starter, 2, false).await;
    let seeded_at = Utc::now().trunc_subsecs(0) - Duration::seconds(600);
    seed_key_calls(&resources, &key.id, 2, seeded_at).await;

    let answer = send(
        &app,
        get_with("/api/usage/status", &[("authorization", &full_key)]),
    )
    .await;
    let now = Utc::now();

    assert_eq!(answer.status, StatusCode::TOO_MANY_REQUESTS, "not a 401");
    assert_eq!(answer.body["code"], "RateLimitExceeded");
    let retry_after = answer.numeric_header("retry-after");
    assert_eq!(
        answer.body["details"]["retry_after_secs"].as_i64(),
        Some(retry_after),
        "header and body carry one value"
    );
    let expected = (seeded_at + Duration::seconds(i64::from(KEY_WINDOW_SECS)) - now).num_seconds();
    assert!(retry_after >= 1);
    assert!(
        seconds_apart(retry_after, expected) <= 2,
        "retry {retry_after}s, expected about {expected}s"
    );
    assert_eq!(answer.header("x-ratelimit-limit"), Some("2"));
    assert_eq!(answer.header("x-ratelimit-remaining"), Some("0"));
    assert!(
        seconds_apart(
            answer.numeric_header("x-ratelimit-reset"),
            now.timestamp() + retry_after
        ) <= 2
    );
    assert_eq!(
        key_calls(&resources, &key.id).await,
        2,
        "a refused request is not counted"
    );
}

#[tokio::test]
async fn test_rate_limited_jwt_session_restore_and_extractor_get_429_not_401() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let exhausted = athlete(&resources, UserTier::Starter, UserRole::User).await;
    seed_jwt_calls(&resources, exhausted.user.id, 10_000).await;
    let cookie = format!("auth_token={}", exhausted.token);

    for uri in ["/api/auth/session", "/api/usage/status"] {
        let answer = send(&app, get_with(uri, &[("cookie", &cookie)])).await;
        let to_reset = (next_utc_month_start(Utc::now()) - Utc::now()).num_seconds();

        assert_eq!(
            answer.status,
            StatusCode::TOO_MANY_REQUESTS,
            "{uri}: a spent budget is a 429, never a sign-out 401: {}",
            answer.body
        );
        assert_eq!(answer.body["code"], "RateLimitExceeded", "{uri}");
        let retry_after = answer.numeric_header("retry-after");
        assert_eq!(
            answer.body["details"]["retry_after_secs"].as_i64(),
            Some(retry_after),
            "{uri}"
        );
        assert!(
            seconds_apart(retry_after, to_reset.max(1)) <= 2,
            "{uri}: retry {retry_after}s, the month resets in {to_reset}s"
        );
        assert_eq!(answer.header("x-ratelimit-limit"), Some("10000"), "{uri}");
        assert_eq!(answer.header("x-ratelimit-remaining"), Some("0"), "{uri}");
    }

    // The exhausted cookie plus a valid header credential: the header admits
    // the request and its numbers are the ones reported.
    let fresh = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let bearer = format!("Bearer {}", fresh.token);
    let answer = send(
        &app,
        get_with(
            "/api/auth/session",
            &[("cookie", &cookie), ("authorization", &bearer)],
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(
        answer.body["user"]["id"].as_str(),
        Some(fresh.user.id.to_string().as_str())
    );
    assert_eq!(answer.header("x-ratelimit-limit"), Some("10000"));
    assert_eq!(
        answer.header("x-ratelimit-remaining"),
        Some("9999"),
        "the credential that admitted the request is the one reported"
    );
}

#[tokio::test]
async fn test_rate_limited_admin_config_request_gets_429() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let admin = athlete(&resources, UserTier::Starter, UserRole::Admin).await;
    let (key, full_key) = api_key(&resources, admin.user.id, ApiKeyTier::Starter, 1, false).await;
    seed_key_calls(&resources, &key.id, 1, Utc::now() - Duration::seconds(60)).await;

    let answer = send(
        &app,
        get_with("/api/admin/config/catalog", &[("authorization", &full_key)]),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::TOO_MANY_REQUESTS,
        "the admin config route keeps the 429: {}",
        answer.body
    );
    assert_eq!(answer.body["code"], "RateLimitExceeded");
    let retry_after = answer.numeric_header("retry-after");
    assert!(retry_after >= 1);
    assert_eq!(
        answer.body["details"]["retry_after_secs"].as_i64(),
        Some(retry_after)
    );
}

#[tokio::test]
async fn test_mcp_request_carries_rate_limit_headers() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let owner = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let (_, full_key) = api_key(&resources, owner.user.id, ApiKeyTier::Starter, 5, false).await;

    let answer = send(&app, mcp_ping(Some(&full_key))).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert!(answer.body["error"].is_null(), "{}", answer.body);
    assert_eq!(answer.header("x-ratelimit-limit"), Some("5"));
    assert_eq!(answer.header("x-ratelimit-remaining"), Some("4"));
}

#[tokio::test]
async fn test_mcp_trial_key_authenticates_and_carries_headers() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let owner = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let (_, full_key) = api_key(&resources, owner.user.id, ApiKeyTier::Trial, 5, true).await;
    assert!(full_key.starts_with("pk_trial_"));

    let answer = send(&app, mcp_ping(Some(&full_key))).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "a trial key authenticates on MCP like a live key: {}",
        answer.body
    );
    assert_eq!(answer.header("x-ratelimit-limit"), Some("5"));
    assert_eq!(answer.header("x-ratelimit-remaining"), Some("4"));
}

#[tokio::test]
async fn test_unauthenticated_routes_carry_no_rate_limit_headers() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);

    let health = send(&app, get_with("/health", &[])).await;
    assert_eq!(health.status, StatusCode::OK);
    health.assert_no_budget_headers("/health");

    let anonymous = send(&app, get_with("/api/usage/status", &[])).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    anonymous.assert_no_budget_headers("/api/usage/status without a credential");

    let mcp = send(&app, mcp_ping(None)).await;
    assert_eq!(mcp.status, StatusCode::UNAUTHORIZED);
    assert!(mcp.headers.get(WWW_AUTHENTICATE).is_some());
    mcp.assert_no_budget_headers("POST /mcp without a bearer");
}

#[tokio::test]
async fn test_cross_origin_response_exposes_rate_limit_headers() {
    let resources = common::create_test_server_resources_with_config(ServerConfig {
        activity_fetch_limit: 100,
        cors: CorsConfig {
            allowed_origins: BROWSER_ORIGIN.to_owned(),
            allow_localhost_dev: false,
        },
        ..ServerConfig::default()
    })
    .await
    .unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let athlete = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let bearer = format!("Bearer {}", athlete.token);

    let answer = send(
        &app,
        get_with(
            "/api/usage/status",
            &[("origin", BROWSER_ORIGIN), ("authorization", &bearer)],
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(
        answer.header(ACCESS_CONTROL_EXPOSE_HEADERS.as_str()),
        Some("www-authenticate,retry-after,x-ratelimit-limit,x-ratelimit-remaining,x-ratelimit-reset")
    );
    assert_eq!(answer.header("x-ratelimit-limit"), Some("10000"));
    assert_eq!(answer.header("x-ratelimit-remaining"), Some("9999"));

    let preflight = send(
        &app,
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/api/usage/status")
            .header(ORIGIN, BROWSER_ORIGIN)
            .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        preflight.header(ACCESS_CONTROL_ALLOW_ORIGIN.as_str()),
        Some(BROWSER_ORIGIN)
    );
    preflight.assert_no_budget_headers("a preflight never authenticates");
}

#[tokio::test]
async fn test_messaging_webhook_through_full_app_carries_no_rate_limit_headers() {
    const WEBHOOK_SECRET: &str = "wa_budget_leak_secret";
    const SENDER: &str = "15550009001";

    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let linked = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(linked.user.id)
        .await
        .unwrap()[0]
        .id;
    let messaging: &dyn MessagingRepository = &*resources.common.repos.messaging;
    messaging
        .upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("wa_budget_token"),
            api_secret: None,
            webhook_secret: Some(WEBHOOK_SECRET),
            verify_token: None,
            account_id: Some("wa_budget_business_id"),
            phone_number: Some("15550000009"),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
    messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &linked.user.id.to_string(),
            channel_type: "whatsapp",
            channel_user_id: SENDER,
            display_name: Some("Linked Athlete"),
        })
        .await
        .unwrap();

    let payload = json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "wa_budget_business_id",
            "changes": [{
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "+15550000009",
                        "phone_number_id": "15550000009"
                    },
                    "messages": [{
                        "from": SENDER,
                        "id": "wamid.budget_leak_1",
                        "timestamp": "1234567890",
                        "type": "text",
                        "text": { "body": "how is my week looking?" }
                    }]
                },
                "field": "messages"
            }]
        }]
    });
    let body = serde_json::to_vec(&payload).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).unwrap();
    mac.update(&body);
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    let answer = send(
        &app,
        Request::post("/api/messaging/webhook/whatsapp")
            .header(CONTENT_TYPE, "application/json")
            .header("x-hub-signature-256", signature)
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    answer.assert_no_budget_headers("a messaging vendor's response");
    let used = resources
        .common
        .repos
        .usage
        .get_jwt_current_usage(linked.user.id)
        .await
        .unwrap();
    assert_eq!(
        used, 1,
        "the channel path authenticated and counted the linked athlete's turn, and reported nothing"
    );
}

#[tokio::test]
async fn test_database_failure_during_auth_is_not_401() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let athlete = athlete(&resources, UserTier::Starter, UserRole::User).await;

    // The usage counter going away is a server fault while authenticating,
    // not a refused credential.
    drop_jwt_usage(&resources).await;

    let bearer = format!("Bearer {}", athlete.token);
    let answer = send(
        &app,
        get_with("/api/usage/status", &[("authorization", &bearer)]),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a database blip must not sign the client out: {}",
        answer.body
    );
    assert_eq!(answer.body["code"], "DatabaseError");
    assert_eq!(
        answer.body["message"], "Database operation failed",
        "the internal detail stays out of the body"
    );
    answer.assert_no_budget_headers("a request whose budget could not be read");
}

#[tokio::test]
async fn test_api_key_usage_row_records_the_real_outcome() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let owner = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let (key, full_key) = api_key(&resources, owner.user.id, ApiKeyTier::Starter, 5, false).await;

    // A route the key is admitted to whose handler fails.
    let missing = Uuid::new_v4();
    let answer = send(
        &app,
        get_with(
            &format!("/api/chat/conversations/{missing}"),
            &[("authorization", &full_key)],
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
    let rows = key_rows(&resources, &key.id).await;
    assert_eq!(rows.len(), 1, "one row per admitted request: {rows:?}");
    assert_eq!(
        rows[0].status_code, 404,
        "the handler's status, never an invented 200"
    );
    assert_eq!(
        rows[0].tool_name, "GET /api/chat/conversations/{conversation_id}",
        "the route the request reached, as its template"
    );
    assert!(rows[0].response_time_ms.is_some(), "the request's latency");

    // An MCP tool call names the tool, not `POST /mcp`.
    let tool = send(
        &app,
        mcp_request(
            &full_key,
            "tools/call",
            &json!({"name": "get_connection_status", "arguments": {}}),
        ),
    )
    .await;
    assert_eq!(tool.status, StatusCode::OK, "{}", tool.body);
    let rows = key_rows(&resources, &key.id).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    let tool_row = rows
        .iter()
        .find(|row| row.tool_name == "get_connection_status")
        .unwrap_or_else(|| panic!("the tool call's row names the tool: {rows:?}"));
    assert_eq!(tool_row.status_code, 200);
    assert!(
        rows.iter().all(|row| row.tool_name != "http:api_key"),
        "no placeholder endpoint: {rows:?}"
    );
}

#[tokio::test]
async fn test_exhausted_key_on_mcp_gets_429_with_retry_after_not_invalid_token() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let owner = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let (key, full_key) = api_key(&resources, owner.user.id, ApiKeyTier::Starter, 1, false).await;
    let seeded_at = Utc::now().trunc_subsecs(0) - Duration::seconds(600);
    seed_key_calls(&resources, &key.id, 1, seeded_at).await;

    let post = send(&app, mcp_ping(Some(&full_key))).await;
    let now = Utc::now();
    assert_eq!(
        post.status,
        StatusCode::TOO_MANY_REQUESTS,
        "a spent budget on POST /mcp is a 429, never a 401: {}",
        post.body
    );
    assert!(
        post.headers.get(WWW_AUTHENTICATE).is_none(),
        "no invalid_token challenge: the token is good, the budget is spent"
    );
    let retry_after = post.numeric_header("retry-after");
    assert_eq!(
        post.body["error"]["data"]["retry_after_secs"].as_i64(),
        Some(retry_after),
        "header and JSON-RPC error carry one value: {}",
        post.body
    );
    let expected = (seeded_at + Duration::seconds(i64::from(KEY_WINDOW_SECS)) - now).num_seconds();
    assert!(
        seconds_apart(retry_after, expected) <= 2,
        "retry {retry_after}s, expected about {expected}s"
    );
    assert_eq!(post.header("x-ratelimit-limit"), Some("1"));
    assert_eq!(post.header("x-ratelimit-remaining"), Some("0"));

    let bearer = format!("Bearer {full_key}");
    let tools = send(&app, get_with("/mcp/tools", &[("authorization", &bearer)])).await;
    assert_eq!(
        tools.status,
        StatusCode::TOO_MANY_REQUESTS,
        "GET /mcp/tools answers as POST /mcp does: {}",
        tools.body
    );
    assert!(tools.headers.get(WWW_AUTHENTICATE).is_none());
    let tools_retry = tools.numeric_header("retry-after");
    assert_eq!(tools.body["retry_after_secs"].as_i64(), Some(tools_retry));
    assert!(seconds_apart(tools_retry, retry_after) <= 2);
    assert_eq!(tools.header("x-ratelimit-remaining"), Some("0"));

    assert_eq!(
        key_calls(&resources, &key.id).await,
        1,
        "a refused request is not counted"
    );
}

#[tokio::test]
async fn test_database_failure_during_mcp_auth_is_500_not_invalid_token() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let athlete = athlete(&resources, UserTier::Starter, UserRole::User).await;
    drop_jwt_usage(&resources).await;

    let post = send(&app, mcp_ping(Some(&athlete.token))).await;
    assert_eq!(
        post.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a database blip must not tell the client its token is dead: {}",
        post.body
    );
    assert!(post.headers.get(WWW_AUTHENTICATE).is_none());
    assert_eq!(post.body["error"]["code"], -32_603);
    assert_eq!(
        post.body["error"]["message"], "Database operation failed",
        "the internal detail stays out of the body"
    );
    post.assert_no_budget_headers("a request whose budget could not be read");

    let bearer = format!("Bearer {}", athlete.token);
    let tools = send(&app, get_with("/mcp/tools", &[("authorization", &bearer)])).await;
    assert_eq!(
        tools.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        tools.body
    );
    assert!(tools.headers.get(WWW_AUTHENTICATE).is_none());
    assert_eq!(tools.body["error"], "server_error");
}

#[tokio::test]
async fn test_a2a_user_jwt_reports_its_budget_and_a_spent_one_gets_429() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);

    let admitted = athlete(&resources, UserTier::Starter, UserRole::User).await;
    seed_jwt_calls(&resources, admitted.user.id, 3).await;
    let card = send(&app, a2a_request(&admitted.token, "GetExtendedAgentCard")).await;
    assert_eq!(card.status, StatusCode::OK, "{}", card.body);
    assert!(card.body["error"].is_null(), "{}", card.body);
    assert_eq!(card.header("x-ratelimit-limit"), Some("10000"));
    assert_eq!(card.header("x-ratelimit-remaining"), Some("9996"));
    assert_eq!(
        resources
            .common
            .repos
            .usage
            .get_jwt_current_usage(admitted.user.id)
            .await
            .unwrap(),
        4,
        "the A2A request counts against the month like any other"
    );

    let exhausted = athlete(&resources, UserTier::Starter, UserRole::User).await;
    seed_jwt_calls(&resources, exhausted.user.id, 10_000).await;
    let refused = send(&app, a2a_request(&exhausted.token, "GetExtendedAgentCard")).await;
    let to_reset = (next_utc_month_start(Utc::now()) - Utc::now()).num_seconds();
    assert_eq!(
        refused.status,
        StatusCode::TOO_MANY_REQUESTS,
        "a spent budget cannot keep running through A2A: {}",
        refused.body
    );
    let retry_after = refused.numeric_header("retry-after");
    assert!(
        seconds_apart(retry_after, to_reset.max(1)) <= 2,
        "retry {retry_after}s, the month resets in {to_reset}s"
    );
    assert_eq!(
        refused.body["error"]["data"][0]["reason"], "RATE_LIMIT_EXCEEDED",
        "{}",
        refused.body
    );
    assert_eq!(
        refused.body["error"]["data"][1]["retryDelay"],
        format!("{retry_after}s"),
        "the RetryInfo detail and the header carry one value"
    );
    assert_eq!(refused.header("x-ratelimit-remaining"), Some("0"));

    // The HTTP+JSON binding maps the same refusal to RESOURCE_EXHAUSTED.
    let bearer = format!("Bearer {}", exhausted.token);
    let rest = send(
        &app,
        get_with("/a2a/tasks?A2A-Version=1.0", &[("authorization", &bearer)]),
    )
    .await;
    assert_eq!(rest.status, StatusCode::TOO_MANY_REQUESTS, "{}", rest.body);
    assert_eq!(rest.body["error"]["status"], "RESOURCE_EXHAUSTED");
    assert!(rest.numeric_header("retry-after") >= 1);
}

#[tokio::test]
async fn test_concurrent_api_key_requests_each_write_one_row() {
    const REQUESTS: usize = 10;
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let owner = athlete(&resources, UserTier::Starter, UserRole::User).await;
    let (key, full_key) = api_key(&resources, owner.user.id, ApiKeyTier::Starter, 100, false).await;

    let statuses: Vec<StatusCode> = stream::iter(0..REQUESTS)
        .map(|_| {
            let app = app.clone();
            let full_key = full_key.clone();
            async move {
                send(
                    &app,
                    get_with("/api/usage/status", &[("authorization", &full_key)]),
                )
                .await
                .status
            }
        })
        .buffer_unordered(REQUESTS)
        .collect()
        .await;
    assert!(
        statuses.iter().all(|status| *status == StatusCode::OK),
        "{statuses:?}"
    );

    let rows = key_rows(&resources, &key.id).await;
    assert_eq!(
        rows.len(),
        REQUESTS,
        "one row per admitted request: {rows:?}"
    );
    assert!(rows
        .iter()
        .all(|row| row.status_code == 200 && row.tool_name == "GET /api/usage/status"));
}
