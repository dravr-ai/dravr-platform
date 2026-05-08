// ABOUTME: Integration coverage for persona-conformance rule helpers
// ABOUTME: Exercises pure detectors (word count, bullet runs, label:value blocks, acronym gloss)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona conformance rule helpers — pure-function coverage.
//!
//! [`pierre_mcp_server::persona_contracts::PersonaContractRegistry`]
//! drives the live behaviour, but its inputs are all detector functions
//! (`count_words`, `longest_bullet_run`, `detects_label_value_block`,
//! `has_unglossed_acronym`, `is_bullet_line`) that have no external
//! dependencies and are cheap to test directly.

use pierre_mcp_server::services::chat_pipeline::stages::persona_conformance::{
    count_words, detects_label_value_block, has_unglossed_acronym, is_bullet_line,
    longest_bullet_run,
};

#[test]
fn counts_words_by_whitespace() {
    assert_eq!(count_words("hello   world\nthird"), 3);
    assert_eq!(count_words(""), 0);
}

#[test]
fn detects_dash_and_star_bullets() {
    assert!(is_bullet_line("- foo"));
    assert!(is_bullet_line("  * indented"));
    assert!(is_bullet_line("1. numbered"));
    assert!(!is_bullet_line("not a bullet"));
    assert!(!is_bullet_line("--- separator"));
}

#[test]
fn longest_run_collapses_on_text() {
    let text = "intro\n- a\n- b\n- c\nback to prose\n- single";
    assert_eq!(longest_bullet_run(text), 3);
}

#[test]
fn label_value_block_needs_two_consecutive() {
    assert!(detects_label_value_block(
        "Distance: 12 km\nTime: 1h05\nPace: 5:30 /km"
    ));
    assert!(!detects_label_value_block(
        "Distance: 12 km\n\nback to prose"
    ));
    assert!(!detects_label_value_block("URL: https://example.com"));
}

#[test]
fn unglossed_acronym_requires_paren_within_window() {
    assert!(has_unglossed_acronym("your CTL is rising", "CTL"));
    assert!(!has_unglossed_acronym(
        "your CTL (chronic load) is rising",
        "CTL"
    ));
    assert!(has_unglossed_acronym(
        "CTL early then CTL again somewhere later with no gloss anywhere near it",
        "CTL"
    ));
}
