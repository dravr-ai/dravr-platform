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
///
/// Asserted on the PROVIDERLESS record, not a connected one. A connected
/// athlete's figure path can only ever return Supported or Unverifiable, so the
/// old `!= Contradicted` assertion held no matter what the unit table did — it
/// could not fail (registre#258). With no provider, any extracted figure IS
/// contradicted, so the assertion now turns on exactly one thing: whether "400
/// mg" was read as a figure at all.
#[test]
fn a_bare_m_does_not_eat_other_units() {
    let outcome = check(
        &claim("Prends 400 mg de caféine avant le départ."),
        &AthleteRecord::providerless(),
    )
    .expect("the layer must adjudicate its own category");

    assert_ne!(
        outcome.status,
        ClaimStatus::Contradicted,
        "mg is not m: reading it as 400 metres of climb makes a supplement dose          into a claim about the record, and with no provider that is a          fabrication verdict: {}",
        outcome.explanation
    );
}

/// Elevation IS read on the same path, so the test above is not passing merely
/// because nothing is extracted.
#[test]
fn the_metres_unit_is_read_at_all() {
    let outcome = check(
        &claim("Tu as fait 2391 m de dénivelé."),
        &AthleteRecord::providerless(),
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Contradicted,
        "a metres figure with no provider behind it is invented, and saying so          is what proves the unit is parsed: {}",
        outcome.explanation
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

/// A weekday or a sport found inside a longer word is not a claim.
///
/// This gated nothing for a year and a half of its short life. It read *"Road 2
/// AUS était un vrai effort de marathon"* and asserted that "mar" was not a
/// Tuesday — but registre#258 had already narrowed the vocabulary to full
/// weekday names, so "mardi" is not in "marathon" by any rule, and the day it
/// would have resolved to was the day the record already held. Deleting the
/// boundary check left it green (registre#260).
///
/// Where the boundary check IS load-bearing is words that genuinely contain
/// one: `vélo` inside `vélodrome`, `vtt` inside `vttiste`, `sonntag` inside
/// `Sonntagsfahrer` — a slow driver, not a Sunday. Each of these resolves,
/// disagrees with the record, and contradicts a sentence asserting nothing.
#[test]
fn a_weekday_or_sport_inside_a_longer_word_is_not_a_claim() {
    for text in [
        // Passion rando is on record as a run; `vélo` would call it cycling.
        "Passion rando, tu tournais comme au vélodrome.",
        "Passion rando, tu cours mieux que bien des vttistes.",
        // Road 2 AUS is on record for Tuesday; `sonntag` would place it Sunday.
        "Bei Road 2 AUS warst du kein Sonntagsfahrer.",
    ] {
        let outcome = check(&claim(text), &raphs_week());
        assert!(
            outcome
                .as_ref()
                .is_none_or(|v| v.status != ClaimStatus::Contradicted),
            "the word merely contains one; it does not assert it: {text:?} \
             gave {outcome:?}"
        );
    }
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

// ============================================================================
// registre#258 — the false positives the pre-merge review reproduced
// ============================================================================

/// Ordinary words are not weekday claims.
///
/// The reading vocabulary carried three-letter abbreviations, which are
/// homographs of common words in every locale we ship. A verifier compiled a
/// replication and reproduced all five of these: each one resolved to a weekday,
/// disagreed with the record, and produced a `Contradicted` at 0.9 confidence —
/// a warning banner on a reply that asserted no day at all.
///
/// French `mon` is a possessive, `jeu` a game, `mer` the sea; English `sun` is
/// the star; the bare Portuguese ordinals are just numbers.
#[test]
fn a_homograph_of_a_weekday_abbreviation_is_not_a_weekday_claim() {
    for text in [
        "Ta sortie Road 2 AUS confirme mon impression: tu montes bien.",
        "Road 2 AUS, tu as fini la montée in the sun.",
        "Road 2 AUS c'était le jeu des relances tout du long.",
        "Road 2 AUS longeait la mer sur vingt bornes.",
        "Na quarta série de Road 2 AUS tu tenais encore.",
        "En Road 2 AUS bordeaste el mar todo el rato.",
    ] {
        let outcome = check(&claim(text), &raphs_week());
        assert!(
            outcome
                .as_ref()
                .is_none_or(|v| v.status != ClaimStatus::Contradicted),
            "no day is asserted here, so nothing may be contradicted: {text:?} \
             gave {outcome:?}"
        );
    }
}

/// The full names still work — narrowing the vocabulary must not disarm the
/// check the issue was filed for.
#[test]
fn a_real_weekday_name_is_still_read() {
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
}

/// Polysemous sport words are not sport claims.
///
/// `course` is an errand, `marche` is "it works", and an English `run`/`ride`/
/// `trail` appears in ordinary prose constantly. Each of these named one of the
/// athlete's activities and a word that resolved to a sport, and contradicted a
/// sentence making no claim about sport at all.
#[test]
fn a_polysemous_sport_word_is_not_a_sport_claim() {
    for text in [
        "Passion rando, tu l'as casée entre deux course en ville.",
        "Passion rando: la stratégie de ravito marche bien pour toi.",
        "Passion rando gave you a long run of good days.",
        "Passion rando was a rough ride mentally.",
    ] {
        let outcome = check(&claim(text), &raphs_week());
        assert!(
            outcome
                .as_ref()
                .is_none_or(|v| v.status != ClaimStatus::Contradicted),
            "no sport is asserted here: {text:?} gave {outcome:?}"
        );
    }
}

/// A session name is matched as a word, not as a substring.
///
/// `lower.contains(name)` let a longer word "name" an activity, and everything
/// downstream then adjudicated a weekday that was never about it.
#[test]
fn an_activity_name_inside_a_longer_word_does_not_name_it() {
    let record = AthleteRecord {
        has_provider: true,
        activities: vec![RecordedActivity {
            date: day(1),
            sport: SportType::Ride,
            name: "Cote".to_owned(),
            distance_km: Some(30.0),
            duration_min: 90.0,
            elevation_m: Some(500.0),
        }],
    };

    let outcome = check(
        &claim("Dimanche tu as bien géré les cotes du parcours."),
        &record,
    );

    assert!(
        outcome
            .as_ref()
            .is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "'cotes' is not the session called 'Cote': {outcome:?}"
    );
}

// ============================================================================
// registre#260 — what the same review found still standing after #258
// ============================================================================

/// A sentence may name one session and correctly date a *different* one.
///
/// *"Road 2 AUS était plus dure que ta sortie de dimanche"* asserts nothing
/// whatever about which day Road 2 AUS fell on — the Sunday belongs to the
/// other ride, the one the sentence does not name. The layer read the two as
/// one claim and contradicted a true sentence at 0.9 confidence.
///
/// The distinction is syntactic and this layer has no syntax, so it is
/// approximated by position: a generic session noun standing between the name
/// and the day is a second referent, and the layer declines.
#[test]
fn a_weekday_belonging_to_a_second_session_is_not_attributed_to_the_named_one() {
    let outcome = check(
        &claim("Road 2 AUS était plus dure que ta sortie de dimanche."),
        &raphs_week(),
    );

    assert!(
        outcome
            .as_ref()
            .is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "the Sunday is the other ride's; Road 2 AUS is given no day here: \
         {outcome:?}"
    );
}

/// The same shape on the sport check, which had the identical hole.
#[test]
fn a_sport_belonging_to_a_second_session_is_not_attributed_to_the_named_one() {
    let outcome = check(
        &claim("Passion rando était plus dure que ta sortie de vélo."),
        &raphs_week(),
    );

    assert!(
        outcome
            .as_ref()
            .is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "the bike session is the other one; Passion rando is called nothing \
         here: {outcome:?}"
    );
}

/// The guard must not disarm the check it guards.
///
/// *"ta sortie Road 2 AUS"* is one session named twice — `sortie` is its head
/// noun and the name is its apposition — so the weekday is the named session's
/// and the contradiction stands. `placing_a_named_activity_on_the_wrong_weekday`
/// asserts the same thing; this states the reason, so a guard that swallowed
/// everything would be read as a deliberate change rather than a regression.
#[test]
fn a_head_noun_beside_the_name_is_the_same_session() {
    let outcome = check(
        &claim("Dimanche, ta sortie Road 2 AUS était la plus grosse."),
        &raphs_week(),
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Contradicted,
        "one session, named twice, placed on the wrong day: {}",
        outcome.explanation
    );
}

/// An activity whose name IS ordinary training vocabulary names nothing.
///
/// Four characters and word-bounded was the whole test, so a session the
/// athlete called "Sortie" was named by every claim containing that word — and
/// the weekday check then adjudicated a sentence that was never about it.
#[test]
fn a_session_named_for_an_ordinary_training_word_is_not_matched() {
    let record = AthleteRecord {
        has_provider: true,
        activities: vec![RecordedActivity {
            date: day(1),
            sport: SportType::Ride,
            name: "Sortie".to_owned(),
            distance_km: Some(40.0),
            duration_min: 90.0,
            elevation_m: Some(300.0),
        }],
    };

    let outcome = check(&claim("Dimanche, ta sortie était courte."), &record);

    assert!(
        outcome
            .as_ref()
            .is_none_or(|v| v.status != ClaimStatus::Contradicted),
        "'sortie' describes any session; it identifies none: {outcome:?}"
    );
}

/// Strava's default names are the common case of the above.
///
/// An unedited activity is called "Morning Ride", "Afternoon Run", "Lunch
/// Ride". Nothing about those words picks out a session, and an athlete with
/// one of them on file had every claim mentioning the same ordinary words
/// adjudicated against it.
#[test]
fn a_default_provider_name_is_not_matched() {
    for name in ["Morning Ride", "Afternoon Run", "Long Run"] {
        let record = AthleteRecord {
            has_provider: true,
            activities: vec![RecordedActivity {
                date: day(1),
                sport: SportType::Ride,
                name: name.to_owned(),
                distance_km: Some(40.0),
                duration_min: 90.0,
                elevation_m: Some(300.0),
            }],
        };

        let outcome = check(
            &claim("Sunday's long run was your best morning ride in weeks."),
            &record,
        );

        assert!(
            outcome
                .as_ref()
                .is_none_or(|v| v.status != ClaimStatus::Contradicted),
            "{name:?} is a description, not an identity: {outcome:?}"
        );
    }
}

/// A distinctive name still identifies its session — the guard above must not
/// have disarmed the whole check.
#[test]
fn a_distinctive_name_is_still_matched() {
    let outcome = check(
        &claim("Dimanche, Road 2 AUS était la plus grosse de la semaine."),
        &raphs_week(),
    )
    .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Contradicted,
        "'road' is nobody's generic vocabulary: {}",
        outcome.explanation
    );
}

/// Metres of distance are not metres of climbing.
///
/// Every metres token routed to the elevation field, so *"tu as tenu tes 400
/// m"* — a track rep — was checked against the session's total ascent. With
/// 400 m of climbing on file it came back **Supported**: the layer corroborated
/// a claim about an interval using a hill.
#[test]
fn a_distance_in_metres_is_not_checked_against_climbing() {
    let record = AthleteRecord {
        has_provider: true,
        activities: vec![RecordedActivity {
            date: day(1),
            sport: SportType::Run,
            name: "Piste".to_owned(),
            distance_km: Some(12.0),
            duration_min: 55.0,
            elevation_m: Some(400.0),
        }],
    };

    let outcome = check(&claim("Tu as tenu tes 400 m."), &record)
        .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Unverifiable,
        "400 m of climbing is not evidence about a 400 m rep: {}",
        outcome.explanation
    );
}

/// And the same figure IS matched against distance when that is what it is.
#[test]
fn a_distance_in_metres_is_matched_against_distance() {
    let record = AthleteRecord {
        has_provider: true,
        activities: vec![RecordedActivity {
            date: day(1),
            sport: SportType::Swim,
            name: "Piscine".to_owned(),
            distance_km: Some(1.5),
            duration_min: 40.0,
            elevation_m: None,
        }],
    };

    let outcome = check(&claim("Tu as fait 1500 m."), &record)
        .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Supported,
        "1.5 km on record IS 1500 m; routing every metres figure to elevation \
         made this unfalsifiable: {}",
        outcome.explanation
    );
}

