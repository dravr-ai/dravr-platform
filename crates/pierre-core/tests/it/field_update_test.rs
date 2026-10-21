// ABOUTME: Unit tests for FieldUpdate — absent vs explicit null vs a supplied value
// ABOUTME: Pins the three-way contract an update DTO relies on to make a set column clearable

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::field_update::FieldUpdate;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize, Serialize)]
struct Patch {
    #[serde(default, skip_serializing_if = "FieldUpdate::is_keep")]
    budget: FieldUpdate<i32>,
}

#[test]
fn an_absent_key_deserializes_to_keep() {
    let patch: Patch = serde_json::from_value(json!({})).unwrap();

    assert_eq!(patch.budget, FieldUpdate::Keep);
    assert_eq!(patch.budget.resolve(Some(23)), Some(23));
    assert_eq!(patch.budget.assigned(), None);
}

#[test]
fn an_explicit_null_deserializes_to_a_clear() {
    let patch: Patch = serde_json::from_value(json!({ "budget": null })).unwrap();

    assert_eq!(patch.budget, FieldUpdate::Set(None));
    assert_eq!(patch.budget.resolve(Some(23)), None);
    assert_eq!(patch.budget.assigned(), None);
}

#[test]
fn a_supplied_value_deserializes_to_a_set() {
    let patch: Patch = serde_json::from_value(json!({ "budget": 7 })).unwrap();

    assert_eq!(patch.budget, FieldUpdate::Set(Some(7)));
    assert_eq!(patch.budget.resolve(Some(23)), Some(7));
    assert_eq!(patch.budget.assigned(), Some(7));
}

#[test]
fn resolve_on_a_column_that_is_already_empty() {
    assert_eq!(FieldUpdate::<i32>::Keep.resolve(None), None);
    assert_eq!(FieldUpdate::Set(Some(4)).resolve(None), Some(4));
    assert_eq!(FieldUpdate::<i32>::Set(None).resolve(None), None);
}

#[test]
fn keep_serializes_as_an_absent_key_and_a_clear_as_null() {
    let keep = serde_json::to_value(Patch {
        budget: FieldUpdate::Keep,
    })
    .unwrap();
    assert_eq!(keep, json!({}));

    let cleared = serde_json::to_value(Patch {
        budget: FieldUpdate::Set(None),
    })
    .unwrap();
    assert_eq!(cleared, json!({ "budget": null }));

    let set = serde_json::to_value(Patch {
        budget: FieldUpdate::Set(Some(31)),
    })
    .unwrap();
    assert_eq!(set, json!({ "budget": 31 }));
}

#[test]
fn a_serialized_patch_round_trips_through_json() {
    for original in [
        FieldUpdate::Keep,
        FieldUpdate::Set(None),
        FieldUpdate::Set(Some(12)),
    ] {
        let encoded = serde_json::to_string(&Patch { budget: original }).unwrap();
        let decoded: Patch = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.budget, original, "round trip of {original:?}");
    }
}

#[test]
fn is_keep_only_answers_true_for_the_untouched_state() {
    assert!(FieldUpdate::<i32>::Keep.is_keep());
    assert!(!FieldUpdate::<i32>::Set(None).is_keep());
    assert!(!FieldUpdate::Set(Some(1)).is_keep());
}
