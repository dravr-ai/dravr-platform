// ABOUTME: Test suite for provider utilities module
// ABOUTME: Tests type conversions, retry config, authentication helpers, and retryable errors
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::errors::provider::ProviderError;
use pierre_providers::core::{CredentialKind, OAuth2Credentials};
use pierre_providers::utils::{conversions, is_authenticated, needs_token_refresh, RetryConfig};
use reqwest::StatusCode;

#[test]
fn test_f64_to_u64_conversion() {
    assert_eq!(conversions::f64_to_u64(0.0), 0);
    assert_eq!(conversions::f64_to_u64(100.5), 100);
    assert_eq!(conversions::f64_to_u64(1000.9), 1000);
    assert_eq!(conversions::f64_to_u64(-10.0), 0); // Clamps negative to 0
}

#[test]
fn test_f32_to_u32_conversion() {
    assert_eq!(conversions::f32_to_u32(0.0), 0);
    assert_eq!(conversions::f32_to_u32(150.7), 150);
    assert_eq!(conversions::f32_to_u32(-5.0), 0); // Clamps negative to 0
}

#[test]
fn test_f64_to_u32_conversion() {
    assert_eq!(conversions::f64_to_u32(0.0), 0);
    assert_eq!(conversions::f64_to_u32(500.5), 500);
    assert_eq!(conversions::f64_to_u32(-10.0), 0); // Clamps negative to 0
}

#[test]
fn test_needs_token_refresh() {
    // No credentials
    assert!(!needs_token_refresh(&None, 5));

    // Token expires in 1 minute (threshold 5 minutes)
    let expires_soon = Some(OAuth2Credentials {
        client_id: "test".to_owned(),
        client_secret: "secret".to_owned(),
        access_token: Some("token".to_owned()),
        refresh_token: Some("refresh".to_owned()),
        expires_at: Some(Utc::now() + chrono::Duration::minutes(1)),
        scopes: vec![],
        kind: CredentialKind::OAuthBearer,
    });
    assert!(needs_token_refresh(&expires_soon, 5));

    // Token expires in 10 minutes (threshold 5 minutes)
    let expires_later = Some(OAuth2Credentials {
        client_id: "test".to_owned(),
        client_secret: "secret".to_owned(),
        access_token: Some("token".to_owned()),
        refresh_token: Some("refresh".to_owned()),
        expires_at: Some(Utc::now() + chrono::Duration::minutes(10)),
        scopes: vec![],
        kind: CredentialKind::OAuthBearer,
    });
    assert!(!needs_token_refresh(&expires_later, 5));
}

#[test]
fn test_is_authenticated() {
    // No credentials
    assert!(!is_authenticated(&None));

    // No access token
    let no_token = Some(OAuth2Credentials {
        client_id: "test".to_owned(),
        client_secret: "secret".to_owned(),
        access_token: None,
        refresh_token: Some("refresh".to_owned()),
        expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
        scopes: vec![],
        kind: CredentialKind::OAuthBearer,
    });
    assert!(!is_authenticated(&no_token));

    // Expired token
    let expired = Some(OAuth2Credentials {
        client_id: "test".to_owned(),
        client_secret: "secret".to_owned(),
        access_token: Some("token".to_owned()),
        refresh_token: Some("refresh".to_owned()),
        expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
        scopes: vec![],
        kind: CredentialKind::OAuthBearer,
    });
    assert!(!is_authenticated(&expired));

    // Valid token
    let valid = Some(OAuth2Credentials {
        client_id: "test".to_owned(),
        client_secret: "secret".to_owned(),
        access_token: Some("token".to_owned()),
        refresh_token: Some("refresh".to_owned()),
        expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
        scopes: vec![],
        kind: CredentialKind::OAuthBearer,
    });
    assert!(is_authenticated(&valid));

    // No expiry (assume valid)
    let no_expiry = Some(OAuth2Credentials {
        client_id: "test".to_owned(),
        client_secret: "secret".to_owned(),
        access_token: Some("token".to_owned()),
        refresh_token: Some("refresh".to_owned()),
        expires_at: None,
        scopes: vec![],
        kind: CredentialKind::OAuthBearer,
    });
    assert!(is_authenticated(&no_expiry));
}

