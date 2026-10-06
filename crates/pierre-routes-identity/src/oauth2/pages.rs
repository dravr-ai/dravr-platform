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
use url::Url;

use super::login_text::{LoginPageLabels, LoginText};
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
    /// Where "Continue with Google" starts the Google sign-in for this
    /// request; `None` hides the button (Google sign-in is not configured,
    /// or the page names no request to return to)
    pub google_start_url: Option<&'a str>,
    /// The form's text in the athlete's language
    pub labels: &'a LoginPageLabels,
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
    /// The name the client registered, else its client id
    pub client_label: &'a str,
    /// The account the athlete is signed in as
    pub account_email: &'a str,
    /// The login page for this request, to sign in as another account
    pub switch_account_url: &'a str,
}

impl OAuth2Routes {
    /// OAuth error template embedded at compile-time
    const OAUTH_ERROR_TEMPLATE: &'static str = include_str!("../../templates/oauth_error.html");

    /// What the error page says when the error carries no description
    const DEFAULT_ERROR_DESCRIPTION: &'static str = "Please try again.";

    /// The error page with no particular error to name.
    pub(super) fn generic_error_html() -> String {
        with_hosted_page_css(Self::OAUTH_ERROR_TEMPLATE).replace(
            "{{DESCRIPTION}}",
            &format!("<p>{}</p>", Self::DEFAULT_ERROR_DESCRIPTION),
        )
    }

    /// OAuth login page template embedded at compile-time
    /// Loaded with `include_str`!() to avoid blocking filesystem IO at runtime
    const OAUTH_LOGIN_TEMPLATE: &'static str = include_str!("../../templates/oauth_login.html");

    /// OAuth consent page template embedded at compile-time
    const OAUTH_CONSENT_TEMPLATE: &'static str = include_str!("../../templates/oauth_consent.html");

    /// The "Continue with Google" block of the login page; `{{GOOGLE_START_URL}}`
    /// is filled with the escaped start URL
    const GOOGLE_SIGN_IN_BLOCK: &'static str = r#"<p class="fineprint">{{T_OR}}</p>
        <a class="btn btn-secondary btn-block" href="{{GOOGLE_START_URL}}">{{T_GOOGLE}}</a>"#;

    /// OAuth login error template embedded at compile-time
    /// Loaded with `include_str`!() to avoid blocking filesystem IO at runtime
    const OAUTH_LOGIN_ERROR_TEMPLATE: &'static str =
        include_str!("../../templates/oauth_login_error.html");

    /// Render HTML error page for OAuth errors shown in browser: 429 for
    /// `too_many_requests`, 503 for `temporarily_unavailable`, else 400
    pub(crate) fn render_oauth_error_response(error: &OAuth2Error) -> Response {
        // The description an OAuth error carries is written for the person
        // reading it (RFC 6749 §5.2), never a server's internal detail.
        let error_description = error
            .error_description
            .as_deref()
            .unwrap_or(Self::DEFAULT_ERROR_DESCRIPTION);

        let html = with_hosted_page_css(Self::OAUTH_ERROR_TEMPLATE).replace(
            "{{DESCRIPTION}}",
            &format!("<p>{}</p>", escape_html_attribute(error_description)),
        );

        (error.http_status(), Html(html)).into_response()
    }

    /// Render the consent screen for a not-yet-granted authorization request,
    /// naming the client, where it returns the athlete, and the account the
    /// athlete is signed in as.
    pub(super) fn render_consent_page(
        request: &AuthorizeRequest,
        csrf_token: &str,
        client_name: Option<&str>,
        account_email: &str,
    ) -> Response {
        let client_label = client_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&request.client_id);
        let switch_account_url = Self::authorize_params_url("/oauth2/login", request);
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
            client_label,
            account_email,
            switch_account_url: &switch_account_url,
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
        let labels = params.labels;
        let google_sign_in = params.google_start_url.map_or_else(String::new, |url| {
            Self::GOOGLE_SIGN_IN_BLOCK
                .replace("{{GOOGLE_START_URL}}", &escape_html_attribute(url))
                .replace("{{T_OR}}", &escape_html_attribute(&labels.or))
                .replace("{{T_GOOGLE}}", &escape_html_attribute(&labels.google))
        });

        with_hosted_page_css(Self::OAUTH_LOGIN_TEMPLATE)
            .replace("{{LANG}}", &escape_html_attribute(&labels.lang))
            .replace("{{T_SIGN_IN}}", &escape_html_attribute(&labels.sign_in))
            .replace("{{T_EMAIL}}", &escape_html_attribute(&labels.email))
            .replace("{{T_PASSWORD}}", &escape_html_attribute(&labels.password))
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
            .replace("{{GOOGLE_SIGN_IN}}", &google_sign_in)
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
            .replace(
                "{{CLIENT_LABEL}}",
                &escape_html_attribute(params.client_label),
            )
            .replace(
                "{{REDIRECT_HOST}}",
                &escape_html_attribute(&redirect_host(params.redirect_uri)),
            )
            .replace(
                "{{ACCOUNT_EMAIL}}",
                &escape_html_attribute(params.account_email),
            )
            .replace(
                "{{SWITCH_ACCOUNT_URL}}",
                &escape_html_attribute(params.switch_account_url),
            )
            .replace("{{SCOPE_ITEMS}}", &scope_items)
    }

    /// Render the page a refused hosted sign-in answers with, in the
    /// athlete's language: `message` says why, and the retry link carries the
    /// form's OAuth parameters back to the login.
    pub(super) fn login_failure_response(
        form: &HashMap<String, String>,
        text: &LoginText<'_>,
        message: &str,
        status: StatusCode,
    ) -> Response {
        // Use embedded template - zero filesystem IO, guaranteed to exist at compile-time
        // Values go into an <a href> URL attribute — URL-encode for URL
        // correctness, then HTML-escape for attribute safety (XSS prevention)
        let labels = text.labels();
        let error_html = with_hosted_page_css(Self::OAUTH_LOGIN_ERROR_TEMPLATE)
            .replace("{{LANG}}", &escape_html_attribute(&labels.lang))
            .replace("{{T_FAILED}}", &escape_html_attribute(&text.failed()))
            .replace("{{T_BACK}}", &escape_html_attribute(&text.back()))
            .replace("{{ERROR_MESSAGE}}", &escape_html_attribute(message))
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

        (status, Html(error_html)).into_response()
    }
}

/// The host `redirect_uri` returns to, as the consent screen names it: the
/// host (and port, when one is named) the browser will actually go to, read
/// by the same WHATWG URL rules browsers apply, so a `\` or a user-info
/// part cannot make the screen name one host while the browser goes to
/// another. A URI with no host (`urn:ietf:wg:oauth:2.0:oob`) is shown whole.
fn redirect_host(redirect_uri: &str) -> String {
    Url::parse(redirect_uri)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_owned();
            Some(match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_else(|| redirect_uri.to_owned())
}
