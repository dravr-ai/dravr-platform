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
    PIERRE_SYSTEM_PROMPT, PLATFORM_CONTRACT_PROMPT, REQUIRED_SYSTEM_PROMPT_PLACEHOLDERS,
};

/// Every placeholder the assembly layer substitutes, and the prompt that must
/// carry it. The requirement follows the content: the contract split moved
/// four of the five out of the persona block, and a stale requirement here
/// rejects the correctly-split prompt at sync time instead of catching a real
/// regression (live alert 2026-08-11).
#[test]
fn every_substituted_placeholder_is_required_of_exactly_one_prompt() {
    let contract = required_placeholders_for_system_prompt("platform_contract")
        .expect("the platform contract must declare its placeholders");

    assert!(
        required_placeholders_for_system_prompt("pierre_system").is_none(),
        "pierre_system is a replaceable voice layer — a bound coach swaps it \
         out wholesale, so any placeholder required of it is lost on coach \
         turns (that is how coach-bound persona steering silently vanished \
         until 2026-09-01)"
    );
    for placeholder in [
        "{{SCOPE_REFUSAL}}",
        "{{CAPABILITY_REFUSAL}}",
        "{{COACH_SCOPE_CARVE_OUT}}",
        "{{CURRENT_DATE}}",
        "{{COACHING_PERSONA_RULES}}",
    ] {
        assert!(
            contract.contains(&placeholder),
            "{placeholder} belongs to the always-injected contract"
        );
    }
}

#[test]
fn compiled_platform_contract_satisfies_its_own_schema() {
    let required = required_placeholders_for_system_prompt("platform_contract").unwrap();
    let missing = missing_placeholders(PLATFORM_CONTRACT_PROMPT, required);
    assert!(
        missing.is_empty(),
        "compiled-in PLATFORM_CONTRACT_PROMPT is missing placeholders: {missing:?}"
    );
}

#[test]
fn unknown_prompt_keys_have_no_required_placeholders() {
    assert!(required_placeholders_for_system_prompt("coach_generation").is_none());
    assert!(required_placeholders_for_system_prompt("nonexistent").is_none());
}

#[test]
fn compiled_pierre_system_needs_no_required_placeholders() {
    // The persona slot's canonical home is platform_contract (2026-09-01).
    // pierre_system may transitionally still carry a copy — deployed
    // binaries hot-sync contremaitre main, and the copy stays until every
    // binary enforcing the old per-file requirement is gone — so this test
    // pins only that nothing is REQUIRED of the replaceable layer.
    assert!(required_placeholders_for_system_prompt("pierre_system").is_none());
    let _ = PIERRE_SYSTEM_PROMPT;
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
