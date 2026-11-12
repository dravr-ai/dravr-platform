// ABOUTME: Tests for OAuth template-based HTML rendering
// ABOUTME: Validates template compilation, placeholder replacement, and HTML generation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for OAuth template-based HTML rendering
//!
//! This test suite validates:
//! 1. Template files exist and compile at build time
//! 2. All placeholders in templates match code expectations
//! 3. HTML rendering produces valid output
//! 4. Error templates render correctly

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::helpers::axum_test::AxumTestRequest;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::html::escape_html_attribute;
use pierre_core::models::SUPPORTED_LOCALES;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_providers::registry::ProviderRegistry;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_identity::{ConsentText, LoginPageLabels};
use regex::Regex;
use std::sync::LazyLock;

/// The catalogue the consent pages read.
static STRINGS: LazyLock<MessagingStringsRegistry> = LazyLock::new(MessagingStringsRegistry::new);

/// A catalogue line's positional slot.
static SLOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\d+\}").unwrap());

/// The login form's English text, from the catalogue the server renders it
/// with.
static ENGLISH_LABELS: LazyLock<LoginPageLabels> =
    LazyLock::new(|| LoginPageLabels::new(&MessagingStringsRegistry::new(), "en"));
use crate::common;

use pierre_routes_identity::OAuth2Routes;

/// Test that OAuth login template compiles and contains required placeholders
#[test]
fn test_oauth_login_template_exists() {
    // This test validates that the template file exists and can be loaded at compile time
    // If the file doesn't exist, compilation will fail
    const TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login.html");

    // Verify all required placeholders exist
    let required_placeholders = [
        "{{CLIENT_ID}}",
        "{{REDIRECT_URI}}",
        "{{RESPONSE_TYPE}}",
        "{{STATE}}",
        "{{SCOPE}}",
        "{{CODE_CHALLENGE}}",
        "{{CODE_CHALLENGE_METHOD}}",
        "{{DEFAULT_EMAIL}}",
        "{{DEFAULT_PASSWORD}}",
        "{{GOOGLE_SIGN_IN}}",
    ];

    for placeholder in &required_placeholders {
        assert!(
            TEMPLATE.contains(placeholder),
            "OAuth login template missing required placeholder: {placeholder}"
        );
    }

    // Verify template structure - should be valid HTML
    assert!(TEMPLATE.contains("<!DOCTYPE html>"), "Missing DOCTYPE");
    assert!(TEMPLATE.contains("<html"), "Missing html tag");
    assert!(TEMPLATE.contains("</html>"), "Missing closing html tag");
    assert!(
        TEMPLATE.contains("<form method=\"post\" action=\"/oauth2/login\">"),
        "Missing login form"
    );
    assert!(
        TEMPLATE.contains("type=\"email\""),
        "Missing email input field"
    );
    assert!(
        TEMPLATE.contains("type=\"password\""),
        "Missing password input field"
    );
    assert!(
        TEMPLATE.contains("type=\"submit\""),
        "Missing submit button"
    );
}

/// Test that OAuth login error template compiles and contains required placeholders
#[test]
fn test_oauth_login_error_template_exists() {
    // This test validates that the error template file exists and can be loaded at compile time
    // If the file doesn't exist, compilation will fail
    const TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login_error.html");

    // Verify all required placeholders exist
    let required_placeholders = [
        "{{ERROR_MESSAGE}}",
        "{{CLIENT_ID}}",
        "{{REDIRECT_URI}}",
        "{{RESPONSE_TYPE}}",
        "{{STATE}}",
        "{{SCOPE}}",
        "{{CODE_CHALLENGE}}",
        "{{CODE_CHALLENGE_METHOD}}",
    ];

    for placeholder in &required_placeholders {
        assert!(
            TEMPLATE.contains(placeholder),
            "OAuth login error template missing required placeholder: {placeholder}"
        );
    }

    // Verify template structure - should be valid HTML with error messaging
    assert!(TEMPLATE.contains("<!DOCTYPE html>"), "Missing DOCTYPE");
    assert!(TEMPLATE.contains("<html"), "Missing html tag");
    assert!(TEMPLATE.contains("</html>"), "Missing closing html tag");
    // The page's words come from the catalogue in the athlete's language
    // (carnet#787): the template names them, never a fixed English string.
    for placeholder in ["{{LANG}}", "{{T_FAILED}}", "{{T_BACK}}"] {
        assert!(
            TEMPLATE.contains(placeholder),
            "Missing localized text placeholder {placeholder}"
        );
    }
}

/// The login page's parameters for one pending request, with the Google
/// start URL given.
fn google_login_params(
    google_start_url: Option<&str>,
) -> pierre_routes_identity::LoginHtmlParams<'_> {
    pierre_routes_identity::LoginHtmlParams {
        client_id: "client&id",
        redirect_uri: "https://claude.ai/api/mcp/auth_callback",
        response_type: "code",
        state: "state with spaces",
        scope: "fitness:read profile:read",
        code_challenge: "challenge_abc",
        code_challenge_method: "S256",
        resource: "https://mcp.example.test",
        default_email: "",
        default_password: "",
        google_start_url,
        labels: &ENGLISH_LABELS,
    }
}

