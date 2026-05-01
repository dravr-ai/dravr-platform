// ABOUTME: Tests for the system-prompt placeholder schema (REQUIRED_*, missing_*, unsubstituted_*)
// ABOUTME: Guards the contremaitre hot-reload contract that drives the assembly layer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]
#![allow(missing_docs)]

//! Schema-level tests for [`pierre_llm::prompts`] — placeholder declaration,
//! drift detection, and the post-substitution scanner used by the assembly
//! layer to surface unsubstituted `{{IDENT}}` tokens.

use pierre_llm::prompts::{
    missing_placeholders, required_placeholders_for_system_prompt, unsubstituted_placeholders,
    PIERRE_SYSTEM_PROMPT, REQUIRED_SYSTEM_PROMPT_PLACEHOLDERS,
};

#[test]
fn pierre_system_required_placeholders_are_declared() {
    let required = required_placeholders_for_system_prompt("pierre_system")
        .expect("pierre_system must declare required placeholders");
    assert!(required.contains(&"{{SCOPE_REFUSAL}}"));
    assert!(required.contains(&"{{CAPABILITY_REFUSAL}}"));
    assert!(required.contains(&"{{COACH_SCOPE_CARVE_OUT}}"));
    assert!(required.contains(&"{{COACHING_PERSONA_RULES}}"));
}

#[test]
fn unknown_prompt_keys_have_no_required_placeholders() {
    assert!(required_placeholders_for_system_prompt("coach_generation").is_none());
    assert!(required_placeholders_for_system_prompt("nonexistent").is_none());
}

#[test]
fn compiled_pierre_system_satisfies_its_own_schema() {
    let required = required_placeholders_for_system_prompt("pierre_system").unwrap();
    let missing = missing_placeholders(PIERRE_SYSTEM_PROMPT, required);
    assert!(
        missing.is_empty(),
        "compiled-in PIERRE_SYSTEM_PROMPT is missing placeholders: {missing:?}"
    );
}

#[test]
fn missing_placeholders_reports_the_absent_ones() {
    let required = ["{{SCOPE_REFUSAL}}", "{{COACHING_PERSONA_RULES}}"];
    let content = "Some prompt body with {{SCOPE_REFUSAL}} but no persona section.";
    let missing = missing_placeholders(content, &required);
    assert_eq!(missing, vec!["{{COACHING_PERSONA_RULES}}"]);
}

#[test]
fn missing_placeholders_returns_empty_when_all_present() {
    let required = ["{{A}}", "{{B}}"];
    let content = "{{A}} and then {{B}}";
    assert!(missing_placeholders(content, &required).is_empty());
}

#[test]
fn unsubstituted_placeholders_finds_uppercase_idents() {
    let assembled = "rest of prompt {{COACHING_PERSONA_RULES}} after substitution";
    assert_eq!(
        unsubstituted_placeholders(assembled),
        vec!["{{COACHING_PERSONA_RULES}}".to_owned()]
    );
}

#[test]
fn unsubstituted_placeholders_finds_multiple() {
    let assembled = "{{A}} middle {{B}} end {{C}}";
    let found = unsubstituted_placeholders(assembled);
    assert_eq!(found, vec!["{{A}}", "{{B}}", "{{C}}"]);
}

#[test]
fn unsubstituted_placeholders_ignores_lowercase_or_mixed() {
    // JSON examples and lowercase tokens must NOT trigger.
    let assembled = "{{ \"key\": \"value\" }} and {{ foo_bar }} and {{Mixed}}";
    assert!(unsubstituted_placeholders(assembled).is_empty());
}

#[test]
fn unsubstituted_placeholders_ignores_empty_braces() {
    assert!(unsubstituted_placeholders("{{}} alone").is_empty());
}

#[test]
fn unsubstituted_placeholders_handles_underscore_and_digits() {
    // Digits are not part of the strict charset — only uppercase + underscore.
    let assembled = "{{FOO_BAR}} valid; {{VAR1}} invalid because of digit";
    let found = unsubstituted_placeholders(assembled);
    assert_eq!(found, vec!["{{FOO_BAR}}".to_owned()]);
}

#[test]
fn unsubstituted_placeholders_empty_when_clean() {
    let assembled = "fully substituted prompt with no braces";
    assert!(unsubstituted_placeholders(assembled).is_empty());
}

#[test]
fn placeholder_table_keys_are_unique() {
    let mut keys: Vec<&str> = REQUIRED_SYSTEM_PROMPT_PLACEHOLDERS
        .iter()
        .map(|(k, _)| *k)
        .collect();
    keys.sort_unstable();
    let original_len = keys.len();
    keys.dedup();
    assert_eq!(
        keys.len(),
        original_len,
        "duplicate keys in placeholder table"
    );
}
