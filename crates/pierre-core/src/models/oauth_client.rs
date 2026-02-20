// ABOUTME: OAuth2 client state for CSRF protection and PKCE verifier storage
// ABOUTME: Server-side state tracked during OAuth authorization flows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Server-side OAuth client state for CSRF protection and PKCE verifier storage
///
/// Stored when generating an authorization URL and consumed atomically during callback.
/// Separate from `OAuth2State` which is for Pierre's `OAuth2` server flow.
#[derive(Debug, Clone)]
pub struct OAuthClientState {
    /// Unique state value sent through the OAuth flow
    pub state: String,
    /// OAuth provider name (e.g., "strava", "fitbit")
    pub provider: String,
    /// User who initiated the OAuth flow
    pub user_id: Option<Uuid>,
    /// Tenant context for multi-tenant flows
    pub tenant_id: Option<String>,
    /// OAuth callback redirect URI
    pub redirect_uri: String,
    /// Requested OAuth scopes
    pub scope: Option<String>,
    /// PKCE code verifier (stored server-side, needed for token exchange)
    pub pkce_code_verifier: Option<String>,
    /// When this state was created
    pub created_at: DateTime<Utc>,
    /// When this state expires (typically 10 minutes)
    pub expires_at: DateTime<Utc>,
    /// Whether this state has been consumed (one-time use)
    pub used: bool,
}
