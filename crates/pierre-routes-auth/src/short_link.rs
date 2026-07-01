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
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use tracing::warn;

use crate::AuthRoutesContext;

/// Pick the page locale from the browser's `Accept-Language` header.
///
/// The redirect is unauthenticated so there is no session locale to read; the
/// primary subtag of the first accepted language is mapped to one of the five
/// supported locales, defaulting to French (the platform is French-first) for
/// `fr` and anything unrecognized.
fn preferred_locale(headers: &HeaderMap) -> &'static str {
    let primary = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .split(['-', ';'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match primary.as_str() {
        "en" => "en",
        "es" => "es",
        "de" => "de",
        "pt" => "pt",
        _ => "fr",
    }
}

/// Render the localized expired/unknown-code page. No template engine, no token
/// — a lapsed short link just means the reconnect window closed; steer the user
/// back to chat in their own language.
fn short_link_gone_html(locale: &str) -> String {
    let (title, body) = match locale {
        "en" => (
            "This link has expired",
            "Reconnect links are only valid for a short time. Please ask again in your chat to get a fresh link.",
        ),
        "es" => (
            "Este enlace ha caducado",
            "Los enlaces de reconexión solo son válidos durante un breve periodo. Pídelo de nuevo en tu chat para obtener un enlace nuevo.",
        ),
        "de" => (
            "Dieser Link ist abgelaufen",
            "Reconnect-Links sind nur kurze Zeit gültig. Bitte fordere in deinem Chat einen neuen Link an.",
        ),
        "pt" => (
            "Este link expirou",
            "Os links de reconexão são válidos apenas por pouco tempo. Peça novamente no seu chat para receber um novo link.",
        ),
        // French-first default (covers "fr" and any unrecognized locale).
        _ => (
            "Ce lien a expiré",
            "Les liens de reconnexion ne sont valides que peu de temps. Redemandez-en un dans votre conversation pour obtenir un nouveau lien.",
        ),
    };
    format!(
        "<!doctype html><html lang=\"{locale}\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title></head>\
<body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem;text-align:center\">\
<h1>{title}</h1>\
<p>{body}</p>\
</body></html>"
    )
}

/// `GET /r/{code}` — resolve a short code to its target URL and 303-redirect.
///
/// Returns `404` with a friendly page when the code is unknown or expired, and
/// `500` when the store read fails.
pub async fn handle_short_link_redirect(
    State(context): State<AuthRoutesContext>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Response {
    match context.repos.short_links.resolve_short_link(&code).await {
        Ok(Some(target)) => Redirect::to(&target).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Html(short_link_gone_html(preferred_locale(&headers))),
        )
            .into_response(),
        Err(e) => {
            warn!(error = %e, "short-link resolve failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(short_link_gone_html(preferred_locale(&headers))),
            )
                .into_response()
        }
    }
}
