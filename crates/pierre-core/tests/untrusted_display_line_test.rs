// ABOUTME: Tests for display_line — the one neutralization every provider-written label passes before a reader
// ABOUTME: Pins the single line, the defanged markup and lead, the character cap, and the activity-name cap
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};

#[test]
fn a_label_that_forges_a_heading_and_an_image_reads_as_one_plain_line() {
    let raw = "## Threshold 2x20\n![x](https://evil.example/leak?q=)";
    assert_eq!(
        display_line(raw, ACTIVITY_NAME_MAX_CHARS),
        "Threshold 2x20 ![x] (https://evil.example/leak?q=)"
    );
}

#[test]
fn markup_and_code_fences_cannot_open() {
    assert_eq!(display_line("<b>Tempo</b> `run`", 80), "‹b›Tempo‹/b› 'run'");
    assert_eq!(display_line(">> quoted line", 80), "quoted line");
}

#[test]
fn an_ordinary_title_crosses_unchanged() {
    assert_eq!(
        display_line("Sortie vélo matinale — 2x20 @ 88-93%", 120),
        "Sortie vélo matinale — 2x20 @ 88-93%"
    );
}

#[test]
fn the_cap_counts_characters_and_marks_the_cut() {
    let long = "é".repeat(ACTIVITY_NAME_MAX_CHARS + 30);
    let capped = display_line(&long, ACTIVITY_NAME_MAX_CHARS);
    assert_eq!(capped.chars().count(), ACTIVITY_NAME_MAX_CHARS);
    assert!(capped.ends_with('…'));
}

#[test]
fn an_activity_name_is_cut_past_120_characters_and_not_before() {
    let at_the_cap = "é".repeat(120);
    assert_eq!(
        display_line(&at_the_cap, ACTIVITY_NAME_MAX_CHARS),
        at_the_cap,
        "a name of exactly 120 characters crosses whole"
    );

    let one_over = "é".repeat(121);
    let capped = display_line(&one_over, ACTIVITY_NAME_MAX_CHARS);
    assert_eq!(capped.chars().count(), 120);
    assert_eq!(
        capped,
        format!("{}…", "é".repeat(119)),
        "the 121st character costs the name its tail, marked with an ellipsis"
    );
}
