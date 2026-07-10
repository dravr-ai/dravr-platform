// ABOUTME: Regression test for sciotte credential-login failure handling (no raw error to the user)
// ABOUTME: Pins the friendly, provider-aware copy and that system failures never leak browser stderr
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guards the user-facing contract of `report_login_system_failure` /
//! `friendly_login_failure_message` in `pierre_routes_auth::sciotte`.
//!
//! A sciotte credential login runs an in-process headless Chrome. When that
//! Chrome fails to launch (the container's dbus/crashpad case), the scraper
//! returns an `Err(ScraperError::Browser)` whose `Display` is a multi-line
//! dump of raw browser stderr. Historically the handler built
//! `format!("Login failed: {error}")` and returned it as an `InvalidInput`
//! `AppError`, which `sanitized_message()` exposes **verbatim** — so the whole
//! stderr blob reached the login modal / hosted page (observed in production
//! 2026-07-10). These tests pin that the *system*-failure path now yields a
//! friendly, provider-aware message carrying no technical detail, while the
//! full reason still goes to the `error!` log + `#dev-dravr-errors` alert.
//!
//! The login handlers themselves need a real browser to exercise, so these
//! two helpers are the testable seam (re-exported `#[doc(hidden)]` from the
//! otherwise-private `sciotte` module).

use pierre_routes_auth::{friendly_login_failure_message, report_login_system_failure};
use uuid::Uuid;

/// The exact class of raw stderr a Chrome-launch failure surfaces — the
/// production 2026-07-10 message, trimmed. None of these tokens may ever reach
/// the end user.
const RAW_BROWSER_ERROR: &str = "browser error: Failed to launch browser: Browser process exited \
     with status ExitStatus(unix_wait_status(5)) before websocket URL could be resolved, stderr: \
     BrowserStderr(\"[98:112:0710/131605.648448:ERROR:dbus/bus.cc:405] Failed to connect to the \
     bus\\n[0710/131605.723617:ERROR:third_party/crashpad/...] scaling_cur_freq: No such file\")";

/// Tokens from the raw browser stderr that must never appear in anything the
/// user sees.
const LEAK_TOKENS: &[&str] = &[
    "browser error",
    "Failed to launch",
    "ExitStatus",
    "unix_wait_status",
    "stderr",
    "BrowserStderr",
    "dbus",
    "crashpad",
    "websocket",
];

#[test]
fn system_failure_returns_friendly_message_not_raw_stderr() {
    let err = report_login_system_failure(
        Uuid::new_v4(),
        Uuid::new_v4(),
        "sciotte",
        "credential_login",
        &RAW_BROWSER_ERROR.to_owned(),
    );

    // `sanitized_message()` is what the client renders. For the screenshot bug
    // it echoed the full stderr; it must now be the friendly copy verbatim.
    let shown = err.sanitized_message();
    assert_eq!(shown, friendly_login_failure_message("sciotte"));

    for &leak in LEAK_TOKENS {
        assert!(
            !shown.contains(leak),
            "user-facing message leaked `{leak}`: {shown}"
        );
    }
    // ...and it stays actionable.
    assert!(shown.contains("Strava"), "expected provider name: {shown}");
    assert!(
        shown.to_lowercase().contains("try again"),
        "expected retry guidance: {shown}"
    );
}

#[test]
fn friendly_message_is_provider_aware() {
    let strava = friendly_login_failure_message("sciotte");
    let garmin = friendly_login_failure_message("sciotte_garmin");

    assert!(strava.contains("Strava"));
    assert!(!strava.contains("Garmin"));
    assert!(garmin.contains("Garmin"));
    assert!(!garmin.contains("Strava"));
}

#[test]
fn friendly_message_carries_no_technical_detail_for_any_provider() {
    for provider in ["sciotte", "sciotte_garmin", "strava", "garmin", ""] {
        let message = friendly_login_failure_message(provider);
        let lower = message.to_lowercase();
        for token in [
            "browser", "error", "chrome", "stderr", "exit", "http", "://",
        ] {
            assert!(
                !lower.contains(token),
                "friendly copy for `{provider}` leaked `{token}`: {message}"
            );
        }
    }
}
