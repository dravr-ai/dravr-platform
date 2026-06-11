// ABOUTME: Unit tests for slack_actions ops-channel trust authorization
// ABOUTME: Verifies channel_matches honors only the configured ops channel (name or ID, # optional)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Slack ops-action authorization unit tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate, so without a surviving crate doc the command-line
// `-D warnings` trips `missing_docs`.
#![cfg(feature = "client-messaging")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pierre_mcp_server::routes::messaging::slack_actions::channel_matches;

#[test]
fn matches_configured_channel_by_name_with_hash() {
    // Config carries the `#` prefix (as set in terraform); payload name omits it.
    assert!(channel_matches(
        "dravr-dev-users",
        "C0ABC123",
        "#dravr-dev-users"
    ));
}

#[test]
fn matches_configured_channel_by_name_without_hash() {
    assert!(channel_matches(
        "dravr-dev-users",
        "C0ABC123",
        "dravr-dev-users"
    ));
}

#[test]
fn matches_configured_channel_by_id() {
    // Operators may configure a raw channel ID instead of a name.
    assert!(channel_matches("", "C0ABC123", "C0ABC123"));
}

#[test]
fn tolerates_surrounding_whitespace_in_config() {
    assert!(channel_matches(
        "dravr-dev-users",
        "C0ABC123",
        "  #dravr-dev-users  "
    ));
}

#[test]
fn rejects_other_channel() {
    assert!(!channel_matches(
        "random-chat",
        "C0XYZ999",
        "#dravr-dev-users"
    ));
}

#[test]
fn rejects_dm_channel() {
    // DMs report a name like "directmessage" and an ID starting with `D`.
    assert!(!channel_matches(
        "directmessage",
        "D0ABC123",
        "#dravr-dev-users"
    ));
}

#[test]
fn rejects_empty_configured_channel_fails_closed() {
    // An unset/blank channel must never authorize, even against a blank payload.
    assert!(!channel_matches("dravr-dev-users", "C0ABC123", ""));
    assert!(!channel_matches("dravr-dev-users", "C0ABC123", "   "));
    assert!(!channel_matches("", "", ""));
}
