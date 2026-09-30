// ABOUTME: Secure HTTP cookie utilities for authentication and session management
// ABOUTME: Provides httpOnly, Secure, SameSite cookie helpers to prevent XSS and CSRF attacks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Secure cookie utilities
//!
//! This module provides helpers for creating secure HTTP cookies with proper
//! security flags to prevent XSS, CSRF, and session hijacking attacks.

use std::env;

use axum::http::{header, HeaderMap, HeaderValue};

/// Cookie security configuration
pub struct SecureCookieConfig {
    /// Cookie name
    pub name: String,
    /// Cookie value
    pub value: String,
    /// Max-Age in seconds
    pub max_age_secs: i64,
    /// `HttpOnly` flag (prevents JavaScript access)
    pub http_only: bool,
    /// Secure flag (HTTPS only)
    pub secure: bool,
    /// `SameSite` policy
    pub same_site: SameSitePolicy,
    /// Cookie path
    pub path: String,
}

/// `SameSite` cookie policy
#[derive(Debug, Clone, Copy)]
pub enum SameSitePolicy {
    /// Strict: Cookie only sent in first-party context
    Strict,
    /// Lax: Cookie sent on top-level navigation
    Lax,
    /// None: Cookie sent in all contexts (requires Secure=true)
    None,
}

impl SecureCookieConfig {
    /// Create a new secure cookie configuration with defaults
    ///
    /// The `Secure` flag is derived from the `BASE_URL` environment variable:
    /// - `https://` URLs set `Secure=true` (production, Cloudflare tunnels)
    /// - `http://` URLs set `Secure=false` (local development)
    /// - If `BASE_URL` is unset, defaults to `Secure=true` (fail-secure)
    #[must_use]
    pub fn new(name: String, value: String, max_age_secs: i64) -> Self {
        let secure = infer_secure_flag();
        // Cross-origin deployment (HTTPS) requires SameSite=None for cookies to be
        // sent with cross-origin requests. Local dev uses Vite proxy (same-origin),
        // so SameSite=Lax is sufficient.
        let same_site = if secure {
            SameSitePolicy::None
        } else {
            SameSitePolicy::Lax
        };
        Self {
            name,
            value,
            max_age_secs,
            http_only: true,
            secure,
            same_site,
            path: "/".to_owned(),
        }
    }

    /// Build the Set-Cookie header value
    #[must_use]
    pub fn build(&self) -> String {
        use std::fmt::Write;
        let mut cookie = format!("{}={}", self.name, self.value);

        // Max-Age
        let _ = write!(cookie, "; Max-Age={}", self.max_age_secs);

        // Path
        let _ = write!(cookie, "; Path={}", self.path);

        // HttpOnly
        if self.http_only {
            cookie.push_str("; HttpOnly");
        }

        // Secure
        if self.secure {
            cookie.push_str("; Secure");
        }

        // SameSite
        match self.same_site {
            SameSitePolicy::Strict => cookie.push_str("; SameSite=Strict"),
            SameSitePolicy::Lax => cookie.push_str("; SameSite=Lax"),
            SameSitePolicy::None => cookie.push_str("; SameSite=None"),
        }

        cookie
    }
}

/// The name a host-only cookie is set and read under.
///
/// A `Secure` cookie takes the `__Host-` prefix, which browsers accept only
/// from a secure origin, with `Path=/` and no `Domain`: a sibling subdomain
/// can then neither set nor overwrite it (cookie tossing). Over plain HTTP,
/// in local development, the prefix would make browsers drop the cookie, so
/// the bare name is kept.
#[must_use]
pub fn host_cookie_name(name: &str, secure: bool) -> String {
    if secure {
        format!("__Host-{name}")
    } else {
        name.to_owned()
    }
}

/// Base name of the web app's session cookie, before the host prefix.
const AUTH_COOKIE: &str = "auth_token";