#[test]
fn test_retry_config_default() {
    let config = RetryConfig::default();
    assert_eq!(config.max_retries, 3);
    assert_eq!(config.initial_backoff_ms, 1000);
    assert_eq!(config.estimated_block_duration_secs, 3600);
    assert!(config
        .retryable_status_codes
        .contains(&StatusCode::TOO_MANY_REQUESTS));
}

#[test]
fn test_retry_config_custom() {
    let config = RetryConfig {
        max_retries: 5,
        initial_backoff_ms: 500,
        retryable_status_codes: vec![
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::SERVICE_UNAVAILABLE,
        ],
        estimated_block_duration_secs: 7200,
    };

    assert_eq!(config.max_retries, 5);
    assert_eq!(config.initial_backoff_ms, 500);
    assert_eq!(config.estimated_block_duration_secs, 7200);
    assert!(config
        .retryable_status_codes
        .contains(&StatusCode::TOO_MANY_REQUESTS));
    assert!(config
        .retryable_status_codes
        .contains(&StatusCode::SERVICE_UNAVAILABLE));
}

#[test]
fn test_conversions_boundary_values() {
    // Test maximum values
    assert_eq!(conversions::f64_to_u64(f64::MAX), u64::MAX);
    assert_eq!(conversions::f32_to_u32(f32::MAX), u32::MAX);
    assert_eq!(conversions::f64_to_u32(f64::from(u32::MAX) + 1.0), u32::MAX);

    // Test zero
    assert_eq!(conversions::f64_to_u64(0.0), 0);
    assert_eq!(conversions::f32_to_u32(0.0), 0);
    assert_eq!(conversions::f64_to_u32(0.0), 0);

    // Test negative values (should clamp to 0)
    assert_eq!(conversions::f64_to_u64(-100.0), 0);
    assert_eq!(conversions::f32_to_u32(-100.0), 0);
    assert_eq!(conversions::f64_to_u32(-100.0), 0);
}

#[test]
fn test_provider_error_is_retryable() {
    // Retryable errors
    assert!(ProviderError::NetworkError("network issue".to_owned()).is_retryable());
    assert!(ProviderError::RateLimitExceeded {
        provider: "test".to_owned(),
        retry_after_secs: 60,
        limit_type: "hourly".to_owned(),
    }
    .is_retryable());
    assert!(ProviderError::Timeout {
        provider: "test".to_owned(),
        operation: "fetch",
        timeout_secs: 30,
    }
    .is_retryable());
    assert!(ProviderError::HttpError {
        provider: "test".to_owned(),
        status: 503,
        body: "service unavailable".to_owned(),
    }
    .is_retryable());
    assert!(ProviderError::ApiError {
        provider: "test".to_owned(),
        status_code: 500,
        message: "internal error".to_owned(),
        retryable: true,
    }
    .is_retryable());

    // Non-retryable errors
    assert!(!ProviderError::AuthenticationFailed {
        provider: "test".to_owned(),
        reason: "invalid token".to_owned(),
    }
    .is_retryable());
    assert!(!ProviderError::NotFound {
        provider: "test".to_owned(),
        resource_type: "activity".to_owned(),
        resource_id: "123".to_owned(),
    }
    .is_retryable());
    assert!(!ProviderError::ConfigurationError {
        provider: "test".to_owned(),
        details: "missing client_id".to_owned(),
    }
    .is_retryable());
}

#[test]
fn test_provider_error_retry_after_secs() {
    let rate_limit = ProviderError::RateLimitExceeded {
        provider: "test".to_owned(),
        retry_after_secs: 120,
        limit_type: "daily".to_owned(),
    };
    assert_eq!(rate_limit.retry_after_secs(), Some(120));

    let network = ProviderError::NetworkError("issue".to_owned());
    assert_eq!(network.retry_after_secs(), None);
}
