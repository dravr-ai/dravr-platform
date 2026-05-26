// ABOUTME: Locale rendering for the no-provider denial message surfaced by the messaging onboarding gate
// ABOUTME: Pins FR/EN/ES/DE/PT compiled-in copies so the {0} URL substitution never regresses to a literal placeholder
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit-level coverage for `KEY_NO_PROVIDER_CONNECTED`.
//!
//! The messaging ingress onboarding gate short-circuits a turn with a
//! deterministic localized denial when the authenticated user has zero
//! connected fitness providers. These tests pin the substitution shape
//! (`{0}` = dravr web connect URL) and the FR/EN/ES/DE/PT compiled-in
//! copies so a refactor of `MessagingStringsRegistry` or the denial
//! renderer doesn't silently regress the user-visible reply path —
//! a regression would leak literal `{0}` to chat users.

use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_NO_PROVIDER_CONNECTED};

const TEST_URL: &str = "https://app.dravr.ai/providers";

#[test]
fn no_provider_connected_renders_fr_with_url() {
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(KEY_NO_PROVIDER_CONNECTED, "fr", &[TEST_URL]);

    assert!(
        rendered.contains(TEST_URL),
        "FR copy must include the connect URL in slot 0: {rendered}"
    );
    assert!(
        !rendered.contains("{0}"),
        "FR render must substitute the {{0}} placeholder, not leak it: {rendered}"
    );
    // Provider list trimmed to the production set — no Fitbit/Terra leak.
    assert!(
        rendered.contains("Strava") && rendered.contains("Garmin") && rendered.contains("Whoop"),
        "FR copy must list Strava/Garmin/Whoop: {rendered}"
    );
    assert!(
        !rendered.contains("Fitbit"),
        "FR copy must not mention Fitbit (removed in provider cleanup): {rendered}"
    );
}

#[test]
fn no_provider_connected_renders_en_with_url() {
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(KEY_NO_PROVIDER_CONNECTED, "en", &[TEST_URL]);

    assert!(
        rendered.contains(TEST_URL),
        "EN copy must include the connect URL in slot 0: {rendered}"
    );
    assert!(
        !rendered.contains("{0}"),
        "EN render must substitute the {{0}} placeholder, not leak it: {rendered}"
    );
    assert!(
        rendered.contains("Strava") && rendered.contains("Garmin") && rendered.contains("Whoop"),
        "EN copy must list Strava/Garmin/Whoop: {rendered}"
    );
}

#[test]
fn no_provider_connected_renders_es_with_url() {
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(KEY_NO_PROVIDER_CONNECTED, "es", &[TEST_URL]);

    assert!(
        rendered.contains(TEST_URL),
        "ES copy must include the connect URL in slot 0: {rendered}"
    );
    assert!(
        !rendered.contains("{0}"),
        "ES placeholder leaked: {rendered}"
    );
}

#[test]
fn no_provider_connected_renders_de_with_url() {
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(KEY_NO_PROVIDER_CONNECTED, "de", &[TEST_URL]);

    assert!(
        rendered.contains(TEST_URL),
        "DE copy must include the connect URL in slot 0: {rendered}"
    );
    assert!(
        !rendered.contains("{0}"),
        "DE placeholder leaked: {rendered}"
    );
}

#[test]
fn no_provider_connected_renders_pt_with_url() {
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(KEY_NO_PROVIDER_CONNECTED, "pt", &[TEST_URL]);

    assert!(
        rendered.contains(TEST_URL),
        "PT copy must include the connect URL in slot 0: {rendered}"
    );
    assert!(
        !rendered.contains("{0}"),
        "PT placeholder leaked: {rendered}"
    );
}

#[test]
fn no_provider_connected_falls_back_to_default_locale_for_unknown() {
    // Unknown locale (e.g. "it") falls back to the default-locale (FR) copy.
    // Pinning this contract for the new placeholder shape.
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(KEY_NO_PROVIDER_CONNECTED, "it", &[TEST_URL]);

    assert!(
        rendered.contains(TEST_URL),
        "fallback render must still substitute the URL: {rendered}"
    );
    assert!(
        !rendered.contains("{0}"),
        "fallback render must not leak {{0}}: {rendered}"
    );
}
