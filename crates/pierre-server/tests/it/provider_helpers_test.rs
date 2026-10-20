// ABOUTME: Tests for provider helper functions
// ABOUTME: Verifies provider extraction and response creation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_tool_runtime::protocol::provider_helpers::{
    create_auth_error_response, extract_provider,
};

#[test]
fn test_extract_provider_with_value() {
    let mut params = serde_json::Map::new();
    params.insert("provider".to_owned(), serde_json::json!("garmin"));
    assert_eq!(extract_provider(&params), Some("garmin".to_owned()));
}

#[test]
fn test_extract_provider_default_returns_none() {
    // The 2026-05-23 fix: no implicit fallback to synthetic. Callers receive
    // None and must resolve via the user's provider_connections (or surface
    // a reconnect signal). See `resolve_provider_for_request`.
    let params = serde_json::Map::new();
    assert!(extract_provider(&params).is_none());
}

#[test]
fn test_extract_provider_empty_string_returns_none() {
    let mut params = serde_json::Map::new();
    params.insert("provider".to_owned(), serde_json::json!(""));
    assert!(extract_provider(&params).is_none());
}

// ============================================================================
// NEW TESTS: Response Building Functions
// ============================================================================

#[test]
fn test_auth_error_response() {
    let response = create_auth_error_response("strava", "Invalid token");

    assert!(response.success); // Success=true with error in result
    assert!(response.error.is_none());
    assert!(response.result.is_some());

    if let Some(ref result) = response.result {
        assert!(result.get("error").is_some());
        if let Some(error) = result.get("error").and_then(serde_json::Value::as_str) {
            assert!(error.contains("Invalid token"));
        }
        assert_eq!(
            result.get("provider"),
            Some(&serde_json::Value::String("strava".to_owned()))
        );
    }
}

#[test]
fn test_auth_error_response_metadata() {
    let response = create_auth_error_response("garmin", "Token expired");

    assert!(response.metadata.is_some());
    if let Some(ref metadata) = response.metadata {
        assert_eq!(
            metadata.get("authentication_error"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            metadata.get("provider"),
            Some(&serde_json::Value::String("garmin".to_owned()))
        );
    }
}

#[test]
fn test_extract_provider_different_providers() {
    let providers = vec!["strava", "garmin", "whoop", "synthetic"];

    for provider in providers {
        let mut params = serde_json::Map::new();
        params.insert("provider".to_owned(), serde_json::json!(provider));
        assert_eq!(extract_provider(&params), Some(provider.to_owned()));
    }
}
