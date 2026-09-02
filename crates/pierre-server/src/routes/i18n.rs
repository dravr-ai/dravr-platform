// ABOUTME: GET /api/i18n/{locale} — the live string catalogue for one locale, as the clients read it
// ABOUTME: Unauthenticated and ETag-revalidated, so a contremaitre overlay reaches a phone on its next open
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The client-facing catalogue.
//!
//! Every user-facing string — a messaging reply the server renders and a
//! label the web or mobile chrome renders alike — lives in one catalogue: the
//! five `translation.json` files the [`MessagingStringsRegistry`] seeds from
//! and both clients embed at build time. The embedded copy is a snapshot of
//! the tree that was built; the registry is live, because the contremaitre
//! sync overlays it whenever a string changes upstream. This route is how a
//! client sees the live version: one request per locale, the fallback chain
//! already resolved so no key is ever empty, and a strong `ETag` so the
//! second request of the day is a 304 with no body.
//!
//! Unauthenticated on purpose. The strings are the product's own copy, the
//! same bytes for every caller, and a client fetches them before anyone has
//! logged in.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use pierre_contremaitre::manifest::compute_sha256;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::SUPPORTED_LOCALES;
use serde::{Deserialize, Serialize};

/// One locale's catalogue, every key resolved.
#[derive(Debug, Serialize, Deserialize)]
pub struct I18nBundleResponse {
    /// The locale the strings were resolved for.
    pub locale: String,
    /// Digest of the strings, the value the `ETag` header carries (unquoted).
    pub etag: String,
    /// Every catalogue key with its text in `locale`, or in the default locale
    /// where `locale` has no entry — never empty for a key the registry holds.
    pub strings: BTreeMap<String, String>,
}

/// Router for the catalogue.
pub struct I18nRoutes;

impl I18nRoutes {
    /// Mount `GET /api/i18n/{locale}` over the live registry.
    ///
    /// Takes the registry rather than the whole server context: the route
    /// reads strings and nothing else, and a test can stand it up on a bare
    /// [`MessagingStringsRegistry`] without a database.
    pub fn routes(registry: Arc<MessagingStringsRegistry>) -> Router {
        Router::new()
            .route("/api/i18n/{locale}", get(bundle))
            .with_state(registry)
    }
}

/// Resolve every key the registry holds for `locale`, through the registry's
/// own fallback chain, in key order.
///
/// Key order is what makes the digest stable: two requests against the same
/// catalogue hash to the same `ETag`, whichever thread answers them.
#[must_use]
pub fn resolve_bundle(
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> BTreeMap<String, String> {
    registry
        .list()
        .into_iter()
        .map(|(key, _, _)| key)
        .collect::<BTreeSet<String>>()
        .into_iter()
        .map(|key| {
            let value = registry.get(&key, locale);
            (key, value)
        })
        .collect()
}

/// Digest of a resolved bundle: every key and value, in order, so the `ETag`
/// changes exactly when a string does.
#[must_use]
pub fn bundle_digest(strings: &BTreeMap<String, String>) -> String {
    let mut bytes = Vec::with_capacity(strings.len() * 64);
    for (key, value) in strings {
        bytes.extend_from_slice(key.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(b'\n');
    }
    compute_sha256(&bytes)
}

/// Serve one locale's catalogue, or a 304 when the caller already has it.
async fn bundle(
    State(registry): State<Arc<MessagingStringsRegistry>>,
    Path(locale): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !SUPPORTED_LOCALES.contains(&locale.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let strings = resolve_bundle(&registry, &locale);
    let digest = bundle_digest(&strings);
    let etag = format!("\"{digest}\"");
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == etag);
    if unchanged {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    (
        StatusCode::OK,
        [
            (header::ETAG, etag),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
        ],
        Json(I18nBundleResponse {
            locale,
            etag: digest,
            strings,
        }),
    )
        .into_response()
}
