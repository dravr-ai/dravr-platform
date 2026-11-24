// ABOUTME: HTTP integration tests for the health and readiness probes the server serves
// ABOUTME: Drives /health, /ready and /health/llm through the production app, with no credentials
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The probes a deployment points at, asserted on the application the server
//! serves (`ProviderToolRouter::build_http_app`) rather than on a router
//! assembled for the test.
//!
//! `/health` is liveness: it answers 200 whatever the LLM probe found, so a
//! slow or failing model never restarts a healthy process. `/ready` is the
//! readiness gate: 200 while the probe has not reported or has succeeded,
//! 503 once it has reported a failure. `/health/llm` is the probe's snapshot.

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

/// The name the probes report the service under.
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
async fn ready_answers_ok_before_the_llm_probe_has_reported() {
    let (_resources, app) = served_app().await;

    let response = AxumTestRequest::get("/ready").send(app).await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Value = response.json();
    assert_eq!(body["status"], "unknown");
    assert_eq!(body["service"], SERVICE_NAME);
    assert_eq!(body["llm"]["status"], "unknown");
}

#[tokio::test]
async fn ready_answers_ok_once_the_llm_probe_succeeded() {
    let (resources, app) = served_app().await;
    resources
        .common
        .llm_health
        .record_healthy_served("gemini", "gemini")
        .await;

    let response = AxumTestRequest::get("/ready").send(app).await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Value = response.json();
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["llm"]["provider"], "gemini");
    assert_eq!(body["llm"]["served_by"], "gemini");
}

#[tokio::test]
async fn ready_answers_unavailable_once_the_llm_probe_failed() {
    let (resources, app) = served_app().await;
    resources
        .common
        .llm_health
        .record_unhealthy("gemini", "probe failed\nsecond line")
        .await;

    let response = AxumTestRequest::get("/ready").send(app).await;

    assert_eq!(response.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = response.json();
    assert_eq!(body["status"], "unhealthy");
    assert_eq!(body["service"], SERVICE_NAME);
    assert_eq!(body["llm"]["error"], "probe failed");
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
async fn llm_health_answers_the_probe_snapshot() {
    let (resources, app) = served_app().await;
    resources
        .common
        .llm_health
        .record_unhealthy("gemini", "probe failed")
        .await;

    let response = AxumTestRequest::get("/health/llm").send(app).await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: Value = response.json();
    assert_eq!(body["status"], "unhealthy");
    assert_eq!(body["provider"], "gemini");
    assert_eq!(body["error"], "probe failed");
}

#[tokio::test]
async fn the_plugin_status_path_is_not_served() {
    let (_resources, app) = served_app().await;

    let response = AxumTestRequest::get("/health/plugins").send(app).await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}
