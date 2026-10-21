// ABOUTME: Tests for form claims in the personalized layer — a share of fitness, a band name and a bare TSB, in five locales
// ABOUTME: Pins that form is checked as the product frames it (a share of CTL, banded) and that a percentage never meets raw TSB
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The product never prints form as a bare TSB. It prints "TSB -77 (-64% of
//! CTL, deep fatigue)": the share of the athlete's own fitness, banded. A reply
//! written that way is checked against the share and the band; only a bare TSB
//! meets the raw number, and a percentage never does, so a correct percentage
//! beside "current form" is not contradicted by the TSB.
//!
//! The athletes below sit in each band, so the same sentence is right for one
//! and wrong for another. Their readings come from the daily row's own
//! `form_reading`, the constructor every tool renders through.

use chrono::NaiveDate;
use pierre_core::models::{DailyTrainingState, FormBand, FormReading};
use pierre_evals::personalized::check as personalized_check;
use pierre_evals::{
    extract_heuristic, AthleteMetrics, ClaimSource, ConservativeStrategy, ExtractedClaim,
    PersonalizedContext, TightStrategy, ToleranceCall, ToleranceStrategy,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus};

// The product's own strings are the vocabulary a reply is checked in. Nothing
// at runtime joins the locale catalogue and this layer's band names, so this
// test reads the catalogue (the same five files the platform ships) and holds
// the two together: a band renamed or added there fails here.
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
    DailyTrainingState {
        ctl,
        atl,
        tsb: ctl - atl,
        form_ctl: ctl,
        ..DailyTrainingState::zero(NaiveDate::from_ymd_opt(2026, 10, 9).unwrap())
    }
    .form_reading()
}

/// The athlete from 2026-09-02: TSB -77 at CTL 120, -64% of fitness.
fn deep() -> FormReading {
    reading(120.0, 197.0)
}

/// TSB -25 at CTL 100.
fn heavy() -> FormReading {
    reading(100.0, 125.0)
}

/// TSB -13 at CTL 100.
fn productive() -> FormReading {
    reading(100.0, 113.0)
}

/// TSB +2 at CTL 100.
fn balanced() -> FormReading {
    reading(100.0, 98.0)
}

/// TSB +12 at CTL 100.
fn fresh() -> FormReading {
    reading(100.0, 88.0)
}

/// TSB +30 at CTL 100.
fn detraining() -> FormReading {
    reading(100.0, 70.0)
}

/// CTL 15: no chronic base to take a share of.
fn no_base() -> FormReading {
    reading(15.0, 25.0)
}

fn metrics(form: FormReading) -> AthleteMetrics {
    AthleteMetrics {
        form: Some(form),
        data_days: 30,
        ..AthleteMetrics::default()
    }
}

fn claim(text: &str) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category: ClaimCategory::Physiological,
        source: ClaimSource::Reply,
    }
}

fn check_with(
    form: FormReading,
    strategy: &dyn ToleranceStrategy,
    text: &str,
) -> Option<ClaimStatus> {
    let m = metrics(form);
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: strategy,
    };
    personalized_check(&claim(text), &ctx).map(|v| v.status)
}

fn check(form: FormReading, text: &str) -> Option<ClaimStatus> {
    check_with(form, &ConservativeStrategy::default(), text)
}

#[test]
fn the_fixture_athletes_sit_in_the_bands_their_names_say() {
    assert_eq!(deep().band, FormBand::DeepFatigue);
    assert_eq!(heavy().band, FormBand::HeavyBlock);
    assert_eq!(productive().band, FormBand::Productive);
    assert_eq!(balanced().band, FormBand::Balanced);
    assert_eq!(fresh().band, FormBand::Fresh);
    assert_eq!(detraining().band, FormBand::Detraining);
    assert_eq!(no_base().band, FormBand::InsufficientHistory);
}

// ---------------------------------------------------------------------------
// A share of fitness
// ---------------------------------------------------------------------------

