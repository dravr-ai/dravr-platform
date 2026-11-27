// ABOUTME: Holds the product's shipped form strings to dravr-colleur's form vocabulary, in five locales
// ABOUTME: Reads packages/i18n, so it lives where the catalogue does: a renamed or added band fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The product prints form as "form -64% of CTL, deep fatigue": a share of
//! the athlete's own fitness, banded. dravr-colleur checks a reply written in
//! that vocabulary. Nothing at runtime joins the two, so this test reads the
//! catalogue the platform ships and runs its strings through colleur's form
//! check: the string for a reading is supported, another band's is not.

use dravr_cageux::algorithms::training_load::DailyTrainingLoad;
use dravr_cageux::form_reading::FormReading;
use dravr_cageux::training_load::FormBand;
use dravr_colleur::claims::{ClaimCategory, ClaimStatus};
use dravr_colleur::personalized;
use dravr_colleur::{
    AthleteMetrics, ClaimSource, ConservativeStrategy, ExtractedClaim, PersonalizedContext,
};
use dravr_tronc::dates::CivilDate;

// The five catalogues the platform ships.
const LOCALES: [(&str, &str); 5] = [
    (
        "en",
        include_str!("../../../../packages/i18n/src/locales/en/translation.json"),
    ),
    (
        "fr",
        include_str!("../../../../packages/i18n/src/locales/fr/translation.json"),
    ),
    (
        "es",
        include_str!("../../../../packages/i18n/src/locales/es/translation.json"),
    ),
    (
        "de",
        include_str!("../../../../packages/i18n/src/locales/de/translation.json"),
    ),
    (
        "pt",
        include_str!("../../../../packages/i18n/src/locales/pt/translation.json"),
    ),
];

/// The band keys the catalogue uses under `home.status.band`.
fn band_key(band: FormBand) -> Option<&'static str> {
    match band {
        FormBand::InsufficientHistory => None,
        FormBand::DeepFatigue => Some("deepFatigue"),
        FormBand::HeavyBlock => Some("heavyBlock"),
        FormBand::Productive => Some("productive"),
        FormBand::Balanced => Some("balanced"),
        FormBand::Fresh => Some("fresh"),
        FormBand::Detraining => Some("detraining"),
    }
}

fn locale_string(catalogue: &str, path: &[&str]) -> String {
    let mut value: serde_json::Value = serde_json::from_str(catalogue).unwrap();
    for key in path {
        value = value[key].take();
    }
    value.as_str().unwrap().to_owned()
}

/// A reading on a day whose CTL did not move overnight.
fn reading(ctl: f64, atl: f64) -> FormReading {
    FormReading::from_daily_load(&DailyTrainingLoad {
        date: CivilDate::from_ymd(2026, 10, 9).unwrap(),
        ctl,
        atl,
        tsb: ctl - atl,
        form_ctl: ctl,
    })
}

/// TSB -77 at CTL 120, -64% of fitness.
fn deep() -> FormReading {
    reading(120.0, 197.0)
}

fn heavy() -> FormReading {
    reading(100.0, 125.0)
}

fn productive() -> FormReading {
    reading(100.0, 113.0)
}

fn balanced() -> FormReading {
    reading(100.0, 98.0)
}

fn fresh() -> FormReading {
    reading(100.0, 88.0)
}

fn detraining() -> FormReading {
    reading(100.0, 70.0)
}

fn check(form: FormReading, text: &str) -> Option<ClaimStatus> {
    let metrics = AthleteMetrics {
        form: Some(form),
        data_days: 30,
        ..AthleteMetrics::default()
    };
    let strategy = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &metrics,
        tolerance: &strategy,
    };
    let claim = ExtractedClaim {
        text: text.to_owned(),
        category: ClaimCategory::Physiological,
        source: ClaimSource::Reply,
    };
    personalized::check(&claim, &ctx).map(|v| v.status)
}

#[test]
fn the_fixture_athletes_sit_in_the_bands_their_names_say() {
    for (name, reading, band) in [
        ("deep", deep(), FormBand::DeepFatigue),
        ("heavy", heavy(), FormBand::HeavyBlock),
        ("productive", productive(), FormBand::Productive),
        ("balanced", balanced(), FormBand::Balanced),
        ("fresh", fresh(), FormBand::Fresh),
        ("detraining", detraining(), FormBand::Detraining),
    ] {
        assert_eq!(
            reading.band, band,
            "bug: the {name} fixture athlete left the {band:?} band the locale checks rely on"
        );
    }
}

#[test]
fn the_products_own_form_string_is_read_in_every_locale() {
    for (locale, catalogue) in LOCALES {
        let template = locale_string(catalogue, &["home", "status", "form"]);
        for form in [deep(), productive(), balanced(), fresh(), detraining()] {
            let pct = form.form_pct.unwrap().round();
            let right = template.replace("{{pct}}", &format!("{pct:.0}"));
            assert_eq!(
                check(form, &right),
                Some(ClaimStatus::Supported),
                "{locale}: {right}"
            );
            let wrong = template.replace("{{pct}}", &format!("{:.0}", pct + 40.0));
            assert_eq!(
                check(form, &wrong),
                Some(ClaimStatus::Contradicted),
                "{locale}: {wrong}"
            );
        }
    }
}

#[test]
fn every_band_name_the_product_ships_is_recognised_in_every_locale() {
    for (locale, catalogue) in LOCALES {
        let template = locale_string(catalogue, &["home", "status", "form"]);
        for form in [
            deep(),
            heavy(),
            productive(),
            balanced(),
            fresh(),
            detraining(),
        ] {
            let key = band_key(form.band).unwrap();
            let name = locale_string(catalogue, &["home", "status", "band", key]);
            let pct = form.form_pct.unwrap().round();
            let stated = template.replace("{{pct}}", &format!("{pct:.0}"));

            let right = format!("{stated}, {name}");
            assert_eq!(
                check(form, &right),
                Some(ClaimStatus::Supported),
                "{locale} {key}: {right}"
            );

            // The correct share beside another band's name: only the band
            // name can make this wrong, so the name must have been read.
            let other = if form.band == FormBand::DeepFatigue {
                "detraining"
            } else {
                "deepFatigue"
            };
            let other_name = locale_string(catalogue, &["home", "status", "band", other]);
            let wrong = format!("{stated}, {other_name}");
            assert_eq!(
                check(form, &wrong),
                Some(ClaimStatus::Contradicted),
                "{locale} {key} named as {other}: {wrong}"
            );
        }
    }
}

#[test]
fn the_group_flag_detail_string_is_read_in_every_locale() {
    for (locale, catalogue) in LOCALES {
        let template = locale_string(catalogue, &["groups", "flagDetail", "deepFatigue"]);
        let right = template.replace("{{pct}}", "-64").replace("{{tsb}}", "-77");
        assert_eq!(
            check(deep(), &right),
            Some(ClaimStatus::Supported),
            "{locale}: {right}"
        );
        let wrong = template.replace("{{pct}}", "-30").replace("{{tsb}}", "-77");
        assert_eq!(
            check(deep(), &wrong),
            Some(ClaimStatus::Contradicted),
            "{locale}: {wrong}"
        );
    }
}
