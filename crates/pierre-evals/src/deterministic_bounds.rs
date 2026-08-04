// ABOUTME: The deterministic-bounds layer of the bullshit detector — hard-coded per-category physiological bounds
// ABOUTME: Pure Rust, no LLM. Cheap first-pass sanity check on every claim.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Deterministic Bounds (population bounds)
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
//! leaving the evidence-retrieval layer to deal with the subtle cases.

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
/// otherwise (meaning the claim is free to continue to the evidence-retrieval layer).
#[must_use]
pub fn check(claim: &ExtractedClaim) -> Option<BoundViolation> {
    // The claim is lowercased once here and every probe below shares that one
    // copy: a category check runs up to six keyword scans plus its own
    // `contains` gates, and this runs on every claim of every verified reply.
    // This string is also the domain of every offset those scans compute — see
    // `extract_number_near_lowercased`.
    let lower = claim.text.to_lowercase();
    match claim.category {
        ClaimCategory::Physiological => check_physiological(&lower),
        ClaimCategory::TrainingPrescription => check_training(&lower),
        ClaimCategory::Nutrition => check_nutrition(&lower),
        ClaimCategory::Recovery => check_recovery(&lower),
        ClaimCategory::Supplement => check_supplement(&lower),
        ClaimCategory::InjuryRehab => None,
    }
}

/// How far to either side of the keyword the numeric scan reaches, in bytes.
///
/// Wide enough to span the clause a value normally shares with its keyword
/// ("your `VO2max` of 55 ml/kg/min"), narrow enough that a number belonging to
/// the next sentence is not attributed to this keyword.
const WINDOW_BYTES: usize = 25;

/// The largest char boundary at or below `i`.
///
/// `str::floor_char_boundary` is still unstable, so this is hand-rolled.
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The smallest char boundary at or above `i`.
///
/// `str::ceil_char_boundary` is still unstable, so this is hand-rolled.
fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Extract the numeric value nearest `keyword` in `text`.
///
/// The entry point for callers holding raw claim text: it owns the one
/// lowercased copy the scan needs. Probes inside this module get theirs from
/// [`check`], which lowercases once for the whole claim, and call
/// `extract_number_near_lowercased` directly.
pub(crate) fn extract_number_near(text: &str, keyword: &str) -> Option<f64> {
    extract_number_near_lowercased(&text.to_lowercase(), keyword)
}

