// ABOUTME: Layer 2 of the bullshit detector — hard-coded per-category physiological bounds
// ABOUTME: Pure Rust, no LLM. Cheap first-pass sanity check on every claim.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Deterministic Bounds (Layer 2)
//!
//! Implausibility filter. For each category, extracts numeric values and
//! their units from the claim text and compares them against conservative
//! physiological / training / nutrition bounds. If any value is clearly
//! outside the range a competent coach would give, the pipeline
//! short-circuits with `ClaimStatus::Contradicted` and a `Deterministic`
//! layer attribution.
//!
//! These bounds are intentionally wide: they should catch only obvious
//! nonsense ("HR max of 300 bpm", "take 500g of creatine per day"),
//! leaving Layer 3 evidence retrieval to deal with the subtle cases.

use crate::claim_extractor::ExtractedClaim;
use pierre_memory::ClaimCategory;

/// A bound that was violated by a claim, with the reason that should be
/// surfaced to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundViolation {
    /// Human-readable description of the violation.
    pub reason: String,
}

/// Check a claim against the deterministic bounds for its category.
///
/// Returns `Some(BoundViolation)` if a bound is clearly violated, `None`
/// otherwise (meaning the claim is free to continue to Layer 3).
#[must_use]
pub fn check(claim: &ExtractedClaim) -> Option<BoundViolation> {
    match claim.category {
        ClaimCategory::Physiological => check_physiological(&claim.text),
        ClaimCategory::TrainingPrescription => check_training(&claim.text),
        ClaimCategory::Nutrition => check_nutrition(&claim.text),
        ClaimCategory::Recovery => check_recovery(&claim.text),
        ClaimCategory::Supplement => check_supplement(&claim.text),
        ClaimCategory::InjuryRehab => None,
    }
}

fn extract_number_near(text: &str, keyword: &str) -> Option<f64> {
    let lower = text.to_lowercase();
    let idx = lower.find(keyword)?;
    let kw_end = idx + keyword.len();

    // Scan a symmetric window around the keyword, return the numeric token
    // whose starting position is closest to the keyword (either side).
    let window_start = idx.saturating_sub(25);
    let window_end = (kw_end + 25).min(text.len());
    let window = &text[window_start..window_end];
    let kw_center_in_window = idx - window_start;

    let mut best: Option<(f64, usize)> = None;
    let mut buf = String::new();
    let mut token_start: Option<usize> = None;

    for (offset, ch) in window.char_indices() {
        if ch.is_ascii_digit() || ch == '.' {
            if buf.is_empty() {
                token_start = Some(offset);
            }
            buf.push(ch);
        } else if !buf.is_empty() {
            if let (Ok(v), Some(start)) = (buf.parse::<f64>(), token_start) {
                let distance = start.abs_diff(kw_center_in_window);
                if best.is_none_or(|(_, d)| distance < d) {
                    best = Some((v, distance));
                }
            }
            buf.clear();
            token_start = None;
        }
    }
    if !buf.is_empty() {
        if let (Ok(v), Some(start)) = (buf.parse::<f64>(), token_start) {
            let distance = start.abs_diff(kw_center_in_window);
            if best.is_none_or(|(_, d)| distance < d) {
                best = Some((v, distance));
            }
        }
    }
    best.map(|(v, _)| v)
}

fn check_physiological(text: &str) -> Option<BoundViolation> {
    // HR max: humans 60–230 bpm, with 250+ biologically implausible.
    for kw in ["max heart rate", "hrmax", "hr max", "maximum heart rate"] {
        if let Some(v) = extract_number_near(text, kw) {
            if !(60.0..=250.0).contains(&v) {
                return Some(BoundViolation {
                    reason: format!("max heart rate of {v} bpm is outside 60-250 bpm"),
                });
            }
        }
    }
    // VO2max: 10 (sedentary elderly) to 95 (world-class XC skier).
    for kw in ["vo2max", "vo2 max"] {
        if let Some(v) = extract_number_near(text, kw) {
            if !(10.0..=95.0).contains(&v) {
                return Some(BoundViolation {
                    reason: format!("VO2max of {v} ml/kg/min is outside 10-95 range"),
                });
            }
        }
    }
    None
}

fn check_training(text: &str) -> Option<BoundViolation> {
    // Weekly volume: single-week endurance running max ~250 km.
    for kw in ["km per week", "km/week", "weekly mileage"] {
        if let Some(v) = extract_number_near(text, kw) {
            if v > 400.0 {
                return Some(BoundViolation {
                    reason: format!("{v} km/week is beyond any documented training volume"),
                });
            }
        }
    }
    None
}

fn check_nutrition(text: &str) -> Option<BoundViolation> {
    // Protein: 0.8 to 3.0 g/kg/day spans the scientific consensus.
    for kw in ["g per kg", "g/kg", "grams per kg"] {
        if let Some(v) = extract_number_near(text, kw) {
            if v > 5.0 {
                return Some(BoundViolation {
                    reason: format!("{v} g/kg is far above the 0.8-3.0 g/kg nutrition range"),
                });
            }
        }
    }
    // Hydration: 1-3 L/hour during exercise; 10 L/hour is dangerous.
    if text.to_lowercase().contains("water") {
        for kw in ["liters per hour", "l per hour", "l/hour"] {
            if let Some(v) = extract_number_near(text, kw) {
                if v > 4.0 {
                    return Some(BoundViolation {
                        reason: format!("{v} L/hour hydration risks hyponatremia"),
                    });
                }
            }
        }
    }
    None
}

fn check_recovery(text: &str) -> Option<BoundViolation> {
    // Sleep: 4-12 hours typical, 0 or >16 implausible.
    for kw in ["hours of sleep", "hours sleep"] {
        if let Some(v) = extract_number_near(text, kw) {
            if !(2.0..=16.0).contains(&v) {
                return Some(BoundViolation {
                    reason: format!("{v} hours of sleep is outside 2-16 hour range"),
                });
            }
        }
    }
    None
}

fn check_supplement(text: &str) -> Option<BoundViolation> {
    // Creatine: 3-20 g/day loading, 3-5 g/day maintenance. 100 g/day is absurd.
    if text.to_lowercase().contains("creatine") {
        for kw in ["g per day", "g/day", "grams per day"] {
            if let Some(v) = extract_number_near(text, kw) {
                if v > 50.0 {
                    return Some(BoundViolation {
                        reason: format!("{v} g/day creatine is far above any protocol"),
                    });
                }
            }
        }
    }
    // Caffeine: 3-9 mg/kg is the ergogenic range; 20 mg/kg is cardiotoxic.
    if text.to_lowercase().contains("caffeine") {
        for kw in ["mg per kg", "mg/kg"] {
            if let Some(v) = extract_number_near(text, kw) {
                if v > 15.0 {
                    return Some(BoundViolation {
                        reason: format!("{v} mg/kg caffeine is beyond safe ergogenic dose"),
                    });
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
