// ABOUTME: HTTP integration tests for dashboard routes
// ABOUTME: Tests the dashboard analytics endpoints with authentication and query parameters
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! Comprehensive HTTP integration tests for dashboard routes
//!
//! This test suite validates that all dashboard endpoints are correctly registered
//! in the router and handle HTTP requests appropriately.

use crate::common;
use crate::helpers;

use chrono::{Duration, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::models::{ApiKey, ApiKeyUsage};
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_dashboard::DashboardRoutes;
use std::sync::Arc;
use uuid::Uuid;

/// Test setup helper for dashboard route testing
struct DashboardTestSetup {
    resources: Arc<ServerContext>,
    jwt_token: String,
    user_id: Uuid,
}

impl DashboardTestSetup {
    async fn new() -> anyhow::Result<Self> {
        common::init_server_config();
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();
        let cache = common::create_test_cache().await?;

        // Create test user
        let (user_id, user) = common::create_test_user(&database).await?;

        // Create ServerContext
        let temp_dir = tempfile::tempdir()?;
        let config = Arc::new(ServerConfig {
            http_port: 8081,
            database: DatabaseConfig {
                url: DatabaseUrl::Memory,
                backup: BackupConfig {
                    directory: temp_dir.path().to_path_buf(),
                    ..Default::default()
                },
                ..Default::default()
            },
            app_behavior: AppBehaviorConfig {
                ci_mode: true,
                auto_approve_users: false,
                ..Default::default()
            },
            security: SecurityConfig {
                headers: SecurityHeadersConfig {
                    environment: Environment::Testing,
                },
                ..Default::default()
            },
            ..Default::default()
        });

        let resources = Arc::new(
            ServerContext::new(
                (*database).clone(),
                (*auth_manager).clone(),
                "test_jwt_secret",
                config,
                cache,
                ServerContextOptions {
                    rsa_key_size_bits: Some(2048),
                    jwks_manager: Some(common::get_shared_test_jwks()),
                    llm_provider: None,
                    chat_provider: None,
                    extra_tools: Vec::new(),
                    billing_provider: None,
                    turn_runner: None,
                    archive_imports: None,
                },
            )
            .await
            .unwrap(),
        );

        // Generate JWT token for the user
        let jwt_token = auth_manager
            .generate_token(&user, &resources.auth.jwks_manager)
            .map_err(|e| anyhow::anyhow!("Failed to generate JWT: {}", e))?;

        // Create test API keys for dashboard data
        common::create_and_store_test_api_key(database.as_ref(), user_id, "Test Dashboard Key")
            .await?;

        Ok(Self {
            resources,
            jwt_token,
            user_id,
        })
    }

    fn routes(&self) -> axum::Router {
        DashboardRoutes::routes::<ServerContext>().with_state(self.resources.clone())
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.jwt_token)
    }
}

// ============================================================================
// GET /api/dashboard/analytics - Usage Analytics Tests (with query params)
// ============================================================================

/// Record one call of `tool_name` on `key`, a minute ago.
async fn record_call(setup: &DashboardTestSetup, key: &ApiKey, tool_name: &str) {
    setup
        .resources
        .agent
        .database
        .repositories()
        .usage
        .record_api_key(&ApiKeyUsage {
            id: None,
            api_key_id: key.id.clone(),
            timestamp: Utc::now() - Duration::minutes(1),
            tool_name: tool_name.to_owned(),
            response_time_ms: Some(20),
            status_code: 200,
            error_message: None,
            request_size_bytes: None,
            response_size_bytes: None,
            ip_address: None,
            user_agent: None,
        })
        .await
        .expect("record usage");
}

#[tokio::test]
async fn test_dashboard_top_tools_ranks_calls_across_all_the_users_keys() {
    let setup = DashboardTestSetup::new().await.expect("Setup failed");
    let database = setup.resources.agent.database.as_ref();
    let first = common::create_and_store_test_api_key(database, setup.user_id, "First Key")
        .await
        .expect("create first key");
    let second = common::create_and_store_test_api_key(database, setup.user_id, "Second Key")
        .await
        .expect("create second key");
    record_call(&setup, &first, "get_athlete").await;
    record_call(&setup, &first, "get_athlete").await;
    record_call(&setup, &second, "get_athlete").await;
    record_call(&setup, &second, "get_activities").await;

    let response = AxumTestRequest::get("/api/dashboard/analytics?days=1")
        .header("authorization", &setup.auth_header())
        .send(setup.routes())
        .await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json();
    let ranked: Vec<(&str, u64)> = body["top_tools"]
        .as_array()
        .expect("top_tools is a list")
        .iter()
        .map(|t| {
            (
                t["tool_name"].as_str().unwrap(),
                t["request_count"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        ranked,
        [("get_athlete", 3), ("get_activities", 1)],
        "get_athlete's calls on both keys are summed"
    );
}

#[tokio::test]
async fn test_dashboard_usage_default_days() {
    let setup = DashboardTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::get("/api/dashboard/analytics")
        .header("authorization", &setup.auth_header())
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body["time_series"].is_array());
    assert!(body["average_response_time"].is_number());
    // No `error_rate`: this analytic is derived from `llm_usage`, which has no
    // status column and records nothing at all for a failed call, so the field
    // was a hardcoded zero rather than a measurement.
    assert!(body["error_rate"].is_null());
}

#[tokio::test]
async fn test_dashboard_usage_with_days_param() {
    let setup = DashboardTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::get("/api/dashboard/analytics?days=7")
        .header("authorization", &setup.auth_header())
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body["time_series"].is_array());

    // Should have 7 days of data
    let time_series = body["time_series"].as_array().unwrap();
    assert_eq!(time_series.len(), 7);
}

#[tokio::test]
async fn test_dashboard_usage_different_timeframes() {
    let setup = DashboardTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    for days in [1, 7, 14, 30, 90] {
        let response = AxumTestRequest::get(&format!("/api/dashboard/analytics?days={}", days))
            .header("authorization", &setup.auth_header())
            .send(routes.clone())
            .await;

        assert_eq!(response.status(), 200);

        let body: serde_json::Value = response.json();
        let time_series = body["time_series"].as_array().unwrap();
        assert_eq!(time_series.len(), days);
    }
}

#[tokio::test]
async fn test_dashboard_usage_missing_auth() {
    let setup = DashboardTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::get("/api/dashboard/analytics?days=7")
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);
}

// ============================================================================
// Additional Integration Tests
// ============================================================================

#[tokio::test]
async fn test_dashboard_analytics_refuses_a_window_outside_a_year() {
    let setup = DashboardTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    for (days, expected) in [
        (0, 400),
        (366, 400),
        (4_294_967_295_u32, 400),
        (1, 200),
        (365, 200),
    ] {
        let response = AxumTestRequest::get(&format!("/api/dashboard/analytics?days={days}"))
            .header("authorization", &setup.auth_header())
            .send(routes.clone())
            .await;
        assert_eq!(response.status(), expected, "days={days}");
    }
}

#[tokio::test]
async fn test_dashboard_all_endpoints_authenticated() {
    let setup = DashboardTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let endpoints = vec!["/api/dashboard/analytics"];

    for endpoint in endpoints {
        let response = AxumTestRequest::get(endpoint)
            .header("authorization", &setup.auth_header())
            .send(routes.clone())
            .await;

        assert_eq!(
            response.status(),
            200,
            "Endpoint {} should return 200",
            endpoint
        );
    }
}
