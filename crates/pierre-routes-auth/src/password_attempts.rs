// ABOUTME: Meters a signed-in account's password re-confirmations through the OAuth2 endpoint limiter, per account
// ABOUTME: change-password and account deletion share one window, so a stolen session cannot guess the password

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_core::errors::{AppError, AppResult};
use tracing::{error, warn};
use uuid::Uuid;

/// Count one password re-confirmation by `user_id`.
///
/// Returns `Ok(None)` when the attempt is within the account's window, and
/// `Ok(Some(seconds))` when it is past it: the caller refuses with 429 and
/// that `Retry-After`. Every attempt counts, right or wrong, before the
/// password is checked, so a refusal never says whether a guess was right.
///
/// # Errors
///
/// Returns a resource-unavailable error when the limiter cannot count the
/// attempt: an uncounted guess is refused rather than let through.
pub async fn meter_password_attempt(
    limiter: &OAuth2RateLimiter,
    user_id: Uuid,
) -> AppResult<Option<u32>> {
    let status = limiter
        .check_account_rate_limit(OAuth2Endpoint::PasswordConfirm, user_id)
        .await
        .map_err(|failure| {
            error!(
                user_id = %user_id,
                error = %failure,
                "Password attempt limiter could not count the attempt"
            );
            AppError::resource_unavailable(
                "Password confirmation is temporarily unavailable; retry later",
            )
        })?;
    if status.is_limited {
        warn!(
            user_id = %user_id,
            limit = status.limit,
            "Password re-confirmation refused: too many attempts"
        );
        return Ok(Some(status.retry_after_seconds.unwrap_or(1)));
    }
    Ok(None)
}
