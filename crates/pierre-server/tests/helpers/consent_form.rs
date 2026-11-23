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

/// The review digest a plain consent screen, or a request's own details,
/// was rendered under: the `review_digest` an approval of it echoes.
pub(crate) fn review_digest(page: &str) -> String {
    hidden_field(page, "review_digest")
        .unwrap_or_else(|| panic!("the consent screen carries its review digest: {page}"))
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

/// The proposed English of the grant picker's and the requested details'
/// lines, not yet in the catalogue: what dravr-contremaitre is asked to add
/// (owl Phase 5, slice 3a). Until the pin carries them the screen shows
/// neither, so these suites serve it from a catalogue carrying them.
pub(crate) const PROPOSED_GRANT_LINES: [(&str, &str); 13] = [
    ("oauth.consent.grantChoiceTitle", "Choose what {0} can read"),
    ("oauth.consent.grantEverything", "Everything, as today"),
    (
        "oauth.consent.grantEverythingHint",
        "Every tool your account offers, with no end date. You can remove it in Connected apps.",
    ),
    ("oauth.consent.grantTrainingCoach", "Training coach"),
    (
        "oauth.consent.grantTrainingCoachHint",
        "Your workouts with their sensor samples, daily activity, training load, sleep, heart and recovery summaries, body measurements and profile, from the last {0} days. No routes, notes or health records.",
    ),
    (
        "oauth.consent.grantTrainingCoachSummaryHint",
        "Your workout summaries, daily activity, training load, sleep, heart and recovery summaries, body measurements and profile, from the last {0} days. No sensor samples, routes, notes or health records.",
    ),
    ("oauth.consent.grantSleepCoach", "Sleep coach"),
    (
        "oauth.consent.grantSleepCoachHint",
        "Your sleep, heart and recovery summaries, daily activity and profile, from the last {0} days. No workouts, routes, notes or health records.",
    ),
    (
        "oauth.consent.grantLastsDays",
        "Access ends {0} days after you allow it.",
    ),
    (
        "oauth.consent.grantLastsHours",
        "Access ends {0} hours after you allow it.",
    ),
    ("oauth.consent.grantRequested", "{0} asks only for:"),
    ("oauth.consent.grantPurpose", "In {0}’s own words: “{1}”"),
    ("oauth.consent.grantCommitments", "{0} says it will:"),
];

/// Give `registry` the picker's proposed English lines, as a runtime sync
/// would deliver them once dravr-contremaitre carries them.
pub(crate) fn with_grant_lines(registry: &pierre_contremaitre::MessagingStringsRegistry) {
    for (key, line) in PROPOSED_GRANT_LINES {
        registry.update(key, "en", line.to_owned(), format!("proposed-{key}"));
    }
}
