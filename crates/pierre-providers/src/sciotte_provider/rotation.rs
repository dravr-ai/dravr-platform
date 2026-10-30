// ABOUTME: A SciotteProvider's reads through its session, and the session a provider rotated on one of them
// ABOUTME: The rotated copy replaces the held one and goes to the token-refresh callback, over the row it came from
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every read a [`SciotteProvider`] makes goes through
//! [`SciotteProvider::read_through`], the provider's use of
//! [`read_imported_persisting`]. A session the scraped platform rotated on a
//! read (Garmin logs the old cookie out) replaces the one the provider holds,
//! so its later reads import the live copy, and reaches the token-refresh
//! callback as the credentials the provider was given with the rotated
//! session as their access token: the auth layer writes it over the row
//! those credentials came from, while that row still stands.

use std::future::Future;

use dravr_sciotte::client::{ClientError, SciotteClient};
use dravr_sciotte::models::AuthSession;
use tracing::{debug, warn};

use super::{SciotteProvider, SciotteTarget};
use crate::core::TokenRefreshCallback;
use crate::errors::AppResult;
use crate::sciotte_session::{self, read_imported_persisting};

impl SciotteProvider {
    /// Run `read` through `session` on `remote`.
    ///
    /// A session the platform rotated on it is adopted
    /// ([`Self::adopt_rotated`]) whatever the read answered, and a failure is
    /// mapped to the platform error.
    pub(super) async fn read_through<T, F, Fut>(
        &self,
        remote: &SciotteClient,
        session: &AuthSession,
        read: F,
    ) -> AppResult<T>
    where
        T: Send,
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = Result<T, ClientError>> + Send,
    {
        let provider = SciotteTarget::from_backend_name(self.provider_name).scraper_provider_name();
        read_imported_persisting(remote, session, provider, read, |rotated| {
            self.adopt_rotated(rotated)
        })
        .await
        .map_err(|e| self.remote_error(e))
    }

    /// Hold `rotated` in place of the session this provider was given.
    ///
    /// It goes to the token-refresh callback as the access token of the
    /// credentials the session came in, serialized as a login stores it, with
    /// the rotated session's expiry and every other field as it came.
    async fn adopt_rotated(&self, rotated: AuthSession) {
        let json = match sciotte_session::session_json(&rotated) {
            Ok(json) => json,
            Err(error) => {
                warn!(
                    provider = self.provider_name,
                    error = %error,
                    "a rotated sciotte session could not be serialized; keeping the session held"
                );
                return;
            }
        };
        let credentials = self.credentials.write().await.as_mut().map(|held| {
            held.access_token = Some(json);
            held.expires_at = rotated.expires_at;
            held.clone()
        });
        *self.session.write().await = Some(rotated);
        if let (Some(credentials), Some(store)) = (credentials, self.token_refresh_callback.get()) {
            store(credentials).await;
        } else {
            warn!(
                provider = self.provider_name,
                "a read rotated the sciotte session, and nothing stores it for this provider: \
                 the stored copy is stale until the athlete signs in again"
            );
        }
    }

    /// Keep the first token-refresh callback this provider is handed.
    pub(super) fn keep_token_refresh_callback(&self, callback: TokenRefreshCallback) {
        if self.token_refresh_callback.set(callback).is_err() {
            debug!(
                provider = self.provider_name,
                "sciotte token refresh callback already set; keeping the first"
            );
        }
    }
}
