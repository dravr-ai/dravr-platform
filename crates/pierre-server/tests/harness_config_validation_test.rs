// ABOUTME: Tests for the Phase B Sprint C3 admin harness config document validation
// ABOUTME: Pure unit tests on validate_document; route integration is covered by frontend tests
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::routes::admin::harness_config::{validate_document, HarnessConfigDocument};

#[test]
fn default_document_is_valid() {
    validate_document(&HarnessConfigDocument::default()).expect("default must validate");
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

#[test]
fn empty_disclaimer_with_triggers_is_rejected() {
    let mut doc = HarnessConfigDocument::default();
    doc.guardrails.disclaimer_text = String::new();
    assert!(validate_document(&doc).is_err());
}

#[test]
fn empty_triggers_allow_empty_disclaimer() {
    let mut doc = HarnessConfigDocument::default();
    doc.guardrails.disclaimer_triggers.clear();
    doc.guardrails.disclaimer_text = String::new();
    validate_document(&doc).expect("no triggers means disclaimer is unused");
}

#[test]
fn json_round_trip_preserves_fields() {
    // Compare via serialized JSON — the document contains f32 fields so it
    // cannot derive PartialEq cleanly (derive_partial_eq_without_eq fires
    // in clippy nursery), and f32 equality through Eq is not total.
    let doc = HarnessConfigDocument::default();
    let json = serde_json::to_string(&doc).expect("serialize");
    let parsed: HarnessConfigDocument = serde_json::from_str(&json).expect("deserialize");
    let reserialized = serde_json::to_string(&parsed).expect("reserialize");
    assert_eq!(reserialized, json);
}
