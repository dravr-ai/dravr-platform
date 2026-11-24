// ABOUTME: HTTP integration tests for the health probe the server serves
// ABOUTME: Drives /health through the production app, with no credentials
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The probe a deployment points at, asserted on the application the server
//! serves (`ProviderToolRouter::build_http_app`) rather than on a router
//! assembled for the test.
//!
//! `/health` is liveness: it answers 200 whatever the LLM probe found, so a
//! slow or failing model never restarts a healthy process.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::Value;

/// The name the probe reports the service under.
const SERVICE_NAME: &str = "pierre-mcp-server";

/// A test server's resources and the production app built over them.
async fn served_app() -> (Arc<ServerContext>, Router) {
    let resources = create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    (resources, app)
}

#[tokio::test]
async fn health_answers_ok_without_credentials() {
    let (_resources, app) = served_app().await;

    let response = AxumTestRequest::get("/health").send(app).await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Value = response.json();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], SERVICE_NAME);
}

#[tokio::test]
async fn health_stays_ok_while_the_llm_probe_reports_a_failure() {
    let (resources, app) = served_app().await;
    resources
        .common
        .llm_health
        .record_unhealthy("gemini", "probe failed")
        .await;

    let response = AxumTestRequest::get("/health").send(app).await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "liveness must not depend on the LLM probe"
    );
}

#[tokio::test]
async fn the_plugin_status_path_is_not_served() {
    let (_resources, app) = served_app().await;

    let response = AxumTestRequest::get("/health/plugins").send(app).await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}
