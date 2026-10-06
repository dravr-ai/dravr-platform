// ABOUTME: The hosted login page's text in the athlete's language, from the apps' own string catalogue
// ABOUTME: The locale is the app's own (ui_locales), else the browser's Accept-Language

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::{header, HeaderMap, StatusCode};
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_contremaitre::hosted_strings::KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::models::page_locale;

/// "Sign in": the page title, its heading and the submit button
const KEY_SIGN_IN: &str = "auth.signInButton";
/// The email field's label
const KEY_EMAIL: &str = "auth.emailFieldLabel";
/// The password field's label
const KEY_PASSWORD: &str = "auth.passwordLabel";
/// The divider above "Continue with Google"
const KEY_OR: &str = "auth.orDivider";
/// "Continue with Google"
const KEY_GOOGLE: &str = "auth.googleContinueButton";
/// "Sign-in failed": the failure page's title and heading
const KEY_FAILED: &str = "auth.loginFailed";
/// A refused email or password
const KEY_INVALID_CREDENTIALS: &str = "auth.invalidCredentials";
/// The failure page's way back to the form
const KEY_BACK: &str = "auth.backToSignIn";
/// A suspended account, once its password verified
const KEY_SUSPENDED: &str = "shell.accountSuspendedBody";
/// A full window of refused passwords (carnet#804)
const KEY_TOO_MANY: &str = "accountDeletion.tooManyAttempts";

/// The login form's text in one locale, owned so the page renders off the
/// request's thread.
#[derive(Debug, Clone)]
pub struct LoginPageLabels {
    /// The page's `lang` attribute
    pub lang: String,
    /// "Sign in": title, heading and submit button
    pub sign_in: String,
    /// The email field's label
    pub email: String,
    /// The password field's label
    pub password: String,
    /// The divider above "Continue with Google"
    pub or: String,
    /// "Continue with Google"
    pub google: String,
}

impl LoginPageLabels {
    /// The labels the catalogue holds for `locale` (the default locale's
    /// for one it does not hold).
    #[must_use]
    pub fn new(strings: &MessagingStringsRegistry, locale: &str) -> Self {
        Self {
            lang: locale.to_owned(),
            sign_in: strings.get(KEY_SIGN_IN, locale),
            email: strings.get(KEY_EMAIL, locale),
            password: strings.get(KEY_PASSWORD, locale),
            or: strings.get(KEY_OR, locale),
            google: strings.get(KEY_GOOGLE, locale),
        }
    }
}

/// What the hosted login and its failure page say, in one locale.
///
/// Dravr's web and mobile apps sign in on this page (carnet#787), so it
/// speaks their words from their catalogue — the same "Sign in" the button
/// that opened it said — in the language the browser asks for.
pub(super) struct LoginText<'a> {
    strings: &'a MessagingStringsRegistry,
    locale: &'static str,
}

impl<'a> LoginText<'a> {
    /// The text for a request: in the language the app that opened the page
    /// asked for (`ui_locales`, which Dravr's apps send as the language they
    /// are showing), else the browser's.
    pub(super) fn for_request(
        strings: &'a MessagingStringsRegistry,
        headers: &HeaderMap,
        ui_locales: Option<&str>,
    ) -> Self {
        let accept_language = headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok());
        Self {
            strings,
            locale: page_locale(ui_locales, accept_language),
        }
    }

    /// The login form's labels.
    pub(super) fn labels(&self) -> LoginPageLabels {
        LoginPageLabels::new(self.strings, self.locale)
    }

    fn get(&self, key: &str) -> String {
        self.strings.get(key, self.locale)
    }

    pub(super) fn failed(&self) -> String {
        self.get(KEY_FAILED)
    }

    pub(super) fn back(&self) -> String {
        self.get(KEY_BACK)
    }

    pub(super) fn invalid_credentials(&self) -> String {
        self.get(KEY_INVALID_CREDENTIALS)
    }

    pub(super) fn suspended(&self) -> String {
        self.get(KEY_SUSPENDED)
    }

    pub(super) fn unavailable(&self) -> String {
        self.strings.render(
            KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE,
            self.locale,
            &["Dravr"],
        )
    }

    /// What a refusal the form met before any password was checked says:
    /// a full window of refused passwords, or a limiter that could not tell.
    pub(super) fn refusal(&self, error: &OAuth2Error) -> String {
        if error.http_status() == StatusCode::TOO_MANY_REQUESTS {
            self.get(KEY_TOO_MANY)
        } else {
            self.unavailable()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::SUPPORTED_LOCALES;

    fn text_for(registry: &MessagingStringsRegistry, accept_language: &str) -> String {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT_LANGUAGE, accept_language.parse().unwrap());
        LoginText::for_request(registry, &headers, None)
            .labels()
            .sign_in
    }

    #[test]
    fn every_key_has_text_in_every_locale() {
        let registry = MessagingStringsRegistry::new();
        for locale in SUPPORTED_LOCALES {
            for key in [
                KEY_SIGN_IN,
                KEY_EMAIL,
                KEY_PASSWORD,
                KEY_OR,
                KEY_GOOGLE,
                KEY_FAILED,
                KEY_INVALID_CREDENTIALS,
                KEY_BACK,
                KEY_SUSPENDED,
                KEY_TOO_MANY,
                KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE,
            ] {
                assert!(!registry.get(key, locale).is_empty(), "{key} in {locale}");
            }
        }
    }

    #[test]
    fn the_browser_language_picks_the_text() {
        let registry = MessagingStringsRegistry::new();
        assert_eq!(text_for(&registry, "en-US,en;q=0.9"), "Sign in");
        assert_eq!(text_for(&registry, "fr-CA"), "Se connecter");
        assert_eq!(text_for(&registry, "de"), "Anmelden");
        // French, the default, for a language the product does not speak
        assert_eq!(text_for(&registry, "ja-JP"), "Se connecter");
    }

    #[test]
    fn the_app_language_wins_over_the_browser() {
        let registry = MessagingStringsRegistry::new();
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT_LANGUAGE, "en-US".parse().unwrap());
        let text = |ui: Option<&str>| LoginText::for_request(&registry, &headers, ui).labels();
        assert_eq!(text(Some("fr")).sign_in, "Se connecter");
        assert_eq!(text(Some("ja de-CH")).lang, "de");
        // Nothing the product speaks: the browser decides
        assert_eq!(text(Some("ja")).lang, "en");
        assert_eq!(text(None).lang, "en");
    }
}