/// The name the web app's session cookie is set, read and cleared under.
///
/// The name follows the same predicate as the cookie's `Secure` flag
/// (derived from `BASE_URL`), because browsers drop a
/// `__Host-` cookie that lacks `Secure`. On an HTTPS deployment the cookie is
/// `__Host-auth_token`: it can only be set by the host itself, with `Path=/`
/// and no `Domain`, so a sibling host under the parent domain cannot plant a
/// session in its place. A plain `auth_token` cookie is never read there —
/// every reader matches this exact name. Over plain HTTP (local development,
/// tests) the bare `auth_token` name is kept.
#[must_use]
pub fn auth_cookie_name() -> String {
    host_cookie_name(AUTH_COOKIE, infer_secure_flag())
}

/// The name the web app's session cookie is read under on an endpoint whose
/// own origin is known to be HTTPS independently of `BASE_URL`.
///
/// The OAuth authorization server's origin is its issuer, which
/// `OAUTH2_ISSUER_URL` can set apart from `BASE_URL`. When either one is
/// HTTPS the host-prefixed name is required: an HTTPS issuer served next to a
/// plain-HTTP `BASE_URL` would otherwise accept a bare `auth_token` that a
/// sibling host can plant. When the two disagree the bridge from the web
/// session simply finds no cookie, and the user signs in to the authorization
/// server itself.
#[must_use]
pub fn auth_cookie_name_on(origin_secure: bool) -> String {
    host_cookie_name(AUTH_COOKIE, origin_secure || infer_secure_flag())
}

/// Set a secure authentication cookie
///
/// # Arguments
/// * `headers` - HTTP headers to modify
/// * `token` - JWT token to store in cookie
/// * `max_age_secs` - Cookie expiration in seconds
pub fn set_auth_cookie(headers: &mut HeaderMap, token: &str, max_age_secs: i64) {
    let cookie = SecureCookieConfig::new(auth_cookie_name(), token.to_owned(), max_age_secs);

    if let Ok(header_value) = HeaderValue::from_str(&cookie.build()) {
        headers.insert(header::SET_COOKIE, header_value);
    }
}

/// Set a CSRF token cookie (less restrictive to allow JavaScript read)
///
/// # Arguments
/// * `headers` - HTTP headers to modify
/// * `csrf_token` - CSRF token to store
/// * `max_age_secs` - Cookie expiration in seconds
pub fn set_csrf_cookie(headers: &mut HeaderMap, csrf_token: &str, max_age_secs: i64) {
    let mut cookie =
        SecureCookieConfig::new("csrf_token".to_owned(), csrf_token.to_owned(), max_age_secs);

    // CSRF cookie should NOT be HttpOnly so JavaScript can read it
    cookie.http_only = false;

    if let Ok(header_value) = HeaderValue::from_str(&cookie.build()) {
        headers.append(header::SET_COOKIE, header_value);
    }
}

/// Clear authentication cookie
///
/// # Arguments
/// * `headers` - HTTP headers to modify
pub fn clear_auth_cookie(headers: &mut HeaderMap) {
    let secure = infer_secure_flag();
    let same_site = if secure { "None" } else { "Lax" };
    let mut cookie = format!(
        "{}=; Max-Age=0; Path=/; HttpOnly; SameSite={same_site}",
        host_cookie_name(AUTH_COOKIE, secure)
    );
    if secure {
        cookie.push_str("; Secure");
    }

    if let Ok(header_value) = HeaderValue::from_str(&cookie) {
        headers.insert(header::SET_COOKIE, header_value);
    }
}

/// Derive the `Secure` cookie flag from the `BASE_URL` environment variable.
///
/// Returns `true` when `BASE_URL` starts with `https://` or is unset (fail-secure),
/// `false` when `BASE_URL` starts with `http://` (plain HTTP dev environments).
fn infer_secure_flag() -> bool {
    env::var("BASE_URL").map_or(true, |url| url.starts_with("https://"))
}

/// Extract cookie value from request headers
///
/// # Arguments
/// * `headers` - Request headers
/// * `cookie_name` - Name of cookie to extract
///
/// # Returns
/// Cookie value if found, None otherwise
#[must_use]
pub fn get_cookie_value(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|cookie| {
            let mut parts = cookie.trim().splitn(2, '=');
            let name = parts.next()?.trim();
            let value = parts.next()?.trim();

            if name == cookie_name {
                Some(value.to_owned())
            } else {
                None
            }
        })
}
