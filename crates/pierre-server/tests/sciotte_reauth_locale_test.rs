// ABOUTME: Locale rendering for the provider re-auth message used by the chat pipeline auth_recovery stage
// ABOUTME: Pins FR + EN compiled-in copies so a regression at the messaging-strings level is caught early
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit-level coverage for `KEY_PROVIDER_REAUTH_REQUIRED`.
//!
//! The chat pipeline's `auth_recovery` stage short-circuits a turn with a
//! deterministic localized message when a tool emits `ProviderAuthRequired`.
//! These tests pin the substitution shape (`{0}` = display name, `{1}` =
//! URL) and the FR + EN compiled-in copies so a refactor of
//! `MessagingStringsRegistry` doesn't silently regress the user-visible
//! reply path.

use pierre_mcp_server::contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PROVIDER_REAUTH_REQUIRED,
};

const TEST_URL: &str = "https://api.dev.dravr.ai/providers/sciotte/login?token=stub.jwt.signature";

#[test]
fn provider_reauth_required_renders_fr_with_provider_and_url() {
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(
        KEY_PROVIDER_REAUTH_REQUIRED,
        "fr",
        &["Garmin Connect", TEST_URL],
    );

    assert!(
        rendered.contains("Garmin Connect"),
        "FR copy must include the provider display name in slot 0: {rendered}"
    );
    assert!(
        rendered.contains(TEST_URL),
        "FR copy must include the hosted-login URL in slot 1: {rendered}"
    );
    // Sanity on the user-facing French copy: the actionable verb pins the
    // tone the bot reaches for during a connection failure.
    assert!(
        rendered.contains("Reconnecte"),
        "FR copy should be imperative ('Reconnecte ...'): {rendered}"
    );
}

#[test]
fn provider_reauth_required_renders_en_with_provider_and_url() {
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(KEY_PROVIDER_REAUTH_REQUIRED, "en", &["Strava", TEST_URL]);

    assert!(
        rendered.contains("Strava"),
        "EN copy must include the provider display name in slot 0: {rendered}"
    );
    assert!(
        rendered.contains(TEST_URL),
        "EN copy must include the hosted-login URL in slot 1: {rendered}"
    );
    assert!(
        rendered.contains("Reconnect"),
        "EN copy should mention reconnection: {rendered}"
    );
}

#[test]
fn provider_reauth_required_falls_back_to_default_locale_for_unknown() {
    // ES isn't shipped for this key in the compiled-in defaults; the
    // registry's documented fallback chain returns the default-locale (FR)
    // copy so the user still gets a usable message instead of an empty
    // string. This pins that contract for the new key.
    let reg = MessagingStringsRegistry::new();
    let rendered = reg.render(
        KEY_PROVIDER_REAUTH_REQUIRED,
        "es",
        &["Garmin Connect", TEST_URL],
    );

    assert!(
        rendered.contains("Garmin Connect"),
        "fallback render must still substitute slot 0: {rendered}"
    );
    assert!(
        rendered.contains(TEST_URL),
        "fallback render must still substitute slot 1: {rendered}"
    );
}
