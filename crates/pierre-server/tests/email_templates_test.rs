// ABOUTME: Tests for registration lifecycle email HTML templates
// ABOUTME: Verifies greeting, status copy, and sign-in CTA rendering for pending/approved emails
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_mcp_server::email::templates::{registration_approved_html, registration_pending_html};

#[test]
fn pending_email_uses_named_greeting_when_display_name_present() {
    let html = registration_pending_html(Some("Alice"));

    assert!(
        html.contains("Hi Alice,"),
        "pending email should greet the user by display name"
    );
    assert!(
        html.contains("pending administrator review"),
        "pending email should explain the account is awaiting review"
    );
    assert!(
        html.contains("Thanks for signing up"),
        "pending email should thank the user for signing up"
    );
    assert!(
        html.starts_with("<!DOCTYPE html>"),
        "pending email should be a well-formed HTML document"
    );
}

#[test]
fn pending_email_falls_back_to_generic_greeting_without_display_name() {
    let html = registration_pending_html(None);

    assert!(
        html.contains("Hi there,"),
        "pending email should use a generic greeting when no display name is provided"
    );
    assert!(
        !html.contains("Hi ,"),
        "pending email must never render an empty-name greeting"
    );
}

#[test]
fn pending_email_treats_blank_display_name_as_absent() {
    let html = registration_pending_html(Some("   "));

    assert!(
        html.contains("Hi there,"),
        "pending email should ignore whitespace-only display names"
    );
}

#[test]
fn approved_email_renders_sign_in_cta_when_url_provided() {
    let html = registration_approved_html(Some("Bob"), Some("https://app.dravr.ai/login"));

    assert!(html.contains("Hi Bob,"));
    assert!(
        html.contains("https://app.dravr.ai/login"),
        "approved email should embed the configured sign-in URL"
    );
    assert!(
        html.contains("Sign in to Dravr"),
        "approved email should label the sign-in CTA"
    );
    assert!(
        html.contains("You're in"),
        "approved email should include a welcome headline"
    );
}

#[test]
fn approved_email_omits_cta_when_no_url_configured() {
    let html = registration_approved_html(Some("Carol"), None);

    assert!(html.contains("Hi Carol,"));
    assert!(
        !html.contains("Sign in to Dravr"),
        "approved email should omit the sign-in CTA when no frontend URL is configured"
    );
    assert!(
        !html.contains("href=\"\""),
        "approved email must not render an empty href"
    );
}

#[test]
fn approved_email_handles_missing_display_name() {
    let html = registration_approved_html(None, Some("https://app.dravr.ai/login"));

    assert!(html.contains("Hi there,"));
    assert!(html.contains("https://app.dravr.ai/login"));
}

#[test]
fn both_templates_reference_dravr_branding() {
    let pending = registration_pending_html(None);
    let approved = registration_approved_html(None, None);

    for (label, html) in [("pending", &pending), ("approved", &approved)] {
        assert!(
            html.contains("Dravr"),
            "{label} email should carry Dravr branding"
        );
        assert!(
            html.contains("&copy; Dravr"),
            "{label} email should render the Dravr footer"
        );
    }
}

#[test]
fn display_name_is_html_escaped_to_prevent_injection() {
    let malicious = "<img src=x onerror=alert(1)>";

    let pending = registration_pending_html(Some(malicious));
    assert!(
        !pending.contains(malicious),
        "pending email must not render raw HTML from display_name"
    );
    assert!(
        pending.contains("&lt;img"),
        "pending email should HTML-escape angle brackets in display_name"
    );

    let approved = registration_approved_html(Some(malicious), None);
    assert!(
        !approved.contains(malicious),
        "approved email must not render raw HTML from display_name"
    );
    assert!(
        approved.contains("&lt;img"),
        "approved email should HTML-escape angle brackets in display_name"
    );
}

#[test]
fn sign_in_url_is_attribute_escaped_to_prevent_injection() {
    let malicious = "https://evil.example/\" onclick=\"alert(1)";

    let html = registration_approved_html(Some("Dave"), Some(malicious));

    assert!(
        !html.contains("onclick=\"alert(1)"),
        "approved email must not leak raw quote-break payloads into the href"
    );
    assert!(
        html.contains("&quot;"),
        "approved email should attribute-escape embedded quotes in sign_in_url"
    );
}
