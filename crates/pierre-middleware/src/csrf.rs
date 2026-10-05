// ABOUTME: CSRF validation middleware for state-changing HTTP requests
// ABOUTME: Validates X-CSRF-Token header against HMAC-signed tokens to prevent request forgery
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! CSRF validation middleware
//!
//! This middleware validates CSRF tokens for state-changing operations (POST, PUT, DELETE, PATCH).
//! It extracts the token from the X-CSRF-Token header and validates it against the user's session.

use axum::body::Body;
use axum::extract::State;
use axum::http::header::COOKIE;
use axum::http::{HeaderMap, HeaderValue, Method, Request};
use axum::middleware::Next;
use axum::response::Response;
use pierre_auth::security::cookies::{auth_cookie_name, get_cookie_value};
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_core::errors::{AppError, AppResult};
use pierre_runtime_context::MiddlewareCtx;
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

/// Paths exempt from CSRF validation. These are either pre-authentication
/// (login, register) or session-teardown (logout) endpoints where the client
/// may not yet have -- or has already discarded -- a CSRF token.
const CSRF_EXEMPT_PATHS: &[&str] = &[
    "/oauth/token",
    "/api/auth/logout",
    "/api/auth/register",
    "/api/auth/firebase",
    // Browser device-approval: credential-gated (email/password in the body), so a
    // cross-site attacker cannot forge it — no ambient session is trusted here.
    "/admin/device/approve-web",
    // The OAuth login form: credential-gated like the device approval above.
    "/oauth2/login",
    // The OAuth consent form: an HTML form cannot send the header, so it
    // carries its own synchronizer token, which the consent handler checks.
    "/oauth2/consent",
    // The hosted Intervals.icu connect form: an HTML form cannot send the
    // header, and the signed connect link-token it posts in its body is what
    // authorizes it — a cross-site page cannot hold that token.
    "/providers/connect/intervals_icu",
];

/// Validate the CSRF token on a state-changing HTTP request.
///
/// GET / HEAD / OPTIONS / etc. return `Ok(())` without inspecting the headers.
/// For POST / PUT / DELETE / PATCH the `X-CSRF-Token` header is required and
/// is validated against `csrf_manager` for the supplied `user_id` (stateless
/// HMAC verification — no server-side storage).
///
/// # Errors
///
/// Returns an error if:
/// - the request is state-changing and the `X-CSRF-Token` header is missing
/// - the token is invalid, expired, or tied to a different user
pub fn validate_csrf_token(
    headers: &HeaderMap,
    method: &Method,
    user_id: Uuid,
    csrf_manager: &CsrfTokenManager,
) -> AppResult<()> {
    if !matches!(
        method,
        &Method::POST | &Method::PUT | &Method::DELETE | &Method::PATCH
    ) {
        return Ok(());
    }

    let csrf_token = headers
        .get("X-CSRF-Token")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            warn!(
                user_id = %user_id,
                method = %method,
                "CSRF token missing for state-changing request"
            );
            AppError::auth_invalid("CSRF token required for this operation")
        })?;

    csrf_manager
        .validate_token(csrf_token, user_id)
        .map_err(|e| {
            warn!(
                user_id = %user_id,
                method = %method,
                error = %e,
                "CSRF token validation failed"
            );
            e
        })?;

    debug!(
        user_id = %user_id,
        method = %method,
        "CSRF token validated successfully"
    );

    Ok(())
}