/// Without Google sign-in configured the page offers no Google button, and
/// no link into the Google sign-in route (carnet#652: never an advertised
/// button that cannot work).
#[tokio::test]
async fn test_login_page_hides_google_sign_in_when_unconfigured() {
    common::init_server_config();

    let html = OAuth2Routes::generate_login_html(google_login_params(None));

    assert!(!html.contains("Continue with Google"), "{html}");
    assert!(!html.contains("/oauth2/login/google"), "{html}");
    assert!(!html.contains("oauth_return"), "{html}");
    assert!(!html.contains("{{"), "every placeholder is filled: {html}");
}

/// With Google sign-in configured, "Continue with Google" goes straight to
/// the authorization server's Google hop for this exact request (carnet#652):
/// the href is the start URL, HTML-escaped, carrying every parameter.
#[tokio::test]
async fn test_login_page_links_google_sign_in_to_the_google_hop() {
    common::init_server_config();

    let start = "https://app.example.test/oauth2/login/google?client_id=client%26id&redirect_uri=https%3A%2F%2Fclaude.ai%2Fapi%2Fmcp%2Fauth_callback&response_type=code&state=state%20with%20spaces&scope=fitness%3Aread%20profile%3Aread&code_challenge=challenge_abc&code_challenge_method=S256&resource=https%3A%2F%2Fmcp.example.test";
    let html = OAuth2Routes::generate_login_html(google_login_params(Some(start)));

    assert!(html.contains("Continue with Google"), "{html}");
    let href = html
        .split("href=\"")
        .filter_map(|rest| rest.split('"').next())
        .find(|href| href.contains("/oauth2/login/google"))
        .unwrap_or_else(|| panic!("the Google button links to the Google hop: {html}"));
    assert_eq!(
        href,
        start.replace('&', "&amp;"),
        "the start URL, HTML-escaped"
    );

    let url = url::Url::parse(&href.replace("&amp;", "&")).unwrap();
    assert_eq!(url.path(), "/oauth2/login/google");
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    assert_eq!(param("client_id").as_deref(), Some("client&id"));
    assert_eq!(
        param("redirect_uri").as_deref(),
        Some("https://claude.ai/api/mcp/auth_callback")
    );
    assert_eq!(param("response_type").as_deref(), Some("code"));
    assert_eq!(param("state").as_deref(), Some("state with spaces"));
    assert_eq!(param("scope").as_deref(), Some("fitness:read profile:read"));
    assert_eq!(param("code_challenge").as_deref(), Some("challenge_abc"));
    assert_eq!(param("code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(
        param("resource").as_deref(),
        Some("https://mcp.example.test")
    );
    assert!(!html.contains("{{"), "every placeholder is filled: {html}");
}

/// A client name is the client's own text: the consent page escapes it.
#[test]
fn test_consent_page_escapes_the_client_name_and_names_the_account() {
    let html = OAuth2Routes::generate_consent_html(pierre_routes_identity::ConsentHtmlParams {
        client_id: "mcp_client_1",
        redirect_uri: "https://claude.ai/api/mcp/auth_callback",
        response_type: "code",
        state: "s",
        scope: "fitness:read",
        code_challenge: "challenge_abc",
        code_challenge_method: "S256",
        resource: "",
        csrf_token: "token",
        client_label: "<script>alert(1)</script>",
        account_email: "athlete@example.test",
        switch_account_url: "/oauth2/login?client_id=mcp_client_1&state=s",
        first_party_only_sources: &["Strava"],
        text: &ConsentText::new(&STRINGS, "en"),
    });

    assert!(!html.contains("<script>"), "{html}");
    assert!(
        html.contains(
            "is not shared with <span class=\"client\">&lt;script&gt;alert(1)&lt;/script&gt;</span>."
        ),
        "the provider-terms line escapes the client name too: {html}"
    );
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{html}"
    );
    assert!(html.contains("It will return you to claude.ai"), "{html}");
    assert!(html.contains("Signed in as athlete@example.test"), "{html}");
    assert!(
        html.contains(
            "href=\"/oauth2/login?client_id=mcp_client_1&amp;state=s\">Use another account</a>"
        ),
        "{html}"
    );
    assert!(!html.contains("{{"), "every placeholder is filled: {html}");
}

/// The consent page names the host the browser will actually go to, read by
/// the WHATWG URL rules browsers apply: a backslash is a `/`, and a
/// user-info part is not the host. A port is shown; a URI without a host is
/// shown whole.
#[test]
fn test_consent_page_names_the_host_the_browser_goes_to() {
    let consent = |redirect_uri: &str| {
        OAuth2Routes::generate_consent_html(pierre_routes_identity::ConsentHtmlParams {
            client_id: "mcp_client_1",
            redirect_uri,
            response_type: "code",
            state: "s",
            scope: "fitness:read",
            code_challenge: "challenge_abc",
            code_challenge_method: "S256",
            resource: "",
            csrf_token: "token",
            client_label: "Claude",
            account_email: "athlete@example.test",
            switch_account_url: "/oauth2/login?client_id=mcp_client_1&state=s",
            first_party_only_sources: &[],
            text: &ConsentText::new(&STRINGS, "en"),
        })
    };

    for (redirect_uri, shown) in [
        (
            "https://evil.example\\@claude.ai/api/mcp/auth_callback",
            "evil.example",
        ),
        (
            "https://claude.ai@evil.example/api/mcp/auth_callback",
            "evil.example",
        ),
        ("https://user:pass@evil.example/cb", "evil.example"),
        ("http://localhost:33418/callback", "localhost:33418"),
        ("urn:ietf:wg:oauth:2.0:oob", "urn:ietf:wg:oauth:2.0:oob"),
    ] {
        let html = consent(redirect_uri);
        assert!(
            html.contains(&format!("It will return you to {shown}</p>")),
            "{redirect_uri} should name {shown}: {html}"
        );
        assert!(
            !html.contains("It will return you to claude.ai"),
            "{redirect_uri}: {html}"
        );
    }
}