/// An elevation cue in the sentence keeps the climbing reading — the split must
/// not cost registre#249 its fix.
#[test]
fn an_elevation_cue_still_routes_metres_to_climbing() {
    for text in [
        "Cette sortie t'a fait 2391 m de dénivelé.",
        "Tu as grimpé 2391 m.",
        "Belle journée: 2391 m+.",
    ] {
        let outcome =
            check(&claim(text), &raphs_week()).expect("the layer must adjudicate its own category");
        assert_eq!(
            outcome.status,
            ClaimStatus::Supported,
            "2391 m of ascent is on record: {text:?} gave {}",
            outcome.explanation
        );
    }
}

/// Thousands separated by a space are one number, not its last group.
///
/// The digit scan stopped at the space, so "2 391 m" was read as 391 — and the
/// unit where four-digit values actually live is exactly the one this broke.
#[test]
fn space_grouped_thousands_are_one_number() {
    for text in [
        "Cette sortie t'a fait 2 391 m de dénivelé.",
        // U+202F, what a French number formatter emits.
        "Cette sortie t'a fait 2\u{202f}391 m de dénivelé.",
        // U+00A0.
        "Cette sortie t'a fait 2\u{a0}391 m de dénivelé.",
    ] {
        let outcome =
            check(&claim(text), &raphs_week()).expect("the layer must adjudicate its own category");
        assert_eq!(
            outcome.status,
            ClaimStatus::Supported,
            "2 391 m is the 2391 m on record, not 391: {text:?} gave {}",
            outcome.explanation
        );
    }
}

/// Grouping must not merge two unrelated numbers.
#[test]
fn a_space_between_two_figures_does_not_group_them() {
    let outcome = check(&claim("Tu as fait 26 km en 200 minutes."), &raphs_week())
        .expect("the layer must adjudicate its own category");

    assert_eq!(
        outcome.status,
        ClaimStatus::Supported,
        "26 km and 200 min are both on record for Passion rando; reading them \
         as one number would lose both: {}",
        outcome.explanation
    );
}
