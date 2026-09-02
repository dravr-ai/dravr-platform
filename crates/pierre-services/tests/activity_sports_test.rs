// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the server's activity-sport vocabulary to the one the clients import
// ABOUTME: Every canonical sport, every alias and every fold rule answers the same key on both sides

//! Tests for the shared activity-sport vocabulary.

use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_services::activity_sports::{activity_sport_label_key, sport_label};

#[test]
fn a_wire_sport_names_its_catalogue_key() {
    assert_eq!(activity_sport_label_key("Run"), Some("app.sportRun"));
    assert_eq!(activity_sport_label_key("ride"), Some("app.sportRide"));
    assert_eq!(
        activity_sport_label_key("trail_run"),
        Some("app.sportTrailRunning")
    );
}

#[test]
fn the_fold_is_the_clients_fold_camel_case_included() {
    // The client folds on whitespace and hyphens only, so a camel-case wire
    // spelling with no separator resolves nothing on either side. Pinned so
    // the two implementations stay identical rather than quietly diverging.
    assert_eq!(activity_sport_label_key("TrailRun"), None);
    assert_eq!(activity_sport_label_key("VirtualRide"), None);
}

#[test]
fn the_fold_matches_the_client_rules() {
    // Spaces, hyphens and case fold onto the canonical name; a version suffix
    // is dropped; an alias resolves to what it aliases.
    for spelling in [
        "Cross Country Skiing",
        "cross-country-skiing",
        "CROSS_COUNTRY_SKIING",
        "nordic_ski",
    ] {
        assert_eq!(
            activity_sport_label_key(spelling),
            Some("app.sportCrossCountrySkiing"),
            "{spelling} should fold onto the canonical cross-country skiing key"
        );
    }
    assert_eq!(
        activity_sport_label_key("virtual_ride_v2"),
        Some("app.sportVirtualRide")
    );
}

#[test]
fn an_unknown_sport_has_no_key() {
    assert_eq!(activity_sport_label_key("underwater_basket_weaving"), None);
    assert_eq!(activity_sport_label_key(""), None);
}

#[test]
fn every_key_the_table_names_ships_in_the_catalogue() {
    // The vocabulary is only useful if the registry can render what it names:
    // a key with no catalogue row would render as an empty string and the
    // caller would silently fall back to the wire spelling.
    let registry = MessagingStringsRegistry::new();
    let table = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../../packages/shared-constants/src/activity-sports.json"
    ))
    .ok();
    let keys = table
        .as_ref()
        .and_then(|table| table["labelKeys"].as_object());
    assert!(
        keys.is_some_and(|keys| keys.len() > 20),
        "the shared activity-sport vocabulary parses and is not a stub"
    );

    if let Some(keys) = keys {
        for key in keys.values().filter_map(serde_json::Value::as_str) {
            for locale in ["fr", "en", "es", "de", "pt"] {
                assert!(
                    !registry.get(key, locale).trim().is_empty(),
                    "{key} renders empty in {locale}"
                );
            }
        }
    }
}

#[test]
fn a_server_rendered_sport_reads_in_the_athletes_language() {
    // The seam the messaging coach proposal goes through: a French athlete
    // whose most-logged sport is trail running is greeted with the French
    // word, not the wire spelling that used to be interpolated raw.
    let registry = MessagingStringsRegistry::new();

    assert_eq!(
        sport_label(&registry, "trail_run", "fr"),
        "Course en sentier"
    );
    assert_eq!(sport_label(&registry, "trail_run", "en"), "Trail running");
    assert_eq!(sport_label(&registry, "run", "fr"), "Course");

    // A spelling the vocabulary has no word for keeps its wire text in every
    // locale, so the sentence still names something rather than nothing.
    assert_eq!(
        sport_label(&registry, "underwater_basket_weaving", "fr"),
        "underwater_basket_weaving"
    );
}