/// The consent page tells the athlete which services' data stays inside
/// Dravr and never reaches the client — the registry's first-party-only list,
/// whatever it holds — and says nothing when that list is empty.
#[test]
fn test_consent_page_names_the_data_that_stays_inside_dravr() {
    let consent = |sources: &[&str]| {
        OAuth2Routes::generate_consent_html(pierre_routes_identity::ConsentHtmlParams {
            client_id: "mcp_client_1",
            redirect_uri: "https://claude.ai/api/mcp/auth_callback",
            response_type: "code",
            state: "s",
            scope: "fitness:read",
            code_challenge: "challenge_abc",
            code_challenge_method: "S256",
            resource: "",
            csrf_token: "token",
            client_label: "Claude",
            account_email: "athlete@example.test",
            switch_account_url: "/oauth2/login?client_id=mcp_client_1&state=s",
            first_party_only_sources: sources,
            text: &ConsentText::new(&STRINGS, "en"),
        })
    };

    let registered = ProviderRegistry::new().first_party_only_sources();
    assert!(
        !registered.is_empty(),
        "the shipped providers include first-party-only ones"
    );
    let html = consent(&registered);
    let (last, rest) = registered.split_last().expect("a source");
    let named = if rest.is_empty() {
        (*last).to_owned()
    } else {
        format!("{} and {last}", rest.join(", "))
    };
    assert!(
        html.contains(&format!(
            "Data Dravr syncs from your {named} connection stays inside Dravr and is not shared with <span class=\"client\">Claude</span>. \
             Export files you import yourself can be read by <span class=\"client\">Claude</span>."
        )),
        "{html}"
    );

    assert!(
        consent(&["WHOOP"])
            .contains("Data Dravr syncs from your WHOOP connection stays inside Dravr"),
        "one service is named alone"
    );
    let none = consent(&[]);
    assert!(!none.contains("stays inside Dravr"), "{none}");
    assert!(!none.contains("{{"), "every placeholder is filled: {none}");
}

/// The consent page says who is asking (a known client's badge, or an
/// unverified warning), that the data is wellness information, that it may
/// leave Québec and that approving is express consent to share it.
#[test]
fn test_consent_page_states_the_client_trust_and_the_data_obligations() {
    let consent = |client_id: &str, redirect_uri: &str, label: &str| {
        OAuth2Routes::generate_consent_html(pierre_routes_identity::ConsentHtmlParams {
            client_id,
            redirect_uri,
            response_type: "code",
            state: "s",
            scope: "fitness:read",
            code_challenge: "challenge_abc",
            code_challenge_method: "S256",
            resource: "",
            csrf_token: "token",
            client_label: label,
            account_email: "athlete@example.test",
            switch_account_url: "/oauth2/login?client_id=x&state=s",
            first_party_only_sources: &[],
            text: &ConsentText::new(&STRINGS, "en"),
        })
    };

    let verified = consent(
        "https://chatgpt.com/oauth/client.json",
        "https://chatgpt.com/connector_platform_oauth_redirect",
        "ChatGPT",
    );
    assert!(
        verified.contains("Verified: ChatGPT by OpenAI"),
        "{verified}"
    );
    assert!(!verified.contains("Unverified app"), "{verified}");

    // A name a client registers for itself earns no badge.
    let unverified = consent("dyn-1f3a", "http://localhost:53124/callback", "Claude");
    assert!(unverified.contains("Unverified app"), "{unverified}");
    assert!(
        unverified.contains("program on this device"),
        "{unverified}"
    );
    assert!(!unverified.contains("Verified: "), "{unverified}");

    for html in [&verified, &unverified] {
        assert!(html.contains("not medical advice"), "{html}");
        assert!(html.contains("communicated outside Québec"), "{html}");
        assert!(html.contains("expressly consent"), "{html}");
        assert!(!html.contains("{{"), "every placeholder is filled: {html}");
    }
    assert!(
        verified.contains("OpenAI") && verified.contains("the United States"),
        "{verified}"
    );
}

