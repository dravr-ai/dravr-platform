// ABOUTME: External tests for the Layer 2 per-category bounds checker (deterministic_bounds.rs)
// ABOUTME: Verifies absurd physiological/nutrition/supplement values are flagged, safe ones pass
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::claim_extractor::ExtractedClaim;
use pierre_evals::deterministic_bounds::check;
use pierre_memory::ClaimCategory;

fn claim(text: &str, category: ClaimCategory) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category,
    }
}

#[test]
fn flags_absurd_max_heart_rate() {
    let v = check(&claim(
        "Your max heart rate is 300 bpm.",
        ClaimCategory::Physiological,
    ));
    assert!(v.is_some());
}

#[test]
fn allows_plausible_max_heart_rate() {
    let v = check(&claim(
        "Your max heart rate is 190 bpm.",
        ClaimCategory::Physiological,
    ));
    assert!(v.is_none());
}

#[test]
fn flags_implausible_vo2max() {
    let v = check(&claim(
        "Your VO2max of 150 ml/kg/min is world-class.",
        ClaimCategory::Physiological,
    ));
    assert!(v.is_some());
}

#[test]
fn flags_absurd_protein_intake() {
    let v = check(&claim(
        "Eat 20 grams per kg of body weight daily.",
        ClaimCategory::Nutrition,
    ));
    assert!(v.is_some());
}

#[test]
fn flags_creatine_overdose() {
    let v = check(&claim(
        "Take 200 g/day of creatine.",
        ClaimCategory::Supplement,
    ));
    assert!(v.is_some());
}

#[test]
fn allows_safe_creatine_dose() {
    let v = check(&claim(
        "Take 5 g/day of creatine.",
        ClaimCategory::Supplement,
    ));
    assert!(v.is_none());
}
