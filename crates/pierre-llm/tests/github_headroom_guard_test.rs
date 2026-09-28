// ABOUTME: Pins the chain guard's GitHub budget gate: embacle's headroom reading in, skip-the-primary out
// ABOUTME: Trips below 200 remaining, passes at 200, and fails closed when the budget cannot be read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The guard's GitHub budget comes from embacle's `GithubHeadroomChecker` on
//! the token Copilot spends. What the platform owns is the floor — skip the
//! primary with fewer than 200 requests left — and what a failed read means:
//! a budget that cannot be read is not headroom, so the guard fails closed.
//!
//! Every test builds its own `ChainGuard` rather than touching the
//! process-wide one, so they cannot reroute each other. The end-to-end cases
//! serve GitHub's `/rate_limit` from a loopback port and read it through the
//! real checker, so the counts the guard acts on are embacle's parse of the
//! wire, not values a test typed in.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::{Duration, UNIX_EPOCH};

use embacle::quota_http::{GithubHeadroomChecker, GithubRateLimit};
use embacle::types::RunnerError;
use pierre_llm::chain_guard::{ChainGuard, RateLimitTransition, GITHUB_BUDGET_THRESHOLD};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const RESET: u64 = 1_790_000_000;

/// A successful read: `remaining` of 5000 left, resetting at [`RESET`].
fn counts(remaining: u64) -> GithubRateLimit {
    GithubRateLimit {
        remaining,
        limit: 5000,
        used: 5000 - remaining,
        resets_at: UNIX_EPOCH + Duration::from_secs(RESET),
    }
}

/// Answer every request on a loopback port with `status` and `body`, and
/// return the URL a checker reads the rate limit from.
async fn rate_limit_endpoint(status: &'static str, body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/rate_limit", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    url
}

fn core_body(remaining: u64) -> String {
    format!(
        r#"{{"resources":{{"core":{{"limit":5000,"used":{},"remaining":{remaining},"reset":{RESET}}}}},"rate":{{"limit":5000,"used":{},"remaining":{remaining},"reset":{RESET}}}}}"#,
        5000 - remaining,
        5000 - remaining
    )
}

async fn read_through_checker(
    status: &'static str,
    body: String,
) -> Result<GithubRateLimit, RunnerError> {
    let url = rate_limit_endpoint(status, body).await;
    GithubHeadroomChecker::new("ghp_test_token")
        .with_url(url)
        .rate_limit()
        .await
}

#[test]
fn the_floor_is_200_remaining() {
    assert_eq!(GITHUB_BUDGET_THRESHOLD, 200);
}

#[test]
fn a_guard_that_has_never_read_the_budget_uses_the_primary() {
    let guard = ChainGuard::new();
    assert!(!guard.is_github_budget_low());
    assert_eq!(guard.github_remaining(), None);
}

#[test]
fn a_reading_of_199_remaining_trips_the_guard() {
    let guard = ChainGuard::new();
    assert_eq!(
        guard.record_github_headroom(&Ok(counts(199))),
        RateLimitTransition::EnteredLow
    );
    assert!(guard.is_github_budget_low());
    assert_eq!(guard.github_remaining(), Some(199));
    assert_eq!(guard.github_reset_at(), RESET);
}

#[test]
fn a_reading_of_200_remaining_passes() {
    let guard = ChainGuard::new();
    assert_eq!(
        guard.record_github_headroom(&Ok(counts(200))),
        RateLimitTransition::StillOk
    );
    assert!(!guard.is_github_budget_low());

    // And climbing back to 200 from below is a recovery.
    guard.record_github_headroom(&Ok(counts(199)));
    assert_eq!(
        guard.record_github_headroom(&Ok(counts(200))),
        RateLimitTransition::ExitedLow
    );
    assert!(!guard.is_github_budget_low());
}

#[test]
fn a_failed_read_fails_closed_until_a_read_succeeds() {
    let guard = ChainGuard::new();
    guard.record_github_headroom(&Ok(counts(4000)));
    assert!(!guard.is_github_budget_low());

    let failed = Err(RunnerError::external_service(
        "github-headroom",
        "rate_limit returned HTTP 503",
    ));
    assert_eq!(
        guard.record_github_headroom(&failed),
        RateLimitTransition::EnteredLow,
        "a budget that cannot be read is not headroom"
    );
    assert!(guard.is_github_budget_low());
    assert_eq!(guard.github_remaining(), None, "no reading is held");
    assert_eq!(
        guard.github_reset_at(),
        RESET,
        "the last known reset is kept"
    );

    assert_eq!(
        guard.record_github_headroom(&failed),
        RateLimitTransition::StillLow
    );
    assert_eq!(
        guard.record_github_headroom(&Ok(counts(4000))),
        RateLimitTransition::ExitedLow
    );
    assert!(!guard.is_github_budget_low());
}

#[tokio::test]
async fn a_wire_reading_of_199_remaining_trips_the_guard() {
    let reading = read_through_checker("200 OK", core_body(199)).await;
    assert_eq!(reading.as_ref().unwrap().remaining, 199);

    let guard = ChainGuard::new();
    assert_eq!(
        guard.record_github_headroom(&reading),
        RateLimitTransition::EnteredLow
    );
    assert!(guard.is_github_budget_low());
}

#[tokio::test]
async fn a_wire_reading_of_200_remaining_passes() {
    let reading = read_through_checker("200 OK", core_body(200)).await;
    assert_eq!(reading.as_ref().unwrap().remaining, 200);

    let guard = ChainGuard::new();
    guard.record_github_headroom(&reading);
    assert!(!guard.is_github_budget_low());
}

#[tokio::test]
async fn an_error_status_or_a_changed_shape_fails_closed() {
    for (status, body) in [
        ("503 Service Unavailable", "{}".to_owned()),
        (
            "401 Unauthorized",
            r#"{"message":"Bad credentials"}"#.to_owned(),
        ),
        ("200 OK", r#"{"rate":{"remaining":4999}}"#.to_owned()),
    ] {
        let reading = read_through_checker(status, body).await;
        assert!(reading.is_err(), "{status}: {reading:?}");

        let guard = ChainGuard::new();
        assert_eq!(
            guard.record_github_headroom(&reading),
            RateLimitTransition::EnteredLow,
            "{status}"
        );
        assert!(guard.is_github_budget_low(), "{status}");
    }
}

#[tokio::test]
async fn an_unreachable_endpoint_fails_closed() {
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let reading = GithubHeadroomChecker::new("ghp_test_token")
        .with_url(format!("http://127.0.0.1:{port}/rate_limit"))
        .rate_limit()
        .await;
    assert!(reading.is_err());

    let guard = ChainGuard::new();
    guard.record_github_headroom(&reading);
    assert!(guard.is_github_budget_low());
}
