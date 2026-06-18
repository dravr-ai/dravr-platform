// ABOUTME: Unit coverage for pierre-email's Resend 429 backoff decision logic
// ABOUTME: Verifies reset-header parsing and the wait-then-retry vs give-up policy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use std::time::Duration;

use pierre_email::{parse_retry_delay, plan_retry};

#[test]
fn parse_prefers_retry_after_over_ratelimit_reset() {
    assert_eq!(
        parse_retry_delay(Some("2"), Some("9")),
        Some(Duration::from_secs(2))
    );
}

#[test]
fn parse_falls_back_to_ratelimit_reset() {
    assert_eq!(
        parse_retry_delay(None, Some("1")),
        Some(Duration::from_secs(1))
    );
    // Unparseable Retry-After must not shadow a valid reset header.
    assert_eq!(
        parse_retry_delay(Some("oops"), Some("3")),
        Some(Duration::from_secs(3))
    );
}

#[test]
fn parse_is_none_when_headers_absent_or_blank() {
    assert_eq!(parse_retry_delay(None, None), None);
    assert_eq!(parse_retry_delay(Some(""), Some(" ")), None);
}

#[test]
fn plan_waits_out_short_window_while_attempts_remain() {
    assert_eq!(
        plan_retry(0, Some(Duration::from_secs(1))),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        plan_retry(2, Some(Duration::from_secs(2))),
        Some(Duration::from_secs(2))
    );
}

#[test]
fn plan_floors_zero_second_reset_to_avoid_tight_loop() {
    // A 0s reset floors to RETRY_DELAY_FLOOR (200ms), never a 0-delay tight loop.
    assert_eq!(
        plan_retry(0, Some(Duration::ZERO)),
        Some(Duration::from_millis(200))
    );
}

#[test]
fn plan_uses_default_delay_when_hint_missing() {
    assert_eq!(plan_retry(0, None), Some(Duration::from_secs(1)));
}

#[test]
fn plan_gives_up_after_max_attempts() {
    assert_eq!(plan_retry(3, Some(Duration::from_secs(1))), None);
    assert_eq!(plan_retry(99, Some(Duration::from_secs(1))), None);
}

#[test]
fn plan_gives_up_on_long_reset_window() {
    // A multi-minute reset signals a daily/monthly cap; don't block the caller.
    assert_eq!(plan_retry(0, Some(Duration::from_mins(2))), None);
}
