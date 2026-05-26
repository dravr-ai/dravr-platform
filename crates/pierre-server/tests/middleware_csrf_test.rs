// ABOUTME: Unit tests for the pure-function CSRF validator
// ABOUTME: Covers method-class gating + header extraction + token verification
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use axum::http::{HeaderMap, Method};
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_middleware::csrf::validate_csrf_token;
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn test_csrf_validator_get_request() {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let headers = HeaderMap::new();
    let user_id = Uuid::new_v4();

    // GET requests should not require CSRF token
    let result = validate_csrf_token(&headers, &Method::GET, user_id, &csrf_manager);

    assert!(result.is_ok(), "GET request should not require CSRF token");
}

#[test]
fn test_csrf_validator_post_without_token() {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let headers = HeaderMap::new();
    let user_id = Uuid::new_v4();

    // POST without CSRF token should fail
    let result = validate_csrf_token(&headers, &Method::POST, user_id, &csrf_manager);

    assert!(
        result.is_err(),
        "POST request without CSRF token should fail"
    );
}

#[test]
fn test_csrf_validator_post_with_valid_token() -> anyhow::Result<()> {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let user_id = Uuid::new_v4();
    let token = csrf_manager.generate_token(user_id)?;

    let mut headers = HeaderMap::new();
    headers.insert("X-CSRF-Token", token.parse()?);

    // POST with valid CSRF token should succeed
    let result = validate_csrf_token(&headers, &Method::POST, user_id, &csrf_manager);

    assert!(
        result.is_ok(),
        "POST request with valid CSRF token should succeed"
    );
    Ok(())
}

#[test]
fn test_csrf_validator_post_with_invalid_token() -> anyhow::Result<()> {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let user_id = Uuid::new_v4();

    let mut headers = HeaderMap::new();
    headers.insert("X-CSRF-Token", "invalid_token".parse()?);

    // POST with invalid CSRF token should fail
    let result = validate_csrf_token(&headers, &Method::POST, user_id, &csrf_manager);

    assert!(
        result.is_err(),
        "POST request with invalid CSRF token should fail"
    );
    Ok(())
}

#[test]
fn test_csrf_validator_method_class_gating() {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let headers = HeaderMap::new();
    let user_id = Uuid::new_v4();

    // State-changing methods reject empty-headers requests
    for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
        let result = validate_csrf_token(&headers, &method, user_id, &csrf_manager);
        assert!(
            result.is_err(),
            "{method} should require a CSRF token (got Ok)"
        );
    }

    // Read-only methods bypass the check entirely
    for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
        let result = validate_csrf_token(&headers, &method, user_id, &csrf_manager);
        assert!(
            result.is_ok(),
            "{method} should not require a CSRF token (got Err)"
        );
    }
}
