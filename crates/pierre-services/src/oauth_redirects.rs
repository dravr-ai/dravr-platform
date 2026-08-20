// ABOUTME: Post-OAuth redirect URL validation and construction — allowlist, state decoding, return URLs
// ABOUTME: Pure helpers shared by the OAuth callback routes and the OAuth flow service
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use tracing::warn;
use urlencoding::encode;

/// App-specific URL schemes that are always allowed for mobile OAuth redirects.
/// These are deep-link schemes that cannot be intercepted by external websites.
const APP_SCHEMES: &[&str] = &["dravr://", "exp://", "http://localhost"];

/// Validate a mobile OAuth redirect URL against the allowlist.
///
/// Allowed redirect targets:
/// - `dravr://` deep links (mobile app)
/// - `exp://` deep links (Expo development)
/// - `http://localhost` (local development)
/// - `https://` URLs whose origin matches `base_url` or an entry in
///   `allowed_redirect_origins` (prevents open-redirect to arbitrary sites)
///
/// The `base_url` is the server's own origin (e.g. `https://api.dravr.ai`).
/// `extra_origins` are additional HTTPS origins configured via
/// `ALLOWED_MOBILE_REDIRECT_ORIGINS` (e.g. Cloudflare tunnel URLs).
#[must_use]
pub fn is_allowed_redirect_url(url: &str, base_url: &str, extra_origins: &[String]) -> bool {
    // App-specific schemes are always safe
    if APP_SCHEMES.iter().any(|scheme| url.starts_with(scheme)) {
        return true;
    }

    // For https:// URLs, verify the origin matches an allowlisted host
    if url.starts_with("https://") {
        return is_origin_allowed(url, base_url, extra_origins);
    }

    false
}

/// Extract the host portion from a URL string (`scheme://host/path` -> host)
fn extract_host(url: &str) -> Option<&str> {
    // Strip scheme
    let after_scheme = url.split("://").nth(1)?;
    // Take host (before first / or ? or :port)
    let host = after_scheme
        .split('/')
        .next()?
        .split('?')
        .next()?
        .split(':')
        .next()?;
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Check whether an HTTPS URL's origin matches the server `base_url` or an extra allowed origin.
fn is_origin_allowed(url: &str, base_url: &str, extra_origins: &[String]) -> bool {
    let Some(redirect_host) = extract_host(url) else {
        warn!("Failed to extract host from redirect URL: {url}");
        return false;
    };

    // Check against server's own base_url
    if let Some(base_host) = extract_host(base_url) {
        if base_host == redirect_host {
            return true;
        }
    }

    // Check against extra allowed origins
    for origin in extra_origins {
        if let Some(allowed_host) = extract_host(origin) {
            if allowed_host == redirect_host {
                return true;
            }
        }
    }

    warn!(
        "Redirect URL host '{}' not in allowlist (base_url: {}, extra: {:?})",
        redirect_host, base_url, extra_origins
    );
    false
}

/// Extract mobile redirect URL from the OAuth state string
///
/// State format: `{user_id}:{random}:{base64_redirect_url}`
/// The redirect URL is embedded as base64-encoded data in the third segment.
///
/// Returns `None` if the state doesn't contain a redirect URL or decoding fails.
/// The `base_url` and `extra_origins` are used to validate HTTPS redirect targets.
#[must_use]
pub fn extract_mobile_redirect_from_state(
    state: &str,
    base_url: &str,
    extra_origins: &[String],
) -> Option<String> {
    let parts: Vec<&str> = state.splitn(3, ':').collect();
    parts
        .get(2)
        .filter(|s| !s.is_empty())
        .and_then(|encoded| decode_and_validate_redirect_url(encoded, base_url, extra_origins))
}

/// Decode a base64-encoded redirect URL and validate against the allowlist
///
/// Only URLs with allowed schemes/origins are accepted to prevent open redirect attacks.
///
/// Returns `None` if decoding fails or the URL is not allowed.
#[must_use]
pub fn decode_and_validate_redirect_url(
    encoded: &str,
    base_url: &str,
    extra_origins: &[String],
) -> Option<String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|e| {
            warn!("Failed to decode base64 redirect URL: {}", e);
            e
        })
        .ok()
        .and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|e| {
                    warn!("Failed to decode redirect URL as UTF-8: {}", e);
                    e
                })
                .ok()
        })
        .and_then(|url| {
            if is_allowed_redirect_url(&url, base_url, extra_origins) {
                Some(url)
            } else {
                warn!("Rejected redirect URL (not in allowlist): {}", url);
                None
            }
        })
}

/// Append the `provider`/`success`/`error` result params to a post-OAuth URL.
///
/// Chooses `&` when `base` already carries a query string and `?` otherwise.
/// Mobile deep links (`dravr://oauth-callback`) and the SPA (`/oauth-callback`)
/// have no query so they get `?`; the channel-initiated hosted connect flow
/// returns to `/providers/connect?token=…`, which already has one and must get
/// `&` — otherwise a second `?` would corrupt the URL and the picker would drop
/// its connect token.
#[must_use]
pub fn oauth_return_url(base: &str, provider: &str, success: bool, error: Option<&str>) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    error.map_or_else(
        || format!("{base}{sep}provider={}&success={success}", encode(provider)),
        |err| {
            format!(
                "{base}{sep}provider={}&success={success}&error={}",
                encode(provider),
                encode(err)
            )
        },
    )
}
