// ABOUTME: Post-OAuth redirect URL validation and construction — allowlist, state decoding, return URLs
// ABOUTME: Pure helpers shared by the OAuth callback routes and the OAuth flow service
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::iter;
use std::net::Ipv4Addr;

use pierre_core::errors::AppError;
use tracing::warn;
use url::{Host, Url};
use urlencoding::encode;

use crate::import_tombstones::{refused_tombstone_deleted_at, refused_tombstone_id};
use crate::oauth_flow::categorize_oauth_error;

/// App deep-link schemes that are always allowed for mobile OAuth redirects.
/// A website cannot claim them, so a redirect there reaches the app.
const APP_SCHEMES: &[&str] = &["dravr", "exp"];

/// Validate a mobile OAuth redirect URL against the allowlist.
///
/// Allowed redirect targets:
/// - `dravr://` deep links (mobile app)
/// - `exp://` deep links (Expo development)
/// - `http://localhost` or `http://127.0.0.1`, any port (local development)
/// - `https://` URLs whose origin — scheme, host and port — equals `base_url`
///   or an entry in `allowed_redirect_origins`
///
/// The URL is parsed, never prefix-matched: `http://localhost.evil.com` and
/// `http://localhost@evil.com` both start with `http://localhost`, and
/// `https://api.dravr.ai:@evil.com` reads as `api.dravr.ai` to a scan that
/// stops at the first `:`. A URL carrying userinfo is refused outright, since
/// userinfo exists only to make a URL's host hard to read.
///
/// The `base_url` is the server's own origin (e.g. `https://api.dravr.ai`).
/// `extra_origins` are additional HTTPS origins configured via
/// `ALLOWED_MOBILE_REDIRECT_ORIGINS` (e.g. Cloudflare tunnel URLs).
#[must_use]
pub fn is_allowed_redirect_url(url: &str, base_url: &str, extra_origins: &[String]) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        warn!("Rejected redirect URL that does not parse: {url}");
        return false;
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        warn!("Rejected redirect URL carrying userinfo: {url}");
        return false;
    }

    match parsed.scheme() {
        scheme if APP_SCHEMES.contains(&scheme) => true,
        "http" => is_loopback_host(&parsed),
        "https" => is_origin_allowed(&parsed, base_url, extra_origins),
        _ => false,
    }
}

/// Whether an `http://` URL names this machine: `localhost` or `127.0.0.1`.
fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain == "localhost",
        Some(Host::Ipv4(ip)) => ip == Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(_)) | None => false,
    }
}

/// Check whether an HTTPS URL's origin equals the server `base_url`'s or an extra allowed origin's.
fn is_origin_allowed(url: &Url, base_url: &str, extra_origins: &[String]) -> bool {
    let origin = url.origin();
    let allowed = iter::once(base_url)
        .chain(extra_origins.iter().map(String::as_str))
        .filter_map(|candidate| Url::parse(candidate).ok())
        .any(|candidate| candidate.origin() == origin);

    if !allowed {
        warn!(
            "Redirect URL origin '{}' not in allowlist (base_url: {}, extra: {:?})",
            origin.ascii_serialization(),
            base_url,
            extra_origins
        );
    }
    allowed
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

/// The return URL for a callback that failed with `error`.
///
/// The success flag is off and `error` is what [`categorize_oauth_error`]
/// names it. A tombstone refusal also carries `tombstone_id`, which the
/// account clears it by, and `deleted_at`, the date the client's sentence
/// names.
#[must_use]
pub fn oauth_failure_url(base: &str, provider: &str, error: &AppError) -> String {
    let mut url = oauth_return_url(base, provider, false, Some(categorize_oauth_error(error).0));
    if let Some(id) = refused_tombstone_id(error) {
        url.push_str("&tombstone_id=");
        url.push_str(&encode(id));
        if let Some(deleted_at) = refused_tombstone_deleted_at(error) {
            url.push_str("&deleted_at=");
            url.push_str(&encode(deleted_at));
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_tombstones::refusal;
    use chrono::{DateTime, Utc};
    use pierre_database::repositories::{
        provider_identity_hash, subject_hash, ImportTombstone, TombstoneKind,
    };
    use uuid::Uuid;

    #[test]
    fn a_tombstone_refusal_returns_its_id_and_deletion_date() {
        let tombstone = ImportTombstone {
            id: "t-1".to_owned(),
            subject_hash: subject_hash(Uuid::new_v4()),
            kind: TombstoneKind::ProviderAccount,
            provider: Some("strava".to_owned()),
            identity_hash: provider_identity_hash("42"),
            created_at: DateTime::parse_from_rfc3339("2026-10-20T08:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            cleared_at: None,
            cleared_by: None,
            claimed_by: None,
            claimed_at: None,
        };
        let url = oauth_failure_url(
            "https://app.example/oauth-callback",
            "strava",
            &refusal(&tombstone),
        );
        assert!(
            url.ends_with(
                "error=import_tombstoned&tombstone_id=t-1&deleted_at=2026-10-20T08%3A00%3A00%2B00%3A00"
            ),
            "{url}"
        );
    }

    #[test]
    fn any_other_failure_carries_no_tombstone_params() {
        let url = oauth_failure_url(
            "https://app.example/oauth-callback",
            "strava",
            &AppError::internal("boom"),
        );
        assert!(!url.contains("tombstone_id"), "{url}");
        assert!(!url.contains("deleted_at"), "{url}");
    }
}
