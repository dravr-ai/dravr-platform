// ABOUTME: The consent flow's text in the reader's language: stored locale, then the browser's, then English
// ABOUTME: Escapes each catalogue line before filling its slots, and words each refusal page from the OAuth error
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use axum::http::{header, HeaderMap};
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_contremaitre::consent_strings::{
    KEY_CONSENT_SCOPE_FITNESS_READ, KEY_CONSENT_SCOPE_FITNESS_WRITE,
    KEY_CONSENT_SCOPE_OFFLINE_ACCESS, KEY_CONSENT_SCOPE_PROFILE_READ,
    KEY_CONSENT_SCOPE_PROFILE_WRITE, KEY_ERROR_ACCOUNT_GONE, KEY_ERROR_ACCOUNT_UNAVAILABLE,
    KEY_ERROR_AWAITS_APPROVAL, KEY_ERROR_AWAITS_EMAIL_PROOF, KEY_ERROR_CONSENT_EXPIRED,
    KEY_ERROR_CONSENT_UNAVAILABLE, KEY_ERROR_DENIED, KEY_ERROR_GOOGLE_EXPIRED,
    KEY_ERROR_GOOGLE_REFUSED, KEY_ERROR_INVALID_CLIENT, KEY_ERROR_INVALID_REQUEST,
    KEY_ERROR_INVALID_REQUEST_URI, KEY_ERROR_LIMITER_UNAVAILABLE, KEY_ERROR_RATE_LIMITED,
    KEY_ERROR_SERVER, KEY_ERROR_SIGNUP_PAUSED, KEY_ERROR_SUSPENDED, KEY_ERROR_TRY_AGAIN,
    KEY_ERROR_UNAVAILABLE, PRESET_KEY_SCOPE_FITNESS_READ,
};
use pierre_contremaitre::messaging_strings::format_template;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::html::escape_html_attribute;
use pierre_core::models::{supported_ui_locale, SUPPORTED_LOCALES};
use pierre_services::signup_policy::SIGNUP_PAUSED;

use super::consent::{
    ACCOUNT_GONE, ACCOUNT_UNAVAILABLE, AWAITS_APPROVAL, AWAITS_EMAIL_PROOF, CONSENT_EXPIRED,
    CONSENT_UNAVAILABLE, SUSPENDED,
};
use super::google_login::{SIGN_IN_REFUSED, TRANSACTION_REFUSED};
use super::par::REQUEST_URI_REFUSED;
use crate::oauth2_rate_limited::{LIMITER_UNAVAILABLE, RATE_LIMIT_EXCEEDED};

/// The language of last resort on these pages. Consent has to be read to be
/// given, and an unknown reader is likelier to read English than the
/// platform's default.
const FALLBACK_LOCALE: &str = "en";

/// The refusals these pages word exactly, by the description the error
/// carries; each key's English is that description.
const WORDED_REFUSALS: [(&str, &str); 14] = [
    (AWAITS_EMAIL_PROOF, KEY_ERROR_AWAITS_EMAIL_PROOF),
    (AWAITS_APPROVAL, KEY_ERROR_AWAITS_APPROVAL),
    (SUSPENDED, KEY_ERROR_SUSPENDED),
    (ACCOUNT_GONE, KEY_ERROR_ACCOUNT_GONE),
    (ACCOUNT_UNAVAILABLE, KEY_ERROR_ACCOUNT_UNAVAILABLE),
    (CONSENT_EXPIRED, KEY_ERROR_CONSENT_EXPIRED),
    (CONSENT_UNAVAILABLE, KEY_ERROR_CONSENT_UNAVAILABLE),
    (RATE_LIMIT_EXCEEDED, KEY_ERROR_RATE_LIMITED),
    (LIMITER_UNAVAILABLE, KEY_ERROR_LIMITER_UNAVAILABLE),
    (SIGN_IN_REFUSED, KEY_ERROR_GOOGLE_REFUSED),
    (TRANSACTION_REFUSED, KEY_ERROR_GOOGLE_EXPIRED),
    (SIGNUP_PAUSED, KEY_ERROR_SIGNUP_PAUSED),
    (REQUEST_URI_REFUSED, KEY_ERROR_INVALID_REQUEST_URI),
    ("Please try again.", KEY_ERROR_TRY_AGAIN),
];