/// A chat reply reaches this layer only through the heuristic extractor, so a
/// form sentence it drops is never checked at all.
#[test]
fn a_form_sentence_in_a_reply_reaches_the_form_check_in_every_locale() {
    for sentence in [
        "You're at -64% of your fitness right now.",
        "Your current form is -64%.",
        "Your TSB is -77 this morning.",
        "Tu es à −64 % de ta condition physique.",
        "Ta forme est en fatigue profonde.",
        "Estás al -64 % de tu condición física.",
        "Du liegst bei -64 % deiner Fitness.",
        "Estás a -64% da tua condição física.",
    ] {
        let claims = extract_heuristic(sentence);
        assert_eq!(claims.len(), 1, "{sentence}: {claims:?}");
        let m = metrics(deep());
        let ctx = PersonalizedContext {
            metrics: &m,
            tolerance: &ConservativeStrategy::default(),
        };
        assert_eq!(
            personalized_check(&claims[0], &ctx).map(|v| v.status),
            Some(ClaimStatus::Supported),
            "{sentence}"
        );
    }
}

#[test]
fn a_share_of_fitness_is_checked_against_form_pct_in_every_locale() {
    for (right, wrong) in [
        (
            "You're at -64% of your fitness right now.",
            "You're at -30% of your fitness right now.",
        ),
        (
            "Tu es à −64 % de ta condition physique.",
            "Tu es à −30 % de ta condition physique.",
        ),
        (
            "Estás al -64 % de tu condición física.",
            "Estás al -30 % de tu condición física.",
        ),
        (
            "Du liegst bei -64 % deiner Fitness.",
            "Du liegst bei -30 % deiner Fitness.",
        ),
        (
            "Estás a -64% da tua condição física.",
            "Estás a -30% da tua condição física.",
        ),
    ] {
        assert_eq!(
            check(deep(), right),
            Some(ClaimStatus::Supported),
            "{right}"
        );
        assert_eq!(
            check(deep(), wrong),
            Some(ClaimStatus::Contradicted),
            "{wrong}"
        );
    }
}

