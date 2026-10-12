// ABOUTME: Period totals end to end — a reply sentence extracted, routed, and its weekly total checked
// ABOUTME: Pins that "you ran 42 km this week" reaches the athlete-data layer in all five shipped locales

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Why the extractor is part of this feature
//!
//! The weekly total is the figure a coach writes most often, and until
//! carnet#846 it never reached the athlete-data layer: "this week" marked a
//! sentence as a prescription, so "you ran 42 km this week" was dropped at
//! extraction, and Spanish, German and Portuguese reports had no way in at all.
//! A period check that nothing routes a claim to is a phantom, so these tests
//! start from the reply text, not from a hand-built claim.

use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone, Utc};
use pierre_core::models::SportType;
use pierre_evals::athlete_data::{
    check, AthleteRecord, BackfillDepth, CacheRead, HistoryInputs, PeriodHistory, RecordedActivity,
    SourceCoverage,
};
use pierre_evals::claim_extractor::{extract_heuristic, ExtractedClaim};
use pierre_memory::{ClaimCategory, ClaimStatus};

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, d).expect("valid date")
}

fn run(date: NaiveDate, km: f64) -> RecordedActivity {
    RecordedActivity {
        date,
        sport: SportType::Run,
        name: String::new(),
        distance_km: Some(km),
        duration_min: km * 6.0,
        elevation_m: None,
    }
}

/// Wednesday 2026-09-09 at noon UTC.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 9, 12, 0, 0).unwrap()
}

fn ride(date: NaiveDate, km: f64) -> RecordedActivity {
    RecordedActivity {
        date,
        sport: SportType::GravelRide,
        name: String::new(),
        distance_km: Some(km),
        duration_min: km * 2.5,
        elevation_m: None,
    }
}

/// A calendar week of 10 km Monday and 12 km Tuesday, from one source whose
/// backfill reached back to January and whose head was last read at
/// `last_sync`.
fn connected_week(last_sync: DateTime<Utc>) -> AthleteRecord {
    connected(vec![run(day(7), 10.0), run(day(8), 12.0)], last_sync)
}

