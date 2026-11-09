// ABOUTME: Tests for the Phase B Sprint C3 admin harness config document validation
// ABOUTME: Pure unit tests on validate_document; route integration is covered by frontend tests
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_contremaitre::harness_config_document::{validate_document, HarnessConfigDocument};

#[test]
fn default_document_is_valid() {
    validate_document(&HarnessConfigDocument::default()).expect("default must validate");
}

#[test]
fn summarize_oldest_n_plus_two_above_max_messages_is_rejected() {
    let mut doc = HarnessConfigDocument::default();
    // 6 + 2 = 8 > 7: summarization could never run before the cap forces a slide.
    doc.compaction.max_messages = 7;
    doc.compaction.summarize_oldest_n = 6;
    assert!(validate_document(&doc).is_err());
}

#[test]
fn summarize_oldest_n_plus_two_equal_to_max_messages_is_valid() {
    let mut doc = HarnessConfigDocument::default();
    // 6 + 2 = 8 <= 8: exactly enough room to compact and keep the last pair.
    doc.compaction.max_messages = 8;
    doc.compaction.summarize_oldest_n = 6;
    validate_document(&doc).expect("n + 2 == max_messages must validate");
}

/// The compactor needs a run of 2 turns: 0 or 1 passed validation and silently
/// turned every over-cap thread into a raw slide with no summary.
#[test]
fn summarize_oldest_n_below_what_a_summary_can_compact_is_rejected() {
    for n in [0, 1] {
        let mut doc = HarnessConfigDocument::default();
        doc.compaction.summarize_oldest_n = n;
        assert!(validate_document(&doc).is_err(), "n = {n}");
    }
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.summarize_oldest_n = 2;
    validate_document(&doc).expect("2 is the shortest run a summary compacts");
}

#[test]
fn warn_must_be_strictly_less_than_emergency() {
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.warn_threshold = 0.95;
    doc.compaction.emergency_threshold = 0.95;
    assert!(validate_document(&doc).is_err());
}

#[test]
fn warn_above_emergency_is_rejected() {
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.warn_threshold = 0.99;
    doc.compaction.emergency_threshold = 0.7;
    assert!(validate_document(&doc).is_err());
}

#[test]
fn zero_window_tokens_is_rejected() {
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.window_tokens = 0;
    assert!(validate_document(&doc).is_err());
}

#[test]
fn zero_warn_threshold_is_rejected() {
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.warn_threshold = 0.0;
    assert!(validate_document(&doc).is_err());
}

#[test]
fn zero_emergency_threshold_is_rejected() {
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.emergency_threshold = 0.0;
    assert!(validate_document(&doc).is_err());
}

#[test]
fn negative_warn_threshold_is_rejected() {
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.warn_threshold = -0.1;
    assert!(validate_document(&doc).is_err());
}

#[test]
fn warn_threshold_above_one_is_rejected() {
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.warn_threshold = 1.5;
    assert!(validate_document(&doc).is_err());
}

/// A row written before the copy moved to the catalogue still parses, and the text
/// it carries is ignored: reading it would leave two sources for one sentence.
#[test]
fn a_row_with_the_old_length_cap_and_disclaimer_text_still_parses() {
    let mut value = serde_json::to_value(HarnessConfigDocument::default()).expect("to value");
    value["guardrails"]["max_response_chars"] = serde_json::json!(5_000);
    value["guardrails"]["locales"]["en"]["disclaimer_text"] = serde_json::json!("Old copy.");
    let parsed: HarnessConfigDocument = serde_json::from_value(value).expect("an old row parses");
    validate_document(&parsed).expect("and is valid");
    assert_eq!(
        parsed.guardrails.locales["en"].disclaimer_triggers,
        HarnessConfigDocument::default().guardrails.locales["en"].disclaimer_triggers
    );
}

#[test]
fn empty_locales_map_is_valid() {
    // Operators are allowed to fully disable the disclaimer feature by
    // clearing the locales map. The pass-through behavior is documented
    // on TextGuardrails::resolve_locale.
    let mut doc = HarnessConfigDocument::default();
    doc.guardrails.locales.clear();
    validate_document(&doc).expect("empty locales disables disclaimers");
}

#[test]
fn json_round_trip_preserves_fields() {
    // Compare via `serde_json::Value` rather than raw strings: the
    // document carries a `HashMap<String, LocaleGuardrails>` whose
    // iteration order is non-deterministic, so byte-equal serialization
    // is not guaranteed even when the data is identical. `Value`
    // equality is structural (recursive map / array compare).
    //
    // Both sides go through `to_value` so the f32 → f64 conversion is
    // applied identically — comparing the raw parsed JSON Value against
    // a doc-to-Value would fail on `0.7_f32` rendering as
    // `0.699999988079071_f64` once it round-trips through f32.
    let doc = HarnessConfigDocument::default();
    let json = serde_json::to_string(&doc).expect("serialize");
    let parsed: HarnessConfigDocument = serde_json::from_str(&json).expect("deserialize");
    let lhs = serde_json::to_value(&doc).expect("default to Value");
    let rhs = serde_json::to_value(&parsed).expect("parsed to Value");
    assert_eq!(lhs, rhs);
}
