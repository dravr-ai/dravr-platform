// ABOUTME: The 429 the OAuth 2.0 endpoints answer a per-IP rate limit with
// ABOUTME: RFC 6749 too_many_requests body plus a Retry-After taken from the limiter's own window
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-IP rate-limit refusals for `/oauth2/register` and `/oauth2/token`.
//!
//! These bodies are RFC 6749 / RFC 7591 error objects, not `AppError`s, so the
//! `Retry-After` header is written here rather than by
//! `impl IntoResponse for AppError`: one owner per wire contract.

use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_auth::rate_limiting::OAuth2RateLimitStatus;

/// A 429 in the OAuth 2.0 error shape (`error` + `error_description`) with the
/// limiter's wait as `Retry-After`.
///
/// The limiter sets `retry_after_seconds` on every limited status, floored at
/// one second ([`OAuth2RateLimitStatus::with_retry_after`]); the body keeps
/// the RFC's shape, which has no field for the wait.
pub fn too_many_requests(status: &OAuth2RateLimitStatus) -> Response {
    let refusal = OAuth2Error::too_many_requests("Rate limit exceeded");
    let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(refusal)).into_response();
    if let Some(secs) = status.retry_after_seconds {
        response
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from(secs));
    }
    response
}
