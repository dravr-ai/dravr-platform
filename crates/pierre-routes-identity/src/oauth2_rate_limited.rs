// ABOUTME: How the OAuth 2.0 endpoints admit or refuse a request by its per-endpoint, per-client window
// ABOUTME: RFC 6749 too_many_requests plus a Retry-After from the window, or a 503 when it cannot be counted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-client rate-limit decisions for `/oauth2/authorize`,
//! `/oauth2/register` and `/oauth2/token`, and their refusals.
//!
//! The client is the address the trusted proxies recorded in front of the
//! TCP peer ([`OAuth2RateLimiter::client_address`]), so clients behind one
//! proxy keep separate windows. The JSON bodies are RFC 6749 / RFC 7591 error
//! objects, not `AppError`s, so the `Retry-After` header is written here
//! rather than by `impl IntoResponse for AppError`: one owner per wire
//! contract.

use std::net::SocketAddr;

use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::rate_limiting::{OAuth2Endpoint, OAuth2RateLimitStatus};
use tracing::error;

/// The `error_description` of a request refused because the limiter could not
/// count it.
pub const LIMITER_UNAVAILABLE: &str = "Rate limiting is temporarily unavailable; retry later";

/// The `error_description` of a request past its window's allowance.
const RATE_LIMIT_EXCEEDED: &str = "Rate limit exceeded";

/// What the limiter decided about one request.
enum Admission {
    /// Within its window's allowance
    Admitted,
    /// Past its window's allowance
    Limited(OAuth2RateLimitStatus),
    /// Undecided: the limiter could not count the request, so the server
    /// cannot tell a client within its allowance from one past it
    Unavailable,
}

/// Count one request to `endpoint` from TCP peer `peer`, carrying `headers`,
/// and decide it.
async fn admit(
    limiter: &OAuth2RateLimiter,
    endpoint: OAuth2Endpoint,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> Admission {
    let client = limiter.client_address(peer.ip(), headers);
    match limiter.check_rate_limit(endpoint, client).await {
        Ok(status) if status.is_limited => Admission::Limited(status),
        Ok(_) => Admission::Admitted,
        Err(failure) => {
            error!(
                endpoint = endpoint.as_str(),
                error = %failure,
                "OAuth2 rate limiter could not count the request"
            );
            Admission::Unavailable
        }
    }
}

/// Decide one request to a JSON endpoint: `None` admits it, `Some` is the
/// response refusing it — a 429 past the window's allowance, a 503 when the
/// limiter could not count it.
pub async fn refusal(
    limiter: &OAuth2RateLimiter,
    endpoint: OAuth2Endpoint,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> Option<Response> {
    match admit(limiter, endpoint, peer, headers).await {
        Admission::Admitted => None,
        Admission::Limited(status) => {
            let refused = OAuth2Error::too_many_requests(RATE_LIMIT_EXCEEDED);
            let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(refused)).into_response();
            insert_retry_after(&mut response, &status);
            Some(response)
        }
        Admission::Unavailable => Some(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(OAuth2Error::temporarily_unavailable(LIMITER_UNAVAILABLE)),
            )
                .into_response(),
        ),
    }
}

/// Decide one request to `/oauth2/authorize`, whose refusals are pages
/// `render` draws from the error: `too_many_requests` past the window's
/// allowance, with its `Retry-After`, and `temporarily_unavailable` when the
/// limiter could not count it. `render` gives each its status (429, 503).
pub async fn page_refusal(
    limiter: &OAuth2RateLimiter,
    peer: SocketAddr,
    headers: &HeaderMap,
    render: impl FnOnce(&OAuth2Error) -> Response,
) -> Option<Response> {
    match admit(limiter, OAuth2Endpoint::Authorize, peer, headers).await {
        Admission::Admitted => None,
        Admission::Limited(status) => {
            let mut response = render(&OAuth2Error::too_many_requests(RATE_LIMIT_EXCEEDED));
            insert_retry_after(&mut response, &status);
            Some(response)
        }
        Admission::Unavailable => Some(render(&OAuth2Error::temporarily_unavailable(
            LIMITER_UNAVAILABLE,
        ))),
    }
}

/// Set the limiter's wait as `Retry-After`.
///
/// The limiter sets `retry_after_seconds` on every limited status, floored at
/// one second ([`OAuth2RateLimitStatus::with_retry_after`]); the RFC's body
/// shape has no field for the wait.
fn insert_retry_after(response: &mut Response, status: &OAuth2RateLimitStatus) {
    if let Some(secs) = status.retry_after_seconds {
        response
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from(secs));
    }
}
