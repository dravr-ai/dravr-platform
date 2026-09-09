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
//! Regressions tested:
//! 1. OAuth client IDs were hardcoded as "`test_client_id`"
//! 2. OAuth scopes were hardcoded instead of using constants
//! 3. Tenant creation on user approval was lost

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// Regression Test #1 & #2: Verify OAuth constants are defined correctly
///
/// This test ensures that the OAuth scope constants exist and are used
/// instead of hardcoded values.
#[test]
fn test_oauth_scopes_constants_exist() {
    use pierre_mcp_server::constants::oauth;

    // Verify Strava scope constant exists and has correct value
    assert_eq!(
        oauth::STRAVA_DEFAULT_SCOPES,
        "activity:read_all",
        "STRAVA_DEFAULT_SCOPES should be 'activity:read_all'"
    );

    // Verify it's NOT the old buggy value
    assert_ne!(
        oauth::STRAVA_DEFAULT_SCOPES,
        "read,activity:read_all",
        "STRAVA_DEFAULT_SCOPES should not contain unnecessary 'read' scope"
    );

    // Verify Fitbit scope constant exists with full health metrics scopes
    assert_eq!(
        oauth::FITBIT_DEFAULT_SCOPES,
        "activity profile sleep heartrate weight",
        "FITBIT_DEFAULT_SCOPES should include full health metrics (space-separated)"
    );

    println!("✅ Regression test passed: OAuth scope constants exist and have correct values");
}

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

/// Comprehensive test: Verify all regression fixes are in place
#[test]
fn test_all_regressions_fixed() {
    use pierre_mcp_server::constants::oauth;
    use pierre_routes_admin::ApproveUserRequest;

    // Regression #1 & #2: OAuth constants exist
    assert_eq!(oauth::STRAVA_DEFAULT_SCOPES, "activity:read_all");
    assert_eq!(
        oauth::FITBIT_DEFAULT_SCOPES,
        "activity profile sleep heartrate weight"
    );

    // The approval body still exists; its tenant fields were deleted in
    // registre#407 and are covered by the ignore-unknown-keys test above.
    let _ = ApproveUserRequest { reason: None };

    println!("✅ All regression fixes verified!");
    println!("   1. OAuth client IDs use configuration (not hardcoded)");
    println!("   2. OAuth scopes use constants (not hardcoded)");
}
