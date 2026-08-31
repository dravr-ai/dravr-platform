// ABOUTME: The turn's resolved locale is stated to the model, in all five locales, on every surface
// ABOUTME: Regression for carnet#159 — a French turn answered in English because nothing named the language

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! On 2026-08-30 a francophone athlete described night sweats, broken sleep and
//! new urinary difficulty on Telegram, in French, and the coach answered
//! entirely in English. The athlete replied "Yes" and got French back. One
//! conversation, two languages.
//!
//! The locale was never wrong. It was `fr` on both turns — the transcript
//! proves it, because `TextGuardrails` matches disclaimer triggers from the
//! *turn locale's* word list: the second turn's French reply hit `médecin` and
//! carried `**Avis médical :**`, while the first turn's English reply contained
//! `doctor` and `clinician`, matched no French trigger, and shipped with no
//! medical disclaimer at all — on the one turn in the conversation that
//! reported symptoms. An `en` locale would have matched `doctor` and prepended
//! the English disclaimer. It did not.
//!
//! So the server knew the language and never said it. `SurfaceProfile::locale`
//! selected the French coach prompt, the French refusals, the French acronym
//! glosses and the French disclaimer, and the coaching text those wrapped was
//! left to the model to infer from the athlete's own words — a few hundred
//! French characters against tens of KB of English contract, coach scaffolding,
//! provider context and tool results. The short second turn had no such weight
//! against it, which is why it stayed French.
//!
//! These tests pin the fix: a directive rendered from the resolved locale,
//! authored in that locale, present for all five.

use pierre_chat_pipeline::turn_service::detect_turn_locale;
use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_TURN_LANGUAGE};

/// Every locale carries a directive, and each is written in its own language.
///
/// Authoring it in the target language is the point, not a nicety: an English
/// sentence saying "reply in French" is one more English sentence on the pile
/// that caused the drift. The anchors below are ordinary words of each
/// language, so a directive accidentally re-authored in English fails here.
#[test]
fn every_locale_states_its_own_language_in_its_own_words() {
    let reg = MessagingStringsRegistry::new();
    let anchors = [
        ("fr", "français"),
        ("en", "English"),
        ("es", "español"),
        ("de", "Deutsch"),
        ("pt", "português"),
    ];
    for (locale, anchor) in anchors {
        let directive = reg.get(KEY_TURN_LANGUAGE, locale);
        assert!(
            !directive.trim().is_empty(),
            "missing turn-language directive for {locale}"
        );
        assert!(
            directive.contains(anchor),
            "the {locale} directive must name its language in its own words; \
             looked for {anchor:?} in: {directive}"
        );
    }
}

/// The five directives are five distinct strings.
///
/// A registry miss silently returns the French default (see
/// `registry_falls_back_to_default_locale_on_unknown`), so a locale whose entry
/// was never registered would answer French to everyone and pass a
/// non-emptiness check. Distinctness is what catches that.
#[test]
fn the_five_directives_are_distinct_strings() {
    let reg = MessagingStringsRegistry::new();
    let mut seen: Vec<String> = Vec::new();
    for locale in ["fr", "en", "es", "de", "pt"] {
        let directive = reg.get(KEY_TURN_LANGUAGE, locale);
        assert!(
            !seen.contains(&directive),
            "the {locale} directive duplicates another locale's — its registry entry is \
             probably missing and falling back to the default"
        );
        seen.push(directive);
    }
    assert_eq!(seen.len(), 5);
}

/// The non-English directives tell the model to translate its English working
/// material rather than pass it through.
///
/// This is the drift mechanism named directly: the tool results, the provider
/// context and the platform contract all arrive in English, and a model that
/// quotes them is already writing English.
#[test]
fn the_non_english_directives_forbid_passing_english_through() {
    let reg = MessagingStringsRegistry::new();
    let anchors = [
        ("fr", "anglais"),
        ("es", "inglés"),
        ("de", "Englisch"),
        ("pt", "inglês"),
    ];
    for (locale, anchor) in anchors {
        let directive = reg.get(KEY_TURN_LANGUAGE, locale);
        assert!(
            directive.contains(anchor),
            "the {locale} directive must say what to do with the English context it is \
             handed; looked for {anchor:?} in: {directive}"
        );
    }
}

/// The athlete's actual message from the incident resolves to `fr`.
///
/// It is French carrying English technical vocabulary — "high non activity
/// stress", "whoop", "ride" — which is exactly the shape that would make a
/// language detector waver. It does not: the detector was never the bug, and
/// pinning the real text keeps a future tweak to `detect_turn_locale` from
/// quietly making it one.
#[test]
fn the_incident_message_resolves_to_french() {
    let message = "En regardant mon high non activity stress qui est à la hausse sur whoop, \
                   devrais je m'inquiéter? Je me sens anormalement fatigué, j'ai de la misère \
                   à bien dormir, j'ai un peu plus de difficulté à uriner que d'habitude et \
                   jai eu anormalement chaud les quelques dernières nuit. J'ai également du, \
                   pour la première fois, m'étendre à la fin de ma ride de samedi dernier à \
                   Magog pour être capable de finir";

    assert_eq!(
        detect_turn_locale(message, "fr"),
        "fr",
        "the incident message is French and must resolve to fr"
    );
    // And it stays French even against an English stored preference, which is
    // the property that makes the directive below it trustworthy.
    assert_eq!(detect_turn_locale(message, "en"), "fr");

    // The follow-up that got the correct French reply is the English word
    // "Yes" — below the reliability floor, so it rides the stored preference.
    // Both turns therefore resolved to fr, which is what makes the English
    // first answer a prompt failure rather than a detection failure.
    assert_eq!(detect_turn_locale("Yes", "fr"), "fr");
}

/// The French directive is what a French turn gets, and it is not the English
/// one.
///
/// The narrow end-to-end claim the incident needs: ask the registry with the
/// locale the turn resolved, receive French.
#[test]
fn a_french_turn_receives_the_french_directive() {
    let reg = MessagingStringsRegistry::new();
    let locale = detect_turn_locale(
        "Je me sens anormalement fatigué et j'ai de la misère à bien dormir depuis une semaine.",
        "en",
    );
    assert_eq!(locale, "fr");

    let directive = reg.get(KEY_TURN_LANGUAGE, &locale);
    assert!(directive.contains("français"));
    assert_ne!(
        directive,
        reg.get(KEY_TURN_LANGUAGE, "en"),
        "a French turn must not receive the English directive"
    );
}