/// The locale a consent-flow page is written in: the reader's stored
/// preference, else the first language their browser lists that the platform
/// speaks, else English.
pub(super) fn reader_locale(stored: Option<&str>, accept_language: Option<&str>) -> &'static str {
    supported_ui_locale(stored)
        .or_else(|| {
            accept_language?
                .split(',')
                .find_map(|entry| supported_ui_locale(entry.split(';').next()))
        })
        .unwrap_or(FALLBACK_LOCALE)
}

/// What the consent flow's pages say, in one locale.
#[derive(Clone, Copy)]
pub struct ConsentText<'a> {
    strings: &'a MessagingStringsRegistry,
    locale: &'static str,
}

impl fmt::Debug for ConsentText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsentText")
            .field("locale", &self.locale)
            .finish_non_exhaustive()
    }
}

impl<'a> ConsentText<'a> {
    /// The text in `locale`, or in English when the platform does not speak it.
    #[must_use]
    pub fn new(strings: &'a MessagingStringsRegistry, locale: &str) -> Self {
        Self {
            strings,
            locale: SUPPORTED_LOCALES
                .into_iter()
                .find(|known| *known == locale)
                .unwrap_or(FALLBACK_LOCALE),
        }
    }

    /// The text for a reader whose stored locale is `stored`, if one is known.
    pub(super) fn for_reader(
        strings: &'a MessagingStringsRegistry,
        stored: Option<&str>,
        headers: &HeaderMap,
    ) -> Self {
        let accept_language = headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok());
        Self {
            strings,
            locale: reader_locale(stored, accept_language),
        }
    }

    /// The text for a reader nobody has identified, which the browser words.
    pub(super) fn for_browser(strings: &'a MessagingStringsRegistry, headers: &HeaderMap) -> Self {
        Self::for_reader(strings, None, headers)
    }

    /// The catalogue the text reads from.
    pub(super) const fn strings(&self) -> &'a MessagingStringsRegistry {
        self.strings
    }

    /// The page's `lang` attribute.
    pub(super) const fn lang(&self) -> &'static str {
        self.locale
    }

    /// The line `key` as page markup with its slots filled.
    ///
    /// The line is escaped first and the slots filled after, so `args` must
    /// already be markup: an escaped value, or a tag around one. A line this
    /// locale has no text for reads in English, never in the platform's
    /// default and never as its key.
    pub(super) fn html(&self, key: &str, args: &[&str]) -> String {
        format_template(&escape_html_attribute(&self.line(key)), args)
    }

    /// The catalogue's text for `key` as this page reads it, slots unfilled:
    /// this locale's, else English, else nothing.
    pub(super) fn line(&self, key: &str) -> String {
        self.strings
            .get_exact(key, self.locale)
            .or_else(|| self.strings.get_exact(key, FALLBACK_LOCALE))
            .unwrap_or_default()
    }

    /// `preset_key` on a preset grant's screen when the catalogue words it,
    /// else `key`: a preset line the pin does not carry yet leaves the
    /// screen reading the line every other grant reads.
    pub(super) fn preset_or<'k>(&self, preset: bool, preset_key: &'k str, key: &'k str) -> &'k str {
        if preset && !self.line(preset_key).is_empty() {
            preset_key
        } else {
            key
        }
    }

    /// The list item naming `scope`: what it allows in this language, then
    /// the token itself, which is all a scope Dravr does not word can show.
    /// On a preset grant's screen (`preset`), `fitness:read` reads its preset
    /// line, which names no injuries and no activity places.
    pub(super) fn scope_item(&self, scope: &str, preset: bool) -> String {
        let token = escape_html_attribute(scope);
        let key = match scope {
            "fitness:read" => self.preset_or(
                preset,
                PRESET_KEY_SCOPE_FITNESS_READ,
                KEY_CONSENT_SCOPE_FITNESS_READ,
            ),
            "fitness:write" => KEY_CONSENT_SCOPE_FITNESS_WRITE,
            "profile:read" => KEY_CONSENT_SCOPE_PROFILE_READ,
            "profile:write" => KEY_CONSENT_SCOPE_PROFILE_WRITE,
            "offline_access" => KEY_CONSENT_SCOPE_OFFLINE_ACCESS,
            _ => return format!("<li>{token}</li>"),
        };
        format!("<li>{} ({token})</li>", self.html(key, &[]))
    }

    /// What the error page says about `error`, as markup: the refusal in this
    /// language when it is one these pages word. Any other description is the
    /// server's English: an English page shows it whole, and another language
    /// says what kind of refusal it is rather than mix the two.
    pub(super) fn refusal(&self, error: &OAuth2Error) -> String {
        let description = error.error_description.as_deref();
        let worded = description.and_then(|text| {
            WORDED_REFUSALS
                .into_iter()
                .find(|(known, _)| *known == text)
                .map(|(_, key)| key)
        });
        if let Some(key) = worded {
            return self.html(key, &[]);
        }
        match description {
            None => self.html(KEY_ERROR_TRY_AGAIN, &[]),
            Some(text) if self.locale == FALLBACK_LOCALE => escape_html_attribute(text),
            Some(_) => self.html(refusal_kind(&error.error), &[]),
        }
    }
}

