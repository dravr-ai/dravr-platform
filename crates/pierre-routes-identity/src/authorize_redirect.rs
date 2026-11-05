// ABOUTME: Where an OAuth 2.0 authorization response goes: a code or an error redirected to the client
// ABOUTME: Only a verified client and redirect_uri is redirected to; an untrusted one gets the error page
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Authorization responses of `/oauth2/authorize` and `/oauth2/consent`
//! (RFC 6749 Sections 4.1.2 and 4.1.2.1).

use axum::response::{IntoResponse, Redirect, Response};
use pierre_auth::oauth2_server::models::{AuthorizeRejection, AuthorizeRequest, OAuth2Error};
use tracing::info;

use crate::oauth2::OAuth2Routes;

/// `redirect_uri` with `params` appended as query parameters, each value
/// percent-encoded, keeping any query the registered URI already carries
/// (RFC 6749 Section 3.1.2).
fn client_redirect_url(redirect_uri: &str, params: &[(&str, &str)]) -> String {
    let mut url = redirect_uri.to_owned();
    let mut separator = if redirect_uri.contains('?') { '&' } else { '?' };
    for (name, value) in params {
        url.push(separator);
        url.push_str(name);
        url.push('=');
        url.push_str(&urlencoding::encode(value));
        separator = '&';
    }
    url
}

/// The authorization code, redirected to the client's verified
/// `redirect_uri` with the request's `state` (RFC 6749 Section 4.1.2) and the
/// `issuer` that issued it (RFC 9207), which lets a client that talks to more
/// than one authorization server refuse a response from the wrong one.
pub(crate) fn code_redirect(
    issuer: &str,
    redirect_uri: &str,
    code: &str,
    state: Option<&str>,
) -> Response {
    let mut params = vec![("code", code)];
    if let Some(state) = state {
        params.push(("state", state));
    }
    params.push(("iss", issuer));
    Redirect::to(&client_redirect_url(redirect_uri, &params)).into_response()
}

/// An authorization error, redirected to the client's verified
/// `redirect_uri` with `error`, `error_description`, the request's `state`
/// (RFC 6749 Section 4.1.2.1) and the `issuer` (RFC 9207), which the error
/// responses carry as the success ones do.
pub(crate) fn error_redirect(
    issuer: &str,
    redirect_uri: &str,
    state: Option<&str>,
    error: &OAuth2Error,
) -> Response {
    let mut params = vec![("error", error.error.as_str())];
    if let Some(description) = error.error_description.as_deref() {
        params.push(("error_description", description));
    }
    if let Some(state) = state {
        params.push(("state", state));
    }
    params.push(("iss", issuer));
    info!(
        error = %error.error,
        "OAuth authorization refused, returning the error to the client"
    );
    Redirect::to(&client_redirect_url(redirect_uri, &params)).into_response()
}

/// A refused authorization request, delivered where RFC 6749 Section 4.1.2.1
/// sends it: an unknown client or unregistered `redirect_uri` to the user as
/// the error page, anything else back to the client's `redirect_uri`.
pub(crate) fn rejection_response(
    issuer: &str,
    rejection: AuthorizeRejection,
    request: &AuthorizeRequest,
) -> Response {
    match rejection {
        AuthorizeRejection::ShownToUser(error) => OAuth2Routes::render_oauth_error_response(&error),
        AuthorizeRejection::RedirectedToClient(error) => error_redirect(
            issuer,
            &request.redirect_uri,
            request.state.as_deref(),
            &error,
        ),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::header::LOCATION;

    use super::*;

    const ISSUER: &str = "https://app.dravr.ai";

    fn location(response: &Response) -> String {
        response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn a_code_response_names_its_issuer() {
        let response = code_redirect(
            ISSUER,
            "https://client.example/cb?tenant=a",
            "the-code",
            Some("st ate"),
        );
        assert_eq!(
            location(&response),
            "https://client.example/cb?tenant=a&code=the-code&state=st%20ate\
             &iss=https%3A%2F%2Fapp.dravr.ai"
        );
    }

    #[test]
    fn an_error_response_names_its_issuer() {
        let response = error_redirect(
            ISSUER,
            "https://client.example/cb",
            Some("s"),
            &OAuth2Error::access_denied("no"),
        );
        assert_eq!(
            location(&response),
            "https://client.example/cb?error=access_denied&error_description=no&state=s\
             &iss=https%3A%2F%2Fapp.dravr.ai"
        );
    }
}
