// ABOUTME: GET /r/{code} — resolve a short code and 302-redirect to its stored target URL
// ABOUTME: Public, unauthenticated redirect; the link-token JWT inside the target is the real gate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Short-link redirect endpoint.
//!
//! The chat reconnect/connect surfaces hand out `<base>/r/<code>` instead of the
//! raw `<base>/providers/...?token=<JWT>` URL, because the JWT's dots make
//! `WhatsApp` truncate linkification mid-token. This handler resolves the code back
//! to the full destination and bounces the browser there with a 303.
//!
//! The redirect is intentionally public: the recipient taps it in a chat client
//! before any auth round-trip, and the embedded link-token JWT (short-lived,
//! single-use nonce) is the authorization gate. Codes are high-entropy
//! (uuid-simple, 122 bits) so enumeration is infeasible.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use tracing::warn;

use crate::AuthRoutesContext;

/// Minimal expired/unknown-code page. No template engine, no token — a lapsed
/// short link just means the reconnect window closed; steer the user back to chat.
const SHORT_LINK_GONE_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>Link expired</title></head>\
<body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem;text-align:center\">\
<h1>This link has expired</h1>\
<p>Reconnect links are only valid for a short time. Please ask again in your chat to get a fresh link.</p>\
</body></html>";

/// `GET /r/{code}` — resolve a short code to its target URL and 303-redirect.
///
/// Returns `404` with a friendly page when the code is unknown or expired, and
/// `500` when the store read fails.
pub async fn handle_short_link_redirect(
    State(context): State<AuthRoutesContext>,
    Path(code): Path<String>,
) -> Response {
    match context.repos.short_links.resolve_short_link(&code).await {
        Ok(Some(target)) => Redirect::to(&target).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Html(SHORT_LINK_GONE_HTML)).into_response(),
        Err(e) => {
            warn!(error = %e, "short-link resolve failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(SHORT_LINK_GONE_HTML),
            )
                .into_response()
        }
    }
}
