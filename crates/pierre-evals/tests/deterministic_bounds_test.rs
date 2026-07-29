// ABOUTME: External tests for the deterministic-bounds per-category checker (deterministic_bounds.rs)
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
    ))
    .expect("150 ml/kg/min is outside the 10-95 VO2max bound");
    // The number in the reason has to be the athlete's 150, not the "2" that
    // sits inside the keyword "VO2max" itself. This assertion is what
    // separates a real bound check from one that only ever reads its own
    // keyword: before the scanner learned to skip the keyword span it
    // returned 2.0 here, which is also out of range, so the bare
    // `is_some()` this test used to make was green for the wrong reason.
    assert!(
        v.reason.contains("150"),
        "expected the athlete's value in the reason, got {}",
        v.reason
    );
}

/// A `VO2max` mention carrying no number of its own must not be flagged.
///
/// The keyword "vo2max" contains an ASCII digit. A scanner that reads the
/// whole window including the keyword finds that `2`, decides the coach
/// claimed a `VO2max` of 2 ml/kg/min, and contradicts a sentence that stated
/// no value at all — a user-facing "unsupported claims" banner on every
/// reply that merely says the word.
#[test]
fn a_bare_vo2max_mention_carries_no_number_to_bound_check() {
    let v = check(&claim(
        "Ton VO2max reste stable cette saison.",
        ClaimCategory::Physiological,
    ));
    assert!(
        v.is_none(),
        "a claim with no numeric value must not violate a bound, got {v:?}"
    );
}

/// Production repro — `dravr-mcp-server-api-00736-wk5`, 2026-07-28 21:52:43 UTC.
///
/// `deterministic_bounds.rs:56` panicked with "start byte index 23 is not a
/// char boundary; it is inside 'é' (bytes 22..24 of string)", discarding a
/// turn whose `save_training_plan` call had committed 6 seconds earlier.
///
/// The geometry is reproduced exactly rather than approximately, down to the
/// offsets in that message: "vo2max" is found at byte 48, so `window_start` is
/// 48 - 25 = 23, and the `é` occupies bytes 22..24 — byte 23 is its
/// continuation byte.
#[test]
fn window_start_landing_inside_a_multibyte_char_does_not_panic() {
    let text = "Semaine 4 pic, dernixxérezzzzzzzzzzzzzzzzzzzzzzVO2max maintenu sans augmenter";
    assert!(
        !text.is_char_boundary(23),
        "repro geometry drifted: byte 23 must be inside a multi-byte char"
    );
    // The window holds no digit other than the keyword's own, so there is no
    // value to bound-check and the claim passes.
    let v = check(&claim(text, ClaimCategory::Physiological));
    assert!(v.is_none(), "no numeric value in the window, got {v:?}");
}

/// The other bound: a multi-byte character straddling `keyword_end + 25`.
///
/// "vo2max" ends at byte 6, so the window ends at byte 31, which is the
/// continuation byte of the `é` in "séance" (bytes 30..32).
#[test]
fn window_end_landing_inside_a_multibyte_char_does_not_panic() {
    let text = "VO2max de 55.0 ml/kg/min, ta séance";
    assert!(
        !text.is_char_boundary(31),
        "repro geometry drifted: byte 31 must be inside a multi-byte char"
    );
    // 55.0 ml/kg/min is a perfectly ordinary VO2max, so the extraction has to
    // both survive the accent and read the real number.
    let v = check(&claim(text, ClaimCategory::Physiological));
    assert!(v.is_none(), "55.0 is inside the 10-95 bound, got {v:?}");
}

/// `to_lowercase()` is not length-preserving, so an offset found in the
/// lowercased string must be applied to that same string.
///
/// Geometry, in lowercased-string bytes: `İ` is 2 bytes and lowercases to 3
/// (`i` + U+0307), so every offset past it is one larger in the lowercased
/// string than in the original. "g/day" starts at byte 15; "99" starts 6
/// bytes before it and "44" 6 bytes after, leaving the two equidistant so the
/// earlier one wins. Applying the lowercased offsets to the original text —
/// what the scanner used to do — puts the keyword one byte to the left of
/// where it really is, which makes "44" look like the nearer token. 99 g/day
/// of creatine violates the bound; 44 does not, so the wrong window silently
/// waved through an overdose.
#[test]
fn a_length_changing_lowercase_does_not_shift_the_window() {
    let v = check(&claim(
        "İabcde 99 xy g/day 44 creatine",
        ClaimCategory::Supplement,
    ))
    .expect("99 g/day of creatine is far above any protocol");
    assert!(
        v.reason.contains("99"),
        "expected the nearer token (99) in the reason, got {}",
        v.reason
    );
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
