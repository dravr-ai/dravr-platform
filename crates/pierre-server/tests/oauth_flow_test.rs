// ABOUTME: Tests for OAuth flow redirect URL validation
// ABOUTME: Ensures open-redirect prevention by validating redirect URLs against allowlist
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for OAuth flow redirect URL validation and open-redirect prevention.

use pierre_mcp_server::oauth_flow;

#[test]
fn test_app_scheme_pierre_allowed() {
    assert!(oauth_flow::is_allowed_redirect_url(
        "pierre://oauth/callback",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_app_scheme_expo_allowed() {
    assert!(oauth_flow::is_allowed_redirect_url(
        "exp://192.168.1.1:8082/--/oauth",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_localhost_allowed() {
    assert!(oauth_flow::is_allowed_redirect_url(
        "http://localhost:8081/oauth/callback",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_https_matching_base_url_allowed() {
    assert!(oauth_flow::is_allowed_redirect_url(
        "https://api.dravr.ai/oauth/callback?success=true",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_https_arbitrary_domain_rejected() {
    assert!(!oauth_flow::is_allowed_redirect_url(
        "https://evil.com/steal-token",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_https_subdomain_attack_rejected() {
    // Attacker uses api.dravr.ai.evil.com — should be rejected
    assert!(!oauth_flow::is_allowed_redirect_url(
        "https://api.dravr.ai.evil.com/steal",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_https_extra_origin_allowed() {
    let extra = vec!["https://tunnel.cloudflare.com".to_owned()];
    assert!(oauth_flow::is_allowed_redirect_url(
        "https://tunnel.cloudflare.com/oauth/callback",
        "https://api.dravr.ai",
        &extra,
    ));
}

#[test]
fn test_https_not_in_extra_origins_rejected() {
    let extra = vec!["https://tunnel.cloudflare.com".to_owned()];
    assert!(!oauth_flow::is_allowed_redirect_url(
        "https://other-tunnel.ngrok.io/steal",
        "https://api.dravr.ai",
        &extra,
    ));
}

#[test]
fn test_http_non_localhost_rejected() {
    assert!(!oauth_flow::is_allowed_redirect_url(
        "http://evil.com/steal",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_javascript_scheme_rejected() {
    assert!(!oauth_flow::is_allowed_redirect_url(
        "javascript:alert(1)",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_data_scheme_rejected() {
    assert!(!oauth_flow::is_allowed_redirect_url(
        "data:text/html,<script>alert(1)</script>",
        "https://api.dravr.ai",
        &[],
    ));
}

#[test]
fn test_extract_state_with_allowed_redirect() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let redirect = "pierre://oauth/callback";
    let encoded = URL_SAFE_NO_PAD.encode(redirect.as_bytes());
    let state = format!("user-id:random-uuid:{encoded}");

    let result =
        oauth_flow::extract_mobile_redirect_from_state(&state, "https://api.dravr.ai", &[]);
    assert_eq!(result, Some(redirect.to_owned()));
}

#[test]
fn test_extract_state_with_rejected_redirect() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let redirect = "https://evil.com/steal";
    let encoded = URL_SAFE_NO_PAD.encode(redirect.as_bytes());
    let state = format!("user-id:random-uuid:{encoded}");

    let result =
        oauth_flow::extract_mobile_redirect_from_state(&state, "https://api.dravr.ai", &[]);
    assert_eq!(result, None);
}

#[test]
fn test_extract_state_without_redirect() {
    let state = "user-id:random-uuid";

    let result = oauth_flow::extract_mobile_redirect_from_state(state, "https://api.dravr.ai", &[]);
    assert_eq!(result, None);
}
