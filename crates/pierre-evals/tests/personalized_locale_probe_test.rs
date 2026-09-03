// ABOUTME: Pins that the personalized numeric probes fire in every shipped locale, not only English
// ABOUTME: Regression for 2026-09-02 — a French coach quoted a metric fifteen turns, never checked
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Every probe in the personalized layer anchored on an English keyword before
//! it would check a number. A reply in any of the other four shipped locales
//! matched none of them, so the layer was blind on those turns — and the first
//! production users are French-speaking.
//!
//! It mattered more than a normal i18n gap because the *other* verifier is
//! locale-aware. On 2026-09-02 the athlete saw "je n'ai pas pu étayer" warnings
//! on four ordinary coaching prescriptions while the one hard number he was
//! repeatedly quoted — and eventually lost confidence over — went unchecked
//! (registre#204).

use pierre_evals::personalized::check as personalized_check;
use pierre_evals::{
    AthleteMetrics, ConservativeStrategy, ExtractedClaim, PersonalizedContext, ToleranceStrategy,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus};

fn metrics() -> AthleteMetrics {
    AthleteMetrics {
        vdot: Some(52.0),
        vo2max: Some(52.0),
        easy_pace_range: Some((360.0, 400.0)),
        threshold_pace_range: Some((300.0, 315.0)),
        max_hr: Some(190.0),
        ftp_watts: Some(380.0),
        recent_tsb: Some(-77.0),
        data_days: 30,
        ..AthleteMetrics::default()
    }
}

fn claim(text: &str) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category: ClaimCategory::Physiological,
    }
}

fn check(text: &str) -> Option<ClaimStatus> {
    let strategy: Box<dyn ToleranceStrategy> = Box::new(ConservativeStrategy::default());
    let m = metrics();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: strategy.as_ref(),
    };
    personalized_check(&claim(text), &ctx).map(|v| v.status)
}

/// The athlete's FTP is 380 W. A reply that misstates it must be caught in the
/// language the reply is written in.
#[test]
fn a_wrong_ftp_is_caught_in_every_shipped_locale() {
    for text in [
        "Your FTP is 250 watts.",
        "Ton seuil de puissance est de 250 watts.",
        "Tu potencia umbral es de 250 vatios.",
        "Deine Schwellenleistung liegt bei 250 Watt.",
        "A tua potência limiar é de 250 watts.",
    ] {
        assert_eq!(
            check(text),
            Some(ClaimStatus::Contradicted),
            "a five-locale platform cannot verify only one of them: {text:?}"
        );
    }
}

/// And a correct one still reads as supported, so localization did not simply
/// make the layer noisier.
#[test]
fn a_correct_ftp_is_supported_in_every_shipped_locale() {
    for text in [
        "Your FTP is 380 watts.",
        "Ton seuil de puissance est de 380 watts.",
        "Tu potencia umbral es de 380 vatios.",
        "Deine Schwellenleistung liegt bei 380 Watt.",
        "A tua potência limiar é de 380 watts.",
    ] {
        assert_eq!(
            check(text),
            Some(ClaimStatus::Supported),
            "widening the anchors must not cost accuracy: {text:?}"
        );
    }
}

#[test]
fn a_wrong_max_hr_is_caught_in_french_and_german() {
    for text in [
        "Ta fréquence cardiaque maximale est de 240.",
        "Deine maximale Herzfrequenz liegt bei 240.",
    ] {
        assert_eq!(check(text), Some(ClaimStatus::Contradicted), "{text:?}");
    }
}

/// The label the coach actually used on 2026-09-02. It invented «indice de
/// fatigue» for TSB and repeated it for fifteen turns; a label the model really
/// uses is worth probing whether or not anybody chose it.
#[test]
fn the_invented_french_label_for_tsb_is_probed() {
    assert_eq!(
        check("Ton indice de fatigue est à -30 cette semaine."),
        Some(ClaimStatus::Contradicted),
        "the real TSB is -77; a claim of -30 under the label the coach coined \
         must not pass unchecked"
    );
    assert_eq!(
        check("Ton indice de fatigue est à -77 cette semaine."),
        Some(ClaimStatus::Supported),
        "and the true value under that label must read as supported"
    );
}

#[test]
fn a_french_threshold_pace_claim_is_probed() {
    assert_eq!(
        check("Ton allure seuil est autour de 4:10/km."),
        Some(ClaimStatus::Contradicted),
        "threshold pace is 5:00-5:15/km; 4:10 is well outside it"
    );
}

/// A range is not a negative number. Without this the sign fix would read
/// «zone 2-3» as -3 and «5:00-5:15/km» as a negative pace.
#[test]
fn a_hyphen_between_digits_is_still_a_range() {
    assert_eq!(
        check("Ton indice de fatigue est à 70-77 cette semaine."),
        Some(ClaimStatus::Contradicted),
        "70 is the nearer token and is positive; a range must not silently \
         become a negative number"
    );
}