/// Test OAuth login HTML generation with template replacement
#[tokio::test]
async fn test_generate_login_html() {
    common::init_server_config();

    // Create test parameters with known values
    let test_client_id = "test_client_123";
    let test_redirect = "https://example.com/callback";
    let test_response_type = "code";
    let test_state = "random_state_xyz";
    let test_scope = "read:activities write:profile";
    let test_challenge = "challenge_abc";
    let test_method = "S256";
    let test_resource = "https://mcp.example.test";
    let test_email = "test@example.com";
    let test_password = "test_pass_123";

    // Generate HTML using the OAuth2Routes method
    let html = OAuth2Routes::generate_login_html(pierre_routes_identity::LoginHtmlParams {
        client_id: test_client_id,
        redirect_uri: test_redirect,
        response_type: test_response_type,
        state: test_state,
        scope: test_scope,
        code_challenge: test_challenge,
        code_challenge_method: test_method,
        resource: test_resource,
        default_email: test_email,
        default_password: test_password,
        google_start_url: None,
        labels: &ENGLISH_LABELS,
    });

    // Verify all placeholders were replaced with actual values
    assert!(
        !html.contains("{{CLIENT_ID}}"),
        "CLIENT_ID placeholder not replaced"
    );
    assert!(
        !html.contains("{{REDIRECT_URI}}"),
        "REDIRECT_URI placeholder not replaced"
    );
    assert!(
        !html.contains("{{RESPONSE_TYPE}}"),
        "RESPONSE_TYPE placeholder not replaced"
    );
    assert!(
        !html.contains("{{STATE}}"),
        "STATE placeholder not replaced"
    );
    assert!(
        !html.contains("{{SCOPE}}"),
        "SCOPE placeholder not replaced"
    );
    assert!(
        !html.contains("{{CODE_CHALLENGE}}"),
        "CODE_CHALLENGE placeholder not replaced"
    );
    assert!(
        !html.contains("{{CODE_CHALLENGE_METHOD}}"),
        "CODE_CHALLENGE_METHOD placeholder not replaced"
    );
    assert!(
        !html.contains("{{RESOURCE}}"),
        "RESOURCE placeholder not replaced"
    );
    assert!(
        !html.contains("{{DEFAULT_EMAIL}}"),
        "DEFAULT_EMAIL placeholder not replaced"
    );
    assert!(
        !html.contains("{{DEFAULT_PASSWORD}}"),
        "DEFAULT_PASSWORD placeholder not replaced"
    );

    // Verify actual values appear in the HTML
    assert!(
        html.contains(test_client_id),
        "Generated HTML missing client_id value"
    );
    assert!(
        html.contains(test_redirect),
        "Generated HTML missing redirect_uri value"
    );
    assert!(
        html.contains(test_response_type),
        "Generated HTML missing response_type value"
    );
    assert!(
        html.contains(test_state),
        "Generated HTML missing state value"
    );
    assert!(
        html.contains(test_scope),
        "Generated HTML missing scope value"
    );
    assert!(
        html.contains(test_challenge),
        "Generated HTML missing code_challenge value"
    );
    assert!(
        html.contains(test_method),
        "Generated HTML missing code_challenge_method value"
    );
    assert!(
        html.contains(&format!(
            "<input type=\"hidden\" name=\"resource\" value=\"{test_resource}\">"
        )),
        "the RFC 8707 resource must survive the login form as a hidden field"
    );
    assert!(
        html.contains(test_email),
        "Generated HTML missing default_email value"
    );

    // Note: password value is in password field, verify it's there
    assert!(
        html.contains(&format!("value=\"{test_password}\"")),
        "Generated HTML missing default_password value"
    );

    // Verify HTML structure is intact
    assert!(html.contains("<!DOCTYPE html>"), "Missing DOCTYPE");
    assert!(
        html.contains("<form method=\"post\" action=\"/oauth2/login\">"),
        "Missing form element"
    );
}

/// Test OAuth login HTML generation with empty scope (should use default)
#[tokio::test]
async fn test_generate_login_html_empty_scope() {
    common::init_server_config();

    // Generate HTML with empty scope
    let html = OAuth2Routes::generate_login_html(pierre_routes_identity::LoginHtmlParams {
        client_id: "test",
        redirect_uri: "https://example.com",
        response_type: "code",
        state: "state",
        scope: "", // Empty scope
        code_challenge: "challenge",
        code_challenge_method: "S256",
        resource: "",
        default_email: "test@example.com",
        default_password: "",
        google_start_url: None,
        labels: &ENGLISH_LABELS,
    });

    // An empty request renders the grant it will actually be ISSUED —
    // `OAuthScope::default_grant`, the same list the registration endpoint
    // applies — so the athlete consents to what the client receives. Asserted
    // against the vocabulary rather than a literal: the two were separate
    // literals before, and both still named `activities:read`, a scope this
    // server never checked.
    let expected = OAuthScope::render_granted(&OAuthScope::default_grant());
    assert!(
        html.contains(&expected),
        "an empty scope must render the default grant, expected {expected:?}"
    );
    assert!(
        !html.contains("activities:read"),
        "activities:read was removed from the vocabulary; nothing should still render it"
    );
    assert!(
        !html.contains("{{SCOPE}}"),
        "SCOPE placeholder not replaced"
    );
}

/// Test OAuth login error HTML rendering
#[tokio::test]
async fn test_oauth_login_error_rendering() {
    // Load the error template
    const TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login_error.html");

    // Simulate error HTML generation
    let test_error_msg = "Authentication Failed: Invalid credentials";
    let test_client_id = "error_client_123";
    let test_redirect = "https://example.com/callback";
    let test_state = "error_state";

    let error_html = TEMPLATE
        .replace("{{ERROR_MESSAGE}}", test_error_msg)
        .replace("{{CLIENT_ID}}", test_client_id)
        .replace("{{REDIRECT_URI}}", test_redirect)
        .replace("{{RESPONSE_TYPE}}", "code")
        .replace("{{STATE}}", test_state)
        .replace("{{SCOPE}}", "read")
        .replace("{{CODE_CHALLENGE}}", "challenge")
        .replace("{{CODE_CHALLENGE_METHOD}}", "S256");

    // Verify error message appears
    assert!(
        error_html.contains(test_error_msg),
        "Error message not in HTML"
    );

    // Verify no placeholders remain
    assert!(
        !error_html.contains("{{ERROR_MESSAGE}}"),
        "ERROR_MESSAGE placeholder not replaced"
    );
    assert!(
        !error_html.contains("{{CLIENT_ID}}"),
        "CLIENT_ID placeholder not replaced in error template"
    );

    // Verify back link contains parameters
    assert!(
        error_html.contains("/oauth2/login?"),
        "Missing back to login link"
    );
    assert!(
        error_html.contains(&format!("client_id={test_client_id}")),
        "Back link missing client_id parameter"
    );
    assert!(
        error_html.contains(&format!("state={test_state}")),
        "Back link missing state parameter"
    );
}

