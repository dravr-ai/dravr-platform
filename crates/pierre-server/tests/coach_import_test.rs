// ABOUTME: Integration tests for coach import endpoints (markdown and URL)
// ABOUTME: Covers import, preview, duplicate detection, and SSRF protection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::routes::coaches::CoachesRoutes;

use axum::http::StatusCode;
use serde_json::json;

// ============================================================================
// Test Data
// ============================================================================

/// Valid coach markdown with all required and optional sections
const VALID_COACH_MARKDOWN: &str = r#"---
name: test-import-coach
title: Test Import Coach
category: training
tags: [test, import]
---

## Purpose
A test coach for import validation.

## Instructions
You are a test coach used for integration testing of the import feature.

## Example Inputs
- "Test question 1"
- "Test question 2"
"#;

/// Markdown without YAML frontmatter delimiters
const NO_FRONTMATTER_MARKDOWN: &str = r#"# Just a Title

Some text without frontmatter.

## Purpose
A purpose section.

## Instructions
Some instructions.
"#;

/// Markdown with valid frontmatter but missing required sections
const MISSING_SECTIONS_MARKDOWN: &str = r#"---
name: incomplete-coach
title: Incomplete Coach
category: training
tags: [test]
---

Some text without any section headers.
"#;

// ============================================================================
// Test Helpers
// ============================================================================

async fn setup_test_environment() -> (axum::Router, String) {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.database).await.unwrap();

    let token = generate_test_token(&resources, &user).await;
    let router = CoachesRoutes::routes(resources);

    (router, format!("Bearer {token}"))
}

// ============================================================================
// Import from Markdown Tests
// ============================================================================

#[tokio::test]
async fn test_import_valid_markdown() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/coaches/import")
        .header("authorization", &auth_token)
        .text(VALID_COACH_MARKDOWN)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: serde_json::Value = response.json();
    assert_eq!(body["parsed_name"], "test-import-coach");
    assert!(body["token_count"].as_u64().unwrap() > 0);

    let coach = &body["coach"];
    assert_eq!(coach["title"], "Test Import Coach");
    assert_eq!(coach["category"], "training");
    assert!(coach["purpose"]
        .as_str()
        .unwrap()
        .contains("import validation"));
    assert!(coach["instructions"]
        .as_str()
        .unwrap()
        .contains("integration testing"));
}

#[tokio::test]
async fn test_import_invalid_markdown_no_frontmatter() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/coaches/import")
        .header("authorization", &auth_token)
        .text(NO_FRONTMATTER_MARKDOWN)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_import_invalid_markdown_missing_sections() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/coaches/import")
        .header("authorization", &auth_token)
        .text(MISSING_SECTIONS_MARKDOWN)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_import_duplicate_content_hash() {
    let (router, auth_token) = setup_test_environment().await;

    // First import succeeds
    let first = AxumTestRequest::post("/api/coaches/import")
        .header("authorization", &auth_token)
        .text(VALID_COACH_MARKDOWN)
        .send(router.clone())
        .await;
    assert_eq!(first.status_code(), StatusCode::CREATED);

    let first_body: serde_json::Value = first.json();
    let first_coach_id = first_body["coach"]["id"].as_str().unwrap().to_owned();

    // Second import of identical content should be rejected as duplicate (409)
    let second = AxumTestRequest::post("/api/coaches/import")
        .header("authorization", &auth_token)
        .text(VALID_COACH_MARKDOWN)
        .send(router.clone())
        .await;
    assert_eq!(second.status_code(), StatusCode::CONFLICT);

    let second_body: serde_json::Value = second.json();
    assert_eq!(second_body["code"], "ResourceAlreadyExists");

    // First import should still be accessible
    assert!(!first_coach_id.is_empty());
}

// ============================================================================
// Import Preview Tests
// ============================================================================

#[tokio::test]
async fn test_preview_valid_markdown() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/coaches/import/preview")
        .header("authorization", &auth_token)
        .text(VALID_COACH_MARKDOWN)
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["valid"], true);
    assert!(!body["duplicate_exists"].as_bool().unwrap());
    assert!(body["token_count"].as_u64().unwrap() > 0);

    let parsed = &body["parsed"];
    assert_eq!(parsed["name"], "test-import-coach");
    assert_eq!(parsed["title"], "Test Import Coach");
    assert_eq!(parsed["category"], "training");
    assert!(parsed["has_instructions"].as_bool().unwrap());
    assert!(parsed["has_example_inputs"].as_bool().unwrap());

    // Verify no side effect: list coaches should be empty
    let list = AxumTestRequest::get("/api/coaches")
        .header("authorization", &auth_token)
        .send(router)
        .await;
    assert_eq!(list.status_code(), StatusCode::OK);
    let list_body: serde_json::Value = list.json();
    assert_eq!(list_body["total"], 0);
}

#[tokio::test]
async fn test_preview_invalid_markdown() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/coaches/import/preview")
        .header("authorization", &auth_token)
        .text(NO_FRONTMATTER_MARKDOWN)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["valid"], false);
    assert!(body["parsed"].is_null());

    let errors = body["errors"].as_array().unwrap();
    assert!(!errors.is_empty());
}

#[tokio::test]
async fn test_preview_detects_duplicate() {
    let (router, auth_token) = setup_test_environment().await;

    // Import a coach first
    let import = AxumTestRequest::post("/api/coaches/import")
        .header("authorization", &auth_token)
        .text(VALID_COACH_MARKDOWN)
        .send(router.clone())
        .await;
    assert_eq!(import.status_code(), StatusCode::CREATED);

    // Preview the same markdown -- should detect the duplicate
    let preview = AxumTestRequest::post("/api/coaches/import/preview")
        .header("authorization", &auth_token)
        .text(VALID_COACH_MARKDOWN)
        .send(router)
        .await;
    assert_eq!(preview.status_code(), StatusCode::OK);

    let body: serde_json::Value = preview.json();
    assert_eq!(body["valid"], true);
    assert!(body["duplicate_exists"].as_bool().unwrap());
    assert!(body["duplicate_coach_id"].as_str().is_some());
}

// ============================================================================
// Import from URL Tests
// ============================================================================

#[tokio::test]
async fn test_import_url_rejects_http() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/coaches/import/url")
        .header("authorization", &auth_token)
        .json(&json!({
            "url": "http://example.com/coach.md",
            "save": true
        }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_import_url_rejects_private_ip() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/coaches/import/url")
        .header("authorization", &auth_token)
        .json(&json!({
            "url": "https://127.0.0.1/coach.md",
            "save": true
        }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}
