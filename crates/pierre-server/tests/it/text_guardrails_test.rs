// ABOUTME: Tier 6 text guardrail tests — disclaimer prefix, blocked topics, length caps
// ABOUTME: Pure-Rust, no provider dependencies; lives in tests/ per the project rule
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;

use pierre_contremaitre::text_guardrails::{
    GuardrailOutcome, GuardrailRejection, LocaleGuardrails, TextGuardrails,
};

/// A small two-locale rule set used by the locale-aware tests:
/// English triggers on injury/pain, French triggers on blessure/douleur.
/// The French list deliberately omits the substring `pain` so the
/// regression case (French bread firing the English disclaimer) is
/// guaranteed to be tested.
fn rules() -> TextGuardrails {
    let mut locales = HashMap::new();
    locales.insert(
        "en".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec!["injury".to_owned(), "pain".to_owned()],
        },
    );
    locales.insert(
        "fr".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec!["blessure".to_owned(), "douleur".to_owned()],
        },
    );
    TextGuardrails::compile(vec!["self-harm".to_owned()], locales)
}

fn needs_disclaimer_in(locale: &str) -> GuardrailOutcome {
    GuardrailOutcome::NeedsDisclaimer {
        locale: locale.to_owned(),
    }
}

#[test]
fn allowed_when_no_trigger() {
    let r = rules();
    let outcome = r.apply("How was your training week?", "en");
    assert_eq!(
        outcome,
        GuardrailOutcome::Allowed("How was your training week?".to_owned())
    );
}

#[test]
fn disclaimer_needed_on_injury_keyword() {
    let r = rules();
    assert_eq!(
        r.apply("Watch your achilles for any injury signs.", "en"),
        needs_disclaimer_in("en")
    );
}

#[test]
fn disclaimer_needed_on_pain_keyword_case_insensitive() {
    let r = rules();
    assert_eq!(
        r.apply("If you feel any PAIN, stop the workout.", "en"),
        needs_disclaimer_in("en")
    );
}

#[test]
fn length_alone_never_rejects_a_response() {
    let r = rules();
    let long = "a".repeat(60_000);
    assert_eq!(r.apply(&long, "en"), GuardrailOutcome::Allowed(long));
}

/// The two rules used to run only under the cap, so a long reply skipped both.
#[test]
fn a_long_response_is_still_held_to_every_rule() {
    let r = rules();
    let filler = "a".repeat(60_000);
    assert!(matches!(
        r.apply(&format!("{filler} self-harm"), "en"),
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { .. })
    ));
    assert_eq!(
        r.apply(&format!("{filler} injury"), "en"),
        needs_disclaimer_in("en")
    );
}

#[test]
fn rejected_when_response_mentions_blocked_topic() {
    let r = rules();
    match r.apply("Please don't engage in self-harm.", "en") {
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            assert_eq!(topic, "self-harm");
        }
        other => panic!("expected BlockedTopic rejection, got {other:?}"),
    }
}

#[test]
fn empty_response_passes_through_unchanged() {
    let r = rules();
    assert_eq!(r.apply("", "en"), GuardrailOutcome::Allowed(String::new()));
}

#[test]
fn safe_default_includes_medical_keywords() {
    let r = TextGuardrails::safe_default();
    assert_eq!(
        r.apply("If your knee pain persists, please see a doctor.", "en"),
        needs_disclaimer_in("en")
    );
}

/// A reply talking about `Schmerzen` got no disclaimer, though the German
/// disclaimer copy itself uses that word: triggers match whole words, and the lists held
/// singulars only.
#[test]
fn safe_default_fires_on_inflected_forms_in_every_locale() {
    let r = TextGuardrails::safe_default();
    for (reply, locale) in [
        ("Those knee injuries need rest.", "en"),
        ("That sounds painful after the run.", "en"),
        ("Ces douleurs au genou méritent du repos.", "fr"),
        ("Tus lesiones necesitan descanso.", "es"),
        ("Deine Schmerzen brauchen Ruhe.", "de"),
        ("Nimm keine Medikamente ohne Rat.", "de"),
        ("As tuas lesões precisam de descanso.", "pt"),
    ] {
        assert_eq!(
            r.apply(reply, locale),
            needs_disclaimer_in(locale),
            "{reply}"
        );
    }
}

