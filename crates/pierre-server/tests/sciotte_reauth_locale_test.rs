// ABOUTME: Locale rendering for the provider re-auth message used by the chat pipeline auth_recovery stage
// ABOUTME: Pins FR + EN compiled-in copies so a regression at the messaging-strings level is caught early
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Unit-level coverage for `KEY_PROVIDER_REAUTH_REQUIRED`.
//!
//! The chat pipeline's `auth_recovery` stage short-circuits a turn with a
//! deterministic localized message when a tool emits `ProviderAuthRequired`.
//! These tests pin the substitution shape (`{0}` = display name, `{1}` =
//! URL) and the FR + EN compiled-in copies so a refactor of
//! `MessagingStringsRegistry` doesn't silently regress the user-visible
//! reply path.

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PROVIDER_REAUTH_REQUIRED, KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
};
use std::fs;
use std::path::PathBuf;

// A host that does NOT resolve, on purpose: this asserts on rendered locale strings
// and must never reach the network, so an unresolvable fixture makes an accidental
// request fail loudly rather than quietly succeed. Do not 'fix' it to a live URL.
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

// ============================================================================
// carnet#108 — a failed mint still names the provider
// ============================================================================

/// The link-less variant carries the provider and no URL slot.
///
/// `auth_recovery` used to `?` out when `mint_reconnect_url` returned `None`,
/// which left the turn with no content at all — the athlete was told « je n'ai
/// pas réussi à formuler une réponse » while `finish_reason` on that same turn
/// read `provider_auth_required`. The corpus caught it as an empty turn; the
/// diagnostic that named it was the finish reason.
#[test]
fn the_link_less_reauth_copy_names_the_provider_in_every_locale() {
    let reg = MessagingStringsRegistry::new();
    for (locale, verb) in [
        ("fr", "Reconnecte"),
        ("en", "Reconnect"),
        ("es", "Vuelve a conectar"),
        ("de", "neu"),
        ("pt", "Liga novamente"),
    ] {
        let rendered = reg.render(
            KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
            locale,
            &["Garmin Connect"],
        );
        assert!(
            rendered.contains("Garmin Connect"),
            "{locale} copy must name the provider: {rendered}"
        );
        assert!(
            rendered.contains(verb),
            "{locale} copy must tell the athlete what to do: {rendered}"
        );
        // The whole point of this variant: no dangling placeholder where a URL
        // would have been, and no empty link for a surface to draw a control on.
        assert!(
            !rendered.contains("{1}"),
            "{locale} copy must not leave an unsubstituted URL slot: {rendered}"
        );
        assert!(
            !rendered.contains("http"),
            "{locale} copy must carry no URL: {rendered}"
        );
    }
}

/// It must be a different sentence from the linked one, not a copy of it.
#[test]
fn the_link_less_copy_is_not_the_linked_copy() {
    let reg = MessagingStringsRegistry::new();
    let linked = reg.render(KEY_PROVIDER_REAUTH_REQUIRED, "fr", &["Strava", TEST_URL]);
    let bare = reg.render(KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK, "fr", &["Strava"]);
    assert_ne!(
        linked, bare,
        "the two variants must say different things — one offers a link, one cannot"
    );
    assert!(
        bare.contains("Strava") && !bare.contains("http"),
        "the bare variant keeps the provider and drops the link: {bare}"
    );
}

/// The failed-mint path must not go back to `?`.
///
/// `apply_auth_recovery` needs a live `ChatPipelineContext` (DB, registries,
/// mint endpoint), so an integration test cannot drive the branch. Same
/// situation as `identity_anchor_reaches_the_wire_test`, and the same answer:
/// attack the seam that IS reachable, because the regression guarded against is
/// a source edit back to a one-character early return.
#[test]
fn a_failed_mint_does_not_return_early() {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../pierre-chat-pipeline/src/stages/auth_recovery.rs"),
    )
    .expect("read auth_recovery.rs");

    assert!(
        !source
            .contains("mint_reconnect_url(&deps, &provider_slug, user_id, input, profile).await?"),
        "`?` on the mint drops the turn's only content: the athlete is told the coach \
         could not formulate a response while finish_reason reads provider_auth_required"
    );
    assert!(
        source.contains("KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK"),
        "the failed-mint path must still tell the athlete which provider dropped"
    );
    // And the message has to actually be delivered, not just built.
    let bare_arm = source
        .split("KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK")
        .nth(1)
        .expect("checked above");
    assert!(
        bare_arm.contains("result.content"),
        "the link-less message must be written to result.content, or the turn is still empty"
    );
}
