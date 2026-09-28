// ABOUTME: Server-rendered pages of the OAuth 2.0 authorization flow — login, consent and error
// ABOUTME: Fills the embedded hosted-page templates with HTML-escaped request values
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use pierre_auth::oauth2_server::{
    endpoints::OAuth2AuthorizationServer,
    models::{AuthorizeRequest, OAuth2Error},
};
use pierre_core::html::{escape_html_attribute, with_hosted_page_css};

use super::OAuth2Routes;

/// Parameters for generating OAuth login HTML
#[derive(Clone, Copy)]
pub struct LoginHtmlParams<'a> {
    /// OAuth client identifier
    pub client_id: &'a str,
    /// OAuth redirect URI after authorization
    pub redirect_uri: &'a str,
    /// OAuth response type (typically "code")
    pub response_type: &'a str,
    /// OAuth state parameter for CSRF protection
    pub state: &'a str,
    /// OAuth scope for requested permissions
    pub scope: &'a str,
    /// PKCE code challenge
    pub code_challenge: &'a str,
    /// PKCE code challenge method (e.g., "S256")
    pub code_challenge_method: &'a str,
    /// RFC 8707 resource the authorization is for (empty when none was named)
    pub resource: &'a str,
    /// Default email to pre-fill in login form (dev/test only)
    pub default_email: &'a str,
    /// Default password to pre-fill in login form (dev/test only)
    pub default_password: &'a str,
}

/// Parameters for rendering the OAuth consent page.
#[derive(Clone, Copy)]
pub struct ConsentHtmlParams<'a> {
    /// OAuth client identifier requesting access
    pub client_id: &'a str,
    /// OAuth redirect URI after authorization
    pub redirect_uri: &'a str,
    /// OAuth response type (typically "code")
    pub response_type: &'a str,
    /// OAuth state parameter for CSRF protection
    pub state: &'a str,
    /// OAuth scope being consented to
    pub scope: &'a str,
    /// PKCE code challenge
    pub code_challenge: &'a str,
    /// PKCE code challenge method (e.g., "S256")
    pub code_challenge_method: &'a str,
    /// RFC 8707 resource the authorization is for (empty when none was named)
    pub resource: &'a str,
    /// Synchronizer token the consent submission is checked against
    pub csrf_token: &'a str,
}

impl OAuth2Routes {
    /// OAuth error template embedded at compile-time
    pub(super) const OAUTH_ERROR_TEMPLATE: &'static str =
        include_str!("../../templates/oauth_error.html");

    /// OAuth login page template embedded at compile-time
    /// Loaded with `include_str`!() to avoid blocking filesystem IO at runtime
    const OAUTH_LOGIN_TEMPLATE: &'static str = include_str!("../../templates/oauth_login.html");

    /// OAuth consent page template embedded at compile-time
    const OAUTH_CONSENT_TEMPLATE: &'static str = include_str!("../../templates/oauth_consent.html");

    /// OAuth login error template embedded at compile-time
    /// Loaded with `include_str`!() to avoid blocking filesystem IO at runtime
    const OAUTH_LOGIN_ERROR_TEMPLATE: &'static str =
        include_str!("../../templates/oauth_login_error.html");

    /// Render HTML error page for OAuth errors shown in browser: 429 for
    /// `too_many_requests`, 503 for `temporarily_unavailable`, else 400
    pub(crate) fn render_oauth_error_response(error: &OAuth2Error) -> Response {
        let error_title = match error.error.as_str() {
            "invalid_client" => "✗ Invalid Client",
            "unauthorized_client" => "✗ Unauthorized Client",
            "access_denied" => "✗ Access Denied",
            "unsupported_response_type" => "✗ Unsupported Response Type",
            "invalid_scope" => "✗ Invalid Scope",
            "server_error" => "✗ Server Error",
            "temporarily_unavailable" => "✗ Temporarily Unavailable",
            _ => "✗ OAuth Error",
        };

        let default_description =
            "An error occurred during the OAuth authorization process.".to_owned();
        let error_description = error
            .error_description
            .as_ref()
            .unwrap_or(&default_description);

        let html = with_hosted_page_css(Self::OAUTH_ERROR_TEMPLATE)
            .replace("{{error_title}}", &escape_html_attribute(error_title))
            .replace("{{ERROR}}", &escape_html_attribute(&error.error))
            .replace("{{PROVIDER}}", "Dravr")
            .replace(
                "{{DESCRIPTION}}",
                &format!(
                    r#"<div class="description">{}</div>"#,
                    escape_html_attribute(error_description)
                ),
            );

        (error.http_status(), Html(html)).into_response()
    }

    /// Render the consent screen for a not-yet-granted authorization request.
    pub(super) fn render_consent_page(request: &AuthorizeRequest, csrf_token: &str) -> Response {
        let html = Self::generate_consent_html(ConsentHtmlParams {
            client_id: &request.client_id,
            redirect_uri: &request.redirect_uri,
            response_type: &request.response_type,
            state: request.state.as_deref().unwrap_or_default(),
            scope: request.scope.as_deref().unwrap_or_default(),
            code_challenge: request.code_challenge.as_deref().unwrap_or_default(),
            code_challenge_method: request.code_challenge_method.as_deref().unwrap_or_default(),
            resource: request.resource.as_deref().unwrap_or_default(),
            csrf_token,
        });
        Html(html).into_response()
    }

