// ABOUTME: Integration test for GET /api/i18n/{locale} — the live catalogue as the clients read it
// ABOUTME: Asserts real French copy, the ETag round trip, and that an overlay shows on the next request
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The catalogue route.
//!
//! A client overlays what this route serves on the copy it embeds, so the
//! assertions are on real values — a known French sentence, every key
//! non-empty, a digest that moves when a string does — never on the request
//! merely succeeding. A route that answered `{"strings": {}}` would satisfy a
//! status test and leave every screen on its embedded copy.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request as HttpRequest, StatusCode};
use axum::response::Response;
use pierre_contremaitre::manifest::compute_sha256;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_EMPTY_REPLY, KEY_HELP_FOOTER,
};
use pierre_core::models::SUPPORTED_LOCALES;
use pierre_mcp_server::routes::i18n::{I18nBundleResponse, I18nRoutes};
use tower::ServiceExt;

/// Body ceiling for `to_bytes`; a locale's catalogue is a few hundred KB.
const BODY_LIMIT: usize = 4 * 1024 * 1024;

async fn get(
    registry: &Arc<MessagingStringsRegistry>,
    locale: &str,
    etag: Option<&str>,
) -> Response {
    let mut request = HttpRequest::builder().uri(format!("/api/i18n/{locale}"));
    if let Some(etag) = etag {
        request = request.header(header::IF_NONE_MATCH, etag);
    }
    I18nRoutes::routes(Arc::clone(registry))
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn bundle(response: Response) -> I18nBundleResponse {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn french_bundle_carries_the_registry_word_for_word() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let response = get(&registry, "fr", None).await;
    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("an ETag header")
        .to_str()
        .unwrap()
        .to_owned();
    let body = bundle(response).await;

    assert_eq!(body.locale, "fr");
    assert_eq!(
        format!("\"{}\"", body.etag),
        etag,
        "the body digest is the ETag"
    );
    assert_eq!(
        body.strings.len(),
        registry.key_count(),
        "every catalogue key is served"
    );
    assert_eq!(
        body.strings[KEY_EMPTY_REPLY],
        registry.get(KEY_EMPTY_REPLY, "fr")
    );
    assert!(body.strings[KEY_EMPTY_REPLY].contains("réponse"));
    let empty: Vec<&String> = body
        .strings
        .iter()
        .filter(|(_, value)| value.trim().is_empty())
        .map(|(key, _)| key)
        .collect();
    assert!(empty.is_empty(), "keys served empty: {empty:?}");
}

#[tokio::test]
async fn every_locale_answers_and_differs_from_the_others() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let mut footers = Vec::new();
    for locale in SUPPORTED_LOCALES {
        let body = bundle(get(&registry, locale, None).await).await;
        assert_eq!(body.locale, locale);
        footers.push(body.strings[KEY_HELP_FOOTER].clone());
    }
    footers.sort();
    footers.dedup();
    assert_eq!(
        footers.len(),
        SUPPORTED_LOCALES.len(),
        "a locale that fell back to French would repeat another's text"
    );
}

#[tokio::test]
async fn a_matching_etag_is_a_304_with_no_body() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let first = get(&registry, "en", None).await;
    let etag = first.headers()[header::ETAG].to_str().unwrap().to_owned();

    let second = get(&registry, "en", Some(&etag)).await;
    assert_eq!(second.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(second.headers()[header::ETAG].to_str().unwrap(), etag);
    let bytes = to_bytes(second.into_body(), BODY_LIMIT).await.unwrap();
    assert!(bytes.is_empty());

    let stale = get(&registry, "en", Some("\"not-the-digest\"")).await;
    assert_eq!(stale.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_overlay_is_visible_on_the_next_request() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let before = get(&registry, "fr", None).await;
    let etag_before = before.headers()[header::ETAG].to_str().unwrap().to_owned();

    let replacement = "Je n'ai pas réussi à formuler une réponse.".to_owned();
    registry.update(
        KEY_EMPTY_REPLY,
        "fr",
        replacement.clone(),
        compute_sha256(replacement.as_bytes()),
    );

    let after = get(&registry, "fr", Some(&etag_before)).await;
    assert_eq!(
        after.status(),
        StatusCode::OK,
        "the old ETag no longer matches once a string changed"
    );
    let body = bundle(after).await;
    assert_eq!(body.strings[KEY_EMPTY_REPLY], replacement);
}

#[tokio::test]
async fn an_unknown_locale_is_not_found() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    assert_eq!(
        get(&registry, "zz", None).await.status(),
        StatusCode::NOT_FOUND
    );
}
