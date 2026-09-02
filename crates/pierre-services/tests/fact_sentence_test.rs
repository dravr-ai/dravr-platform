// ABOUTME: The one fact renderer — a predicate code plus the athlete's words become one sentence per locale
// ABOUTME: Asserts exact French and English text, the PAR-Q id-to-question bridge, and that `states` adds nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::SUPPORTED_LOCALES;
use pierre_memory::{FactKind, PredicateCode};
use pierre_services::memory_facts::SentenceRenderer;

#[test]
fn a_french_athlete_reads_her_goal_in_french() {
    let strings = MessagingStringsRegistry::new();
    let fr = SentenceRenderer::new(&strings, "fr");
    assert_eq!(
        fr.render(
            PredicateCode::TrainingFor,
            "un ultra de 26 km au Mont Albert"
        ),
        "Tu t'entraînes pour un ultra de 26 km au Mont Albert"
    );
    assert_eq!(
        fr.render(PredicateCode::WorkingToward, "courir 5 km sans m'arrêter"),
        "Tu vises courir 5 km sans m'arrêter"
    );
    let en = SentenceRenderer::new(&strings, "en");
    assert_eq!(
        en.render(PredicateCode::TrainingFor, "Boston in April"),
        "You are training for Boston in April"
    );
}

#[test]
fn states_is_the_athletes_words_alone() {
    let strings = MessagingStringsRegistry::new();
    for locale in SUPPORTED_LOCALES {
        let sentence = SentenceRenderer::new(&strings, locale).render(
            PredicateCode::States,
            "je veux rester en forme pour mes enfants",
        );
        assert_eq!(
            sentence, "je veux rester en forme pour mes enfants",
            "{locale}: states must add no verb of ours"
        );
    }
}

#[test]
fn a_parq_flag_names_the_question_in_the_athletes_locale() {
    let strings = MessagingStringsRegistry::new();
    let fr =
        SentenceRenderer::new(&strings, "fr").render(PredicateCode::ParqYes, "heart_condition");
    let question_fr = strings.get("messaging.intake.parq.heart_condition", "fr");
    assert!(
        !question_fr.is_empty(),
        "the PAR-Q question exists in the catalogue"
    );
    assert!(fr.starts_with("Tu as répondu oui : "), "{fr}");
    assert!(fr.ends_with(&question_fr), "{fr}");
    assert!(
        !fr.contains("heart_condition"),
        "the raw id never reaches the athlete: {fr}"
    );

    let en =
        SentenceRenderer::new(&strings, "en").render(PredicateCode::ParqYes, "heart_condition");
    assert_ne!(fr, en, "each locale asks the question in its own words");
}

#[test]
fn every_code_renders_non_empty_in_every_locale() {
    let strings = MessagingStringsRegistry::new();
    for locale in SUPPORTED_LOCALES {
        let sentences = SentenceRenderer::new(&strings, locale);
        for code in PredicateCode::ALL {
            let sentence = sentences.render(code, "X");
            assert!(
                sentence.contains('X'),
                "{locale} {}: the object is missing from {sentence:?}",
                code.as_str()
            );
            assert!(
                !sentence.contains("{0}"),
                "{locale} {}: placeholder left in {sentence:?}",
                code.as_str()
            );
        }
    }
}

#[test]
fn every_code_belongs_to_exactly_the_kinds_it_says() {
    assert!(PredicateCode::TrainingFor.allowed_for(FactKind::Goal));
    assert!(!PredicateCode::TrainingFor.allowed_for(FactKind::Injury));
    assert!(PredicateCode::ParqYes.allowed_for(FactKind::Medical));
    assert!(!PredicateCode::ParqYes.allowed_for(FactKind::Goal));
    for kind in [
        FactKind::Preference,
        FactKind::Physiology,
        FactKind::Injury,
        FactKind::Goal,
        FactKind::Schedule,
        FactKind::Equipment,
        FactKind::NorthStar,
        FactKind::Medical,
        FactKind::Other,
    ] {
        assert!(
            PredicateCode::States.allowed_for(kind),
            "states fits every kind"
        );
    }
    for code in PredicateCode::ALL {
        assert_eq!(
            PredicateCode::parse(code.as_str()),
            Some(code),
            "round trip"
        );
    }
    assert_eq!(
        PredicateCode::parse("targets"),
        None,
        "a phrase is not a code"
    );
    assert_eq!(
        PredicateCode::legacy_from_phrase("train because"),
        Some(PredicateCode::TrainBecause)
    );
    assert_eq!(PredicateCode::legacy_from_phrase("are racing"), None);
}
