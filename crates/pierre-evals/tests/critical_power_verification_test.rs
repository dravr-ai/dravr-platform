// ABOUTME: Pins the personalized layer's critical-power family probes and its estimate-framing check
// ABOUTME: A modelled CP quoted as a measurement is flagged even when the number itself is right (carnet#714)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The profile stores critical power, W′, critical speed and D′ with whether
//! each was measured or estimated. Two things are checked against it: the
//! number, within the parameter's test-retest band (CP ±5%, CS ±3%, W′ and D′
//! ±25%), and — when the stored value is an estimate — that the sentence
//! quotes it as one. "Your CP is 312 W" is false of a Vekta estimate even at
//! exactly 312; "Vekta estimates your CP at 312 W" is true.

use pierre_core::models::MeasurementKind;
use pierre_evals::claim_extractor::{classify_heuristic, extract_heuristic};
use pierre_evals::personalized::check as personalized_check;
use pierre_evals::verdict_engine::VerdictOutcome;
use pierre_evals::{
    AthleteMetrics, ClaimSource, ConservativeStrategy, ExtractedClaim, PersonalizedContext,
    StoredMetric, ToleranceStrategy,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, VerdictLayer};

/// The explanation prefix of the framing check. Pinned as a literal on
/// purpose: it is a persisted value — verdict rows already written carry it,
/// and they are counted by it (D6).
const ESTIMATE_STATED_AS_MEASUREMENT: &str = "Estimate stated as a measurement";

fn stored(value: f64, kind: MeasurementKind, origin: &str) -> StoredMetric {
    StoredMetric {
        value,
        kind,
        origin: Some(origin.to_owned()),
    }
}

/// A cyclist whose CP and W′ Vekta estimated, and a runner's lab-measured CS
/// and D′, on one snapshot so each probe can be reached.
fn metrics() -> AthleteMetrics {
    AthleteMetrics {
        ftp_watts: Some(300.0),
        critical_power_watts: Some(stored(312.0, MeasurementKind::Estimated, "Vekta")),
        w_prime_joules: Some(stored(21_500.0, MeasurementKind::Estimated, "Vekta")),
        critical_speed_mps: Some(stored(4.17, MeasurementKind::Measured, "lab")),
        d_prime_meters: Some(stored(250.0, MeasurementKind::Measured, "lab")),
        data_days: 30,
        ..AthleteMetrics::default()
    }
}

fn outcome_for(m: &AthleteMetrics, text: &str) -> Option<VerdictOutcome> {
    let strategy: Box<dyn ToleranceStrategy> = Box::new(ConservativeStrategy::default());
    let ctx = PersonalizedContext {
        metrics: m,
        tolerance: strategy.as_ref(),
    };
    let claim = ExtractedClaim {
        text: text.to_owned(),
        category: ClaimCategory::Physiological,
        source: ClaimSource::Reply,
    };
    personalized_check(&claim, &ctx)
}

fn status(text: &str) -> Option<ClaimStatus> {
    outcome_for(&metrics(), text).map(|v| v.status)
}

#[test]
fn an_estimated_cp_stated_as_a_measurement_is_flagged_in_every_locale() {
    for text in [
        "Your critical power is 312 W.",
        "Ta puissance critique est de 312 W.",
        "Tu potencia crítica es de 312 W.",
        "Deine kritische Leistung liegt bei 312 W.",
        "A tua potência crítica é de 312 W.",
    ] {
        let outcome = outcome_for(&metrics(), text).expect("the layer fires");
        assert_eq!(outcome.status, ClaimStatus::Unsupported, "{text}");
        assert_eq!(outcome.layer_fired, VerdictLayer::Personalized);
        assert!(
            outcome
                .explanation
                .starts_with(ESTIMATE_STATED_AS_MEASUREMENT),
            "the explanation prefix is what counts these: {}",
            outcome.explanation
        );
        assert!(
            outcome.explanation.contains("Vekta"),
            "{}",
            outcome.explanation
        );
    }
}

#[test]
fn an_estimated_cp_quoted_as_an_estimate_is_supported_in_every_locale() {
    for text in [
        "Vekta estimates your critical power at 312 W.",
        "Your estimated critical power is around 312 W.",
        "Selon Vekta, ta puissance critique est de 312 W.",
        "Tu potencia crítica estimada es de 312 W.",
        "Deine geschätzte kritische Leistung liegt bei 312 W.",
        "A tua potência crítica estimada é de 312 W.",
    ] {
        assert_eq!(status(text), Some(ClaimStatus::Supported), "{text}");
    }
}

#[test]
fn a_wrong_cp_is_contradicted_however_it_is_framed() {
    for text in [
        "Your critical power is 360 W.",
        "Vekta estimates your critical power at 360 W.",
    ] {
        assert_eq!(status(text), Some(ClaimStatus::Contradicted), "{text}");
    }
}

