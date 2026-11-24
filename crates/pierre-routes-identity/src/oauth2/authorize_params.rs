// ABOUTME: How a pending authorization request travels between the flow's pages — query, login form, Google hop, consent
// ABOUTME: Parses it from parameters and writes it back into URLs; a pushed request travels as its client_id and request_uri
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt::Write as _;

use pierre_auth::oauth2_server::models::{AuthorizeRequest, OAuth2Error};
use tracing::trace;

use super::OAuth2Routes;

/// A parameter's value, absent when missing or empty: the login and consent
/// forms always post every field, empty when the request named none.
fn present<'a>(params: &'a HashMap<String, String>, name: &str) -> Option<&'a String> {
    params.get(name).filter(|value| !value.is_empty())
}

impl OAuth2Routes {
    /// Build login URL with OAuth parameters preserved for redirect
    pub(super) fn build_login_url_with_oauth_params(request: &AuthorizeRequest) -> String {
        Self::authorize_params_url("/oauth2/login", request)
    }

    /// Build authorization URL from the login form's fields, with the OAuth
    /// parameters preserved for the redirect
    pub(super) fn build_authorization_url_from_form(form: &HashMap<String, String>) -> String {
        let field = |name: &str| present(form, name).cloned();
        let request = AuthorizeRequest {
            response_type: field("response_type").unwrap_or_default(),
            client_id: field("client_id").unwrap_or_default(),
            redirect_uri: field("redirect_uri").unwrap_or_default(),
            scope: field("scope"),
            state: field("state"),
            code_challenge: field("code_challenge"),
            code_challenge_method: field("code_challenge_method"),
            resource: field("resource"),
            request_uri: field("request_uri"),
            authorization_details: field("authorization_details"),
        };
        Self::authorize_params_url("/oauth2/authorize", &request)
    }

    /// `base` carrying the authorization request's parameters, every value
    /// URL-encoded: `client_id`, `redirect_uri`, `response_type` and `state`
    /// always, the scope, PKCE challenge and method, the RFC 8707 resource
    /// and the RFC 9396 `authorization_details` when present and non-empty.
    /// The login page, the Google sign-in hop and the return to
    /// `/oauth2/authorize` all carry a pending request this one way. A pushed
    /// request (RFC 9126) carries only its `client_id` and `request_uri`: its
    /// parameters are resolved again from the store at each step, never read
    /// from a URL.
    pub(super) fn authorize_params_url(base: &str, request: &AuthorizeRequest) -> String {
        if let Some(request_uri) = request.request_uri.as_deref().filter(|v| !v.is_empty()) {
            return format!(
                "{base}?client_id={}&request_uri={}",
                urlencoding::encode(&request.client_id),
                urlencoding::encode(request_uri)
            );
        }
        let mut url = format!(
            "{base}?client_id={}&redirect_uri={}&response_type={}&state={}",
            urlencoding::encode(&request.client_id),
            urlencoding::encode(&request.redirect_uri),
            urlencoding::encode(&request.response_type),
            urlencoding::encode(request.state.as_deref().unwrap_or(""))
        );
        let optional = [
            ("scope", &request.scope),
            ("code_challenge", &request.code_challenge),
            ("code_challenge_method", &request.code_challenge_method),
            ("resource", &request.resource),
            ("authorization_details", &request.authorization_details),
        ];
        for (name, value) in optional {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                write!(&mut url, "&{name}={}", urlencoding::encode(value)).ok();
            }
        }
        url
    }

    /// The pushed request a page names by its `client_id` and `request_uri`,
    /// unresolved: what the login page needs to carry it on. `None` when the
    /// page names no pushed request.
    pub(super) fn pushed_reference(params: &HashMap<String, String>) -> Option<AuthorizeRequest> {
        let request_uri = present(params, "request_uri")?;
        let client_id = present(params, "client_id")?;
        Some(AuthorizeRequest {
            response_type: String::new(),
            client_id: client_id.clone(),
            redirect_uri: String::new(),
            scope: None,
            state: None,
            code_challenge: None,
            code_challenge_method: None,
            resource: None,
            request_uri: Some(request_uri.clone()),
            authorization_details: None,
        })
    }

    /// Parse query parameters into `AuthorizeRequest`
    pub(super) fn parse_authorize_request(
        params: &HashMap<String, String>,
    ) -> Result<AuthorizeRequest, OAuth2Error> {
        trace!(
            "Parsing OAuth authorize request with {} parameters",
            params.len()
        );

        // A missing response_type is refused by `check_authorize_request`, once
        // the client and redirect_uri are known, so it can reach the client.
        let response_type = params.get("response_type").cloned().unwrap_or_default();

        let client_id = params
            .get("client_id")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing client_id parameter"))?
            .clone(); // Safe: String ownership required for OAuth2 request struct

        let redirect_uri = params
            .get("redirect_uri")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing redirect_uri parameter"))?
            .clone(); // Safe: String ownership required for OAuth2 request struct

        let scope = params.get("scope").cloned();
        let state = params.get("state").cloned();
        let code_challenge = params.get("code_challenge").cloned();
        let code_challenge_method = params.get("code_challenge_method").cloned();
        // The login and consent forms always post a `resource` field, empty
        // when the client named none; empty is absent, not a resource.
        let resource = present(params, "resource").cloned();
        let request_uri = present(params, "request_uri").cloned();
        // The login and consent forms post it empty for a request with none.
        let authorization_details = present(params, "authorization_details").cloned();

        Ok(AuthorizeRequest {
            response_type,
            client_id,
            redirect_uri,
            scope,
            state,
            code_challenge,
            code_challenge_method,
            resource,
            request_uri,
            authorization_details,
        })
    }
}

/// Whether an authorization request carries `OpenID` Connect `prompt=login`
/// (Core §3.1.2.1): the athlete signs in again, whatever session exists.
pub(super) fn requests_fresh_login(params: &HashMap<String, String>) -> bool {
    params.get("prompt").is_some_and(|prompt| {
        prompt
            .split_ascii_whitespace()
            .any(|value| value == "login")
    })
}
