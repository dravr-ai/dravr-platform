// ABOUTME: Write-back of a token a provider refreshed or rotated on its own, over the row it was read from
// ABOUTME: Only while that row still stands, so a reconnect that replaced it meanwhile is never overwritten
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one write-back of a token a provider changed on its own.
//!
//! An OAuth provider refreshes its pair when a call is refused; a scraped
//! provider's session can come back from a read with a rotated cookie
//! (Garmin logs the old value out). [`StoredTokenRow::write_back`] is that
//! write, whoever holds the token. It goes over the one row the provider's
//! credentials were read from, and only while that row still stands: it must
//! still carry the `id` it was read with (a reconnect writes a fresh one),
//! and a scrape session row must still carry the session's `session_id`.
//! The write is a compare-and-swap on the row exactly as it was just read
//! ([`OAuthTokenRepository::refresh_token`]), scoped to that row's user,
//! tenant and provider. No other row is written, a super admin's clone of the
//! session included.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pierre_core::constants::oauth::providers::TOKEN_TYPE_SESSION;
use pierre_database::repositories::OAuthTokenRepository;
use secrecy::ExposeSecret;
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::core::{OAuth2Credentials, TokenRefreshCallback};
use crate::errors::{AppError, AppResult};
use crate::models::{TenantId, UserOAuthToken};

/// The token row a provider's credentials were read from, captured before the
/// provider reads anything through them.
#[derive(Debug, Clone)]
pub struct StoredTokenRow {
    user_id: Uuid,
    tenant_id: TenantId,
    provider: String,
    row_id: String,
}

/// The one field of a stored scrape session read back.
#[derive(Deserialize)]
struct SessionId {
    session_id: String,
}

/// The `session_id` of the scrape session `json` serializes; `None` when
/// `json` is not one.
#[must_use]
pub fn session_id_of(json: &str) -> Option<String> {
    serde_json::from_str::<SessionId>(json)
        .ok()
        .map(|session| session.session_id)
}

impl StoredTokenRow {
    /// The row `row_id` names, stored for `user_id` under `provider` in
    /// `tenant_id`.
    #[must_use]
    pub fn new(
        user_id: Uuid,
        tenant_id: TenantId,
        provider: impl Into<String>,
        row_id: impl Into<String>,
    ) -> Self {
        Self {
            user_id,
            tenant_id,
            provider: provider.into(),
            row_id: row_id.into(),
        }
    }

    /// The row `token` was read as.
    ///
    /// # Errors
    ///
    /// When the row's tenant id is not a tenant id.
    pub fn of(token: &UserOAuthToken) -> AppResult<Self> {
        let tenant_id = TenantId::parse_str(&token.tenant_id).map_err(|e| {
            AppError::internal(format!("Stored token has an invalid tenant id: {e}"))
        })?;
        Ok(Self::new(
            token.user_id,
            tenant_id,
            token.provider.clone(),
            token.id.clone(),
        ))
    }

    /// The backend the row is stored under.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Write what a provider refreshed or rotated, `credentials`, back.
    ///
    /// The row is read again first and must still carry its `id`. A scrape
    /// session row takes the session in `credentials`' access token
    /// ([`Self::write_session`]); any other row takes the access token,
    /// refresh token and expiry.
    ///
    /// Logged, never returned: the token the provider holds in memory still
    /// serves the request that changed it.
    pub async fn write_back(
        &self,
        tokens: &dyn OAuthTokenRepository,
        credentials: &OAuth2Credentials,
    ) {
        let access_token = credentials
            .access_token
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .unwrap_or_default();
        let Some(stored) = self.standing(tokens).await else {
            return;
        };
        if stored.token_type == TOKEN_TYPE_SESSION {
            self.write_session_over(tokens, &stored, access_token, credentials.expires_at)
                .await;
            return;
        }
        let written = tokens
            .refresh_token(
                &stored,
                access_token,
                credentials
                    .refresh_token
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                credentials.expires_at,
            )
            .await;
        self.log_write(written);
    }

    /// Write a rotated scrape session, `session_json`, over this row, while
    /// it still stands.
    ///
    /// The row must still carry its `id` and the session's `session_id`. It
    /// keeps its own refresh token (a scrape session has none, so a `NULL`
    /// stays `NULL`) and takes `expires_at`, the rotated session's expiry,
    /// exactly as a login stores a session.
    ///
    /// Logged, never returned.
    pub async fn write_session(
        &self,
        tokens: &dyn OAuthTokenRepository,
        session_json: &str,
        expires_at: Option<DateTime<Utc>>,
    ) {
        if let Some(stored) = self.standing(tokens).await {
            self.write_session_over(tokens, &stored, session_json, expires_at)
                .await;
        }
    }

    /// Write `session_json` over `stored`, this row as just read, while it
    /// is a scrape session row carrying the session's `session_id`.
    async fn write_session_over(
        &self,
        tokens: &dyn OAuthTokenRepository,
        stored: &UserOAuthToken,
        session_json: &str,
        expires_at: Option<DateTime<Utc>>,
    ) {
        let Some(session_id) = session_id_of(session_json) else {
            warn!(provider = %self.provider, "a rotated sciotte session names no session id; nothing stored");
            return;
        };
        let carries_session = stored.token_type == TOKEN_TYPE_SESSION
            && session_id_of(stored.access_token.expose_secret()).as_deref()
                == Some(session_id.as_str());
        if !carries_session {
            self.log_replaced();
            return;
        }
        let written = tokens
            .refresh_token(
                stored,
                session_json,
                stored
                    .refresh_token
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                expires_at,
            )
            .await;
        self.log_write(written);
    }

    /// The row as stored now, while it still carries the `id` it was read
    /// with; `None`, logged, once a reconnect replaced it or it cannot be read.
    async fn standing(&self, tokens: &dyn OAuthTokenRepository) -> Option<UserOAuthToken> {
        match tokens
            .get_token(self.user_id, self.tenant_id, &self.provider)
            .await
        {
            Ok(stored) => {
                let standing = stored.filter(|stored| stored.id == self.row_id);
                if standing.is_none() {
                    self.log_replaced();
                }
                standing
            }
            Err(e) => {
                self.log_write(Err(e));
                None
            }
        }
    }

    fn log_replaced(&self) {
        info!(
            user_id = %self.user_id,
            provider = %self.provider,
            "Provider-refreshed token not persisted: the token it refreshed was replaced since"
        );
    }

    fn log_write(&self, written: AppResult<bool>) {
        match written {
            Ok(true) => info!(
                user_id = %self.user_id,
                provider = %self.provider,
                "Persisted a provider-refreshed token"
            ),
            Ok(false) => info!(
                user_id = %self.user_id,
                provider = %self.provider,
                "Provider-refreshed token not persisted: another write to the row landed first"
            ),
            Err(e) => warn!(
                user_id = %self.user_id,
                provider = %self.provider,
                error = %e,
                "Failed to persist a provider-refreshed token"
            ),
        }
    }

    /// The callback a provider is handed
    /// ([`crate::core::FitnessProvider::set_token_refresh_callback`]) to
    /// write what it refreshes or rotates back ([`Self::write_back`]).
    #[must_use]
    pub fn write_back_callback(
        self,
        tokens: Arc<dyn OAuthTokenRepository>,
    ) -> TokenRefreshCallback {
        let row = Arc::new(self);
        Arc::new(move |credentials: OAuth2Credentials| {
            let row = Arc::clone(&row);
            let tokens = Arc::clone(&tokens);
            Box::pin(async move { row.write_back(&*tokens, &credentials).await })
        })
    }
}
