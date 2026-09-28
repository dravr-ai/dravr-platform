// ABOUTME: The one refresh-token rotation rule both token stores follow — exchange once, a replay revokes the chain
// ABOUTME: Shared by first-party session refresh and the OAuth2 server's refresh grant, with one lifetime setting

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Refresh-token rotation, written once.
//!
//! Two stores hold refresh tokens: `session_refresh_tokens` for a first-party
//! login and `oauth2_refresh_tokens` for a registered OAuth client (whose
//! `client_id` foreign key the first-party login has no counterpart for). They
//! keep separate tables, but they follow one rule, and this module is it:
//!
//! - A token exchanges exactly once. The store consumes it in the same
//!   statement that reads it, and the exchange issues a successor in the same
//!   rotation chain (its *family*).
//! - A token that no longer exchanges but is still on record was rotated out
//!   and then presented again — the shape a stolen credential takes once the
//!   legitimate holder has moved on. Its whole family is revoked, live
//!   successor included.
//! - Every token lives [`refresh_token_lifetime`] from the moment it is issued,
//!   read from the one `REFRESH_TOKEN_EXPIRY_DAYS` setting.

use std::future::Future;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::Duration;
use pierre_core::errors::{AppError, AppResult};
use ring::rand::{SecureRandom, SystemRandom};
use tracing::warn;

/// Exchange a presented refresh token once, or treat it as a replay.
///
/// `consume` is the store's single-statement check-and-revoke; it yields the
/// token's record when the token was live. When it yields nothing,
/// `revoke_family` runs and revokes every live member of the presented token's
/// family; a token that is unknown to the store revokes nothing.
///
/// # Errors
/// Returns whatever error either store operation returns.
pub async fn consume_or_revoke_family<T, Consume, Revoke, RevokeFut>(
    consume: Consume,
    revoke_family: Revoke,
) -> AppResult<Option<T>>
where
    Consume: Future<Output = AppResult<Option<T>>>,
    Revoke: FnOnce() -> RevokeFut,
    RevokeFut: Future<Output = AppResult<u64>>,
{
    if let Some(record) = consume.await? {
        return Ok(Some(record));
    }
    let revoked = revoke_family().await?;
    if revoked > 0 {
        warn!(
            revoked,
            "Refresh token replayed after rotation; its family is revoked"
        );
    }
    Ok(None)
}

/// How long a freshly issued refresh token stays exchangeable, from the
/// configured number of days.
#[must_use]
pub fn refresh_token_lifetime(expiry_days: i64) -> Duration {
    Duration::days(expiry_days)
}

/// A fresh refresh token value: 32 bytes of OS randomness, base64url without
/// padding, so it travels in a form field or a header unchanged.
///
/// # Errors
/// Returns an internal error if the system RNG fails.
pub fn generate_refresh_token() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| AppError::internal("System RNG failure while minting a refresh token"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
