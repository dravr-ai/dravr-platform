// ABOUTME: OAuth2 client state for CSRF protection and PKCE verifier storage
// ABOUTME: Server-side state tracked during OAuth authorization flows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Server-side OAuth client state for CSRF protection and PKCE verifier storage
///
/// Stored when generating an authorization URL and consumed atomically during callback.
/// Separate from `OAuth2State` which is for Pierre's `OAuth2` server flow.
///
/// `Debug` is hand-written: the state value (the CSRF secret) and the PKCE
/// code verifier print as `[REDACTED]`.
#[derive(Clone)]
pub struct OAuthClientState {
    /// Unique state value sent through the OAuth flow
    pub state: String,
    /// OAuth provider name (e.g., "strava", "whoop")
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
    /// For a Strava authorization served by the shared-app OAuth pool, the
    /// `client_id` of the app chosen at authorize time — pinned here so the
    /// code exchange uses the same app's `client_id`/`client_secret` that built
    /// the authorize URL (a mismatch makes the provider reject the code).
    /// `None` means the env-default `STRAVA_CLIENT_ID` app.
    pub oauth_app_client_id: Option<String>,
    /// When this state was created
    pub created_at: DateTime<Utc>,
    /// When this state expires (typically 10 minutes)
    pub expires_at: DateTime<Utc>,
    /// Whether this state has been consumed (one-time use)
    pub used: bool,
}

impl fmt::Debug for OAuthClientState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthClientState")
            .field("state", &"[REDACTED]")
            .field("provider", &self.provider)
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .field("redirect_uri", &self.redirect_uri)
            .field("scope", &self.scope)
            .field(
                "pkce_code_verifier",
                &self.pkce_code_verifier.as_ref().map(|_| "[REDACTED]"),
            )
            .field("oauth_app_client_id", &self.oauth_app_client_id)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("used", &self.used)
            .finish()
    }
}
