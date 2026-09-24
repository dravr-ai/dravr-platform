// ABOUTME: Tests for sport type alias resolution
// ABOUTME: English, LLM and French aliases, separators and casing, family heads

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::*;

#[test]
fn resolves_english_canonical() {
    assert_eq!(resolve_sport_type("run"), Some(SportType::Run));
    assert_eq!(
        resolve_sport_type("cross_country_skiing"),
        Some(SportType::CrossCountrySkiing)
    );
}

#[test]
fn resolves_llm_aliases() {
    assert_eq!(
        resolve_sport_type("nordicski"),
        Some(SportType::CrossCountrySkiing)
    );
    assert_eq!(
        resolve_sport_type("xc_ski"),
        Some(SportType::CrossCountrySkiing)
    );
    assert_eq!(resolve_sport_type("mtb"), Some(SportType::MountainBike));
    // Strava API sport_type values (granular), not just short forms.
    assert_eq!(
        resolve_sport_type("MountainBikeRide"),
        Some(SportType::MountainBike)
    );
    assert_eq!(
        resolve_sport_type("GravelRide"),
        Some(SportType::GravelRide)
    );
}

#[test]
fn resolves_french_aliases() {
    assert_eq!(
        resolve_sport_type("ski de fond"),
        Some(SportType::CrossCountrySkiing)
    );
    assert_eq!(resolve_sport_type("vélo"), Some(SportType::Ride));
    assert_eq!(resolve_sport_type("randonnée"), Some(SportType::Hike));
    assert_eq!(resolve_sport_type("vtt"), Some(SportType::MountainBike));
    assert_eq!(
        resolve_sport_type("muscu"),
        Some(SportType::StrengthTraining)
    );
}

#[test]
fn handles_separator_and_casing_variation() {
    assert_eq!(
        resolve_sport_type("Cross-Country Skiing"),
        Some(SportType::CrossCountrySkiing)
    );
    assert_eq!(
        resolve_sport_type("CROSSCOUNTRYSKIING"),
        Some(SportType::CrossCountrySkiing)
    );
    assert_eq!(
        resolve_sport_type("  xc-ski  "),
        Some(SportType::CrossCountrySkiing)
    );
}

#[test]
fn returns_none_for_unknown() {
    assert_eq!(resolve_sport_type("quidditch"), None);
    assert_eq!(resolve_sport_type(""), None);
}

/// The two halves of the family rule must agree, in one direction and the
/// other. A discipline given a head that the head does not widen to — or
/// widened to by a head it does not name — is the drift this pair exists to
/// prevent, and it is one system's internal consistency, not two systems
/// policing each other.
#[test]
fn family_head_and_family_match_agree() {
    let disciplines = [
        SportType::Run,
        SportType::TrailRunning,
        SportType::VirtualRun,
        SportType::Ride,
        SportType::MountainBike,
        SportType::GravelRide,
        SportType::EbikeRide,
        SportType::VirtualRide,
        SportType::Swim,
        SportType::Hike,
        SportType::Walk,
    ];

    for sport in &disciplines {
        match sport_family_head(sport) {
            Some(head) => assert!(
                sport_matches_family(sport, &head),
                "{sport:?} claims head {head:?}, but that head does not widen to it"
            ),
            None => assert!(
                !disciplines.iter().any(|other| other != sport
                    && sport_matches_family(sport, other)
                    && sport_family_head(other).is_none()),
                "{sport:?} reports no head yet another head widens to it"
            ),
        }
    }
}
