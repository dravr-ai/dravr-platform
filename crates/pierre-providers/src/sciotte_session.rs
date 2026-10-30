// ABOUTME: Every platform read through a sciotte session: import, read, then store a session the provider rotated
// ABOUTME: The platform is the session-of-record (ADR-021), so a rotated copy replaces the stored one or the session dies
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reads through a platform-held sciotte session.
//!
//! The platform stores each scrape session on the athlete's token row and
//! imports it into the dravr-sciotte service before every read, so a service
//! instance that scaled to zero is re-seeded from the stored copy (ADR-021).
//! A provider can rotate a session's cookies on a read: Garmin changes its
//! `session` cookie and logs the old value out. The service says so on the
//! read's answer, and unless the platform then stores the copy the service
//! now holds, the next import re-seeds a session the provider has already
//! signed out.
//!
//! [`read_imported_persisting`] is the one way the platform reads through a
//! session: it imports and reads ([`SciotteClient::read_imported`]), then,
//! whatever the read answered, exports a session the read noted as rotated
//! and hands it to the caller's store. [`store_over_row`] is that store for a
//! session read from a token row: the rotated copy goes over that row only.

use std::future::Future;

use dravr_sciotte::client::{ClientError, SciotteClient};
use dravr_sciotte::models::AuthSession;
use pierre_database::repositories::OAuthTokenRepository;
use secrecy::{ExposeSecret, SecretString};
use tracing::warn;

use crate::errors::{AppError, AppResult};
use crate::token_writeback::StoredTokenRow;

/// `session` as the JSON the platform stores on its token row, the access
/// token every read imports again.
///
/// # Errors
///
/// When the session does not serialize.
pub fn session_json(session: &AuthSession) -> AppResult<SecretString> {
    serde_json::to_string(session)
        .map(SecretString::from)
        .map_err(|e| AppError::internal(format!("Failed to serialize session: {e}")))
}

/// Read through `session`, then store it if the provider rotated it.
///
/// Imports `session` for `provider` and runs `read` through it
/// ([`SciotteClient::read_imported`]), then hands `store_rotated` the session
/// the service now holds when a read noted that the provider rotated it.
///
/// The rotation is looked for whether the read succeeded or failed: a
/// provider can rotate a cookie on a read that then fails. An export that
/// fails is logged and never fails the read; the client keeps its note, so
/// the next read through the session exports it again. No rotation noted
/// sends nothing and calls nothing.
///
/// # Errors
///
/// The read's own error, exactly as [`SciotteClient::read_imported`]
/// returns it.
pub async fn read_imported_persisting<T, F, Fut, S, SFut>(
    remote: &SciotteClient,
    session: &AuthSession,
    provider: &str,
    read: F,
    store_rotated: S,
) -> Result<T, ClientError>
where
    T: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<T, ClientError>> + Send,
    S: FnOnce(AuthSession) -> SFut + Send,
    SFut: Future<Output = ()> + Send,
{
    let answer = remote.read_imported(session, provider, read).await;
    match remote.export_rotated_session(&session.session_id).await {
        Ok(Some(rotated)) => store_rotated(rotated).await,
        Ok(None) => {}
        Err(error) => warn!(
            provider,
            error = %error,
            "a read rotated the sciotte session, but exporting the rotated copy failed; \
             the next read through the session exports it again"
        ),
    }
    answer
}

/// Store `rotated` over `row`, the token row the session was read from.
///
/// Written only while that row still stands
/// ([`StoredTokenRow::write_session`]); logged, never returned.
pub async fn store_over_row(
    tokens: &dyn OAuthTokenRepository,
    row: &StoredTokenRow,
    rotated: &AuthSession,
) {
    match session_json(rotated) {
        Ok(json) => {
            row.write_session(tokens, json.expose_secret(), rotated.expires_at)
                .await;
        }
        Err(error) => warn!(
            provider = row.provider(),
            error = %error,
            "a rotated sciotte session could not be stored"
        ),
    }
}
