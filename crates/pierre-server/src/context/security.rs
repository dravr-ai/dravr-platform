// ABOUTME: Security context for dependency injection of security-related services
// ABOUTME: Contains CSRF protection, PII redaction, and rate limiting for secure operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_middleware::redaction::RedactionConfig;
use std::sync::Arc;

/// Security context containing security-related dependencies
///
/// This context provides all security-related dependencies needed for
/// request protection, PII handling, and rate limiting.
///
/// # Dependencies
/// - `redaction_config`: Configuration for PII redaction in logs and responses
/// - `oauth2_rate_limiter`: Rate limiter for `OAuth2` endpoints
/// - `csrf_manager`: CSRF token manager for request forgery protection (used
///   by `middleware::csrf::validate_csrf_token` and the `csrf_protection_layer`)
#[derive(Clone)]
pub struct SecurityContext {
    redaction_config: Arc<RedactionConfig>,
    oauth2_rate_limiter: Arc<OAuth2RateLimiter>,
    csrf_manager: Arc<CsrfTokenManager>,
}

impl SecurityContext {
    /// Create new security context
    #[must_use]
    pub const fn new(
        redaction_config: Arc<RedactionConfig>,
        oauth2_rate_limiter: Arc<OAuth2RateLimiter>,
        csrf_manager: Arc<CsrfTokenManager>,
    ) -> Self {
        Self {
            redaction_config,
            oauth2_rate_limiter,
            csrf_manager,
        }
    }

    /// Get redaction configuration for PII handling
    #[must_use]
    pub const fn redaction_config(&self) -> &Arc<RedactionConfig> {
        &self.redaction_config
    }

    /// Get `OAuth2` rate limiter for endpoint protection
    #[must_use]
    pub const fn oauth2_rate_limiter(&self) -> &Arc<OAuth2RateLimiter> {
        &self.oauth2_rate_limiter
    }

    /// Get CSRF token manager for request forgery protection
    #[must_use]
    pub const fn csrf_manager(&self) -> &Arc<CsrfTokenManager> {
        &self.csrf_manager
    }
}