/// Extract the numeric value nearest `keyword` in `lower`, which is the claim
/// text already lowercased.
///
/// Every offset computed here indexes `lower`, so the lowercased copy the
/// caller passes in is the single authoritative domain. `to_lowercase` is not
/// length-preserving — "İ" is 2 bytes and lowercases to 3 — so an offset found
/// in the lowercased text and applied to the original reads a window shifted
/// off the keyword, which extracts the wrong number without any outward sign.
/// Reading digits out of the lowercased text is equivalent because lowercasing
/// leaves ASCII digits and `.` untouched, and those are all this scanner
/// consumes.
fn extract_number_near_lowercased(lower: &str, keyword: &str) -> Option<f64> {
    let idx = lower.find(keyword)?;
    let kw_end = idx + keyword.len();

    // Scan a symmetric window around the keyword, return the numeric token
    // whose starting position is closest to the keyword (either side).
    //
    // The window bounds are byte arithmetic with no relationship to character
    // boundaries, so both are snapped outward to the nearest one. Without the
    // snap any accented reply can put a bound inside a multi-byte character
    // and slicing there panics — on 2026-07-28 a French coach reply did
    // exactly that at `window_start`, taking down a turn that had already
    // saved a training plan.
    let window_start = floor_char_boundary(lower, idx.saturating_sub(WINDOW_BYTES));
    let window_end = ceil_char_boundary(lower, (kw_end + WINDOW_BYTES).min(lower.len()));
    let window = &lower[window_start..window_end];
    let kw_center_in_window = idx - window_start;
    // The keyword's own span, in window coordinates. Two of the keywords
    // ("vo2max", "vo2 max") contain an ASCII digit, and reading it back as
    // the athlete's value made every bare mention of VO2max a claim of "2
    // ml/kg/min" — out of range, so a sentence stating no value at all got
    // contradicted. The keyword is the anchor of the search, never a value
    // found by it.
    let kw_span_in_window = kw_center_in_window..(kw_end - window_start);

    let mut best: Option<(f64, usize)> = None;
    let mut buf = String::new();
    let mut token_start: Option<usize> = None;

    for (offset, ch) in window.char_indices() {
        if (ch.is_ascii_digit() || ch == '.') && !kw_span_in_window.contains(&offset) {
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

fn check_physiological(lower: &str) -> Option<BoundViolation> {
    // HR max: humans 60–230 bpm, with 250+ biologically implausible.
    for kw in ["max heart rate", "hrmax", "hr max", "maximum heart rate"] {
        if let Some(v) = extract_number_near_lowercased(lower, kw) {
            if !(60.0..=250.0).contains(&v) {
                return Some(BoundViolation {
                    reason: format!("max heart rate of {v} bpm is outside 60-250 bpm"),
                });
            }
        }
    }
    // VO2max: 10 (sedentary elderly) to 95 (world-class XC skier).
    for kw in ["vo2max", "vo2 max"] {
        if let Some(v) = extract_number_near_lowercased(lower, kw) {
            if !(10.0..=95.0).contains(&v) {
                return Some(BoundViolation {
                    reason: format!("VO2max of {v} ml/kg/min is outside 10-95 range"),
                });
            }
        }
    }
    None
}

fn check_training(lower: &str) -> Option<BoundViolation> {
    // Weekly volume: single-week endurance running max ~250 km.
    for kw in ["km per week", "km/week", "weekly mileage"] {
        if let Some(v) = extract_number_near_lowercased(lower, kw) {
            if v > 400.0 {
                return Some(BoundViolation {
                    reason: format!("{v} km/week is beyond any documented training volume"),
                });
            }
        }
    }
    None
}

fn check_nutrition(lower: &str) -> Option<BoundViolation> {
    // Protein: 0.8 to 3.0 g/kg/day spans the scientific consensus.
    for kw in ["g per kg", "g/kg", "grams per kg"] {
        if let Some(v) = extract_number_near_lowercased(lower, kw) {
            if v > 5.0 {
                return Some(BoundViolation {
                    reason: format!("{v} g/kg is far above the 0.8-3.0 g/kg nutrition range"),
                });
            }
        }
    }
    // Hydration: 1-3 L/hour during exercise; 10 L/hour is dangerous.
    if lower.contains("water") {
        for kw in ["liters per hour", "l per hour", "l/hour"] {
            if let Some(v) = extract_number_near_lowercased(lower, kw) {
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

fn check_recovery(lower: &str) -> Option<BoundViolation> {
    // Sleep: 4-12 hours typical, 0 or >16 implausible.
    for kw in ["hours of sleep", "hours sleep"] {
        if let Some(v) = extract_number_near_lowercased(lower, kw) {
            if !(2.0..=16.0).contains(&v) {
                return Some(BoundViolation {
                    reason: format!("{v} hours of sleep is outside 2-16 hour range"),
                });
            }
        }
    }
    None
}

fn check_supplement(lower: &str) -> Option<BoundViolation> {
    // Creatine: 3-20 g/day loading, 3-5 g/day maintenance. 100 g/day is absurd.
    if lower.contains("creatine") {
        for kw in ["g per day", "g/day", "grams per day"] {
            if let Some(v) = extract_number_near_lowercased(lower, kw) {
                if v > 50.0 {
                    return Some(BoundViolation {
                        reason: format!("{v} g/day creatine is far above any protocol"),
                    });
                }
            }
        }
    }
    // Caffeine: 3-9 mg/kg is the ergogenic range; 20 mg/kg is cardiotoxic.
    if lower.contains("caffeine") {
        for kw in ["mg per kg", "mg/kg"] {
            if let Some(v) = extract_number_near_lowercased(lower, kw) {
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
