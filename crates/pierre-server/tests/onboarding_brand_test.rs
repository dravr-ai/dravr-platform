// ABOUTME: Pins the user-facing onboarding copy to the Dravr brand, in every locale
// ABOUTME: Guards the regression where the OTP flow greeted users as "Pierre" in English
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Brand + locale invariants for the channel-linking onboarding flow.
//!
//! `Pierre` is the internal codename (crate names, module paths, the
//! `X-Pierre-Signature` header). It must never reach a user. Two regressions
//! motivated these tests, both in the flow a new Telegram/WhatsApp user walks
//! on their very first message:
//!
//! 1. The opening prompt of the in-chat OTP flow was a hardcoded English
//!    literal reading "To link your Pierre account …". It bypassed the string
//!    catalogue entirely, so the rebrand pass that fixed every neighbouring
//!    reply never saw it.
//! 2. Because it bypassed the catalogue it also bypassed locale resolution,
//!    so the first reply arrived in English while every following reply in the
//!    same conversation rendered in [`DEFAULT_LOCALE`] (French).
//!
//! The catalogue is the fix for both: routing the prompt through
//! [`KEY_LINK_EMAIL_PROMPT`] makes it localized *and* brand-correct.

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, DEFAULT_LOCALE, KEY_LINK_CANCELLED, KEY_LINK_EMAIL_NOT_CONFIGURED,
    KEY_LINK_EMAIL_PROMPT, KEY_LINK_EMAIL_SEND_FAILED, KEY_LINK_GENERIC_ERROR,
    KEY_LINK_IDENTITY_COLLISION, KEY_LINK_INCORRECT_CODE, KEY_LINK_INITIAL_PROMPT,
    KEY_LINK_INVALID_EMAIL, KEY_LINK_LOGOUT_COMPLETE, KEY_LINK_NO_TENANT, KEY_LINK_OTP_PROMPT,
    KEY_LINK_OTP_SENT, KEY_LINK_SESSION_EXPIRED, KEY_LINK_SIGNUP_CREATED, KEY_LINK_SIGNUP_FAILED,
    KEY_LINK_SIGNUP_OFFER, KEY_LINK_SUCCESS, KEY_LINK_TOO_MANY_ATTEMPTS,
    KEY_LINK_VERIFICATION_ERROR,
};
use pierre_email::templates::channel_linking_code_html;
use pierre_email::{
    SUBJECT_CHANNEL_LINKING_CODE, SUBJECT_PASSWORD_RESET, SUBJECT_REGISTRATION_APPROVED,
    SUBJECT_REGISTRATION_PENDING,
};

/// Every locale the compiled-in catalogue ships.
const LOCALES: [&str; 5] = ["fr", "en", "es", "de", "pt"];

/// Every string the channel-linking flow can send before the user is linked.
const LINK_FLOW_KEYS: [&str; 20] = [
    KEY_LINK_EMAIL_PROMPT,
    KEY_LINK_INITIAL_PROMPT,
    KEY_LINK_OTP_PROMPT,
    KEY_LINK_OTP_SENT,
    KEY_LINK_SUCCESS,
    KEY_LINK_CANCELLED,
    KEY_LINK_LOGOUT_COMPLETE,
    KEY_LINK_SIGNUP_OFFER,
    KEY_LINK_SIGNUP_CREATED,
    KEY_LINK_SIGNUP_FAILED,
    KEY_LINK_NO_TENANT,
    KEY_LINK_INVALID_EMAIL,
    KEY_LINK_INCORRECT_CODE,
    KEY_LINK_TOO_MANY_ATTEMPTS,
    KEY_LINK_SESSION_EXPIRED,
    KEY_LINK_IDENTITY_COLLISION,
    KEY_LINK_VERIFICATION_ERROR,
    KEY_LINK_GENERIC_ERROR,
    KEY_LINK_EMAIL_NOT_CONFIGURED,
    KEY_LINK_EMAIL_SEND_FAILED,
];

