// ABOUTME: Pins the OAuth login and consent pages' Debug impls, which hide state, CSRF and passwords
// ABOUTME: A future derive(Debug) on either page's parameters fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Debug;

use pierre_contremaitre::MessagingStringsRegistry;
use pierre_routes_identity::{ConsentHtmlParams, LoginHtmlParams, LoginPageLabels};

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
fn the_login_page_never_prints_its_state_password_or_google_link() {
    let labels = LoginPageLabels::new(&MessagingStringsRegistry::new(), "en");
    let params = LoginHtmlParams {
        client_id: "mcp-client",
        redirect_uri: "https://client.example/callback",
        response_type: "code",
        state: "sentinel-state-7f3a",
        scope: "read",
        code_challenge: "challenge",
        code_challenge_method: "S256",
        resource: "",
        default_email: "dev@example.com",
        default_password: "sentinel-password-7f3a",
        google_start_url: Some("/oauth2/google/start?state=sentinel-google-7f3a"),
        labels: &labels,
    };
    let printed = printed_without(
        &params,
        &[
            "sentinel-state-7f3a",
            "sentinel-password-7f3a",
            "sentinel-google-7f3a",
        ],
    );
    assert_eq!(printed.matches("[REDACTED]").count(), 3, "{printed}");
    assert!(printed.contains("mcp-client"), "{printed}");
}

#[test]
fn the_consent_page_never_prints_its_state_csrf_token_or_switch_link() {
    let params = ConsentHtmlParams {
        client_id: "mcp-client",
        redirect_uri: "https://client.example/callback",
        response_type: "code",
        state: "sentinel-state-7f3a",
        scope: "read",
        code_challenge: "challenge",
        code_challenge_method: "S256",
        resource: "",
        csrf_token: "sentinel-csrf-7f3a",
        client_label: "Example Client",
        account_email: "athlete@example.com",
        switch_account_url: "/oauth2/login?state=sentinel-switch-7f3a",
    };
    let printed = printed_without(
        &params,
        &[
            "sentinel-state-7f3a",
            "sentinel-csrf-7f3a",
            "sentinel-switch-7f3a",
        ],
    );
    assert_eq!(printed.matches("[REDACTED]").count(), 3, "{printed}");
    assert!(printed.contains("Example Client"), "{printed}");
}
