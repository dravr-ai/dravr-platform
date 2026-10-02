// ABOUTME: Regression tests locking in the Warp → Axum HTTP-stack migration
// ABOUTME: Verifies handler signatures, extractor behaviour, and routing parity vs the old Warp surface
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression tests for Warp to Axum migration
//!
//! This test suite verifies that critical regressions introduced during the
//! Warp → Axum migration (commit 439da5853fbc209e36d34b4dd56eb2a3aed8c6f6)
//! have been fixed and do not reoccur.
//!
//! The migration lost three things: OAuth client ids became a hardcoded
//! "`test_client_id`", OAuth scopes were hardcoded, and tenant creation on user
//! approval disappeared. The first two are read from configuration now and
//! tested where that happens (`pierre-auth`'s `oauth_env_reader_test`); what is
//! left here is the approval request body.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// `ApproveUserRequest` deserializes, and ignores the tenant keys it no longer has
///
/// The struct carried `create_default_tenant`, `tenant_name` and `tenant_slug` until
/// registre#407: no client ever sent them, and whether an approved user needs a
/// default tenant is a property of the user rather than an operator's choice —
/// the Slack approval path already decides it that way, provisioning only
/// `if !has_tenants`. They were deleted rather than mirrored onto the cookie
/// surface.
///
/// What is worth pinning after a wire field is removed is that removing it did
/// not turn old callers into 400s. serde ignores unknown keys by default and
/// this struct sets no `deny_unknown_fields`, so a caller still sending the old
/// body is accepted and the keys are dropped. That is the intended behaviour,
/// and it is one `deny_unknown_fields` away from silently breaking.
#[test]
fn test_approve_user_request_ignores_removed_tenant_keys() {
    use pierre_routes_admin::ApproveUserRequest;

    let json = r#"{
        "reason": "Approved for testing",
        "create_default_tenant": true,
        "tenant_name": "My Company",
        "tenant_slug": "my-company"
    }"#;

    let request: ApproveUserRequest = serde_json::from_str(json)
        .expect("a body carrying the removed tenant keys must still parse");
    assert_eq!(request.reason, Some("Approved for testing".to_owned()));

    // And the field that remains is genuinely optional.
    let bare: ApproveUserRequest =
        serde_json::from_str("{}").expect("an empty approval body must parse");
    assert_eq!(bare.reason, None);

    println!("✅ ApproveUserRequest parses, and ignores the removed tenant keys");
}