/// Axum middleware layer for CSRF validation on cookie-authenticated requests.
///
/// This middleware enforces CSRF protection for state-changing HTTP methods
/// (POST, PUT, DELETE, PATCH) when the request is authenticated via cookies.
/// Requests using Bearer tokens or API keys (programmatic clients) bypass
/// CSRF validation since they are not susceptible to cross-site request forgery.
///
/// Auth endpoints (login, logout, register) are exempt since they operate
/// before or after a valid session exists.
///
/// # Errors
///
/// Returns 401 if a cookie-authenticated state-changing request lacks a valid
/// X-CSRF-Token header.
pub async fn csrf_protection_layer<C: MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let method = request.method().clone();
    let headers = request.headers().clone();
    let path = request.uri().path();

    // Only validate state-changing methods
    if !matches!(
        method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    ) {
        return Ok(next.run(request).await);
    }

    // Skip CSRF for auth endpoints that operate before or after a session
    if CSRF_EXEMPT_PATHS.contains(&path) {
        return Ok(next.run(request).await);
    }

    // Only enforce CSRF for cookie-authenticated requests (browser clients).
    // Programmatic clients using Bearer tokens or API keys are not vulnerable
    // to CSRF and should not be required to send CSRF tokens.
    let has_bearer_or_api_key = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| v.starts_with("Bearer ") || v.starts_with("Api-Key "));

    if has_bearer_or_api_key {
        // The header authenticates this request, never an ambient cookie
        // riding beside it: the auth middleware tries the cookie first, so a
        // cookie left on a request that skipped the check would authenticate
        // it unchecked (carnet#768).
        strip_cookie(request.headers_mut(), &auth_cookie_name());
        return Ok(next.run(request).await);
    }

    // Check if request has cookie auth; skip CSRF for non-cookie requests
    let Some(auth_token) = get_cookie_value(&headers, &auth_cookie_name()) else {
        return Ok(next.run(request).await);
    };

    // Extract user_id from the cookie JWT to validate CSRF token ownership.
    // A cookie this check cannot read (expired, a retired signing key, an
    // audience it does not take) is removed before the request goes on, so
    // the request IS unauthenticated rather than merely treated as such: no
    // handler behind this layer can authenticate with a cookie whose CSRF
    // token was never checked, whatever its own validation would make of it.
    // Refusing outright instead would lock a browser holding a stale cookie
    // out of the unauthenticated forms (forgot-password, register) until the
    // cookie expired (carnet#768).
    let Ok(claims) = resources
        .auth_manager()
        .validate_token(&auth_token, resources.jwks_manager())
    else {
        debug!("Stale or invalid auth cookie in CSRF check, removed from the request");
        strip_cookie(request.headers_mut(), &auth_cookie_name());
        return Ok(next.run(request).await);
    };

    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::auth_invalid("Invalid user ID in authentication token"))?;

    validate_csrf_token(&headers, &method, user_id, resources.csrf_manager())?;

    Ok(next.run(request).await)
}

/// Remove every cookie named `name` from the request's `Cookie` headers,
/// keeping the others. A header left with no cookie is dropped.
fn strip_cookie(headers: &mut HeaderMap, name: &str) {
    let kept: Vec<String> = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|pair| {
            !pair.is_empty() && pair.split_once('=').map_or(*pair, |(key, _)| key.trim()) != name
        })
        .map(str::to_owned)
        .collect();
    headers.remove(COOKIE);
    if kept.is_empty() {
        return;
    }
    if let Ok(value) = HeaderValue::from_str(&kept.join("; ")) {
        headers.insert(COOKIE, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookies(headers: &HeaderMap) -> Vec<&str> {
        headers
            .get_all(COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect()
    }

    #[test]
    fn strip_cookie_removes_only_the_named_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("theme=dark; auth_token=abc; csrf=x"),
        );
        strip_cookie(&mut headers, "auth_token");
        assert_eq!(cookies(&headers), vec!["theme=dark; csrf=x"]);
    }

    #[test]
    fn strip_cookie_drops_the_header_when_nothing_is_left() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, HeaderValue::from_static("auth_token=abc"));
        headers.append(COOKIE, HeaderValue::from_static(" auth_token=def "));
        strip_cookie(&mut headers, "auth_token");
        assert!(headers.get(COOKIE).is_none());
    }

    #[test]
    fn strip_cookie_does_not_match_a_name_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("auth_token_hint=1; __Host-auth_token=abc"),
        );
        strip_cookie(&mut headers, "auth_token");
        assert_eq!(
            cookies(&headers),
            vec!["auth_token_hint=1; __Host-auth_token=abc"]
        );
    }
}
