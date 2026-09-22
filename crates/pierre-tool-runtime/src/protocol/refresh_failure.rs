// ABOUTME: Classifies a failed OAuth token refresh as a refusal of the grant or client, or as transient
// ABOUTME: Reads the HTTP status first, then Strava's error resource, then the RFC 6749 codes and auth markers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde_json::Value as JsonValue;

/// Classify a token-refresh failure as either "the user must re-authorize" or transient.
///
/// Returns `Some(error_code)` when the provider's token endpoint definitively rejected the
/// refresh — a dead/rotated refresh token, a revoked grant, or a rejected client. These do
/// not recover on retry: the connection must be flipped to `needs_reauth` and the user
/// prompted to reconnect. Returns `None` for transient failures (network, timeout, 5xx),
/// which must NOT disconnect a healthy connection — a flake is not a disconnect.
///
/// Conservative by design: transient transport/server signatures are checked first and
/// short-circuit to `None`, so an auth marker that merely appears in a 5xx body can't
/// trip a false disconnect. WHOOP returns `invalid_request` for a consumed/rotated refresh
/// token; RFC 6749's code for a dead refresh token is `invalid_grant`.
///
/// The HTTP status is read first. A 429 (rate limited) or any 5xx is the provider saying
/// "not now", whatever its body names: Strava's documented rate-limit answer is HTTP 429
/// with `{"errors":[{"resource":"Application","field":"rate limit","code":"exceeded"}]}`,
/// which names the application without rejecting it.
///
/// Strava names no RFC 6749 code: it answers a dead refresh token with HTTP 400 and
/// `{"errors":[{"resource":"RefreshToken","field":"refresh_token","code":"invalid"}]}`,
/// and a rejected client with the same shape naming `"resource":"Application"` with
/// `"code":"invalid"`. Those are read by the resource and code they name, before the
/// generic markers, so a dead Strava grant is recorded as `invalid_grant` (its seat goes
/// back to the pool) and a rejected client as `invalid_client` (the grant stays live and
/// keeps its seat).
pub fn classify_refresh_failure(reason: &str) -> Option<&'static str> {
    // Transient transport failures carry no status and recover on retry.
    const TRANSIENT_MARKERS: [&str; 4] = ["timed out", "timeout", "connection", "dns"];
    // Definitive OAuth-layer rejections of the refresh grant or client → require reconnect.
    const REAUTH_MARKERS: [&str; 8] = [
        "invalid_grant",
        "invalid_request",
        "invalid_client",
        "unauthorized_client",
        "unauthorized",
        "forbidden",
        "http 401",
        "http 403",
    ];

    let r = reason.to_ascii_lowercase();
    if http_status_in(&r).is_some_and(|status| status == 429 || (500..600).contains(&status)) {
        return None;
    }
    if TRANSIENT_MARKERS.into_iter().any(|m| r.contains(m)) {
        return None;
    }
    if let Some(code) = strava_rejection(&r) {
        return Some(code);
    }
    REAUTH_MARKERS.into_iter().find(|m| r.contains(*m))
}

/// The HTTP status a lowercased refresh failure reports, as the token clients
/// write it (`... returned http 429 too many requests: {body}`).
fn http_status_in(reason: &str) -> Option<u16> {
    reason.match_indices("http ").find_map(|(at, marker)| {
        let digits = reason.get(at + marker.len()..at + marker.len() + 3)?;
        if digits.bytes().all(|b| b.is_ascii_digit()) {
            digits.parse().ok()
        } else {
            None
        }
    })
}

/// The refresh rejection a Strava error body names, read from its `errors`
/// array: a `RefreshToken` resource is a dead grant (`invalid_grant`), an
/// `Application` resource refused as `invalid` is our own client
/// (`invalid_client`). Any other body names neither.
fn strava_rejection(reason: &str) -> Option<&'static str> {
    let body = reason.get(reason.find('{')?..)?;
    let errors: JsonValue = serde_json::Deserializer::from_str(body)
        .into_iter::<JsonValue>()
        .next()?
        .ok()?;
    errors.get("errors")?.as_array()?.iter().find_map(|error| {
        let resource = error.get("resource")?.as_str()?;
        let code = error.get("code").and_then(JsonValue::as_str);
        match resource {
            "refreshtoken" => Some("invalid_grant"),
            "application" if code == Some("invalid") => Some("invalid_client"),
            _ => None,
        }
    })
}