#[test]
fn cp_is_scored_within_five_percent() {
    // 312 × 1.05 = 327.6: inside, then past the band and the conservative buffer.
    assert_eq!(
        status("Vekta estimates your critical power at 325 W."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        status("Vekta estimates your critical power at 380 W."),
        Some(ClaimStatus::Contradicted)
    );
}

#[test]
fn a_measured_value_may_be_stated_plainly() {
    assert_eq!(
        status("Your critical speed is 4.17 m/s."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        status("Your D prime is 250 m."),
        Some(ClaimStatus::Supported)
    );
}

#[test]
fn critical_speed_reads_a_pace_or_km_per_hour() {
    // 4.17 m/s is 4:00/km and 15.0 km/h.
    assert_eq!(
        status("Your critical speed is 4:00/km."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        status("Your critical speed is 15 km/h."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        status("Your critical speed is 3:20/km."),
        Some(ClaimStatus::Contradicted)
    );
}

#[test]
fn w_prime_reads_kilojoules_or_joules_within_a_quarter() {
    assert_eq!(
        status("Vekta estimates your W′ at 21.5 kJ."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        status("Vekta estimates your W' at 21500 J."),
        Some(ClaimStatus::Supported)
    );
    // 25% wide: W′ is the noisiest of the four.
    assert_eq!(
        status("Vekta estimates your W′ at 25 kJ."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        status("Vekta estimates your W′ at 40 kJ."),
        Some(ClaimStatus::Contradicted)
    );
    assert_eq!(
        status("Your W′ is 21.5 kJ."),
        Some(ClaimStatus::Unsupported),
        "an estimated W′ stated bare is flagged like CP"
    );
}

/// "threshold power" is an FTP keyword. A CP sentence that also says it must
/// be scored as CP, against 312, not as FTP against 300.
#[test]
fn a_cp_sentence_is_not_captured_by_the_ftp_probe() {
    // An FTP of 250 makes the FTP probe contradict 312 if it reads first.
    let m = AthleteMetrics {
        ftp_watts: Some(250.0),
        ..metrics()
    };
    let text = "Vekta estimates your critical power at 312 W, well above your threshold power.";
    assert_eq!(
        outcome_for(&m, text).map(|v| v.status),
        Some(ClaimStatus::Supported)
    );
}

#[test]
fn no_stored_value_leaves_the_claim_to_the_other_layers() {
    let m = AthleteMetrics {
        ftp_watts: Some(300.0),
        data_days: 30,
        ..AthleteMetrics::default()
    };
    assert!(outcome_for(&m, "Your critical power is 312 W.").is_none());
}

#[test]
fn the_source_named_in_the_sentence_is_attribution_enough() {
    assert_eq!(
        status("Per Vekta your critical power sits at 312 W."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        status("Your critical power sits at 312 W."),
        Some(ClaimStatus::Unsupported)
    );
}

#[test]
fn an_athlete_reported_estimate_has_no_source_to_name() {
    let m = AthleteMetrics {
        critical_power_watts: Some(stored(
            312.0,
            MeasurementKind::Estimated,
            "athlete-reported",
        )),
        ..metrics()
    };
    assert_eq!(
        outcome_for(&m, "Your athlete-reported critical power is 312 W.").map(|v| v.status),
        Some(ClaimStatus::Unsupported),
        "athlete-reported is no attribution: the athlete named no test"
    );
}

/// "Por segundo" is how Spanish and Portuguese state a speed. It is not the
/// Portuguese "segundo" (according to), so it frames nothing as an estimate.
#[test]
fn metres_per_second_is_not_an_estimate_marker() {
    let m = AthleteMetrics {
        critical_speed_mps: Some(stored(4.17, MeasurementKind::Estimated, "Vekta")),
        ..metrics()
    };
    for text in [
        "Tu velocidad crítica es de 4.17 metros por segundo.",
        "A tua velocidade crítica é de 4.17 metros por segundo.",
    ] {
        assert_eq!(
            outcome_for(&m, text).map(|v| v.status),
            Some(ClaimStatus::Unsupported),
            "{text}"
        );
    }
}

/// A critical speed is a two-decimal number: the explanation must not round
/// 4.17 m/s to 4.
#[test]
fn a_critical_speed_explanation_keeps_its_decimals() {
    let m = AthleteMetrics {
        critical_speed_mps: Some(stored(4.17, MeasurementKind::Estimated, "Vekta")),
        ..metrics()
    };
    let outcome = outcome_for(&m, "Your critical speed is 4.17 m/s.").expect("the layer fires");
    assert_eq!(outcome.status, ClaimStatus::Unsupported);
    assert!(
        outcome.explanation.contains("critical speed of 4.17"),
        "{}",
        outcome.explanation
    );
    let contradicted =
        outcome_for(&metrics(), "Your critical speed is 3.50 m/s.").expect("the layer fires");
    assert_eq!(contradicted.status, ClaimStatus::Contradicted);
    assert!(
        contradicted
            .explanation
            .contains("critical speed 3.50 vs your 4.17"),
        "{}",
        contradicted.explanation
    );
}

#[test]
fn critical_power_sentences_are_extracted_as_physiological_claims() {
    for text in [
        "Your critical power is 312 W today.",
        "Ta puissance critique est de 312 W.",
        "Deine kritische Leistung liegt bei 312 W.",
        "Your critical speed is about 4:00/km.",
    ] {
        assert_eq!(
            classify_heuristic(text),
            Some(ClaimCategory::Physiological),
            "{text}"
        );
    }
    let claims = extract_heuristic("Your critical power is 312 W, so hold 300 W for the climb.");
    assert!(
        claims
            .iter()
            .any(|c| c.category == ClaimCategory::Physiological),
        "{claims:?}"
    );
}
