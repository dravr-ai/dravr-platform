// ABOUTME: Tests the first-use acronym-expansion pass against a contremaitre glossary snapshot
// ABOUTME: Covers first-use-only, locale fallback, word boundaries, multibyte safety, and the ambiguous-token carve-out
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_chat_pipeline::stages::acronym_expansion::expand_acronyms_first_use;
use pierre_contremaitre::persona_contracts::{PersonaContractRegistry, PersonaContractsSnapshot};

/// Fixture glossary. `TSB` deliberately omits the `de` locale so the
/// fallback-to-`en` path is exercised; `IF` is absent entirely to assert
/// the ambiguous-token carve-out (it must never be auto-expanded).
const FIXTURE_YAML: &str = r"
version: 2
glossary:
  ATL:
    en: acute training load
    fr: charge aiguë d'entraînement
    de: akute Trainingsbelastung
  TSB:
    en: training stress balance
    fr: équilibre de charge
  MTB:
    en: mountain bike
    fr: vélo de montagne
personas:
  casual:
    max_words: 150
";

fn snapshot() -> Arc<PersonaContractsSnapshot> {
    let registry = PersonaContractRegistry::new();
    assert!(
        registry.apply_overlay(FIXTURE_YAML).is_ok(),
        "fixture glossary YAML parses"
    );
    registry.snapshot()
}

#[test]
fn expands_first_occurrence_in_english() {
    let snap = snapshot();
    let out = expand_acronyms_first_use(&snap, "Your ATL is 74 today.", "en");
    assert_eq!(out, "Your ATL (acute training load) is 74 today.");
}

#[test]
fn expands_in_requested_locale() {
    let snap = snapshot();
    let out = expand_acronyms_first_use(&snap, "Ton ATL monte.", "fr");
    assert_eq!(out, "Ton ATL (charge aiguë d'entraînement) monte.");
}

#[test]
fn falls_back_to_english_when_locale_missing() {
    let snap = snapshot();
    // TSB has no `de` entry — fall back to the English full form.
    let out = expand_acronyms_first_use(&snap, "Dein TSB liegt bei -14.", "de");
    assert_eq!(out, "Dein TSB (training stress balance) liegt bei -14.");
}

#[test]
fn glosses_only_the_first_occurrence() {
    let snap = snapshot();
    let out = expand_acronyms_first_use(&snap, "ATL today, ATL tomorrow.", "en");
    assert_eq!(out, "ATL (acute training load) today, ATL tomorrow.");
}

#[test]
fn skips_acronym_already_glossed_by_model() {
    let snap = snapshot();
    let text = "ATL (acute training load) is high; ATL keeps climbing.";
    let out = expand_acronyms_first_use(&snap, text, "en");
    assert_eq!(out, text, "first use already glossed — leave untouched");
}

#[test]
fn does_not_match_inside_a_larger_word() {
    let snap = snapshot();
    // `ATL` inside `ATLAS`, and lowercase `atl` inside `atlas`, must not fire.
    let text = "The ATLAS range and the atlas page.";
    let out = expand_acronyms_first_use(&snap, text, "en");
    assert_eq!(out, text);
}

#[test]
fn expands_multiple_acronyms_in_one_pass() {
    let snap = snapshot();
    let out = expand_acronyms_first_use(&snap, "ATL 74, TSB -14, rode MTB.", "en");
    assert_eq!(
        out,
        "ATL (acute training load) 74, TSB (training stress balance) -14, rode MTB (mountain bike)."
    );
}

#[test]
fn handles_punctuation_immediately_after_acronym() {
    let snap = snapshot();
    let out = expand_acronyms_first_use(&snap, "Watch your ATL.", "en");
    assert_eq!(out, "Watch your ATL (acute training load).");
}

#[test]
fn multibyte_char_after_acronym_does_not_panic() {
    let snap = snapshot();
    // A French right single quote (multibyte) sits one char past the
    // acronym; the gloss-window scan must not split the byte boundary.
    let out = expand_acronyms_first_use(&snap, "TSB\u{2019}est bas.", "fr");
    assert_eq!(out, "TSB (équilibre de charge)\u{2019}est bas.");
}

#[test]
fn uncatalogued_ambiguous_token_is_never_expanded() {
    let snap = snapshot();
    // `IF` is intentionally absent from the glossary — auto-expanding it
    // would mangle the ordinary word. It must pass through verbatim.
    let text = "IF you ride hard, your CTL climbs.";
    let out = expand_acronyms_first_use(&snap, text, "en");
    assert_eq!(out, text);
}

#[test]
fn empty_glossary_is_a_noop() {
    let registry = PersonaContractRegistry::new();
    assert!(
        registry
            .apply_overlay("version: 2\npersonas:\n  casual:\n    max_words: 150\n")
            .is_ok(),
        "YAML without a glossary parses"
    );
    let snap = registry.snapshot();
    let text = "Your ATL is 74 and TSB is -14.";
    let out = expand_acronyms_first_use(&snap, text, "en");
    assert_eq!(out, text, "no glossary -> reply passes through untouched");
}