    /// Generate OAuth login page HTML
    #[must_use]
    pub fn generate_login_html(params: LoginHtmlParams<'_>) -> String {
        // Use embedded template - zero filesystem IO, guaranteed to exist at compile-time
        let displayed_scope = if params.scope.is_empty() {
            OAuth2AuthorizationServer::default_scope_display()
        } else {
            params.scope.to_owned()
        };

        with_hosted_page_css(Self::OAUTH_LOGIN_TEMPLATE)
            .replace("{{CLIENT_ID}}", &escape_html_attribute(params.client_id))
            .replace(
                "{{REDIRECT_URI}}",
                &escape_html_attribute(params.redirect_uri),
            )
            .replace(
                "{{RESPONSE_TYPE}}",
                &escape_html_attribute(params.response_type),
            )
            .replace("{{STATE}}", &escape_html_attribute(params.state))
            .replace("{{SCOPE}}", &escape_html_attribute(&displayed_scope))
            .replace(
                "{{CODE_CHALLENGE}}",
                &escape_html_attribute(params.code_challenge),
            )
            .replace(
                "{{CODE_CHALLENGE_METHOD}}",
                &escape_html_attribute(params.code_challenge_method),
            )
            .replace("{{RESOURCE}}", &escape_html_attribute(params.resource))
            .replace(
                "{{DEFAULT_EMAIL}}",
                &escape_html_attribute(params.default_email),
            )
            .replace(
                "{{DEFAULT_PASSWORD}}",
                &escape_html_attribute(params.default_password),
            )
    }

    /// Render the OAuth consent page for a client requesting access.
    #[must_use]
    pub fn generate_consent_html(params: ConsentHtmlParams<'_>) -> String {
        use std::fmt::Write;

        let displayed_scope = if params.scope.is_empty() {
            OAuth2AuthorizationServer::default_scope_display()
        } else {
            params.scope.to_owned()
        };
        // Each scope token becomes a list item; the token text is HTML-escaped
        // before the (literal) <li> markup is substituted into the template.
        let scope_items =
            displayed_scope
                .split_whitespace()
                .fold(String::new(), |mut acc, scope| {
                    write!(acc, "<li>{}</li>", escape_html_attribute(scope)).ok();
                    acc
                });

        with_hosted_page_css(Self::OAUTH_CONSENT_TEMPLATE)
            .replace("{{CLIENT_ID}}", &escape_html_attribute(params.client_id))
            .replace(
                "{{REDIRECT_URI}}",
                &escape_html_attribute(params.redirect_uri),
            )
            .replace(
                "{{RESPONSE_TYPE}}",
                &escape_html_attribute(params.response_type),
            )
            .replace("{{STATE}}", &escape_html_attribute(params.state))
            .replace("{{SCOPE}}", &escape_html_attribute(&displayed_scope))
            .replace(
                "{{CODE_CHALLENGE}}",
                &escape_html_attribute(params.code_challenge),
            )
            .replace(
                "{{CODE_CHALLENGE_METHOD}}",
                &escape_html_attribute(params.code_challenge_method),
            )
            .replace("{{RESOURCE}}", &escape_html_attribute(params.resource))
            .replace("{{CSRF_TOKEN}}", &escape_html_attribute(params.csrf_token))
            .replace("{{SCOPE_ITEMS}}", &scope_items)
    }

    /// Render the 401 page shown when the OAuth login form's credentials are
    /// refused, carrying the form's OAuth parameters back to the retry link.
    pub(super) fn login_failure_response(form: &HashMap<String, String>) -> Response {
        // Use embedded template - zero filesystem IO, guaranteed to exist at compile-time
        // Values go into an <a href> URL attribute — URL-encode for URL
        // correctness, then HTML-escape for attribute safety (XSS prevention)
        let error_html = with_hosted_page_css(Self::OAUTH_LOGIN_ERROR_TEMPLATE)
            .replace(
                "{{ERROR_MESSAGE}}",
                &escape_html_attribute(
                    "Authentication Failed: Invalid email or password. Please try again.",
                ),
            )
            .replace(
                "{{CLIENT_ID}}",
                &escape_html_attribute(
                    urlencoding::encode(form.get("client_id").map_or("", |v| v)).as_ref(),
                ),
            )
            .replace(
                "{{REDIRECT_URI}}",
                &escape_html_attribute(
                    urlencoding::encode(form.get("redirect_uri").map_or("", |v| v)).as_ref(),
                ),
            )
            .replace(
                "{{RESPONSE_TYPE}}",
                &escape_html_attribute(
                    urlencoding::encode(form.get("response_type").map_or("", |v| v)).as_ref(),
                ),
            )
            .replace(
                "{{STATE}}",
                &escape_html_attribute(
                    urlencoding::encode(form.get("state").map_or("", |v| v)).as_ref(),
                ),
            )
            .replace(
                "{{SCOPE}}",
                &escape_html_attribute(
                    urlencoding::encode(form.get("scope").map_or("", |v| v)).as_ref(),
                ),
            )
            .replace(
                "{{CODE_CHALLENGE}}",
                &escape_html_attribute(
                    urlencoding::encode(form.get("code_challenge").map_or("", |v| v)).as_ref(),
                ),
            )
            .replace(
                "{{CODE_CHALLENGE_METHOD}}",
                &escape_html_attribute(
                    urlencoding::encode(form.get("code_challenge_method").map_or("", |v| v))
                        .as_ref(),
                ),
            )
            .replace(
                "{{RESOURCE}}",
                &escape_html_attribute(
                    urlencoding::encode(form.get("resource").map_or("", |v| v)).as_ref(),
                ),
            );

        (StatusCode::UNAUTHORIZED, Html(error_html)).into_response()
    }
}
