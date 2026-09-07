// ABOUTME: Pins what a channel-config read means for a turn — absent and failed are different facts
// ABOUTME: The distinction whose absence deleted a durable turn row on a single transient pool fault
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One `None` used to mean two things.
//!
//! `load_channel_config` folded "this tenant has no channel config" and "the
//! database did not answer" into the same `None`, and the dispatcher read that
//! as a turn that had ended — which finishes the turn by DELETING its row from
//! `messaging_resumable_turns`. A single transient pool fault therefore cost
//! the athlete their answer permanently: nothing sent, no record left, so no
//! sweep and no re-enqueue, and on a resumed run the status placeholder the
//! previous run opened stood on the channel forever (registre#109).
//!
//! The classification is asserted here rather than through the dispatcher
//! because the fault it must survive is a database error, and injecting one
//! through the real path would mean a forty-method fault-injecting repository
//! — more mock than the thing under test. The consequence of each variant is
//! covered where it belongs: `messaging_turn_lifecycle_test` proves a handed
//! back row is re-run and answered by the next instance.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::errors::AppError;
use pierre_mcp_server::services::messaging_ingress::{
    classify_channel_config, ChannelConfigLookup,
};
use serde_json::json;

/// A config shaped the way `dravr_canot::models::ChannelConfig` deserializes,
/// so a `Loaded` here means the real type parsed rather than a permissive one.
fn stored_config() -> serde_json::Value {
    json!({
        "id": "cfg-1",
        "tenant_id": "11111111-1111-1111-1111-111111111111",
        "channel_type": "telegram",
        "api_key": null,
        "api_secret": null,
        "webhook_secret": "wh-secret",
        "verify_token": null,
        "account_id": null,
        "phone_number": null,
        "bot_token": "123456:AAHk-test-token",
        "is_active": true,
    })
}

#[test]
fn a_failed_read_is_never_treated_as_an_absent_config() {
    // The whole point. A database error must NOT reach the caller as the same
    // answer that an unconfigured tenant produces, because the caller finishes
    // — and deletes — the turn on that answer.
    let failed = classify_channel_config(Err(AppError::database("connection closed")));
    assert!(
        matches!(failed, ChannelConfigLookup::Unavailable),
        "a read failure must hand the turn back, not end it"
    );

    let absent = classify_channel_config(Ok(None));
    assert!(
        matches!(absent, ChannelConfigLookup::Absent),
        "a tenant with no channel config genuinely cannot be replied to"
    );

    // Stated as the inequality the bug violated, so a future refactor that
    // re-merges the two arms fails here rather than in production.
    assert!(
        !matches!(
            classify_channel_config(Err(AppError::database("connection closed"))),
            ChannelConfigLookup::Absent
        ),
        "a transient fault must never classify as absent"
    );
}

#[test]
fn a_stored_config_loads() {
    let loaded = classify_channel_config(Ok(Some(stored_config())));
    assert!(
        matches!(loaded, ChannelConfigLookup::Loaded(_)),
        "a well-formed stored config is the turn's credentials"
    );
}

#[test]
fn a_config_that_does_not_deserialize_is_absent_not_retryable() {
    // Retrying reads the same bytes, so this is as terminal as having none —
    // and must not be confused with the transient case, which is retryable.
    let malformed = classify_channel_config(Ok(Some(json!({"channel_type": 12345, "id": 7}))));
    assert!(
        matches!(malformed, ChannelConfigLookup::Absent),
        "malformed stored config cannot be fixed by running the turn again"
    );
}