/// `sessions` from one source whose backfill reached back to January and
/// whose head was last read at `last_sync`.
fn connected(sessions: Vec<RecordedActivity>, last_sync: DateTime<Utc>) -> AthleteRecord {
    AthleteRecord {
        has_provider: true,
        today: day(9),
        locale: "fr".to_owned(),
        activities: sessions.clone(),
        history: PeriodHistory::vouched(HistoryInputs {
            sessions,
            by_source: Vec::new(),
            read: CacheRead {
                from: now() - TimeDelta::days(90),
                cut_day: None,
            },
            sources: vec![SourceCoverage {
                last_sync: Some(last_sync),
                depth: BackfillDepth::Reached(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            }],
            now: now(),
            zone: Utc,
            block_start: None,
        }),
    }
}

/// Read this turn.
fn synced_this_turn() -> DateTime<Utc> {
    now() - TimeDelta::minutes(2)
}

/// The one athlete-data claim a sentence must yield.
fn athlete_claim(sentence: &str) -> ExtractedClaim {
    let claims: Vec<ExtractedClaim> = extract_heuristic(sentence)
        .into_iter()
        .filter(|c| c.category == ClaimCategory::AthleteData)
        .collect();
    assert_eq!(
        claims.len(),
        1,
        "the sentence must reach the athlete-data layer exactly once: {sentence}"
    );
    claims.into_iter().next().expect("one claim")
}

/// The calendar week holds 22 km. Each locale's report of it is supported, and
/// each locale's wrong figure is contradicted.
#[test]
fn a_weekly_total_is_extracted_and_checked_in_every_shipped_locale() {
    let record = connected_week(synced_this_turn());
    for (right, wrong) in [
        ("You ran 22 km this week.", "You ran 35 km this week."),
        (
            "Tu as couru 22 km cette semaine.",
            "Tu as couru 35 km cette semaine.",
        ),
        (
            "Esta semana corriste 22 km en total.",
            "Esta semana corriste 35 km en total.",
        ),
        (
            "Diese Woche bist du insgesamt 22 km gelaufen.",
            "Diese Woche bist du insgesamt 35 km gelaufen.",
        ),
        (
            "Nesta semana correste 22 km ao todo.",
            "Nesta semana correste 35 km ao todo.",
        ),
    ] {
        let supported = check(&athlete_claim(right), &record).expect("adjudicated");
        assert_eq!(
            supported.status,
            ClaimStatus::Supported,
            "{right} -> {}",
            supported.explanation
        );
        let contradicted = check(&athlete_claim(wrong), &record).expect("adjudicated");
        assert_eq!(
            contradicted.status,
            ClaimStatus::Contradicted,
            "{wrong} -> {}",
            contradicted.explanation
        );
        assert!(
            contradicted.explanation.contains("22 km"),
            "the operator sees the total on record: {}",
            contradicted.explanation
        );
    }
}

/// "This week" still marks a plan when nothing in the sentence reports past
/// training. A plan checked against the week's sessions would miss and, for a
/// providerless athlete, read as invented.
#[test]
fn a_plan_for_this_week_is_still_not_an_athlete_data_claim() {
    for plan in [
        "This week, aim for 40 km at an easy pace.",
        "This week you have three runs of 10 km planned.",
        "Cette semaine, vise 40 km en endurance fondamentale.",
        "Cette semaine on garde trois sorties de 10 km.",
    ] {
        assert!(
            extract_heuristic(plan)
                .iter()
                .all(|c| c.category != ClaimCategory::AthleteData),
            "a plan is not a claim about the record: {plan}"
        );
    }
}

/// Another athlete's result is not this athlete's record. The verbs that route
/// a report here are second person, and a German participle counts only beside
/// `du`; otherwise a providerless athlete would see Kipchoge's marathon called
/// invented.
#[test]
fn someone_elses_result_is_not_an_athlete_data_claim() {
    for sentence in [
        "Kipchoge ist den Marathon in 2 h gelaufen, ein Weltrekord.",
        "Kipchoge corrió el maratón en 2 h, un récord mundial.",
        "Kipchoge correu a maratona em 2 h, um recorde mundial.",
    ] {
        assert!(
            extract_heuristic(sentence)
                .iter()
                .all(|c| c.category != ClaimCategory::AthleteData),
            "a sentence about another athlete is not about this one: {sentence}"
        );
    }
}

/// A weekly total stated to an athlete with nothing connected had no source,
/// so it is contradicted like any other figure about their training — in the
/// locales that were not extracted at all before.
#[test]
fn a_providerless_weekly_total_is_contradicted() {
    let providerless = AthleteRecord::providerless(day(9), "es");
    for sentence in [
        "Tu as couru 42 km cette semaine.",
        "Esta semana corriste 42 km en total.",
        "Diese Woche bist du insgesamt 42 km gelaufen.",
    ] {
        let outcome = check(&athlete_claim(sentence), &providerless).expect("adjudicated");
        assert_eq!(outcome.status, ClaimStatus::Contradicted, "{sentence}");
    }
}

/// A sync that has not reached today leaves this week open: the missing days
/// may hold the difference.
#[test]
fn a_week_the_sync_has_not_reached_is_unverifiable() {
    let record = connected_week(Utc.with_ymd_and_hms(2026, 9, 8, 21, 0, 0).unwrap());
    let outcome = check(&athlete_claim("You ran 35 km this week."), &record).expect("adjudicated");
    assert_eq!(
        outcome.status,
        ClaimStatus::Unverifiable,
        "{}",
        outcome.explanation
    );
}

/// A coach asking what the athlete did claims nothing. Spanish, German and
/// Portuguese ask with the verb form they state with, and French often does,
/// so a question must not reach the layer that calls a providerless figure
/// invented.
#[test]
fn a_question_is_not_an_athlete_data_claim() {
    let providerless = AthleteRecord::providerless(day(9), "es");
    for question in [
        "¿Corriste los 10 km que tocaban ayer?",
        "¿Cuántos km corriste esta semana, 30 km?",
        "Corriste los 10 km de ayer?",
        "Você correu os 10 km de ontem?",
        "Correste os 10 km de ontem?",
        "Bist du gestern die 10 km gelaufen?",
        "Tu as couru 10 km hier ?",
        "Tu as couru 30 km cette semaine ?",
        "Did you run 10 km yesterday?",
    ] {
        let claims = extract_heuristic(question);
        assert!(
            claims
                .iter()
                .all(|c| c.category != ClaimCategory::AthleteData),
            "a question is not a claim about the record: {question}"
        );
        for claim in &claims {
            if let Some(outcome) = check(claim, &providerless) {
                assert_ne!(outcome.status, ClaimStatus::Contradicted, "{question}");
            }
        }
    }
}

/// A supposition claims nothing either, whether it opens the sentence or sits
/// right before the verb.
#[test]
fn a_supposition_is_not_an_athlete_data_claim() {
    for sentence in [
        "If you ran 30 km this week, take Sunday off.",
        "Take Sunday off if you ran 30 km this week.",
        "Si tu as couru 30 km cette semaine, repose-toi dimanche.",
        "Repose-toi dimanche si tu as couru 30 km cette semaine.",
        "Si corriste 30 km esta semana, descansa el domingo.",
        "Wenn du diese Woche 30 km gelaufen bist, ruh dich am Sonntag aus.",
        "Se você correu 30 km nesta semana, descanse no domingo.",
        "Descanse no domingo se você correu 30 km nesta semana.",
    ] {
        assert!(
            extract_heuristic(sentence)
                .iter()
                .all(|c| c.category != ClaimCategory::AthleteData),
            "a supposition is not a claim about the record: {sentence}"
        );
    }
}

/// The same reports, stated, still reach the layer: the question and
/// supposition guards key on the question mark and the conditional, not on the
/// verb.
#[test]
fn a_statement_beside_those_guards_still_reaches_the_layer() {
    for sentence in [
        "Corriste 30 km esta semana, buen trabajo.",
        "Você correu 30 km nesta semana, ótimo trabalho.",
        "Du bist diese Woche 30 km gelaufen, stark.",
        "Tu as couru 30 km cette semaine, bravo.",
    ] {
        athlete_claim(sentence);
    }
}

/// A week of 30 km of runs and 120 km of rides. A figure whose sentence names
/// the sport — by the noun or by the verb — is read against that sport's total
/// alone, in each shipped locale: the sum of both sports does not support a
/// ride, and the ride total does not support a run.
#[test]
fn a_named_sport_is_summed_alone_in_every_shipped_locale() {
    let record = connected(
        vec![
            run(day(7), 10.0),
            run(day(8), 20.0),
            ride(day(7), 50.0),
            ride(day(8), 70.0),
        ],
        synced_this_turn(),
    );
    for (right, wrong) in [
        ("You rode 120 km this week.", "You rode 150 km this week."),
        (
            "Tu as roulé 120 km en vélo cette semaine.",
            "Tu as roulé 150 km en vélo cette semaine.",
        ),
        (
            "Esta semana pedaleaste 120 km en ciclismo.",
            "Esta semana pedaleaste 150 km en ciclismo.",
        ),
        (
            "Diese Woche bist du 120 km geradelt.",
            "Diese Woche bist du 150 km geradelt.",
        ),
        (
            "Você pedalou 120 km nesta semana.",
            "Você pedalou 150 km nesta semana.",
        ),
        ("You ran 30 km this week.", "You ran 120 km this week."),
        (
            "Tu as couru 30 km cette semaine.",
            "Tu as couru 120 km cette semaine.",
        ),
        (
            "Esta semana corriste 30 km.",
            "Esta semana corriste 120 km.",
        ),
        (
            "Diese Woche bist du 30 km gelaufen.",
            "Diese Woche bist du 120 km gelaufen.",
        ),
        (
            "Nesta semana correste 30 km.",
            "Nesta semana correste 120 km.",
        ),
    ] {
        let supported = check(&athlete_claim(right), &record).expect("adjudicated");
        assert_eq!(
            supported.status,
            ClaimStatus::Supported,
            "{right} -> {}",
            supported.explanation
        );
        let contradicted = check(&athlete_claim(wrong), &record).expect("adjudicated");
        assert_eq!(
            contradicted.status,
            ClaimStatus::Contradicted,
            "{wrong} -> {}",
            contradicted.explanation
        );
    }
    // The run total stated as a ride is not supported either. Below the ride
    // total, it may be some of the rides, so it is not contradicted.
    let runs_as_rides =
        check(&athlete_claim("You rode 30 km this week."), &record).expect("adjudicated");
    assert_eq!(
        runs_as_rides.status,
        ClaimStatus::Unverifiable,
        "{}",
        runs_as_rides.explanation
    );
}

/// A week of runs of 8, 10 and 8 km: 26 km. A reply that adds up two of them
/// and says which in its own words is extracted, reaches the period check, and
/// is never contradicted, in every shipped locale; the same figure said to be
/// the week's total is.
#[test]
fn some_of_the_weeks_sessions_are_not_contradicted_in_every_shipped_locale() {
    let record = connected(
        vec![run(day(7), 8.0), run(day(8), 10.0), run(day(9), 8.0)],
        synced_this_turn(),
    );
    for (subset, total) in [
        (
            "You ran 18 km over your last two runs this week.",
            "You ran 18 km in total this week.",
        ),
        (
            "Tu as couru 18 km lors de tes deux dernières sorties cette semaine.",
            "Tu as couru 18 km en tout cette semaine.",
        ),
        (
            "Esta semana corriste 18 km con el club.",
            "Esta semana corriste 18 km en total.",
        ),
        (
            "Diese Woche bist du 18 km mit dem Verein gelaufen.",
            "Diese Woche bist du insgesamt 18 km gelaufen.",
        ),
        (
            "Nesta semana correste 18 km com o clube.",
            "Nesta semana correste 18 km ao todo.",
        ),
    ] {
        let unsettled = check(&athlete_claim(subset), &record).expect("adjudicated");
        assert_eq!(
            unsettled.status,
            ClaimStatus::Unverifiable,
            "{subset} -> {}",
            unsettled.explanation
        );
        let contradicted = check(&athlete_claim(total), &record).expect("adjudicated");
        assert_eq!(
            contradicted.status,
            ClaimStatus::Contradicted,
            "{total} -> {}",
            contradicted.explanation
        );
    }
}

/// A sentence that mentions a second sport is not narrowed to the first: its
/// figure may be both added up.
#[test]
fn a_second_sport_keeps_the_all_sports_reading() {
    let record = connected(
        vec![run(day(7), 30.0), ride(day(8), 120.0)],
        synced_this_turn(),
    );
    for sentence in [
        "You rode 150 km this week, runs included.",
        "You rode and ran 150 km this week.",
        "Tu as roulé et couru 150 km cette semaine.",
    ] {
        let outcome = check(&athlete_claim(sentence), &record).expect("adjudicated");
        assert_eq!(
            outcome.status,
            ClaimStatus::Supported,
            "{sentence} -> {}",
            outcome.explanation
        );
    }
}
