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
            disclaimer_text: "EN Disclaimer.".to_owned(),
        },
    );
    locales.insert(
        "fr".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec!["blessure".to_owned(), "douleur".to_owned()],
            disclaimer_text: "FR Avis.".to_owned(),
        },
    );
    TextGuardrails::compile(200, vec!["self-harm".to_owned()], locales)
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
fn disclaimer_prepended_on_injury_keyword() {
    let r = rules();
    let outcome = r.apply("Watch your achilles for any injury signs.", "en");
    if let GuardrailOutcome::Allowed(text) = outcome {
        assert!(text.starts_with("EN Disclaimer."));
        assert!(text.contains("achilles"));
    } else {
        panic!("expected allowed with disclaimer");
    }
}

#[test]
fn disclaimer_prepended_on_pain_keyword_case_insensitive() {
    let r = rules();
    let outcome = r.apply("If you feel any PAIN, stop the workout.", "en");
    if let GuardrailOutcome::Allowed(text) = outcome {
        assert!(text.starts_with("EN Disclaimer."));
    } else {
        panic!("expected allowed with disclaimer");
    }
}

#[test]
fn rejected_when_response_exceeds_cap() {
    let r = rules();
    let long = "a".repeat(300);
    match r.apply(&long, "en") {
        GuardrailOutcome::Rejected(GuardrailRejection::TooLong { length, cap }) => {
            assert_eq!(length, 300);
            assert_eq!(cap, 200);
        }
        other => panic!("expected TooLong rejection, got {other:?}"),
    }
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
    let outcome = r.apply("If your knee pain persists, please see a doctor.", "en");
    if let GuardrailOutcome::Allowed(text) = outcome {
        assert!(text.contains("Medical disclaimer"));
        assert!(text.contains("knee pain"));
    } else {
        panic!("expected allowed with disclaimer prepended");
    }
}

#[test]
fn safe_default_caps_long_responses() {
    let r = TextGuardrails::safe_default();
    let huge = "x".repeat(10_000);
    matches!(
        r.apply(&huge, "en"),
        GuardrailOutcome::Rejected(GuardrailRejection::TooLong { .. })
    );
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
fn french_locale_uses_french_triggers_and_text() {
    let r = rules();
    let outcome = r.apply("Si tu sens une douleur, arrête.", "fr");
    let GuardrailOutcome::Allowed(text) = outcome else {
        panic!("expected allowed with French disclaimer");
    };
    assert!(text.starts_with("FR Avis."));
    assert!(
        !text.contains("EN Disclaimer."),
        "French locale must not leak the English disclaimer"
    );
}

#[test]
fn unconfigured_locale_falls_back_to_english() {
    // The Italian locale isn't present in `rules()`; with `en` available,
    // resolve_locale returns the English entry. An Italian-text reply
    // containing the English trigger word would then fire the EN
    // disclaimer — that's the documented contract, surfaced here so
    // operators don't ship partial locale coverage by accident.
    let r = rules();
    let outcome = r.apply("Persistent ankle pain after training.", "it");
    let GuardrailOutcome::Allowed(text) = outcome else {
        panic!("expected allowed with fallback disclaimer");
    };
    assert!(text.starts_with("EN Disclaimer."));
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
    let outcome = r.apply(
        "Souper : Pâtes avec sauce à la viande, parmesan, pain, salade.",
        "fr",
    );
    let GuardrailOutcome::Allowed(text) = outcome else {
        panic!("expected allowed pass-through");
    };
    assert!(
        !text.contains("Medical disclaimer"),
        "French meal recommendation must not leak the English disclaimer"
    );
    assert!(
        !text.contains("Avis médical"),
        "No French trigger word present, so the French disclaimer should not fire either"
    );
}

#[test]
fn safe_default_french_medical_trigger_prepends_french_disclaimer() {
    let r = TextGuardrails::safe_default();
    let outcome = r.apply("Si la douleur persiste, consulte un médecin.", "fr");
    let GuardrailOutcome::Allowed(text) = outcome else {
        panic!("expected allowed with French disclaimer prepended");
    };
    assert!(
        text.contains("Avis médical"),
        "French medical trigger must prepend the French disclaimer"
    );
    assert!(
        !text.contains("Medical disclaimer:"),
        "French response must not also carry the English disclaimer header"
    );
}
