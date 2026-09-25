// ABOUTME: Pins how each AppError constructor renders Retry-After through IntoResponse
// ABOUTME: One value in details and header, 429/503 only, fixed caps carry none, auth refusals keep a 429 and a 5xx
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `impl IntoResponse for AppError` is the one owner of `Retry-After` on a
//! REST response: it renders `details.retry_after_secs`, which the producer
//! attached once, on a 429 or a 503. These tests render each constructor and
//! read back the header, the status and the body.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use axum::body::to_bytes;
use axum::http::header::RETRY_AFTER;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{Duration, SecondsFormat, Utc};
use pierre_core::errors::{AppError, ErrorCode, RETRY_AFTER_SECS_DETAIL};
use serde_json::Value;

/// Status, `Retry-After` header and JSON body of `error` as a response.
async fn render(error: AppError) -> (StatusCode, Option<String>, Value) {
    let response = error.into_response();
    let status = response.status();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, retry_after, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn test_rate_limit_exceeded_renders_its_retry_window() {
    let (status, retry_after, body) = render(AppError::rate_limit_exceeded(5, 5, 42)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(retry_after.as_deref(), Some("42"));
    assert_eq!(body["code"], "RateLimitExceeded");
    assert_eq!(body["details"]["retry_after_secs"], 42);
    assert_eq!(body["details"]["limit_type"], "requests");
    assert_eq!(body["details"]["current"], 5);
    assert_eq!(body["details"]["limit"], 5);
}

#[tokio::test]
async fn test_rate_limit_exceeded_floors_a_zero_wait_at_one_second() {
    let error = AppError::rate_limit_exceeded(5, 5, 0);
    assert!(
        error.message.contains("retry after 1s"),
        "the message names the floored wait: {}",
        error.message
    );
    let (_, retry_after, body) = render(error).await;
    assert_eq!(retry_after.as_deref(), Some("1"));
    assert_eq!(body["details"]["retry_after_secs"], 1);
}

#[tokio::test]
async fn test_time_windowed_quota_attaches_the_seconds_to_its_reset() {
    let resets_at = (Utc::now() + Duration::hours(3)).to_rfc3339_opts(SecondsFormat::Secs, true);
    let (status, retry_after, body) = render(AppError::quota_exceeded(
        "daily_messages",
        50,
        50,
        &resets_at,
    ))
    .await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let secs = body["details"]["retry_after_secs"].as_u64().unwrap();
    assert!(
        (3 * 3600 - 2..=3 * 3600).contains(&secs),
        "three hours to the reset, got {secs}s"
    );
    assert_eq!(retry_after, Some(secs.to_string()), "header equals body");
    assert_eq!(
        body["details"]["resets_at"].as_str(),
        Some(resets_at.as_str()),
        "the reset instant is kept byte for byte"
    );
    assert_eq!(body["details"]["limit_type"], "daily_messages");
}

#[tokio::test]
async fn test_fixed_cap_quota_carries_no_retry_window() {
    let (status, retry_after, body) = render(AppError::quota_exceeded(
        "max_active_conversations",
        10,
        10,
        "",
    ))
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(retry_after, None, "waiting does not lift a fixed cap");
    assert!(body["details"].get(RETRY_AFTER_SECS_DETAIL).is_none());
    assert_eq!(body["details"]["limit"], 10);
}

#[tokio::test]
async fn test_unparseable_reset_instant_attaches_nothing() {
    let error = AppError::quota_exceeded("daily_tokens", 1, 1, "tomorrow-ish");
    assert_eq!(error.retry_after_secs(), None);
    let (_, retry_after, body) = render(error).await;
    assert_eq!(retry_after, None);
    assert_eq!(body["details"]["resets_at"], "tomorrow-ish");
}

#[tokio::test]
async fn test_backpressure_503_renders_retry_after() {
    let error =
        AppError::new(ErrorCode::ExternalRateLimited, "the scraper is busy").with_retry_after(7);
    assert_eq!(error.retry_after_secs(), Some(7));
    let (status, retry_after, body) = render(error).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(retry_after.as_deref(), Some("7"));
    assert_eq!(body["details"]["retry_after_secs"], 7);
}

#[tokio::test]
async fn test_with_retry_after_merges_into_existing_details() {
    let error = AppError::quota_exceeded("max_active_conversations", 3, 3, "").with_retry_after(0);
    assert_eq!(error.retry_after_secs(), Some(1), "floored at one second");
    let details = error.details.as_deref().unwrap();
    assert_eq!(details["limit_type"], "max_active_conversations");
    assert_eq!(details["current"], 3);
}

#[tokio::test]
async fn test_errors_without_a_window_render_no_retry_after() {
    for error in [
        AppError::internal("boom"),
        AppError::auth_invalid("bad token"),
        AppError::not_found("thing"),
    ] {
        let (_, retry_after, _) = render(error).await;
        assert_eq!(retry_after, None);
    }
    // A retry window on a status waiting cannot fix is not rendered.
    let (status, retry_after, _) = render(AppError::internal("boom").with_retry_after(5)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(retry_after, None);
}

#[test]
fn test_auth_refusal_keeps_a_spent_budget_and_a_server_fault() {
    let spent = AppError::rate_limit_exceeded(10_000, 10_000, 90)
        .into_auth_refusal("Authentication failed");
    assert_eq!(spent.code, ErrorCode::RateLimitExceeded);
    assert_eq!(spent.http_status(), 429);
    assert_eq!(spent.retry_after_secs(), Some(90));
    assert_eq!(spent.details.as_deref().unwrap()["limit"], 10_000);

    let database =
        AppError::database("connection reset").into_auth_refusal("Authentication failed");
    assert_eq!(database.code, ErrorCode::DatabaseError);
    assert_eq!(database.http_status(), 500);

    for refused in [
        AppError::auth_invalid("JWT validation failed: expired"),
        AppError::account_pending("pending approval"),
        AppError::not_found("User 42"),
    ] {
        let original = refused.to_string();
        let refusal = refused.into_auth_refusal("Authentication failed");
        assert_eq!(refusal.code, ErrorCode::AuthInvalid);
        assert_eq!(refusal.http_status(), 401);
        assert_eq!(
            refusal.message,
            format!("Authentication failed: {original}")
        );
    }
}
