// ABOUTME: Declares the transport each HTTP request is served over, from the shape of its credential, before any route runs
// ABOUTME: An API key is a caller outside Dravr's apps; a session is the athlete in one of them (carnet#724)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The REST half of the transport gate (carnet#724).
//!
//! Authentication runs inside each handler's extractor, after every layer has
//! entered, so no layer can read the request's `AuthMethod` — and
//! re-authenticating here would write a second usage row. This layer
//! classifies the credential by its shape instead, with the same
//! [`is_api_key_format`] test REST authentication applies first, and serves
//! the rest of the request over the transport it names:
//!
//! - an API key in `Authorization` (or a personal MCP token, which REST
//!   refuses anyway) → [`Transport::ApiKey`], external: a
//!   first-party-only provider's data is withheld from every read the
//!   handler makes, whether a model reads it or not;
//! - a session (a bearer token or the web session cookie) → the athlete's own
//!   app ([`Transport::app_session`]);
//! - no credential → no declaration, as for any work outside a request.
//!
//! It authenticates nothing: a malformed or revoked key is refused by the
//! handler's extractor, which this layer never bypasses. A request carrying an
//! API key in `Authorization` beside a session cookie is classified by the
//! key, which is the credential REST authentication reads — the safe direction
//! either way.
//!
//! `/mcp` and A2A narrow the declaration to their own external transport
//! inside; work a handler spawns does not inherit it, so a cache writer in a
//! spawned task stays ungated.

use axum::extract::Request;
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use pierre_auth::security::cookies::{auth_cookie_name, get_cookie_value};
use pierre_core::auth_header::{extract_bearer_token, is_api_key_format, is_user_mcp_token_format};
use pierre_core::transport::{Transport, CLIENT_PLATFORM_HEADER};
use pierre_providers::ai_scope;

/// Serve the rest of the request over the transport its credential names.
pub async fn declare_request_transport(request: Request, next: Next) -> Response {
    match request_transport(request.headers()) {
        Some(transport) => ai_scope::serve_over(transport, next.run(request)).await,
        None => next.run(request).await,
    }
}

/// The transport a request's credential names, or `None` when it carries
/// none.
fn request_transport(headers: &HeaderMap) -> Option<Transport> {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    if authorization.is_some_and(presents_api_key) {
        return Some(Transport::ApiKey);
    }
    let has_session =
        authorization.is_some() || get_cookie_value(headers, &auth_cookie_name()).is_some();
    has_session.then(|| {
        Transport::app_session(
            headers
                .get(CLIENT_PLATFORM_HEADER)
                .and_then(|value| value.to_str().ok()),
        )
    })
}

/// Whether an `Authorization` value is an API key, raw or behind a `Bearer`
/// scheme, or a personal MCP token. Authentication refuses the MCP token on
/// every REST route (carnet#788); classifying it as external anyway keeps a
/// credential no app holds off the first-party side whatever happens next.
fn presents_api_key(authorization: &str) -> bool {
    is_api_key_format(authorization)
        || extract_bearer_token(authorization)
            .is_ok_and(|token| is_api_key_format(token) || is_user_mcp_token_format(token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn an_api_key_is_external_however_it_is_presented() {
        assert_eq!(
            request_transport(&headers(&[("authorization", "pk_live_abc")])),
            Some(Transport::ApiKey)
        );
        assert_eq!(
            request_transport(&headers(&[("authorization", "Bearer pk_trial_abc")])),
            Some(Transport::ApiKey)
        );
        assert_eq!(
            request_transport(&headers(&[
                ("authorization", "pk_live_abc"),
                ("x-client-platform", "mobile"),
            ])),
            Some(Transport::ApiKey),
            "the client header never turns a key into an app"
        );
        assert_eq!(
            request_transport(&headers(&[
                ("authorization", "Bearer pmcp_abc"),
                ("x-client-platform", "mobile"),
            ])),
            Some(Transport::ApiKey),
            "a personal MCP token is never an app session"
        );
    }

    #[test]
    fn a_session_is_the_athletes_own_app_and_no_credential_declares_nothing() {
        assert_eq!(
            request_transport(&headers(&[("authorization", "Bearer eyJhbGciOi")])),
            Some(Transport::WebApp)
        );
        assert_eq!(
            request_transport(&headers(&[
                ("authorization", "Bearer eyJhbGciOi"),
                ("x-client-platform", "mobile"),
            ])),
            Some(Transport::MobileApp)
        );
        let cookie = format!("{}=eyJhbGciOi", auth_cookie_name());
        assert_eq!(
            request_transport(&headers(&[("cookie", cookie.as_str())])),
            Some(Transport::WebApp)
        );
        assert_eq!(request_transport(&HeaderMap::new()), None);
    }
}
