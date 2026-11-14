// ABOUTME: Folding text for matching what a person typed against a stored name: accents, case and spacing set aside
// ABOUTME: NFD, combining marks dropped, lowercased, every run of whitespace one space, the ends trimmed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Text folded for matching.
//!
//! A segment named "La Bête" is asked for as "la bete", an answer typed `sí`
//! means `si`. [`fold_for_match`] is the one fold both sides of such a
//! comparison go through, so a stored name and what a person typed compare
//! equal when they differ only in accents, case or spacing.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// `text` folded for matching: decomposed (NFD), its combining marks
/// dropped, lowercased, and every run of whitespace one space, with none at
/// either end.
///
/// Only accents go: punctuation and a letter with no decomposition (`ø`,
/// `ß`) stay as they are, so a caller that ignores punctuation strips it
/// itself.
#[must_use]
pub fn fold_for_match(text: &str) -> String {
    let unaccented: String = text.nfd().filter(|c| !is_combining_mark(*c)).collect();
    unaccented
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::fold_for_match;

    #[test]
    fn accents_case_and_spacing_fold_away() {
        assert_eq!(
            fold_for_match("  La   Bête\t"),
            "la bete",
            "bug: accents, case and spacing must not keep two names apart"
        );
        assert_eq!(
            fold_for_match("Côte-des-Neiges"),
            fold_for_match("cote-des-neiges"),
            "bug: an accented climb name must match its plain spelling"
        );
    }

    #[test]
    fn precomposed_and_decomposed_accents_fold_alike() {
        // U+00E9 against `e` + U+0301: the same letter, typed two ways.
        assert_eq!(
            fold_for_match("caf\u{e9}"),
            fold_for_match("cafe\u{301}"),
            "bug: one accented letter typed two ways folds two ways"
        );
        assert_eq!(fold_for_match("caf\u{e9}"), "cafe");
    }

    #[test]
    fn every_accent_the_intake_answers_carry_folds() {
        assert_eq!(
            fold_for_match("ÁÀÂÄÃ éèêë íìîï óòôöõ úùûü ñ ç"),
            "aaaaa eeee iiii ooooo uuuu n c",
            "bug: a vowel or consonant an athlete types accented keeps its accent"
        );
    }

    #[test]
    fn punctuation_and_letters_without_a_decomposition_stay() {
        assert_eq!(
            fold_for_match("Mont-Royal: ø!"),
            "mont-royal: ø!",
            "bug: the fold strips accents only, never punctuation or base letters"
        );
    }
}
