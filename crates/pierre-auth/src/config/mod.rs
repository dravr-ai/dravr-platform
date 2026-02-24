// ABOUTME: Auth-related configuration types for pierre-auth
// ABOUTME: OAuth provider config and rate limiting settings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// OAuth provider configuration for fitness platforms
pub mod oauth;

/// Rate limiting configuration
pub mod rate_limit;

pub use oauth::{
    FirebaseConfig, OAuth2ServerConfig, OAuthConfig, OAuthProviderConfig, ProviderEnvConfig,
};
pub use rate_limit::RateLimitConfig;