/// The line for a refusal of the OAuth error code `code`.
fn refusal_kind(code: &str) -> &'static str {
    match code {
        "invalid_request" => KEY_ERROR_INVALID_REQUEST,
        "invalid_request_uri" => KEY_ERROR_INVALID_REQUEST_URI,
        "invalid_client" => KEY_ERROR_INVALID_CLIENT,
        "access_denied" => KEY_ERROR_DENIED,
        "server_error" => KEY_ERROR_SERVER,
        "temporarily_unavailable" => KEY_ERROR_UNAVAILABLE,
        "too_many_requests" => KEY_ERROR_RATE_LIMITED,
        _ => KEY_ERROR_TRY_AGAIN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_contremaitre::consent_strings::{KEY_CONSENT_ALLOW, KEY_CONSENT_LEAD};

    /// A preset line the catalogue does not word leaves a preset screen
    /// reading the line every grant reads; one it words replaces it there,
    /// and never on any other screen.
    #[test]
    fn a_preset_line_replaces_the_generic_one_only_on_a_preset_screen_that_has_it() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        let unworded = "oauth.consent.noSuchPresetLine";
        assert_eq!(
            text.preset_or(true, unworded, KEY_CONSENT_LEAD),
            KEY_CONSENT_LEAD,
            "bug: a preset line the catalogue does not word replaced the generic one"
        );
        assert_eq!(
            text.preset_or(true, KEY_CONSENT_ALLOW, KEY_CONSENT_LEAD),
            KEY_CONSENT_ALLOW,
            "bug: a worded preset line is not read on a preset screen"
        );
        assert_eq!(
            text.preset_or(false, KEY_CONSENT_ALLOW, KEY_CONSENT_LEAD),
            KEY_CONSENT_LEAD,
            "bug: a preset line is read on a screen that is not a preset's"
        );
    }

    #[test]
    fn the_stored_locale_wins_then_the_browser_then_english() {
        assert_eq!(reader_locale(Some("de"), Some("fr-CA,en;q=0.9")), "de");
        assert_eq!(reader_locale(None, Some("fr-CA,en;q=0.9")), "fr");
        // A stored value the platform does not speak says nothing about the reader
        assert_eq!(reader_locale(Some("ja"), Some("pt-BR")), "pt");
        assert_eq!(reader_locale(Some(""), Some("es")), "es");
        // The first language it speaks, not only the first listed
        assert_eq!(reader_locale(None, Some("ja,fr;q=0.8,en;q=0.5")), "fr");
        assert_eq!(reader_locale(None, Some("ja-JP")), "en");
        assert_eq!(reader_locale(None, Some("*")), "en");
        assert_eq!(reader_locale(None, None), "en");
    }

    #[test]
    fn a_locale_the_platform_does_not_speak_reads_in_english() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "ja");
        assert_eq!(text.lang(), "en");
        assert_eq!(text.html(KEY_CONSENT_ALLOW, &[]), "Allow access");
    }

    /// A blank entry reads in English: not French, the registry's own
    /// fallback, and not a blank line on a page asking for consent.
    #[test]
    fn a_line_missing_from_a_locale_reads_in_english() {
        let registry = MessagingStringsRegistry::new();
        registry.update(KEY_CONSENT_ALLOW, "de", String::new(), "blank".to_owned());
        let text = ConsentText::new(&registry, "de");
        assert_eq!(text.lang(), "de");
        assert_eq!(text.html(KEY_CONSENT_ALLOW, &[]), "Allow access");
    }

    #[test]
    fn a_value_filling_a_slot_is_not_read_as_another_slot() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        assert_eq!(
            text.html(KEY_CONSENT_LEAD, &["{1}"]),
            "{1} wants to access your Dravr fitness data."
        );
    }

    #[test]
    fn every_worded_refusal_reads_in_english_as_the_description_it_answers() {
        let registry = MessagingStringsRegistry::new();
        let text = ConsentText::new(&registry, "en");
        for (description, key) in WORDED_REFUSALS {
            assert_eq!(
                text.html(key, &[]),
                escape_html_attribute(description),
                "{key}"
            );
        }
    }

    #[test]
    fn a_refusal_french_does_not_word_says_its_kind_in_french_and_keeps_english_whole() {
        let registry = MessagingStringsRegistry::new();
        let technical = OAuth2Error::invalid_request("Missing client_id parameter");
        let french = ConsentText::new(&registry, "fr").refusal(&technical);
        assert!(!french.contains("client_id"), "{french}");
        assert!(french.contains("demande de connexion"), "{french}");
        let english = ConsentText::new(&registry, "en").refusal(&technical);
        assert_eq!(english, "Missing client_id parameter");
    }

    #[test]
    fn a_spent_request_uri_says_so_in_french_and_keeps_english_whole() {
        let registry = MessagingStringsRegistry::new();
        let spent = OAuth2Error::invalid_request_uri("The request_uri expired");
        let french = ConsentText::new(&registry, "fr").refusal(&spent);
        assert_eq!(
            french,
            escape_html_attribute(&registry.get(KEY_ERROR_INVALID_REQUEST_URI, "fr")),
        );
        assert!(french.contains("lien de connexion"), "{french}");
        let english = ConsentText::new(&registry, "en").refusal(&spent);
        assert_eq!(english, "The request_uri expired");
    }

    /// The refusal a spent or unknown `request_uri` carries reads as the
    /// catalogue's line on an English page too, not only in another language.
    #[test]
    fn the_request_uri_refusal_reads_as_its_catalogue_line_in_english_and_french() {
        let registry = MessagingStringsRegistry::new();
        let refused = OAuth2Error::invalid_request_uri(REQUEST_URI_REFUSED);
        for locale in ["en", "fr"] {
            let line = registry.get(KEY_ERROR_INVALID_REQUEST_URI, locale);
            assert!(
                !line.is_empty(),
                "bug: {locale} has no invalidRequestUri line"
            );
            assert_eq!(
                ConsentText::new(&registry, locale).refusal(&refused),
                escape_html_attribute(&line),
                "{locale}"
            );
        }
    }

    #[test]
    fn an_account_refusal_is_worded_in_the_page_language() {
        let registry = MessagingStringsRegistry::new();
        let suspended = OAuth2Error::access_denied(SUSPENDED);
        assert_eq!(
            ConsentText::new(&registry, "fr").refusal(&suspended),
            "Votre compte Dravr est suspendu"
        );
    }
}
