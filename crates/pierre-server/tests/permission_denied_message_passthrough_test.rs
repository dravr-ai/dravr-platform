// ABOUTME: Pins that a PermissionDenied message reaches the client verbatim, and that the
// ABOUTME: operator-only codes beside it stay replaced by their constant description.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A refusal is the one error class whose message is the whole answer: "Group
//! coaching requires a Professional or Enterprise plan" tells the athlete what
//! to do, and `ErrorCode::PermissionDenied.description()` — "You do not have
//! permission to perform this action" — tells them nothing. So
//! `AppError::sanitized_message` passes those messages through, and these tests
//! hold that open while holding the internal codes shut.
//!
//! The standing commitment that comes with it — every future `PermissionDenied`
//! message being client-safe — is enforced by
//! `scripts/ci/check-permission-denied-messages.sh` against the reviewed
//! inventory in `scripts/ci/permission-denied-messages.txt`. This file pins the
//! behaviour; that gate pins the review.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::errors::{AppError, ErrorCode, ErrorResponse};

/// The refusals below are real messages from the reviewed inventory, one per
/// shape a handler writes: a plan gate, an ownership check, a membership check,
/// and an operator-privilege check.
const REVIEWED_REFUSALS: &[&str] = &[
    "Group coaching requires a Professional or Enterprise plan",
    "Only the conversation's owner can delete it",
    "Cannot remove the group owner",
    "Super-admin privileges required to create super-admin tokens",
];

#[test]
fn test_permission_denied_message_reaches_the_client_verbatim() {
    for refusal in REVIEWED_REFUSALS {
        let error = AppError::new(ErrorCode::PermissionDenied, *refusal);
        assert_eq!(
            error.sanitized_message(),
            *refusal,
            "a reviewed refusal must reach the client as written"
        );
        assert_ne!(
            error.sanitized_message(),
            ErrorCode::PermissionDenied.description(),
            "'{refusal}' was replaced by the constant description, which names nothing"
        );
    }
}

#[test]
fn test_permission_denied_refusal_survives_the_http_response_body() {
    let error = AppError::new(
        ErrorCode::PermissionDenied,
        "Group coaching requires a Professional or Enterprise plan",
    );
    let json = serde_json::to_string(&ErrorResponse::from(error)).unwrap();

    assert!(
        json.contains("Group coaching requires a Professional or Enterprise plan"),
        "the refusal must survive ErrorResponse, which is what the 403 body carries: {json}"
    );
    assert!(
        json.contains("\"code\":\"PermissionDenied\""),
        "the client keys recovery off the code, so it must still be PermissionDenied: {json}"
    );
    assert!(
        !json.contains("You do not have permission to perform this action"),
        "the constant description must not replace the refusal: {json}"
    );
}

#[test]
fn test_internal_and_database_messages_stay_sanitized() {
    let internal = AppError::internal("MEK decrypt failed for tenant row 41ad3c02");
    assert_eq!(
        internal.sanitized_message(),
        "An internal server error occurred"
    );
    assert!(
        !internal.sanitized_message().contains("41ad3c02"),
        "an internal message must never reach the client"
    );

    let database = AppError::database("relation \"user_mcp_tokens\" does not exist");
    assert_eq!(database.sanitized_message(), "Database operation failed");
    assert!(
        !database.sanitized_message().contains("user_mcp_tokens"),
        "a database message must never reach the client"
    );

    // The full text stays available to the operator on both.
    assert!(internal.internal_details().contains("41ad3c02"));
    assert!(database.internal_details().contains("user_mcp_tokens"));
}

#[test]
fn test_account_status_codes_share_the_403_but_not_the_passthrough() {
    // Every code below is a 403 like PermissionDenied, so status alone cannot be
    // what decides passthrough — the code is.
    let pending = AppError::account_pending("user 7f21 awaits approval by admin@example.com");
    assert_eq!(pending.http_status(), 403);
    assert_eq!(
        pending.sanitized_message(),
        "Account is pending admin approval"
    );

    let expired = AppError::auth_expired();
    assert_eq!(expired.http_status(), 403);
    assert_eq!(
        expired.sanitized_message(),
        "The authentication token has expired"
    );

    let refused = AppError::new(ErrorCode::PermissionDenied, "Cannot remove the group owner");
    assert_eq!(refused.http_status(), 403);
    assert_eq!(refused.sanitized_message(), "Cannot remove the group owner");
}
