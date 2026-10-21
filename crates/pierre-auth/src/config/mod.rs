// ABOUTME: Auth-related configuration types for pierre-auth
// ABOUTME: OAuth provider config and rate limiting settings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The Google OAuth web client behind "Continue with Google" on the hosted login page
pub mod google_sign_in;

/// OAuth provider configuration for fitness platforms
pub mod oauth;

/// The satellite MCP servers the authorization server mints for
mod satellites;

/// Rate limiting configuration
pub mod rate_limit;

pub use google_sign_in::GoogleSignInConfig;
pub use oauth::{
    configured_credential, credential_env, provider_callback_uri, resolve_issuer_url,
    resolve_mcp_resource_aliases, resolve_mcp_resource_url, ClientRetentionConfig, FirebaseConfig,
    OAuth2ServerConfig, OAuthConfig, OAuthProviderConfig, ProviderEndpointOverride,
    ProviderEndpointOverrides, DIALED_HOST_HEADERS,
};
pub use rate_limit::RateLimitConfig;
