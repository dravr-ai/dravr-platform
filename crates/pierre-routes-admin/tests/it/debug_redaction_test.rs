// ABOUTME: Pins the device-approval page's Debug impls, which hide the user code and password
// ABOUTME: A future derive(Debug) on the approval form or page query fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Debug;

use pierre_routes_admin::handlers::device_web::{DeviceApproveForm, DevicePageQuery};

/// Formats `value` both ways, asserts none of `secrets` appears in either,
/// and returns the compact form for further checks.
fn printed_without(value: &impl Debug, secrets: &[&str]) -> String {
    let compact = format!("{value:?}");
    let pretty = format!("{value:#?}");
    for secret in secrets {
        assert!(!compact.contains(secret), "{secret} leaked: {compact}");
        assert!(!pretty.contains(secret), "{secret} leaked: {pretty}");
    }
    compact
}

#[test]
fn the_device_page_query_never_prints_the_user_code() {
    let query = DevicePageQuery {
        user_code: Some("SENT-7F3A".to_owned()),
    };
    let printed = printed_without(&query, &["SENT-7F3A"]);
    assert!(printed.contains("user_code: \"[REDACTED]\""), "{printed}");
}

#[test]
fn the_device_approval_form_never_prints_the_password_or_user_code() {
    let form = DeviceApproveForm {
        email: "operator@dravr.ai".to_owned(),
        password: "sentinel-password-7f3a".to_owned(),
        user_code: "SENT-7F3A".to_owned(),
        action: Some("approve".to_owned()),
    };
    let printed = printed_without(&form, &["sentinel-password-7f3a", "SENT-7F3A"]);
    assert_eq!(printed.matches("[REDACTED]").count(), 2, "{printed}");
    assert!(printed.contains("operator@dravr.ai"), "{printed}");
}
