// ABOUTME: CORS middleware configuration for HTTP API endpoints
// ABOUTME: Provides Cross-Origin Resource Sharing setup for web client access
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use http::{header::HeaderName, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Configure CORS settings for the MCP server
///
/// Configures cross-origin requests based on the supplied `allowed_origins` string
/// (typically resolved from the `CORS_ALLOWED_ORIGINS` environment variable by the
/// caller). Supports both wildcard ("*") for development and specific origin
/// lists for production.
///
/// # Security Considerations
///
/// - `allowed_origins` follows the `CORS_ALLOWED_ORIGINS` env-var convention
/// - Falls back to wildcard (*) if `allowed_origins` is empty or "*"
/// - Permits standard HTTP methods (GET, POST, PUT, DELETE, OPTIONS, PATCH)
/// - Includes custom headers for fitness provider authentication
/// - Includes tenant identification headers for multi-tenancy
/// - Exposes `WWW-Authenticate` so browser JS can read the refusal challenge
///
/// # Allowed Headers
///
/// - Standard headers: content-type, authorization, accept, origin
/// - CORS headers: x-requested-with, access-control-request-*
/// - Provider headers: x-strava-client-id, x-fitbit-client-id, etc.
/// - Tenant headers: x-tenant-name, x-tenant-id
/// - API key header: x-pierre-api-key
///
/// # Examples
///
/// ```bash
/// # Allow all origins (development)
/// export CORS_ALLOWED_ORIGINS="*"
///
/// # Allow specific origins (production)
/// export CORS_ALLOWED_ORIGINS="https://app.example.com,https://admin.example.com"
/// ```
pub fn setup_cors(allowed_origins: &str) -> CorsLayer {
    // Parse allowed origins from configuration
    let (allow_origin, credentials_required) =
        if allowed_origins.is_empty() || allowed_origins == "*" {
            // Development mode: allow any origin (credentials not supported with wildcard)
            (AllowOrigin::any(), false)
        } else {
            // Production mode: parse comma-separated origin list
            let origins: Vec<HeaderValue> = allowed_origins
                .split(',')
                .filter_map(|s| {
                    let trimmed = s.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        HeaderValue::from_str(trimmed).ok()
                    }
                })
                .collect();

            if origins.is_empty() {
                // Fallback to any if parsing failed
                (AllowOrigin::any(), false)
            } else {
                // Specific origins: enable credentials for cross-origin cookie auth
                (AllowOrigin::list(origins), true)
            }
        };

    let cors = CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_headers([
            HeaderName::from_static("content-type"),
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-requested-with"),
            HeaderName::from_static("accept"),
            HeaderName::from_static("origin"),
            HeaderName::from_static("access-control-request-method"),
            HeaderName::from_static("access-control-request-headers"),
            HeaderName::from_static("x-strava-client-id"),
            HeaderName::from_static("x-strava-client-secret"),
            HeaderName::from_static("x-fitbit-client-id"),
            HeaderName::from_static("x-fitbit-client-secret"),
            HeaderName::from_static("x-csrf-token"),
            HeaderName::from_static("x-pierre-api-key"),
            HeaderName::from_static("x-tenant-name"),
            HeaderName::from_static("x-tenant-id"),
        ])
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
            Method::PATCH,
        ])
        // `WWW-Authenticate` carries the RFC 6750 challenge (`error="..."`,
        // `scope="..."`), the only signal that separates a refusal a client
        // recovers from by re-authenticating — 401, or 403 with
        // `error="insufficient_scope"` — from a standing authorization
        // decision that must leave the session signed in. The header is not
        // CORS-safelisted, so a browser withholds it from JS on a cross-origin
        // response unless it is named here; the client then reads an empty
        // challenge and treats every refusal alike. The frontend image bakes no
        // `VITE_API_BASE_URL` and nginx proxies `/api` to this service, so the
        // shipped deployment is same-origin — exposing the header is what makes
        // the challenge readable in any origin configuration instead of by
        // accident of that proxy. Named rather than `Any`, because `Any` emits
        // `Access-Control-Expose-Headers: *`, which browsers ignore on a
        // credentialed request — the shape the web adapter sends.
        .expose_headers([HeaderName::from_static("www-authenticate")]);

    if credentials_required {
        cors.allow_credentials(true)
    } else {
        cors
    }
}