/// The OAuth pages draw with the shared Boreal sheet in both schemes: the
/// templates ask for it and keep no palette of their own, and the rendered
/// page carries the sheet with nothing of the retired Pierre violet left.
#[tokio::test]
async fn test_templates_use_boreal_design_system() {
    const LOGIN_TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login.html");
    const ERROR_TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login_error.html");
    common::init_server_config();

    for template in [LOGIN_TEMPLATE, ERROR_TEMPLATE] {
        assert!(
            template.contains("<style>{{HOSTED_PAGE_CSS}}</style>"),
            "an OAuth template asks for the shared sheet"
        );
        for retired in ["#7C3AED", "#06B6D4", "--pierre-", "linear-gradient"] {
            assert!(
                !template.contains(retired),
                "an OAuth template still carries the retired {retired}"
            );
        }
        assert!(
            template.contains(r#"<div class="lockup" role="img" aria-label="Dravr"></div>"#),
            "an OAuth template opens with the Dravr lockup"
        );
    }

    let html = OAuth2Routes::generate_login_html(pierre_routes_identity::LoginHtmlParams {
        client_id: "boreal_client",
        redirect_uri: "https://example.com/callback",
        response_type: "code",
        state: "state",
        scope: "fitness:read",
        code_challenge: "challenge",
        code_challenge_method: "S256",
        resource: "",
        default_email: "",
        default_password: "",
        google_start_url: None,
        labels: &ENGLISH_LABELS,
    });
    let dark_at = html
        .find("@media (prefers-color-scheme: dark)")
        .expect("the rendered login page carries the dark scheme");
    // Sage-forest #255f4d is the light primary, mint #a3d0be the dark one.
    assert!(html[..dark_at].contains("--color-primary: 37 95 77;"));
    assert!(html[dark_at..].contains("--color-primary: 163 208 190;"));
    assert!(html.contains(r#"class="btn btn-primary btn-block">Sign in</button>"#));
    assert!(!html.contains("#7C3AED"));
}

/// Test template accessibility features
#[test]
fn test_templates_accessibility() {
    const LOGIN_TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login.html");
    const ERROR_TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login_error.html");

    // Verify proper HTML lang attribute
    // The lang attribute is the locale the page is rendered in
    assert!(
        LOGIN_TEMPLATE.contains("lang=\"{{LANG}}\""),
        "Login template missing lang attribute"
    );
    assert!(
        ERROR_TEMPLATE.contains("lang=\"{{LANG}}\""),
        "Error template missing lang attribute"
    );

    // Verify meta viewport for responsive design
    assert!(
        LOGIN_TEMPLATE.contains("viewport"),
        "Login template missing viewport meta"
    );
    assert!(
        ERROR_TEMPLATE.contains("viewport"),
        "Error template missing viewport meta"
    );

    // Verify form inputs have proper labels or aria-labels
    assert!(
        LOGIN_TEMPLATE.contains("aria-label") || LOGIN_TEMPLATE.contains("<label"),
        "Login template missing accessibility labels"
    );

    // Verify no autoplay or autofocus that could be disruptive
    assert!(
        !LOGIN_TEMPLATE.contains("autofocus"),
        "Login template should not use autofocus"
    );
}

/// Test template security features
#[test]
fn test_templates_security_features() {
    const LOGIN_TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login.html");
    const ERROR_TEMPLATE: &str =
        include_str!("../../../pierre-routes-identity/templates/oauth_login_error.html");

    // Verify noindex meta tag (OAuth pages should not be indexed)
    assert!(
        LOGIN_TEMPLATE.contains("noindex"),
        "Login template should have noindex meta"
    );
    assert!(
        ERROR_TEMPLATE.contains("noindex"),
        "Error template should have noindex meta"
    );

    // Verify referrer policy
    assert!(
        LOGIN_TEMPLATE.contains("referrer"),
        "Login template should have referrer policy"
    );
    assert!(
        ERROR_TEMPLATE.contains("referrer"),
        "Error template should have referrer policy"
    );

    // Verify autocomplete attributes for security-sensitive fields
    assert!(
        LOGIN_TEMPLATE.contains("autocomplete"),
        "Login template should have autocomplete attributes"
    );
}

// ============================================================================
// HTML Escaping Tests
// ============================================================================

#[test]
fn test_escape_html_attribute_no_special_chars() {
    assert_eq!(escape_html_attribute("hello world"), "hello world");
}

#[test]
fn test_escape_html_attribute_quotes() {
    assert_eq!(
        escape_html_attribute(r#"value"with"quotes"#),
        "value&quot;with&quot;quotes"
    );
}

#[test]
fn test_escape_html_attribute_xss_payload() {
    assert_eq!(
        escape_html_attribute(r#""><script>alert(1)</script>"#),
        "&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;"
    );
}

#[test]
fn test_escape_html_attribute_ampersand() {
    assert_eq!(escape_html_attribute("a&b=c"), "a&amp;b=c");
}

#[test]
fn test_escape_html_attribute_single_quotes() {
    assert_eq!(escape_html_attribute("it's"), "it&#x27;s");
}

#[test]
fn test_escape_html_attribute_empty_string() {
    assert_eq!(escape_html_attribute(""), "");
}

#[test]
fn test_escape_html_attribute_base64url_safe() {
    // Base64url characters should pass through unchanged
    assert_eq!(
        escape_html_attribute("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
    );
}

// ============================================================================
// XSS Prevention Tests
// ============================================================================

/// Test that XSS payloads in the state parameter are properly escaped in login HTML
#[tokio::test]
async fn test_login_html_escapes_xss_in_state() {
    common::init_server_config();

    let xss_state = r#""><script>alert('xss')</script>"#;
    let html = OAuth2Routes::generate_login_html(pierre_routes_identity::LoginHtmlParams {
        client_id: "test_client",
        redirect_uri: "https://example.com/callback",
        response_type: "code",
        state: xss_state,
        scope: "fitness:read",
        code_challenge: "challenge",
        code_challenge_method: "S256",
        resource: "",
        default_email: "",
        default_password: "",
        google_start_url: None,
        labels: &ENGLISH_LABELS,
    });

    // The XSS payload should NOT appear unescaped
    assert!(
        !html.contains("<script>"),
        "XSS payload in state was not HTML-escaped"
    );
    // The escaped version should appear
    assert!(
        html.contains("&lt;script&gt;"),
        "State value should be HTML-escaped"
    );
}

/// Test that XSS payloads in `redirect_uri` are properly escaped in login HTML
#[tokio::test]
async fn test_login_html_escapes_xss_in_redirect_uri() {
    common::init_server_config();

    let xss_redirect = r#"https://evil.com" onload="alert(1)"#;
    let html = OAuth2Routes::generate_login_html(pierre_routes_identity::LoginHtmlParams {
        client_id: "test_client",
        redirect_uri: xss_redirect,
        response_type: "code",
        state: "safe_state",
        scope: "fitness:read",
        code_challenge: "challenge",
        code_challenge_method: "S256",
        resource: "",
        default_email: "",
        default_password: "",
        google_start_url: None,
        labels: &ENGLISH_LABELS,
    });

    // The attribute breakout should NOT appear unescaped
    assert!(
        !html.contains(r#"" onload="#),
        "XSS payload in redirect_uri was not HTML-escaped"
    );
    assert!(
        html.contains("&quot;"),
        "Quotes in redirect_uri should be HTML-escaped"
    );
}

/// Integration test: Full OAuth login page rendering flow
#[tokio::test]
async fn test_oauth_login_page_integration() {
    common::init_server_config();

    // Generate login HTML with test values (no full server config needed for this test)
    let html = OAuth2Routes::generate_login_html(pierre_routes_identity::LoginHtmlParams {
        client_id: "integration_test_client",
        redirect_uri: "https://integration.test/callback",
        response_type: "code",
        state: "integration_state_123",
        scope: "read:all write:all",
        code_challenge: "integration_challenge",
        code_challenge_method: "S256",
        resource: "",
        default_email: "test@pierre.test",
        default_password: "test123",
        google_start_url: None,
        labels: &ENGLISH_LABELS,
    });

    // Verify complete HTML structure
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("integration_test_client"));
    assert!(html.contains("integration_state_123"));
    assert!(html.contains("test@pierre.test"));
    assert!(html.contains("read:all write:all"));

    // Verify no template placeholders remain
    assert!(!html.contains("{{"));
    assert!(!html.contains("}}"));

    // Verify form has all required hidden fields for OAuth flow
    assert!(html.contains("name=\"client_id\""));
    assert!(html.contains("name=\"redirect_uri\""));
    assert!(html.contains("name=\"response_type\""));
    assert!(html.contains("name=\"state\""));
    assert!(html.contains("name=\"scope\""));
    assert!(html.contains("name=\"code_challenge\""));
    assert!(html.contains("name=\"code_challenge_method\""));

    // Verify visible form fields
    assert!(html.contains("name=\"email\""));
    assert!(html.contains("name=\"password\""));
}

// ============================================================================
// Provider OAuth callback pages (pierre-mcp-transport)
// ============================================================================

use pierre_mcp_transport::oauth_flow_manager::OAuthTemplateRenderer;
use pierre_mcp_transport::OAuthCallbackResponse;

/// The provider callback pages draw with the shared Boreal sheet, and the
/// success page still tells its opener which provider completed.
#[test]
fn test_provider_callback_pages_use_boreal_design_system() {
    let success = OAuthTemplateRenderer::render_success_template(
        "strava",
        &OAuthCallbackResponse {
            user_id: "user-1".to_owned(),
            provider: "strava".to_owned(),
            expires_at: Some("2026-09-23T00:00:00Z".to_owned()),
            scopes: "read".to_owned(),
            mobile_redirect_url: None,
            tenant_id: "tenant-1".to_owned(),
        },
    );
    assert!(success.contains("<h1>Strava Connected</h1>"));
    assert!(success.contains("provider: 'strava',"));
    assert!(success.contains(
        r#"<div class="status-icon status-icon-success" aria-hidden="true">&#10003;</div>"#
    ));

    let failure = OAuthTemplateRenderer::render_error_template(
        "en",
        "Connection Failed",
        "Please try again.",
    );
    assert!(failure.contains("<h1>Connection Failed</h1>"));
    assert!(failure.contains("<html lang=\"en\">"));

    for page in [&success, &failure] {
        let dark_at = page
            .find("@media (prefers-color-scheme: dark)")
            .expect("the callback page carries the dark scheme");
        assert!(page[..dark_at].contains("--color-primary: 37 95 77;"));
        assert!(page[dark_at..].contains("--color-primary: 163 208 190;"));
        assert!(!page.contains("{{"), "no placeholder survives the render");
        assert!(!page.contains("#7C3AED"));
    }
}

/// A consent page for a client this test names, in `locale`.
fn consent_in(locale: &str, client_id: &str, redirect_uri: &str, scope: &str) -> String {
    OAuth2Routes::generate_consent_html(pierre_routes_identity::ConsentHtmlParams {
        client_id,
        redirect_uri,
        response_type: "code",
        state: "s",
        scope,
        code_challenge: "challenge_abc",
        code_challenge_method: "S256",
        resource: "",
        csrf_token: "token",
        client_label: "Agent",
        account_email: "athlete@example.test",
        switch_account_url: "/oauth2/login?client_id=x&state=s",
        first_party_only_sources: &["Strava", "Garmin", "WHOOP"],
        text: &ConsentText::new(&STRINGS, locale),
    })
}

/// Everything the page shows, without the stylesheet or the form's values.
fn page_body(html: &str) -> &str {
    html.split("<body>")
        .nth(1)
        .unwrap_or_else(|| panic!("{html}"))
}

/// Every catalogue key `consent_strings.rs` declares, as the module spells it,
/// so a key added there is covered here without being listed twice.
fn consent_flow_keys() -> Vec<&'static str> {
    include_str!("../../../pierre-contremaitre/src/consent_strings.rs")
        .lines()
        .filter(|line| line.starts_with("pub const KEY_"))
        .filter_map(|line| line.split('"').nth(1))
        .collect()
}

/// The parts of a catalogue line between its slots, as the page escapes them.
fn fixed_parts(line: &str) -> Vec<String> {
    SLOT.split(line)
        .map(str::trim)
        .filter(|part| part.len() > 3)
        .map(escape_html_attribute)
        .collect()
}

/// The two consent pages that between them show every line: a verified client,
/// and an unverified one with a metadata document returning to this device.
fn every_consent_line_in(locale: &str) -> [String; 2] {
    let scopes = "fitness:read fitness:write profile:read profile:write offline_access";
    [
        consent_in(
            locale,
            "https://chatgpt.com/oauth/client.json",
            "https://chatgpt.com/cb",
            scopes,
        ),
        consent_in(
            locale,
            "https://tool.example.test/client.json",
            "http://localhost:53124/callback",
            scopes,
        ),
    ]
}

/// `offline_access` reads as what it grants in every locale, like the other
/// scopes, never as the bare token alone.
#[test]
fn offline_access_is_worded_in_every_locale() {
    for locale in SUPPORTED_LOCALES {
        let worded =
            escape_html_attribute(&STRINGS.get("oauth.consent.scope.offlineAccess", locale));
        assert!(
            !worded.is_empty(),
            "bug: {locale} has no offline_access line"
        );
        for html in every_consent_line_in(locale) {
            assert!(
                html.contains(&format!("<li>{worded} (offline_access)</li>")),
                "bug: {locale} shows offline_access unworded: {html}"
            );
            assert!(
                !html.contains("<li>offline_access</li>"),
                "bug: {locale} shows the bare token: {html}"
            );
        }
    }
}

#[test]
fn every_consent_flow_key_has_text_and_the_slots_english_has_in_every_locale() {
    let keys = consent_flow_keys();
    assert!(keys.len() > 40, "the keys were read: {keys:?}");
    let slots = |line: &str| {
        let mut found: Vec<String> = SLOT
            .find_iter(line)
            .map(|slot| slot.as_str().to_owned())
            .collect();
        found.sort();
        found
    };
    for key in keys {
        let english = STRINGS
            .get_exact(key, "en")
            .unwrap_or_else(|| panic!("{key} in en"));
        for locale in SUPPORTED_LOCALES {
            let line = STRINGS
                .get_exact(key, locale)
                .unwrap_or_else(|| panic!("{key} in {locale}"));
            assert_eq!(slots(&line), slots(&english), "{key} in {locale}");
        }
    }
}

/// Every line of the screen, the trust lines and the Law 25 lines included,
/// reads in French on a French page, and no English line is left behind.
#[test]
fn a_french_consent_page_says_every_line_in_french() {
    let keys: Vec<&str> = consent_flow_keys()
        .into_iter()
        .filter(|key| key.starts_with("oauth.consent."))
        .collect();
    let french = every_consent_line_in("fr");
    let bodies = [page_body(&french[0]), page_body(&french[1])];
    for html in &french {
        assert!(html.contains("<html lang=\"fr\">"), "{html}");
        assert!(
            html.contains("<title>Autoriser l’accès - Dravr</title>"),
            "{html}"
        );
        assert!(!html.contains("{{"), "every placeholder is filled: {html}");
        assert!(!SLOT.is_match(page_body(html)), "{html}");
    }
    for key in keys {
        for part in fixed_parts(&STRINGS.get(key, "fr")) {
            assert!(
                bodies.iter().any(|body| body.contains(&part)),
                "{key} shows {part:?}: {bodies:?}"
            );
        }
        for part in fixed_parts(&STRINGS.get(key, "en")) {
            assert!(
                !bodies.iter().any(|body| body.contains(&part)),
                "{key} left English {part:?}: {bodies:?}"
            );
        }
    }
}

/// The lines with legal weight keep theirs in French: the Act's own terms, and
/// a button name the athlete can actually find on the page.
#[test]
fn the_french_law_25_lines_say_what_the_english_says() {
    let [verified, unverified] = every_consent_line_in("fr");
    for html in [&verified, &unverified] {
        assert!(
            html.contains("peuvent être communiqués à l’extérieur du Québec"),
            "{html}"
        );
        assert!(
            html.contains("renseignements personnels sensibles"),
            "{html}"
        );
        assert!(html.contains("vous consentez expressément"), "{html}");
        assert!(html.contains("En choisissant Autoriser l’accès,"), "{html}");
        assert!(html.contains(">Autoriser l’accès</button>"), "{html}");
        assert!(html.contains("et non des conseils médicaux"), "{html}");
    }
    assert!(verified.contains("exploité par OpenAI"), "{verified}");
    assert!(verified.contains("aux États-Unis"), "{verified}");
    assert!(unverified.contains("Dravr ne sait pas où"), "{unverified}");
}

/// In every locale the express-consent line names the button as the button
/// reads, and the screen is whole.
#[test]
fn every_locale_names_its_own_allow_button_and_fills_the_page() {
    for locale in SUPPORTED_LOCALES {
        let allow = escape_html_attribute(&STRINGS.get("oauth.consent.allow", locale));
        for html in every_consent_line_in(locale) {
            assert!(
                html.contains(&format!("<html lang=\"{locale}\">")),
                "{html}"
            );
            assert!(html.contains(&format!(">{allow}</button>")), "{html}");
            let express = html
                .split("<p class=\"disclosure\">")
                .skip(1)
                .filter_map(|line| line.split("</p>").next())
                .filter(|line| line.contains(allow.as_str()))
                .count();
            assert_eq!(express, 1, "{locale}: {html}");
            assert!(!html.contains("{{"), "{html}");
            assert!(!SLOT.is_match(page_body(&html)), "{html}");
            assert!(!html.contains("oauth.consent"), "no raw key: {html}");
            assert!(!html.contains("<p class=\"disclosure\"></p>"), "{html}");
        }
    }
}

/// A locale the platform does not speak reads in English, whole: no raw key,
/// no blank line, not the platform's French default.
#[test]
fn a_consent_page_in_a_locale_the_platform_lacks_reads_in_english() {
    let [verified, unverified] = every_consent_line_in("ja");
    assert_eq!(verified, every_consent_line_in("en")[0]);
    assert_eq!(unverified, every_consent_line_in("en")[1]);
    for html in [&verified, &unverified] {
        assert!(html.contains("<html lang=\"en\">"), "{html}");
        assert!(html.contains("expressly consent"), "{html}");
        assert!(!html.contains("oauth.consent"), "no raw key: {html}");
    }
}

/// A slot's value is the client's own text: it is escaped however the locale
/// words the line around it, and it is not read as a slot or a placeholder.
#[test]
fn a_french_consent_page_escapes_what_the_client_supplies() {
    let html = OAuth2Routes::generate_consent_html(pierre_routes_identity::ConsentHtmlParams {
        client_id: "dyn-1",
        redirect_uri: "https://client.example.test/cb",
        response_type: "code",
        state: "s",
        scope: "<b>scope</b>",
        code_challenge: "c",
        code_challenge_method: "S256",
        resource: "",
        csrf_token: "token",
        client_label: "<script>alert(1)</script>{1}{{T_ALLOW}}",
        account_email: "<i>a</i>@example.test",
        switch_account_url: "/oauth2/login",
        first_party_only_sources: &["<u>x</u>"],
        text: &ConsentText::new(&STRINGS, "fr"),
    });
    for raw in ["<script>", "<b>scope", "<i>a", "<u>x"] {
        assert!(!html.contains(raw), "{raw}: {html}");
    }
    assert!(html.contains("<li>&lt;b&gt;scope&lt;/b&gt;</li>"), "{html}");
    assert!(
        html.contains("{1}{{T_ALLOW}}"),
        "the client's text stays its own: {html}"
    );
}

/// Each scope says what it allows in the page's language, and still names the
/// token the client asked for.
#[test]
fn a_scope_is_worded_in_the_page_language_with_its_token() {
    let french = consent_in("fr", "dyn-1", "https://c.example.test/cb", "fitness:read");
    assert!(
        french.contains("lire vos données de conditionnement physique"),
        "{french}"
    );
    assert!(french.contains("(fitness:read)</li>"), "{french}");
    let english = consent_in("en", "dyn-1", "https://c.example.test/cb", "fitness:read");
    assert!(english.contains("read your fitness data"), "{english}");
    assert!(english.contains("(fitness:read)</li>"), "{english}");
}

/// The page a provider's failed callback lands on reads in the browser's
/// language, from the one template every OAuth failure page shares.
#[tokio::test]
async fn a_failed_provider_callback_reads_in_the_browsers_language() {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await.unwrap();
    for (accept, lang) in [("fr-CA", "fr"), ("de", "de"), ("en-US", "en")] {
        let page = AxumTestRequest::get("/api/oauth/callback/strava?code=forged&state=forged")
            .header("accept-language", accept)
            .send(AuthRoutes::routes(resources.auth_routes_context()))
            .await;
        assert_eq!(page.status(), 400, "{}", page.body_text());
        let text = page.body_text();
        assert!(text.contains(&format!("<html lang=\"{lang}\">")), "{text}");
        assert!(!text.contains("{{"), "no placeholder survives: {text}");
    }
    let french = AxumTestRequest::get("/api/oauth/callback/strava?code=forged&state=forged")
        .header("accept-language", "fr-CA")
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await
        .body_text();
    assert!(!french.contains("Connection Failed"), "{french}");
}
