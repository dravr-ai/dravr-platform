// ABOUTME: Unit tests for the device-login approval prompt's intro-line selection
// ABOUTME: Pins the three-way UX contract (headless / browser-launched / copy-paste) without a browser
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_cli::remote::device_flow::approval_intro;

#[test]
fn intro_when_browser_launched_says_opening() {
    let line = approval_intro(false, true);
    assert!(
        line.contains("Opening your browser"),
        "launched intro must announce the auto-open: {line}"
    );
}

#[test]
fn intro_when_not_launched_asks_to_open_the_url() {
    // --no-browser, or open() failed: the URL is present but we didn't launch it.
    let line = approval_intro(false, false);
    assert!(
        line.contains("open this URL in your browser"),
        "copy-paste intro must ask the operator to open the URL: {line}"
    );
    assert!(
        !line.contains("Opening your browser"),
        "must not claim to have opened a browser it did not: {line}"
    );
}

#[test]
fn intro_when_no_url_points_at_headless_approval() {
    // Server BASE_URL unset → no browser URL at all; steer to the headless path.
    let line = approval_intro(true, false);
    assert!(
        line.contains("headless"),
        "empty-URL intro must steer to headless approval: {line}"
    );
}
