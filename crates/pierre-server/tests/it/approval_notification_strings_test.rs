// ABOUTME: Verifies the account-approved messaging string resolves in every shipped locale.
// ABOUTME: Guards the KEY_REGISTRATION_APPROVED registration + its 5 compiled-in defaults.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Account-approved i18n string coverage.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_REGISTRATION_APPROVED};

/// Every shipped locale renders a non-empty, brand-bearing approval message.
#[test]
fn registration_approved_renders_in_all_locales() {
    let registry = MessagingStringsRegistry::new();
    for locale in ["fr", "en", "es", "de", "pt"] {
        let message = registry.render(KEY_REGISTRATION_APPROVED, locale, &[]);
        assert!(
            !message.trim().is_empty(),
            "approval message is empty for locale {locale}"
        );
        assert!(
            message.contains("Dravr"),
            "approval message for {locale} should name the brand: {message}"
        );
    }
}

/// Locales are distinct (the table isn't accidentally aliasing one default).
#[test]
fn registration_approved_locales_differ() {
    let registry = MessagingStringsRegistry::new();
    let fr = registry.render(KEY_REGISTRATION_APPROVED, "fr", &[]);
    let en = registry.render(KEY_REGISTRATION_APPROVED, "en", &[]);
    assert_ne!(fr, en, "fr and en approval messages should differ");
}

/// An unknown locale falls back to the default rather than returning empty.
#[test]
fn registration_approved_unknown_locale_falls_back() {
    let registry = MessagingStringsRegistry::new();
    let fallback = registry.render(KEY_REGISTRATION_APPROVED, "zz", &[]);
    assert!(
        !fallback.trim().is_empty(),
        "unknown locale should fall back to a non-empty default"
    );
}
