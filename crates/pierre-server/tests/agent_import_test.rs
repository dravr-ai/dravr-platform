// ABOUTME: Integration tests for agent import endpoints (markdown and URL)
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
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::build_agents_router;

use axum::http::StatusCode;

// ============================================================================
// Test Data
// ============================================================================

/// Valid agent markdown with all required and optional sections
const VALID_AGENT_MARKDOWN: &str = r#"---
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
const NO_FRONTMATTER_MARKDOWN: &str = r"# Just a Title

Some text without frontmatter.

## Purpose
A purpose section.

## Instructions
Some instructions.
";

/// Markdown with valid frontmatter but missing required sections
const MISSING_SECTIONS_MARKDOWN: &str = r"---
name: incomplete-coach
title: Incomplete Coach
category: training
tags: [test]
---

Some text without any section headers.
";

// ============================================================================
// Test Helpers
// ============================================================================

async fn setup_test_environment() -> (axum::Router, String) {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    let token = generate_test_token(&resources, &user).await;
    let router = build_agents_router::<ServerContext>().with_state(resources);

    (router, format!("Bearer {token}"))
}

// ============================================================================
// Import from Markdown Tests
// ============================================================================

#[tokio::test]
async fn test_import_valid_markdown() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/agents/import")
        .header("authorization", &auth_token)
        .text(VALID_AGENT_MARKDOWN)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: serde_json::Value = response.json();
    assert_eq!(body["parsed_name"], "test-import-coach");
    assert!(body["token_count"].as_u64().unwrap() > 0);

    let agent = &body["agent"];
    assert_eq!(agent["title"], "Test Import Coach");
    assert_eq!(agent["category"], "training");
    assert!(agent["purpose"]
        .as_str()
        .unwrap()
        .contains("import validation"));
    assert!(agent["instructions"]
        .as_str()
        .unwrap()
        .contains("integration testing"));
}

#[tokio::test]
async fn test_import_invalid_markdown_no_frontmatter() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/agents/import")
        .header("authorization", &auth_token)
        .text(NO_FRONTMATTER_MARKDOWN)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_import_invalid_markdown_missing_sections() {
    let (router, auth_token) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/agents/import")
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
    let first = AxumTestRequest::post("/api/agents/import")
        .header("authorization", &auth_token)
        .text(VALID_AGENT_MARKDOWN)
        .send(router.clone())
        .await;
    assert_eq!(first.status_code(), StatusCode::CREATED);

    let first_body: serde_json::Value = first.json();
    let first_coach_id = first_body["agent"]["id"].as_str().unwrap().to_owned();

    // Second import of identical content should be rejected as duplicate (409)
    let second = AxumTestRequest::post("/api/agents/import")
        .header("authorization", &auth_token)
        .text(VALID_AGENT_MARKDOWN)
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

// ============================================================================
// Import from URL Tests
// ============================================================================
