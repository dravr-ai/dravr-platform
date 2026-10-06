// ABOUTME: Meters password checks through the OAuth2 endpoint limiter: sign-ins per address and account, re-confirmations per account
// ABOUTME: A sign-in past its window is a 429 with Retry-After; change-password and account deletion share one window

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::net::IpAddr;

use axum::http::header::RETRY_AFTER;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_auth::dto::auth::OAuth2ErrorResponse;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_core::errors::{AppError, AppResult};
use tracing::{error, warn};
use uuid::Uuid;

/// Decide whether a password sign-in from `client` naming `email` may be
/// tried, before the password is checked: `None` admits it, `Some` is the
/// RFC 6749 §5.2 body refusing it — a 429 `too_many_requests` with
/// `Retry-After` when the address's or the account's window of refused
/// passwords is full, a 503 `temporarily_unavailable` when the limiter could
/// not read them.
pub async fn password_sign_in_refusal(
    limiter: &OAuth2RateLimiter,
    client: Option<IpAddr>,
    email: &str,
) -> Option<Response> {
    let (status, error, description, retry_after) = match limiter.sign_in_wait(client, email).await
    {
        Ok(None) => return None,
        Ok(Some(retry_after)) => {
            warn!(retry_after, "Password sign-in refused: too many attempts");
            (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                "Too many sign-in attempts; retry later",
                Some(retry_after),
            )
        }
        Err(failure) => {
            error!(error = %failure, "Password sign-in limiter could not read its windows");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
                "Sign-in is temporarily unavailable; retry later",
                None,
            )
        }
    };
    let body = OAuth2ErrorResponse {
        error: error.to_owned(),
        error_description: Some(description.to_owned()),
    };
    let mut response = (status, Json(body)).into_response();
    if let Some(secs) = retry_after {
        response.headers_mut().insert(RETRY_AFTER, secs.into());
    }
    Some(response)
}

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