#[test]
fn a_percentage_near_current_form_is_not_compared_to_raw_tsb() {
    // The athlete's TSB is -77 and their form is -64% of their fitness; the
    // figure beside "current form" is the share, not the TSB.
    assert_eq!(
        check(deep(), "Your current form is -64%."),
        Some(ClaimStatus::Supported),
        "the product's own share must not be contradicted by the raw TSB"
    );
    assert_eq!(
        check(deep(), "Ta forme actuelle est de −64 %."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        check(deep(), "Your current form is -77%."),
        Some(ClaimStatus::Contradicted),
        "a percentage that happens to equal the TSB must not be supported by it"
    );
}

#[test]
fn a_share_within_tolerance_is_supported_and_one_far_off_is_contradicted() {
    assert_eq!(
        check(deep(), "You're at -61% of your fitness."),
        Some(ClaimStatus::Supported),
        "3 points off a -64% reading is rounding, not a mistake"
    );
    assert_eq!(
        check(deep(), "You're at -45% of your fitness."),
        Some(ClaimStatus::Contradicted)
    );
}

/// The strategy's buffer widens a range outward whatever the sign of its ends.
/// Form is negative in a hard block, and a buffer that scaled a negative end by
/// `1 - margin` would sit inside the range, leaving no claim to fall in it.
#[test]
fn the_strategy_buffer_applies_to_a_share_below_zero() {
    let beside = "You're at -56% of your fitness.";
    assert_eq!(
        check(deep(), beside),
        None,
        "-56 is outside the ±5 tolerance and inside the conservative buffer"
    );
    assert_eq!(
        check_with(deep(), &TightStrategy, beside),
        Some(ClaimStatus::Contradicted),
        "the tight strategy has no buffer"
    );
}

#[test]
fn a_margin_widens_a_negative_range_outward() {
    let s = ConservativeStrategy::default();
    assert_eq!(
        s.assess(-56.0, (-69.0, -59.0)),
        ToleranceCall::Indeterminate
    );
    assert_eq!(s.assess(-60.0, (-69.0, -59.0)), ToleranceCall::Supported);
    assert_eq!(s.assess(-30.0, (-69.0, -59.0)), ToleranceCall::Contradicted);
}

#[test]
fn a_positive_form_is_written_without_a_sign_and_read_that_way() {
    let form = fresh();
    for right in [
        "Form 12% of your fitness",
        "Your form is at +12% of your fitness.",
        "You're 12% above your fitness.",
        "Ta forme est à 12 % de ta condition physique.",
    ] {
        assert_eq!(check(form, right), Some(ClaimStatus::Supported), "{right}");
    }
    for wrong in [
        "Form 40% of your fitness",
        "You're 12% below your fitness.",
        "Your form is at -12% of your fitness.",
    ] {
        assert_eq!(
            check(form, wrong),
            Some(ClaimStatus::Contradicted),
            "{wrong}"
        );
    }
}

#[test]
fn a_share_the_reading_cannot_settle_is_left_unchecked() {
    for text in [
        // No chronic base: there is no share to compare it to.
        "You're at -30% of your fitness.",
        "Your form is at +12% of your fitness.",
    ] {
        assert_eq!(check(no_base(), text), None, "{text}");
    }
    for text in [
        // An unsigned share that names no fitness is not a reading of form.
        "Your form improved 5% this week.",
        "Your current form is 18%.",
        // "80% of your fitness" after a layoff is a fitness level.
        "You are back at 80% of your fitness since the break.",
        // Not form at all.
        "Your weekly volume grew 8% this month.",
    ] {
        assert_eq!(check(deep(), text), None, "{text}");
    }
}

/// "40% above your chronic load" said of the acute load is the acute:chronic
/// ratio. Read as form it puts +40% on an athlete whose form is -40%, so a true
/// sentence would be contradicted, for every athlete, with the wrong sign.
#[test]
fn a_share_of_another_load_is_not_read_as_form() {
    let tired = reading(100.0, 140.0);
    for form in [tired, fresh()] {
        for text in [
            "Your acute load is 40% above your chronic load.",
            "Your ATL is 40% above your CTL, so your form is negative.",
            "Your fatigue is 40% above your fitness.",
            "Your weekly volume is 20% above your chronic load.",
            "Ta charge aiguë est à +40 % de ta charge chronique.",
            "Tu carga aguda está un 40 % por encima de tu CTL.",
            "Deine akute Belastung liegt 40 % über deiner Fitness.",
            "A tua carga aguda está 40% acima da tua fitness.",
        ] {
            assert_eq!(check(form, text), None, "{text}");
        }
    }
    // Form named after the other load is still the subject of the share.
    assert_eq!(
        check(
            tired,
            "Your acute load is high, and your form is 40% below your fitness."
        ),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        check(
            fresh(),
            "Your load is up, so you're at -40% of your fitness."
        ),
        Some(ClaimStatus::Contradicted)
    );
}

#[test]
fn a_range_is_a_limit_and_neither_end_is_read_as_the_athletes_form() {
    // TSB -50 at CTL 200 is -25% of fitness. Read as a bare TSB, the "-20"
    // would be 30 points off and contradicted; read as a share, both ends of
    // the range are limits, not readings.
    let form = reading(200.0, 250.0);
    for text in [
        "Your current form is between -20 and -27% of your fitness.",
        "Your current form is between -50 and -57% of your fitness.",
        "Ta forme actuelle est entre -50 et -57 % de ta condition physique.",
    ] {
        assert_eq!(check(form, text), None, "{text}");
    }
}

#[test]
fn a_definition_of_the_bands_is_not_a_claim_about_the_athlete() {
    // When an athlete asks how form is read, the agent explains the bands. The
    // tools hand it the edges ("deep_fatigue below -30%, heavy_block -30% to
    // -20%"), and repeating them is not a statement about this athlete.
    for form in [deep(), fresh()] {
        for text in [
            "Deep fatigue is below -30% of CTL.",
            "A heavy block sits at -30% to -20% of your fitness.",
            "Balanced is -10% to +5% of your fitness, and fresh is above +5%.",
            "If your form drops below -30% of your fitness, take a rest day.",
            "Aim to keep your form above -20% of your CTL during the block.",
            "Fatigue profonde : forme en dessous de -30 % de ta condition physique.",
        ] {
            assert_eq!(check(form, text), None, "{text}");
        }
    }
}

// ---------------------------------------------------------------------------
// A band
// ---------------------------------------------------------------------------

#[test]
fn a_band_name_is_checked_against_the_band_in_every_locale() {
    for (right, wrong) in [
        ("You're in deep fatigue.", "Your form is fresh."),
        ("Tu es en fatigue profonde.", "Ta forme est fraîche."),
        ("Estás en fatiga profunda.", "Tu forma está fresca."),
        ("Deine Form: Tiefe Ermüdung.", "Deine Form ist frisch."),
        ("Estás em fadiga profunda.", "A tua forma está fresca."),
    ] {
        assert_eq!(
            check(deep(), right),
            Some(ClaimStatus::Supported),
            "{right}"
        );
        assert_eq!(
            check(deep(), wrong),
            Some(ClaimStatus::Contradicted),
            "{wrong}"
        );
    }
}

#[test]
fn a_band_needs_an_anchor_in_its_sentence() {
    assert_eq!(
        check(fresh(), "You're in deep fatigue."),
        Some(ClaimStatus::Contradicted),
        "a second-person state phrase puts the band on the athlete"
    );
    for text in [
        "Deep fatigue is rare in a base block.",
        "Une fatigue profonde demande plusieurs jours de repos.",
        // A heavy block is also a stretch of training, so "you're in" alone
        // does not make it a claim about form.
        "You're in a heavy block this week, so keep the long run easy.",
        "Tu es dans un bloc chargé cette semaine, garde la sortie longue facile.",
    ] {
        assert_eq!(check(fresh(), text), None, "{text}");
    }
}

#[test]
fn an_everyday_word_is_not_a_band_without_a_form_anchor() {
    for text in [
        "Eat a balanced diet.",
        "It was a productive week.",
        "Une semaine productive.",
        "Ein ausgeglichener Tag.",
    ] {
        assert_eq!(check(deep(), text), None, "{text}");
    }
    assert_eq!(
        check(deep(), "Your current form is balanced."),
        Some(ClaimStatus::Contradicted),
        "the same word beside a form label is a claim about form"
    );
}

#[test]
fn a_band_the_reading_sits_beside_is_left_alone() {
    // -13% of fitness is productive, three points from the balanced edge.
    assert_eq!(check(productive(), "Your current form is balanced."), None);
    assert_eq!(
        check(productive(), "Your current form is productive."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        check(productive(), "Your current form is in deep fatigue."),
        Some(ClaimStatus::Contradicted)
    );
}

#[test]
fn a_longer_band_name_is_not_read_as_a_shorter_one() {
    assert_eq!(
        check(detraining(), "Your current form is very fresh."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        check(fresh(), "Your current form is very fresh."),
        Some(ClaimStatus::Contradicted),
        "very fresh is the band above fresh"
    );
    assert_eq!(
        check(detraining(), "Your current form is fresh."),
        Some(ClaimStatus::Contradicted)
    );
}

#[test]
fn a_band_cannot_be_checked_without_a_chronic_base() {
    assert_eq!(check(no_base(), "You're in deep fatigue."), None);
}

// ---------------------------------------------------------------------------
// The product's prose, read back
// ---------------------------------------------------------------------------

/// `FormReading::inline` is what the tools put in front of the agent, and the
/// agent repeats it. Whatever it renders for a band, the checker must accept
/// it as true of that athlete and reject it with one figure changed.
#[test]
fn the_prose_the_tools_render_is_supported_for_the_athlete_it_describes() {
    for form in [
        deep(),
        heavy(),
        productive(),
        balanced(),
        fresh(),
        detraining(),
        no_base(),
    ] {
        let prose = form.inline();
        assert_eq!(check(form, &prose), Some(ClaimStatus::Supported), "{prose}");
    }
}

#[test]
fn the_prose_the_tools_render_is_contradicted_with_its_share_changed() {
    let prose = deep().inline();
    assert!(prose.contains("-64% of CTL"), "{prose}");
    assert_eq!(
        check(deep(), &prose.replace("-64%", "-30%")),
        Some(ClaimStatus::Contradicted)
    );
    assert_eq!(
        check(deep(), &prose.replace("TSB -77", "TSB -30")),
        Some(ClaimStatus::Contradicted)
    );
    // The same prose, for a different athlete, is wrong.
    assert_eq!(
        check(fresh(), &prose),
        Some(ClaimStatus::Contradicted),
        "-64% and deep fatigue are not this athlete's"
    );
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

// ---------------------------------------------------------------------------
// A bare TSB
// ---------------------------------------------------------------------------

#[test]
fn a_bare_tsb_is_checked_against_the_raw_number() {
    for (right, wrong) in [
        ("Your TSB is -77.", "Your TSB is -30."),
        (
            "Your training stress balance is -77.",
            "Your training stress balance is -30.",
        ),
        ("Ton TSB est de −77.", "Ton TSB est de −30."),
        ("Dein TSB liegt bei -77.", "Dein TSB liegt bei -30."),
    ] {
        assert_eq!(
            check(deep(), right),
            Some(ClaimStatus::Supported),
            "{right}"
        );
        assert_eq!(
            check(deep(), wrong),
            Some(ClaimStatus::Contradicted),
            "{wrong}"
        );
    }
}

#[test]
fn the_tsb_tolerance_scales_with_the_ctl_it_is_a_share_of() {
    // CTL 200, TSB -36: 5% of fitness is 10 TSB points, so -43 is within it
    // where a fixed ±5 would contradict it.
    let wide = reading(200.0, 236.0);
    assert_eq!(
        check(wide, "Your TSB is -43."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        check(wide, "Your TSB is -60."),
        Some(ClaimStatus::Contradicted)
    );

    // CTL 30, TSB -4.5: 5% of fitness is 1.5 TSB points, so -8 is out of it
    // where a fixed ±5 would support a figure nearly twice the athlete's own.
    let narrow = reading(30.0, 34.5);
    assert_eq!(
        check(narrow, "Your TSB is -8."),
        Some(ClaimStatus::Contradicted)
    );
    assert_eq!(
        check(narrow, "Your TSB is -4."),
        Some(ClaimStatus::Supported)
    );
}

/// A label is written before its value, so a figure the same distance before
/// it belongs to the quantity before: in "ATL 140 TSB -40" the TSB is -40.
#[test]
fn a_bare_tsb_is_the_figure_after_its_label() {
    let tired = reading(100.0, 140.0);
    for text in [
        "ATL 140 TSB -40",
        "Your ATL 140 and training stress balance -40",
    ] {
        assert_eq!(check(tired, text), Some(ClaimStatus::Supported), "{text}");
        assert_eq!(
            check(tired, &text.replace("-40", "-10")),
            Some(ClaimStatus::Contradicted),
            "{text}"
        );
    }
}

#[test]
fn the_rendered_integer_tsb_is_always_supported_however_small_the_ctl() {
    // The product rounds TSB to a whole number, so the printed figure can sit
    // up to half a point off the reading. CTL 22 leaves 5% of fitness at 1.1
    // TSB points, which absorbs that; CTL 6 leaves 0.3, and only the 1-point
    // floor keeps the printed -2 of a -2.4 reading true.
    for thin in [reading(22.0, 25.4), reading(6.0, 8.4)] {
        let printed = format!("Your TSB is {:+.0}.", thin.tsb);
        assert_eq!(
            check(thin, &printed),
            Some(ClaimStatus::Supported),
            "{printed} at CTL {}",
            thin.form_ctl
        );
    }
}

#[test]
fn a_count_of_days_beside_current_form_is_not_a_tsb() {
    assert_eq!(
        check(
            deep(),
            "Your current form over the last 7 days has been low."
        ),
        None
    );
    assert_eq!(
        check(deep(), "Ta forme actuelle sur 7 jours est basse."),
        None
    );
}

#[test]
fn a_percentage_and_a_bare_tsb_in_one_sentence_are_each_read_for_what_they_are() {
    assert_eq!(
        check(deep(), "Your TSB is -77, which is -64% of your fitness."),
        Some(ClaimStatus::Supported)
    );
    assert_eq!(
        check(deep(), "Your TSB is -77, which is -30% of your fitness."),
        Some(ClaimStatus::Contradicted)
    );
    assert_eq!(
        check(deep(), "Your TSB is -30, which is -64% of your fitness."),
        Some(ClaimStatus::Contradicted)
    );
}

#[test]
fn no_form_in_the_snapshot_means_no_form_verdict() {
    let m = AthleteMetrics {
        ftp_watts: Some(250.0),
        data_days: 30,
        ..AthleteMetrics::default()
    };
    let strategy = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &m,
        tolerance: &strategy,
    };
    assert!(personalized_check(&claim("Your TSB is -77."), &ctx).is_none());
    assert!(personalized_check(&claim("You're at -64% of your fitness."), &ctx).is_none());
}
