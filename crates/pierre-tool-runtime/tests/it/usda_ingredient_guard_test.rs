// ABOUTME: One fan-out guard for both USDA tools — the cap holds and the refusal names the numbers
// ABOUTME: analyze_meal_nutrition and validate_recipe bound the same array against the same constant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `validate_recipe` costs two USDA calls per ingredient and
//! `analyze_meal_nutrition` one, so both bound the caller's array by a
//! constant. The check was copy-pasted beside each call site, with one copy
//! carrying a `validate_recipe:` prefix the other lacked — a difference that
//! reaches the athlete as two different messages for one rule.
//!
//! The boundary is asserted on both sides: exactly the cap must pass, because a
//! guard that refused at the cap would silently cost a real recipe its last
//! ingredient.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_tool_runtime::implementations::usda_shared::{
    check_ingredient_count, MAX_USDA_INGREDIENTS,
};
use serde_json::{json, Value};

fn ingredients(count: usize) -> Vec<Value> {
    (0..count)
        .map(|i| json!({"name": format!("item {i}")}))
        .collect()
}

#[test]
fn an_array_at_the_cap_is_allowed() {
    assert!(
        check_ingredient_count(&ingredients(MAX_USDA_INGREDIENTS)).is_ok(),
        "exactly {MAX_USDA_INGREDIENTS} ingredients is a real recipe, not an abuse"
    );
}

#[test]
fn an_empty_array_is_allowed() {
    assert!(check_ingredient_count(&[]).is_ok());
}

#[test]
fn one_past_the_cap_is_refused_and_names_both_numbers() {
    let over = MAX_USDA_INGREDIENTS + 1;
    let err = check_ingredient_count(&ingredients(over))
        .expect_err("one past the cap must refuse before any USDA call");
    let message = err.to_string();
    assert!(
        message.contains(&over.to_string()),
        "the refusal must name the length it saw, got: {message}"
    );
    assert!(
        message.contains(&format!("max {MAX_USDA_INGREDIENTS}")),
        "the refusal must name the cap so the caller can trim to it, got: {message}"
    );
}
