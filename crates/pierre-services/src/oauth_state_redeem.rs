// ABOUTME: Redeeming an OAuth callback's state — verified server-side, marked used, redirect read
// ABOUTME: Split out of oauth_flow; the only path to a flow's mobile redirect URL

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The state a callback carries, redeemed once against server-side storage.
//!
//! Kept out of `oauth_flow` so that orchestrator stays within its size budget,
//! and so the one rule of this module sits in one place: a flow's mobile
//! redirect URL is read from a state only after that state redeems. The URL
//! rides base64-encoded inside the state string, and the callback route is
//! public, so a redirect read from an unredeemed state is an open redirect
//! (carnet#556).

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use tracing::{error, warn};

use crate::oauth_flow::OAuthService;
use crate::oauth_redirects::extract_mobile_redirect_from_state;

/// A callback `state` that [`OAuthService::redeem_state`] verified against
/// server-side storage and marked used.
pub struct ParsedOAuthState {
    pub(crate) user_id: uuid::Uuid,
    /// Optional redirect URL for mobile OAuth flows (base64 encoded in state)
    pub(crate) mobile_redirect_url: Option<String>,
    /// PKCE code verifier recovered from server-side state storage
    pub(crate) pkce_code_verifier: Option<String>,
    /// Tenant ID from the OAuth state, used for tenant-specific credential lookup
    pub(crate) tenant_id: Option<uuid::Uuid>,
    /// Strava shared-app pool member pinned at authorize (`None` = env-default).
    pub(crate) oauth_app_client_id: Option<String>,
}

impl ParsedOAuthState {
    /// The mobile redirect URL the flow was started with, already checked
    /// against the redirect allowlist.
    #[must_use]
    pub fn mobile_redirect_url(&self) -> Option<&str> {
        self.mobile_redirect_url.as_deref()
    }
}

impl OAuthService {
    /// Redeem a callback's `state`: verify it was issued by this server for
    /// `provider`, is unexpired and unused, and mark it used.
    ///
    /// This is the only way to learn a flow's mobile redirect URL. The URL
    /// rides inside the state string, so reading it from a state nobody looked
    /// up would let anyone who builds a state of their own choose where the
    /// callback redirects (carnet#556).
    ///
    /// # Errors
    /// Returns an error when the provider is unsupported or the state is
    /// unknown, expired, already used, or issued for another provider.
    pub async fn redeem_state(&self, state: &str, provider: &str) -> AppResult<ParsedOAuthState> {
        // Validate provider is supported before consuming state
        self.validate_provider(provider)?;

        // Consume state atomically from database (verifies it was server-issued,
        // not expired, not reused, and matches the expected provider)
        self.consume_and_validate_state(state, provider).await
    }

    /// Consume and validate OAuth state from server-side storage
    ///
    /// Atomically verifies the state was issued by this server, has not expired,
    /// and has not been used before (one-time use). Uses the provider name as the
    /// `client_id` for additional validation that the callback matches the initiated flow.
    ///
    /// State format: `{user_id}:{random}` or `{user_id}:{random}:{base64_redirect_url}`
    /// The redirect URL allows mobile apps to specify where to redirect after OAuth completes.
    pub(crate) async fn consume_and_validate_state(
        &self,
        state: &str,
        provider: &str,
    ) -> AppResult<ParsedOAuthState> {
        // Atomically consume the state from database (marks as used, checks expiry)
        let consumed = self
            .data
            .repos()
            .oauth_client_state
            .consume_oauth_client_state(state, provider, Utc::now())
            .await
            .map_err(|e| {
                warn!("Failed to consume OAuth state from database: {}", e);
                AppError::auth_invalid("OAuth state validation failed")
            })?;

        let client_state = consumed.ok_or_else(|| {
            warn!(
                "OAuth state not found, expired, or already used for provider {}",
                provider
            );
            AppError::auth_invalid("Invalid, expired, or already used OAuth state parameter")
        })?;

        let user_id = client_state.user_id.ok_or_else(|| {
            error!("OAuth state missing user_id for provider {}", provider);
            AppError::auth_invalid("OAuth state missing user identity")
        })?;

        // Extract optional mobile redirect URL from the state string
        // (embedded as base64 in the third segment of the state format)
        let mobile_redirect_url = self.extract_mobile_redirect_from_state_str(state);

        // PKCE code verifier stored server-side during authorization URL generation
        let pkce_code_verifier = client_state.pkce_code_verifier;

        // Parse tenant_id from the stored OAuth client state for credential lookup
        let tenant_id = client_state
            .tenant_id
            .as_deref()
            .and_then(|tid| uuid::Uuid::parse_str(tid).ok());

        Ok(ParsedOAuthState {
            user_id,
            mobile_redirect_url,
            pkce_code_verifier,
            tenant_id,
            oauth_app_client_id: client_state.oauth_app_client_id,
        })
    }

    /// Extract mobile redirect URL from state string format
    ///
    /// State format: `{user_id}:{random}:{base64_redirect_url}`
    /// Delegates to `extract_mobile_redirect_from_state`.
    fn extract_mobile_redirect_from_state_str(&self, state: &str) -> Option<String> {
        extract_mobile_redirect_from_state(
            state,
            &self.config().base_url,
            &self.config().security.allowed_mobile_redirect_origins,
        )
    }
}
