// ABOUTME: Localized short sport and self-report labels for the prose surfaces the athlete and the agent both read
// ABOUTME: Five-locale tables per SportType and Feel variant, exhaustive so a new variant cannot ship untranslated
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The sport nouns and self-report words an activity row carries.
//!
//! fr "course à pied" / "rando", not the English `display_name`, so a French
//! list reads natively. Split out of `fitness_support` so the tables can be
//! found and extended on their own.

use pierre_core::models::{Feel, SportType};

/// Column of a `[fr, en, es, de, pt]` table for a BCP-47 locale; English for
/// any locale without a column.
fn locale_column(locale: &str) -> usize {
    match locale.get(0..2).unwrap_or("en") {
        "fr" => 0,
        "es" => 2,
        "de" => 3,
        "pt" => 4,
        _ => 1,
    }
}

/// Localized short sport-type label for the activity list, keyed by BCP-47
/// locale.
///
/// The names are intentionally short agent/athlete nouns (fr "course à
/// pied"/"rando"/"ski de fond", not the English `display_name`) so a French
/// chat reads natively. Falls back to English for an unrecognized locale and
/// keeps the provider-supplied label for `SportType::Other`; the match has no
/// wildcard arm, so a new `SportType` variant fails compilation here until its
/// five translations are added. `pub` so the locale table (human-language rows
/// the compiler can't check for wrong-column pastes) is exercisable by the
/// render tests.
#[must_use]
pub fn localized_sport_name(sport: &SportType, locale: &str) -> String {
    // [fr, en, es, de, pt]
    let names: [&str; 5] = match sport {
        SportType::Other(s) => return s.clone(),
        SportType::Run => ["course à pied", "run", "carrera", "laufen", "corrida"],
        SportType::Ride => ["vélo", "bike ride", "bici", "radtour", "pedalada"],
        SportType::Swim => ["natation", "swim", "natación", "schwimmen", "natação"],
        SportType::Walk => ["marche", "walk", "caminata", "spaziergang", "caminhada"],
        SportType::Hike => ["rando", "hike", "senderismo", "wanderung", "trilha"],
        SportType::VirtualRide => [
            "home trainer",
            "indoor ride",
            "bici indoor",
            "indoor-radfahren",
            "bike indoor",
        ],
        SportType::VirtualRun => [
            "tapis de course",
            "treadmill run",
            "cinta de correr",
            "laufband",
            "esteira",
        ],
        SportType::Workout => [
            "entraînement",
            "workout",
            "entrenamiento",
            "training",
            "treino",
        ],
        SportType::Yoga => ["yoga", "yoga", "yoga", "yoga", "yoga"],
        SportType::EbikeRide => [
            "vélo électrique",
            "e-bike ride",
            "bici eléctrica",
            "e-bike-tour",
            "e-bike",
        ],
        SportType::MountainBike => [
            "VTT",
            "mountain bike",
            "btt",
            "mountainbike",
            "mountain bike",
        ],
        SportType::GravelRide => ["gravel", "gravel ride", "gravel", "gravel-tour", "gravel"],
        SportType::CrossCountrySkiing => [
            "ski de fond",
            "cross-country ski",
            "esquí de fondo",
            "langlauf",
            "esqui de fundo",
        ],
        SportType::AlpineSkiing => [
            "ski alpin",
            "alpine ski",
            "esquí alpino",
            "ski alpin",
            "esqui alpino",
        ],
        SportType::Snowboarding => [
            "snowboard",
            "snowboard",
            "snowboard",
            "snowboard",
            "snowboard",
        ],
        SportType::Snowshoe => [
            "raquette",
            "snowshoe",
            "raquetas de nieve",
            "schneeschuhwandern",
            "raquetes de neve",
        ],
        SportType::IceSkating => [
            "patin à glace",
            "ice skating",
            "patinaje sobre hielo",
            "schlittschuhlaufen",
            "patinação no gelo",
        ],
        SportType::BackcountrySkiing => [
            "ski de rando",
            "backcountry ski",
            "esquí de travesía",
            "skitour",
            "esqui de travessia",
        ],
        SportType::Kayaking => ["kayak", "kayak", "kayak", "kajak", "caiaque"],
        SportType::Canoeing => ["canoë", "canoe", "canoa", "kanu", "canoagem"],
        SportType::Rowing => ["aviron", "rowing", "remo", "rudern", "remo"],
        SportType::Paddleboarding => [
            "paddle",
            "paddleboard",
            "paddle surf",
            "stand-up-paddling",
            "stand up paddle",
        ],
        SportType::Surfing => ["surf", "surf", "surf", "surfen", "surf"],
        SportType::Kitesurfing => ["kitesurf", "kitesurf", "kitesurf", "kitesurfen", "kitesurf"],
        SportType::StrengthTraining => [
            "musculation",
            "strength training",
            "fuerza",
            "krafttraining",
            "musculação",
        ],
        SportType::Crossfit => ["CrossFit", "CrossFit", "CrossFit", "CrossFit", "CrossFit"],
        SportType::Pilates => ["Pilates", "Pilates", "Pilates", "Pilates", "Pilates"],
        SportType::RockClimbing => ["escalade", "climbing", "escalada", "klettern", "escalada"],
        SportType::TrailRunning => ["trail", "trail run", "trail", "trailrunning", "trail run"],
        SportType::Soccer => ["foot", "soccer", "fútbol", "fußball", "futebol"],
        SportType::Basketball => [
            "basket",
            "basketball",
            "baloncesto",
            "basketball",
            "basquete",
        ],
        SportType::Tennis => ["tennis", "tennis", "tenis", "tennis", "tênis"],
        SportType::Golf => ["golf", "golf", "golf", "golf", "golfe"],
        SportType::Skateboarding => ["skate", "skate", "skate", "skateboarden", "skate"],
        SportType::InlineSkating => [
            "roller",
            "inline skating",
            "patinaje en línea",
            "inlineskaten",
            "patinação inline",
        ],
    };
    names[locale_column(locale)].to_owned()
}

/// How the athlete said they felt, as the localized phrase an activity row
/// carries (fr "ressenti : mauvais", en "felt poor").
///
/// The rating is spelled out rather than numbered: the one provider that
/// reports it ranks 1 as the best, and a number invites exactly the backwards
/// reading the named [`Feel`] scale exists to prevent. The match has no
/// wildcard arm, so a new variant fails compilation here until its five
/// translations are added.
#[must_use]
pub fn localized_feel(feel: Feel, locale: &str) -> &'static str {
    // [fr, en, es, de, pt]
    let phrases: [&str; 5] = match feel {
        Feel::Strong => [
            "ressenti : très bon",
            "felt strong",
            "sensación: muy buena",
            "Gefühl: sehr gut",
            "sensação: muito boa",
        ],
        Feel::Good => [
            "ressenti : bon",
            "felt good",
            "sensación: buena",
            "Gefühl: gut",
            "sensação: boa",
        ],
        Feel::Normal => [
            "ressenti : normal",
            "felt normal",
            "sensación: normal",
            "Gefühl: normal",
            "sensação: normal",
        ],
        Feel::Poor => [
            "ressenti : mauvais",
            "felt poor",
            "sensación: mala",
            "Gefühl: schlecht",
            "sensação: ruim",
        ],
        Feel::Weak => [
            "ressenti : très mauvais",
            "felt weak",
            "sensación: muy mala",
            "Gefühl: sehr schlecht",
            "sensação: muito ruim",
        ],
    };
    phrases[locale_column(locale)]
}
