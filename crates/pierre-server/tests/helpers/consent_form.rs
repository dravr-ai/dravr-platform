// ABOUTME: The consent screen as a browser reads it — its hidden fields, and the review digest an approval must echo
// ABOUTME: Renders the screen for a pending request and reads back what the form posts, so a test approves the screen it was shown
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::panic)]
// Every binary that loads `helpers` compiles this; only the OAuth suites call it.
#![allow(dead_code)]

//! An approval carries the review digest of the screen it was rendered under
//! (owl PDPP §3.3): a form a test builds by hand must read it off the
//! rendered screen, as a browser posts it.

use axum::Router;

use super::axum_test::AxumTestRequest;

/// The value of the hidden field `name` in `page`, unescaped of the
/// quote-free values the consent form carries; `None` when it has none.
pub(crate) fn hidden_field(page: &str, name: &str) -> Option<String> {
    page.split(&format!("name=\"{name}\" value=\""))
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .map(|value| value.replace("&quot;", "\"").replace("&amp;", "&"))
}

/// The field and the review digest a plain approval of `page` echoes:
/// `review_digest` on a plain screen or a request's own details, and
/// `review_digest_everything` on the grant picker, whose preselected
/// Everything is the plain grant.
pub(crate) fn review_digest(page: &str) -> (&'static str, String) {
    plain_digest(page)
        .unwrap_or_else(|| panic!("the consent screen carries its review digest: {page}"))
}

/// [`review_digest`], or `None` for a page that is no consent screen.
pub(crate) fn plain_digest(page: &str) -> Option<(&'static str, String)> {
    ["review_digest", "review_digest_everything"]
        .into_iter()
        .find_map(|field| hidden_field(page, field).map(|digest| (field, digest)))
}

/// The consent screen `cookie`'s athlete is shown for the authorization
/// request `query` (the query string after `/oauth2/authorize?`).
pub(crate) async fn consent_page(app: Router, cookie: &str, query: &str) -> String {
    let response = AxumTestRequest::get(&format!("/oauth2/authorize?{query}"))
        .header("cookie", cookie)
        .send(app)
        .await;
    assert_eq!(
        response.status(),
        200,
        "the consent screen is shown: {}",
        response.body_text()
    );
    response.body_text()
}

/// The query string of an authorization request carrying `pairs`, every
/// value URL-encoded.
pub(crate) fn authorize_query(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}
