// ABOUTME: Pins the weekday, sport and elevation checks against the athlete's own record
// ABOUTME: Regression for 2026-09-02 — the three claim classes the athlete corrected by hand
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The layer held three fields — `has_provider`, `distances_km`,
//! `durations_min`. No dates, no sports, no elevation. So it could not falsify
//! *which day* an activity fell on, *which sport* it was, or *how much climbing*
//! it had, in any locale including English.
//!
//! Those are exactly the three classes the athlete corrected on 2026-09-02, and
//! all three passed the verifier untouched — while four benign coaching
//! prescriptions were flagged with "je n'ai pas pu étayer". He saw warnings on
//! the advice and none on the facts (registre#249).

use chrono::NaiveDate;
use pierre_core::models::SportType;
use pierre_evals::athlete_data::{check, AthleteRecord, RecordedActivity};
use pierre_evals::claim_extractor::ExtractedClaim;
use pierre_memory::{ClaimCategory, ClaimStatus, VerdictLayer};

fn claim(text: &str) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category: ClaimCategory::AthleteData,
    }
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, d).expect("valid date")
}

/// Raph's week, as the record actually held it.
///
/// Road 2 AUS was **Tuesday** the 1st. Passion rando was a **run**. Both facts
/// were in the provider data and neither reached this layer.
fn raphs_week() -> AthleteRecord {
    AthleteRecord {
        has_provider: true,
        activities: vec![
            RecordedActivity {
                date: day(1),
                sport: SportType::Ride,
                name: "Road 2 AUS".to_owned(),
                distance_km: Some(161.0),
                duration_min: 372.0,
                elevation_m: Some(2391.0),
            },
            RecordedActivity {
                date: NaiveDate::from_ymd_opt(2026, 8, 28).expect("valid date"),
                sport: SportType::Run,
                name: "Passion rando".to_owned(),
                distance_km: Some(26.0),
                duration_min: 200.0,
                elevation_m: Some(895.0),
            },
        ],
    }
}

/// *"road 2 aus etait hier, mardi. T'es melé big"* — he said it twice.
#[test]
fn placing_a_named_activity_on_the_wrong_weekday_is_contradicted() {
    let outcome = check(
        &claim("Dimanche, ta sortie Road 2 AUS était la plus grosse de la semaine."),
        &raphs_week(),
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Contradicted,
        "Road 2 AUS is on record for Tuesday: {}",
        outcome.explanation
    );
    assert_eq!(outcome.layer_fired, VerdictLayer::AthleteData);
    assert!(
        outcome.explanation.contains("Road 2 AUS"),
        "the operator explanation must name the session: {}",
        outcome.explanation
    );
}

#[test]
fn the_right_weekday_is_not_contradicted() {
    let outcome = check(
        &claim("Mardi, ta sortie Road 2 AUS était la plus grosse de la semaine."),
        &raphs_week(),
    );

    assert!(
        outcome.is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "a correct weekday must never be contradicted"
    );
}

/// *"Passion rando etait de la course a pied et non du velo"* — the coach put a
/// run into a cycling plan and prescribed it 2-3x a week.
#[test]
fn calling_a_run_a_bike_session_is_contradicted() {
    let outcome = check(
        &claim("Passion rando, c'est du vélo — refais-en 2-3x par semaine."),
        &raphs_week(),
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Contradicted,
        "Passion rando is on record as a run: {}",
        outcome.explanation
    );
}

#[test]
fn naming_the_right_sport_is_not_contradicted() {
    let outcome = check(
        &claim("Passion rando, c'était de la course à pied."),
        &raphs_week(),
    );

    assert!(
        outcome
            .as_ref()
            .is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "the record says run and so did the coach; got {outcome:?}"
    );
}

/// A mountain-bike ride called "vélo" is not a false claim about the sport.
#[test]
fn a_sub_discipline_matches_its_family() {
    let record = AthleteRecord {
        has_provider: true,
        activities: vec![RecordedActivity {
            date: day(1),
            sport: SportType::MountainBike,
            name: "Date ride".to_owned(),
            distance_km: Some(16.25),
            duration_min: 87.0,
            elevation_m: Some(414.0),
        }],
    };

    let outcome = check(&claim("Ton Date ride, c'était du vélo."), &record);

    assert!(
        outcome.is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "a mountain bike ride IS cycling; contradicting that would be noise"
    );
}

/// Elevation was in every one of the coach's summaries — 2391 m, 895 m, 414 m —
/// and the extractor had no metres unit at all, so none of it was checkable.
#[test]
fn an_elevation_figure_is_checked_against_the_record() {
    let supported = check(
        &claim("Cette sortie t'a fait 2391 m de dénivelé."),
        &raphs_week(),
    )
    .expect("the layer must adjudicate its own category");
    assert_eq!(
        supported.status,
        ClaimStatus::Supported,
        "2391 m is on record: {}",
        supported.explanation
    );

    let missed = check(
        &claim("Cette sortie t'a fait 5000 m de dénivelé."),
        &raphs_week(),
    )
    .expect("the layer must adjudicate its own category");
    assert_ne!(
        missed.status,
        ClaimStatus::Supported,
        "5000 m matches nothing held: {}",
        missed.explanation
    );
}

/// A milligram dose is not elevation. The bare `m` unit must not swallow it.
#[test]
fn a_bare_m_does_not_eat_other_units() {
    let outcome = check(
        &claim("Prends 400 mg de caféine avant le départ."),
        &raphs_week(),
    );

    assert!(
        outcome.is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "mg is not m, and a supplement dose is not a claim about the record"
    );
}

/// Two named activities and one weekday is ambiguous — the layer abstains
/// rather than guessing which session the day belongs to.
#[test]
fn two_named_activities_is_not_adjudicated_on_weekday() {
    let outcome = check(
        &claim("Dimanche tu as fait Road 2 AUS puis Passion rando."),
        &raphs_week(),
    );

    assert!(
        outcome.is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "a wrong contradiction costs more than a missed one"
    );
}

/// "mar" must not be found inside "marathon", nor "run" inside "brunch".
#[test]
fn a_weekday_abbreviation_is_not_matched_inside_another_word() {
    let outcome = check(
        &claim("Road 2 AUS était un vrai effort de marathon."),
        &raphs_week(),
    );

    assert!(
        outcome.is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "'mar' inside 'marathon' is not a Tuesday claim"
    );
}

/// A name too short to be distinctive is not matched at all.
#[test]
fn a_two_letter_activity_name_is_never_matched() {
    let record = AthleteRecord {
        has_provider: true,
        activities: vec![RecordedActivity {
            date: day(1),
            sport: SportType::Ride,
            name: "AM".to_owned(),
            distance_km: Some(20.0),
            duration_min: 60.0,
            elevation_m: None,
        }],
    };

    let outcome = check(&claim("Dimanche, tu as bien récupéré."), &record);

    assert!(
        outcome.is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "a two-letter name appears inside ordinary prose by accident"
    );
}
