// ABOUTME: Unit tests for pierre_core::markdown::strip_emphasis
// ABOUTME: Covers single/double asterisk and underscore emphasis, escapes, unbalanced markers, non-ASCII
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_core::markdown::strip_emphasis;

#[test]
fn strips_single_asterisk_emphasis() {
    assert_eq!(strip_emphasis("what *not* to do"), "what not to do");
}

#[test]
fn strips_double_asterisk_strong() {
    assert_eq!(strip_emphasis("**Approach**: be calm"), "Approach: be calm");
}

#[test]
fn strips_underscore_emphasis() {
    assert_eq!(strip_emphasis("an _italicized_ word"), "an italicized word");
}

#[test]
fn strips_double_underscore_strong() {
    assert_eq!(strip_emphasis("__bold__ statement"), "bold statement");
}

#[test]
fn preserves_unbalanced_single_marker() {
    // A lone asterisk with no closing partner stays literal — we never
    // silently swallow author content.
    assert_eq!(strip_emphasis("2 * 3 = 6"), "2 * 3 = 6");
}

#[test]
fn respects_backslash_escape() {
    assert_eq!(
        strip_emphasis(r"literal \*star\* here"),
        "literal *star* here"
    );
}

#[test]
fn handles_multiple_emphasis_spans() {
    assert_eq!(
        strip_emphasis("**bold** and *italic* and _underscore_"),
        "bold and italic and underscore"
    );
}

#[test]
fn preserves_non_ascii_text() {
    assert_eq!(
        strip_emphasis("Entraînement *en douceur* — café"),
        "Entraînement en douceur — café"
    );
}

#[test]
fn empty_input_yields_empty_output() {
    assert_eq!(strip_emphasis(""), "");
}

#[test]
fn coach_description_with_emphasis_is_flattened() {
    // Regression: this exact string is the Purpose body of
    // coaches/mobility/pre-workout-mobility-coach/en.md. It reached Telegram
    // with literal asterisks before strip_emphasis was wired into
    // CoachListHandler.
    let raw = "Expert in dynamic warm-ups and muscle activation routines \
               before training. Helps athletes prepare their bodies for \
               exercise through movement preparation that enhances \
               performance and reduces injury risk — including clear \
               guidance on what *not* to do before a workout.";
    let out = strip_emphasis(raw);
    assert!(!out.contains('*'), "expected no literal asterisks in {out}");
    assert!(out.contains("what not to do"));
}
