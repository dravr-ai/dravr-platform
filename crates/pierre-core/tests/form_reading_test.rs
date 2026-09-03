// ABOUTME: Content tests for FormReading — the one serializer every TSB surface renders through
// ABOUTME: Regression for 2026-09-02, when a bare TSB -77 reached an athlete as a diagnosis
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The number that ended the 2026-09-02 conversation was `-77`.
//!
//! It reached the coach with no percentage, no band and no method. The coach
//! named it *"ton indice de fatigue"*, placed it in *"la zone de surentraînement
//! profond"*, and built a race plan on it for an athlete in a deliberate
//! overload block. Asked to show the calculation, it could not — and it was
//! telling the truth.
//!
//! Every assertion here is about what travels *with* the number.

use pierre_core::models::{FormBand, FormReading};

/// The athlete from the incident: CTL 120, ATL 197, TSB -77.
fn raph() -> FormReading {
    FormReading::new(120.0, 197.0, -77.0)
}

#[test]
fn the_reading_carries_the_share_of_ctl_not_only_the_raw_number() {
    let reading = raph();

    let pct = reading.form_pct.expect("CTL 120 is a chronic base");
    assert!(
        (pct - (-64.1667)).abs() < 0.01,
        "-77 against CTL 120 is -64% of fitness, and that ratio is the whole \
         point: got {pct}"
    );
    assert_eq!(reading.band, FormBand::DeepFatigue);
}

#[test]
fn the_inline_form_names_the_band_beside_the_number() {
    let text = raph().inline();

    assert!(text.contains("-77"), "the raw number still travels: {text}");
    assert!(
        text.contains("-64% of CTL"),
        "so does the share of the athlete's own fitness: {text}"
    );
    assert!(
        text.contains("deep fatigue - form far below this athlete's own fitness"),
        "and so does the band's own reading, which is what stops the model \
         inventing one: {text}"
    );
}

/// The same TSB is a different reading for a different athlete. This is the
/// entire reason absolute banding was retired, and it must be visible in the
/// rendered text, not only in the enum.
#[test]
fn the_same_tsb_reads_differently_at_a_different_fitness() {
    let elite = FormReading::new(300.0, 377.0, -77.0);
    let novice = FormReading::new(40.0, 117.0, -77.0);

    assert_eq!(elite.band, FormBand::HeavyBlock);
    assert_eq!(novice.band, FormBand::DeepFatigue);
    assert!(
        elite.inline().contains("-26% of CTL"),
        "got: {}",
        elite.inline()
    );
    assert!(
        novice.inline().contains("-193% of CTL"),
        "the prose percentage must match what metrics_json puts on the wire \
         for the same reading: {}",
        novice.inline()
    );
    assert_eq!(
        novice.metrics_json()["tsb_pct_of_ctl"],
        -193.0,
        "and the wire must agree with the prose"
    );
}

#[test]
fn without_a_chronic_base_the_reading_says_so_instead_of_shipping_a_bare_number() {
    let reading = FormReading::new(0.5, 20.0, -19.7);

    assert!(reading.form_pct.is_none());
    assert_eq!(reading.band, FormBand::InsufficientHistory);
    assert!(
        reading
            .inline()
            .contains("no chronic base - form not interpretable"),
        "an un-normalizable TSB must carry its reason: {}",
        reading.inline()
    );
    assert!(
        !reading.inline().contains("% of CTL"),
        "there is nothing to take a percentage of: {}",
        reading.inline()
    );
}

#[test]
fn the_json_metrics_carry_the_band_and_the_percentage() {
    let value = raph().metrics_json();

    assert_eq!(value["tsb"], -77.0);
    assert_eq!(value["tsb_pct_of_ctl"], -64.0);
    assert_eq!(
        value["form_band"], "deep_fatigue",
        "the band serializes as the snake_case token every surface shares: {value}"
    );
}

/// The athlete asked *"Montre moi exactement comment tu calcules l'indice"* and
/// the coach could not answer. The payload now carries the method.
#[test]
fn the_interpretation_states_the_method_and_the_windows() {
    let value = FormReading::interpretation(42, 7);
    let method = value["method"].as_str().expect("method key must exist");

    assert!(
        method.contains("CTL - ATL"),
        "the formula itself must be stated: {method}"
    );
    assert!(
        method.contains("42") && method.contains('7'),
        "with the configured EMA windows, not hardcoded prose: {method}"
    );
    assert!(
        method.contains("exponentially-weighted"),
        "and how the averages are taken: {method}"
    );
}

/// The windows are read from config, so a tenant that retunes them gets an
/// interpretation that matches its own numbers rather than the defaults.
#[test]
fn the_interpretation_follows_the_configured_windows() {
    let value = FormReading::interpretation(28, 5);
    let method = value["method"].as_str().unwrap();

    assert!(
        method.contains("28") && method.contains('5'),
        "got: {method}"
    );
    assert!(
        !method.contains("42"),
        "a hardcoded 42 would be a second source of truth: {method}"
    );
}

#[test]
fn the_interpretation_separates_deep_fatigue_from_overtraining() {
    let value = FormReading::interpretation(42, 7);
    let note = value["deep_fatigue_is_not_overtraining"]
        .as_str()
        .expect("the distinction must be stated in the payload");

    assert!(
        note.contains("planned overload"),
        "the athlete was peaking on purpose and said so; the payload has to \
         allow for that: {note}"
    );
}

#[test]
fn no_key_in_the_interpretation_frames_form_as_injury_risk() {
    let value = FormReading::interpretation(42, 7);
    let serialized = value.to_string().to_lowercase();

    for banned in ["injury risk", "risk of injury", "dangerous", "red zone"] {
        assert!(
            !serialized.contains(banned),
            "form is a fatigue reading, never a risk verdict; found {banned:?} \
             in: {serialized}"
        );
    }
}