/// French `pain` was fixed in afe018126, but the same class of false positive lived
/// on: these words mean a recipe or a training plan, not a doctor's prescription.
#[test]
fn safe_default_does_not_read_a_recipe_as_medical() {
    let r = TextGuardrails::safe_default();
    for (reply, locale) in [
        ("Aquí tienes una receta de pasta para esta noche.", "es"),
        ("Hier ist ein Rezept für heute Abend.", "de"),
        ("Aqui tens uma receita de massa para hoje.", "pt"),
        (
            "Your training prescription for this block is unchanged.",
            "en",
        ),
    ] {
        assert_eq!(
            r.apply(reply, locale),
            GuardrailOutcome::Allowed(reply.to_owned()),
            "{reply}"
        );
    }
}

// ---------------------------------------------------------------------------
// Regression: locale scoping. These exist precisely because the original
// implementation matched English triggers against French content as plain
// substrings — the word `pain` (meaning "bread" in French) lit up the
// English medical disclaimer on every meal recommendation.
// ---------------------------------------------------------------------------

#[test]
fn french_bread_does_not_trigger_english_medical_disclaimer() {
    let r = rules();
    let outcome = r.apply("Pâtes avec parmesan, pain, salade.", "fr");
    assert_eq!(
        outcome,
        GuardrailOutcome::Allowed("Pâtes avec parmesan, pain, salade.".to_owned()),
        "French 'pain' (bread) must not fire the English disclaimer when locale='fr'"
    );
}

#[test]
fn french_locale_uses_french_triggers() {
    let r = rules();
    assert_eq!(
        r.apply("Si tu sens une douleur, arrête.", "fr"),
        needs_disclaimer_in("fr")
    );
}

#[test]
fn unconfigured_locale_falls_back_to_english() {
    // The Italian locale isn't present in `rules()`; with `en` available,
    // resolve_locale returns the English entry, and the outcome names `en` so the
    // disclaimer copy reads in the language of the rule that matched. That's the
    // documented contract, surfaced here so operators don't ship partial locale
    // coverage by accident.
    let r = rules();
    assert_eq!(
        r.apply("Persistent ankle pain after training.", "it"),
        needs_disclaimer_in("en")
    );
}

#[test]
fn word_boundary_prevents_substring_false_positives_within_locale() {
    // Within English, `pain` should match the standalone word but not
    // `painstaking`. This catches a regression where the substring
    // matcher fired on common English words containing the trigger
    // root.
    let r = rules();
    let outcome = r.apply("Your painstaking taper looks great.", "en");
    assert_eq!(
        outcome,
        GuardrailOutcome::Allowed("Your painstaking taper looks great.".to_owned()),
        "Word-boundary regex must not match 'pain' inside 'painstaking'"
    );
}

#[test]
fn safe_default_french_meal_recommendation_is_clean() {
    // The exact regression from production: a French meal suggestion
    // mentioning bread must not prepend the English medical disclaimer.
    let r = TextGuardrails::safe_default();
    let reply = "Souper : Pâtes avec sauce à la viande, parmesan, pain, salade.";
    assert_eq!(
        r.apply(reply, "fr"),
        GuardrailOutcome::Allowed(reply.to_owned()),
        "No French trigger word present, so no disclaimer fires in any language"
    );
}

#[test]
fn safe_default_french_medical_trigger_needs_the_french_disclaimer() {
    let r = TextGuardrails::safe_default();
    assert_eq!(
        r.apply("Si la douleur persiste, consulte un médecin.", "fr"),
        needs_disclaimer_in("fr")
    );
}
