// ABOUTME: Maps user-friendly sport-type strings (LLM/UI input) to canonical SportType variants
// ABOUTME: Handles English and French aliases plus separator/casing variation (xc_ski, xcski, ski-de-fond)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_cageux::models::sport::SportType;

/// Resolve any reasonable sport-type input string to a canonical
/// [`SportType`] enum variant.
///
/// The input is normalised (trim, lowercase, strip `_`/`-`/spaces)
/// before matching, so `nordic_ski`, `Nordic Ski`, and `nordicski`
/// all resolve to [`SportType::CrossCountrySkiing`].
///
/// Returns `None` when the input doesn't match any known alias —
/// callers can then fall back to `SportType::Other(input)` for
/// pass-through filtering, but most LLM/UI inputs should resolve to
/// a built-in variant.
///
/// French aliases are included because the messaging coach is
/// bilingual; users say "ski de fond", "vélo", or "rando" naturally.
#[must_use]
pub fn resolve_sport_type(input: &str) -> Option<SportType> {
    let normalised = input
        .trim()
        .to_lowercase()
        .replace(['_', '-', ' ', '\''], "");
    Some(match normalised.as_str() {
        // Run / Walk / Hike
        "run" | "running" | "course" | "courseapied" | "jogging" => SportType::Run,
        "trailrun" | "trailrunning" | "trail" | "courseentrail" => SportType::TrailRunning,
        "walk" | "walking" | "marche" | "marcheapied" => SportType::Walk,
        "hike" | "hiking" | "rando" | "randonnée" | "randonnee" => SportType::Hike,
        "virtualrun" | "treadmill" | "tapis" | "tapisdecourse" => SportType::VirtualRun,

        // Cycling
        "ride" | "cycling" | "bike" | "bicycle" | "vélo" | "velo" | "sortieavélo"
        | "sortieavelo" => SportType::Ride,
        "mountainbike" | "mtb" | "vtt" => SportType::MountainBike,
        "gravelride" | "gravel" | "gravelvelo" => SportType::GravelRide,
        "ebikeride" | "ebike" | "véloélectrique" | "veloelectrique" => SportType::EbikeRide,
        "virtualride" | "indoorbike" | "vélointérieur" | "velointerieur" | "trainer"
        | "homebike" => SportType::VirtualRide,

        // Winter sports
        "crosscountryskiing" | "crosscountryski" | "xcski" | "xc" | "nordicski"
        | "nordicskiing" | "nordic" | "skidefond" | "skifond" | "skinordique" => {
            SportType::CrossCountrySkiing
        }
        "alpineskiing" | "alpineski" | "downhill" | "downhillskiing" | "skialpin" | "ski"
        | "skialpinisme" => SportType::AlpineSkiing,
        "backcountryskiing" | "backcountryski" | "backcountry" | "skitour" | "skidetour"
        | "skihorspiste" => SportType::BackcountrySkiing,
        "snowshoe" | "snowshoeing" | "raquette" | "raquettes" => SportType::Snowshoe,
        "snowboard" | "snowboarding" | "plancheaneige" => SportType::Snowboarding,
        "iceskate" | "iceskating" | "patin" | "patinage" => SportType::IceSkating,

        // Water sports
        "swim" | "swimming" | "natation" | "nage" => SportType::Swim,
        "kayak" | "kayaking" => SportType::Kayaking,
        "canoe" | "canoeing" => SportType::Canoeing,
        "row" | "rowing" | "aviron" => SportType::Rowing,
        "paddleboard" | "paddleboarding" | "sup" | "standuppaddle" => SportType::Paddleboarding,
        "surf" | "surfing" => SportType::Surfing,
        "kitesurf" | "kitesurfing" => SportType::Kitesurfing,

        // Strength and fitness
        "strengthtraining" | "weighttraining" | "weights" | "musculation" | "muscu" => {
            SportType::StrengthTraining
        }
        "crossfit" => SportType::Crossfit,
        "yoga" => SportType::Yoga,
        "pilates" => SportType::Pilates,
        "rockclimbing" | "climbing" | "escalade" | "grimpe" => SportType::RockClimbing,
        "workout" | "exercise" | "entrainement" | "entraînement" | "exo" => SportType::Workout,

        // Team / racket / target sports
        "soccer" | "football" | "foot" => SportType::Soccer,
        "basketball" | "basket" => SportType::Basketball,
        "tennis" => SportType::Tennis,
        "golf" => SportType::Golf,
        "skateboard" | "skateboarding" | "skate" => SportType::Skateboarding,
        "inlineskate" | "inlineskating" | "rollerblade" | "rollers" | "patinaroulettes"
        | "patinsaroulettes" => SportType::InlineSkating,

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(resolve_sport_type("muscu"), Some(SportType::StrengthTraining));
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
}