/// The opening OTP prompt exists, is Dravr-branded, and keeps its escape hatch.
///
/// Asserts concrete content in all five locales — a registry that returned an
/// empty string or fell through to a missing key would fail here.
#[test]
fn test_link_email_prompt_is_dravr_branded_in_every_locale() {
    let reg = MessagingStringsRegistry::new();

    for locale in LOCALES {
        let prompt = reg.get(KEY_LINK_EMAIL_PROMPT, locale);

        assert!(
            prompt.contains("Dravr"),
            "[{locale}] the opening OTP prompt must name the product: {prompt:?}"
        );
        assert!(
            !prompt.contains("Pierre"),
            "[{locale}] internal codename leaked to a user-facing prompt: {prompt:?}"
        );
        assert!(
            prompt.to_lowercase().contains("cancel"),
            "[{locale}] the prompt must tell the user how to abort the flow: {prompt:?}"
        );
        assert!(
            prompt.len() > 40,
            "[{locale}] prompt looks truncated or unresolved: {prompt:?}"
        );
    }
}

/// The prompt a user actually receives is French, not English.
///
/// The flow runs before the user is linked, so there is no `users.locale` to
/// consult and every reply resolves through [`DEFAULT_LOCALE`]. This pins the
/// half of the regression that made the first reply English and the second
/// French inside one conversation.
#[test]
fn test_link_email_prompt_default_locale_matches_flow_language() {
    let reg = MessagingStringsRegistry::new();

    let default_prompt = reg.get(KEY_LINK_EMAIL_PROMPT, DEFAULT_LOCALE);
    let french_prompt = reg.get(KEY_LINK_EMAIL_PROMPT, "fr");
    let english_prompt = reg.get(KEY_LINK_EMAIL_PROMPT, "en");

    assert_eq!(
        default_prompt, french_prompt,
        "the default locale must resolve to the French copy"
    );
    assert_ne!(
        default_prompt, english_prompt,
        "the default-locale prompt must not be the English copy — that was the original bug"
    );

    // The prompt must speak the same language as the reply that follows it.
    let next_reply = reg.get(KEY_LINK_SIGNUP_OFFER, DEFAULT_LOCALE);
    assert!(
        default_prompt.contains("ton") && next_reply.contains("compte"),
        "prompt {default_prompt:?} and follow-up {next_reply:?} must both be French"
    );
}

/// No reply in the whole pre-link flow leaks the internal codename.
///
/// Broader than the single fixed string: any future addition to the linking
/// catalogue is covered the moment it lands in [`LINK_FLOW_KEYS`].
#[test]
fn test_no_link_flow_string_leaks_internal_codename() {
    let reg = MessagingStringsRegistry::new();

    for key in LINK_FLOW_KEYS {
        for locale in LOCALES {
            let value = reg.get(key, locale);
            assert!(
                !value.is_empty(),
                "[{locale}] {key} resolved to an empty string"
            );
            assert!(
                !value.contains("Pierre"),
                "[{locale}] {key} leaks the internal codename: {value:?}"
            );
        }
    }
}

/// Transactional email subjects carry the product name, not the codename.
///
/// The channel-linking subject is the one that regressed: its HTML body was
/// rebranded (`<title>Your Dravr verification code</title>`) while the subject
/// introducing that body still said otherwise.
#[test]
fn test_email_subjects_are_dravr_branded() {
    assert_eq!(
        SUBJECT_CHANNEL_LINKING_CODE, "Your Dravr verification code",
        "the OTP email subject must match the Dravr-branded body it introduces"
    );

    for subject in [
        SUBJECT_CHANNEL_LINKING_CODE,
        SUBJECT_REGISTRATION_PENDING,
        SUBJECT_REGISTRATION_APPROVED,
        SUBJECT_PASSWORD_RESET,
    ] {
        assert!(
            !subject.contains("Pierre"),
            "email subject leaks the internal codename: {subject:?}"
        );
    }
}

/// The rendered OTP email body stays Dravr-branded end to end.
#[test]
fn test_channel_linking_email_body_is_dravr_branded() {
    let html = channel_linking_code_html("123456", "Telegram");

    assert!(
        html.contains("Dravr"),
        "the verification email body must name the product"
    );
    assert!(
        !html.contains("Pierre"),
        "the verification email body leaks the internal codename"
    );
    assert!(
        html.contains("123456"),
        "the verification email must contain the code it was built with"
    );
    assert!(
        html.contains("Telegram"),
        "the verification email must name the channel being linked"
    );
}
