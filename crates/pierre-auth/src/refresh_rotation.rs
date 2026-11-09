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
//! - An `OAuth2` family also has an absolute life, [`REFRESH_FAMILY_MAX_DAYS`]
//!   from its first token, however often it rotates: a forgotten connection
//!   eventually asks its athlete to consent again. A family with no recorded
//!   start is a legacy one and has no absolute limit.

use std::future::Future;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
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

/// The absolute lifetime of an `OAuth2` refresh-token family, in days, counted
/// from the family's first token and unaffected by rotation.
pub const REFRESH_FAMILY_MAX_DAYS: i64 = 180;

/// When a successor issued at `now` stops exchanging, or `None` when its
/// family has run out its absolute life and must not be continued.
///
/// The usual sliding window (`lifetime` from `now`), capped at the family's
/// deadline. `family_created_at` of `None` is a legacy family: no cap.
#[must_use]
pub(crate) fn successor_expiry(
    now: DateTime<Utc>,
    lifetime: Duration,
    family_created_at: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    let sliding = now + lifetime;
    let Some(started) = family_created_at else {
        return Some(sliding);
    };
    let deadline = started + Duration::days(REFRESH_FAMILY_MAX_DAYS);
    (deadline > now).then_some(sliding.min(deadline))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_family_slides_without_limit() {
        let now = Utc::now();
        assert_eq!(
            successor_expiry(now, Duration::days(30), None),
            Some(now + Duration::days(30))
        );
    }

    #[test]
    fn a_young_family_slides_by_the_usual_window() {
        let now = Utc::now();
        let started = now - Duration::days(10);
        assert_eq!(
            successor_expiry(now, Duration::days(30), Some(started)),
            Some(now + Duration::days(30))
        );
    }

    #[test]
    fn the_window_is_cut_at_the_family_deadline() {
        let now = Utc::now();
        let started = now - Duration::days(REFRESH_FAMILY_MAX_DAYS - 5);
        assert_eq!(
            successor_expiry(now, Duration::days(30), Some(started)),
            Some(started + Duration::days(REFRESH_FAMILY_MAX_DAYS))
        );
    }

    #[test]
    fn a_family_past_its_deadline_is_not_continued() {
        let now = Utc::now();
        let started = now - Duration::days(REFRESH_FAMILY_MAX_DAYS + 1);
        assert_eq!(
            successor_expiry(now, Duration::days(30), Some(started)),
            None
        );
    }
}
