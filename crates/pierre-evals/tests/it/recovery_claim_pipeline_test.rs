// ABOUTME: HRV, resting-HR and sleep claims through the whole verifier — extraction, personalized layer, athlete-data fallback
// ABOUTME: Pins five-locale coverage end to end, and that a sleep figure never matches an activity duration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Until carnet#853 the verifier sorted "your HRV dropped 15%" and "you slept 8
//! hours" into categories and then had nothing to compare their numbers with:
//! the athlete snapshot held no overnight reading, and the athlete-data layer
//! (rightly) refuses to read a night against a 480-minute ride. These tests run
//! a reply through the same path a chat turn takes — the heuristic extractor,
//! then every layer — with the athlete's synced nights in the snapshot.

use chrono::{Duration, NaiveDate};
use pierre_core::models::SportType;
use pierre_evals::athlete_data::{AthleteRecord, RecordedActivity};
use pierre_evals::{
    check_claim, extract_heuristic, AthleteMetrics, ConservativeStrategy, EvidenceCorpus,
    NightlyMetric, PersonalizedContext, RecoveryReadings,
};
use pierre_memory::{ClaimStatus, EvidenceStrength, VerdictLayer};

/// The literature line a sleep sentence shares words with. If any sleep claim
/// below ever reached the evidence layer, this is what would "support" it.
const CORPUS: &str = r#"{"id":"doi:10.1/d","category":"recovery","proposition":"Adults require 7 to 9 hours of sleep per night","strength":"strong","citation":"AASM 2015"}"#;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 9).unwrap()
}

/// A week of nights: HRV 60 ms then 51 last night (−15%), resting HR 48 then
/// 52, sleep 7 h 30 then 6 h.
fn readings() -> RecoveryReadings {
    let series = |earlier: f64, last: f64| {
        let mut daily: Vec<(NaiveDate, f64)> = (1..=6)
            .map(|back| (today() - Duration::days(back), earlier))
            .collect();
        daily.push((today(), last));
        NightlyMetric::from_daily(&daily, today())
    };
    RecoveryReadings {
        hrv_ms: series(60.0, 51.0),
        resting_hr_bpm: series(48.0, 52.0),
        sleep_hours: series(7.5, 6.0),
        ..RecoveryReadings::default()
    }
}

/// A connected athlete whose only activity this week is an 8-hour ride: the
/// duration a sleep figure must never be matched against.
fn record() -> AthleteRecord {
    AthleteRecord {
        has_provider: true,
        today: today(),
        locale: "en".to_owned(),
        activities: vec![RecordedActivity {
            date: today(),
            sport: SportType::Ride,
            name: "Gran fondo".to_owned(),
            distance_km: Some(180.0),
            duration_min: 480.0,
            elevation_m: None,
            average_hr_bpm: None,
        }],
        history: None,
    }
}

/// Every claim the extractor finds in `reply`, with its verdict and the layer
/// that settled it.
fn verify(reply: &str, recovery: &RecoveryReadings) -> Vec<(String, ClaimStatus, VerdictLayer)> {
    let corpus = EvidenceCorpus::from_jsonl(CORPUS).unwrap();
    let metrics = AthleteMetrics {
        recovery: *recovery,
        ..AthleteMetrics::default()
    };
    let strategy = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &metrics,
        tolerance: &strategy,
    };
    let record = record();
    let claims = extract_heuristic(reply);
    claims
        .iter()
        .map(|claim| {
            let verdict = check_claim(
                claim,
                &claims,
                &corpus,
                EvidenceStrength::Mixed,
                Some(&ctx),
                Some(&record),
            );
            (claim.text.clone(), verdict.status, verdict.layer_fired)
        })
        .collect()
}

/// The same three statements in each shipped locale: a sleep figure two hours
/// over last night's, the HRV drop the nights show, and this morning's resting
/// heart rate.
const REPLIES: [&str; 5] = [
    "You slept 8 hours last night. Your HRV dropped 15% this week. \
     Your resting heart rate this morning is 52 bpm.",
    "Tu as dormi 8 heures cette nuit. Ta VFC a baissé de 15 % cette semaine. \
     Ta FC au repos ce matin est de 52 bpm.",
    "Anoche dormiste 8 horas en total. Tu VFC bajó un 15 % esta semana. \
     Tu frecuencia cardíaca en reposo es de 52 lpm.",
    "Du hast letzte Nacht 8 Stunden geschlafen. Deine HRV ist diese Woche um 15 % gesunken. \
     Dein Ruhepuls liegt heute bei 52.",
    "Você dormiu 8 horas esta noite. A tua VFC caiu 15% esta semana. \
     A tua frequência cardíaca em repouso é de 52 bpm.",
];

#[test]
fn hrv_resting_hr_and_sleep_are_checked_against_the_nights_in_every_shipped_locale() {
    for reply in REPLIES {
        let verdicts = verify(reply, &readings());
        let statuses: Vec<ClaimStatus> = verdicts.iter().map(|(_, s, _)| *s).collect();
        assert_eq!(
            statuses,
            vec![
                ClaimStatus::Contradicted,
                ClaimStatus::Supported,
                ClaimStatus::Supported
            ],
            "{verdicts:#?}"
        );
        assert!(
            verdicts
                .iter()
                .all(|(_, _, layer)| *layer == VerdictLayer::Personalized),
            "the athlete's own nights settle every one: {verdicts:#?}"
        );
    }
}

#[test]
fn without_synced_nights_a_sleep_figure_is_declined_not_matched_to_a_ride() {
    for reply in [
        "You slept 8 hours last night.",
        "Tu as dormi 8 heures cette nuit.",
    ] {
        let verdicts = verify(reply, &RecoveryReadings::default());
        assert_eq!(verdicts.len(), 1, "{verdicts:#?}");
        let (_, status, layer) = &verdicts[0];
        assert_eq!(
            (*status, *layer),
            (ClaimStatus::Unverifiable, VerdictLayer::AthleteData),
            "neither the 480-minute ride nor the sleep literature is evidence about this \
             athlete's night: {verdicts:#?}"
        );
    }
}
