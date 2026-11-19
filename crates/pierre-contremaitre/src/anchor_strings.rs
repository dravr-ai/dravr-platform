// ABOUTME: How one of an athlete's anchors reads: its value (plain only when measured at medium or high confidence), its pace, who set it
// ABOUTME: The envelope notice and the physiology screen print an anchor through these, so both read the same words
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How an anchor reads (carnet#1495, carnet#1516, carnet#1562).
//!
//! A low-confidence anchor is never shown as one precise number: it reads as
//! the range it states, or as an estimate when it states none. Watts and
//! beats per minute read as whole numbers. A speed reads as a pace: per
//! 100 m in the pool in either unit system, since the app has no pool
//! setting and the activity view writes pool pace per 100 m too; per
//! kilometre or per mile on land, as the reader's setting says. Who set it
//! reads as the reader's own value, as the athlete's own in a coach's view,
//! or as "from" whoever produced it; an origin stored as a code (imported,
//! estimated, promoted) reads in words, never as the code (carnet#1563).
//!
//! Only a measured anchor is stated as a plain number. One estimated, or
//! recording no kind, reads as an estimate at any confidence: the
//! Methodology framing rule (an estimate is quoted with an estimate marker,
//! never stated as a measurement) turns on how a value was produced, not on
//! how far it is trusted.
//!
//! The envelope notice names the anchor a session sits on with these, and
//! the athlete's physiology screen lists every stored anchor with them, so
//! the same anchor never reads two ways.

use pierre_core::constants::units::METERS_PER_KM;
use pierre_core::models::periodization::{AnchorConfidence, AnchorValue, SportFamily};
use pierre_core::models::{
    AnchorOrigin, AnchorUnit, MeasurementKind, UnitSystem, COACH_STATED_ORIGIN,
};

use crate::messaging_strings::{
    MessagingStringsRegistry, KEY_ENVELOPE_ANCHOR_ESTIMATE, KEY_ENVELOPE_ANCHOR_RANGE,
    KEY_ENVELOPE_SOURCE_ATHLETE, KEY_ENVELOPE_SOURCE_ESTIMATED, KEY_ENVELOPE_SOURCE_FROM,
    KEY_ENVELOPE_SOURCE_IMPORTED, KEY_ENVELOPE_SOURCE_OWN, KEY_ENVELOPE_SOURCE_PROMOTED,
    KEY_HUMAN_COACH_ANCHOR_SOURCE_FOR_ATHLETE, KEY_HUMAN_COACH_ANCHOR_SOURCE_FOR_COACH,
};

/// Seconds in a minute, for writing a pace.
const SECONDS_PER_MINUTE: u32 = 60;

/// The distance a swim pace is written per, in metres, in either unit system.
const SWIM_PACE_METRES: f64 = 100.0;

/// Who reads an anchor, which decides how one the athlete set is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorReader {
    /// The athlete, about their own anchors: "your own value".
    Athlete,
    /// A coach, about one athlete's anchors: "the athlete's own value".
    Coach,
}

/// `point`'s value as it reads in `locale`, `kind` saying whether it was
/// measured or estimated.
///
/// At low confidence it is the range the anchor states ("about 245–265 W
/// (estimate)"), or an estimate when it states none. At medium or high
/// confidence a measured anchor reads as the value itself, and any other as
/// an estimate of it ("about 280 W (estimate)"): an estimate is never stated
/// as a measurement, and a kind that was never recorded is no measurement.
/// A speed reads as a pace in `sport`, per 100 m in the pool and per
/// kilometre or mile on land as `units` reads.
///
/// Empty when the catalogue holds no string for an estimate or a range; the
/// caller decides whether to show the anchor without it.
#[must_use]
pub fn anchor_value_text(
    strings: &MessagingStringsRegistry,
    locale: &str,
    unit: AnchorUnit,
    sport: SportFamily,
    units: UnitSystem,
    point: &AnchorValue,
    kind: Option<MeasurementKind>,
) -> String {
    if point.confidence != AnchorConfidence::Low {
        let value = with_unit(unit, sport, units, point.value);
        return if kind == Some(MeasurementKind::Measured) {
            value
        } else {
            strings.render(KEY_ENVELOPE_ANCHOR_ESTIMATE, locale, &[&value])
        };
    }
    match (point.low, point.high) {
        (Some(low), Some(high)) => {
            let (first, second) = ordered_for_display(unit, low, high);
            strings.render(
                KEY_ENVELOPE_ANCHOR_RANGE,
                locale,
                &[
                    &bare(unit, sport, units, first),
                    &with_unit(unit, sport, units, second),
                ],
            )
        }
        _ => strings.render(
            KEY_ENVELOPE_ANCHOR_ESTIMATE,
            locale,
            &[&with_unit(unit, sport, units, point.value)],
        ),
    }
}

/// The range `point` states, as "365–395 W" or "4:05–4:15/km".
///
/// Both ends are written as [`anchor_value_text`] writes them, a pace range
/// from its faster end. `None` unless the anchor states both ends.
#[must_use]
pub fn anchor_range_text(
    unit: AnchorUnit,
    sport: SportFamily,
    units: UnitSystem,
    point: &AnchorValue,
) -> Option<String> {
    let (Some(low), Some(high)) = (point.low, point.high) else {
        return None;
    };
    let (first, second) = ordered_for_display(unit, low, high);
    Some(format!(
        "{}–{}",
        bare(unit, sport, units, first),
        with_unit(unit, sport, units, second)
    ))
}

/// Who set an anchor whose origin is `origin`, as `reader` reads it, in
/// `locale`.
///
/// A stored origin code ([`AnchorOrigin::parse`]), in any letter case,
/// reads in the words [`origin_code_key`] gives it, never as the code: an
/// anchor the athlete stated reads as the reader's own value, or as the
/// athlete's own in a coach's view, and an anchor naming no origin is the
/// athlete's. One the athlete's human coach set ([`COACH_STATED_ORIGIN`], in
/// any letter case) reads as set by their coach, or by a coach in a coach's
/// view (carnet#1561). Any other origin is the provider, app or test that
/// produced the anchor, and reads as "from" it, exactly as named.
#[must_use]
pub fn anchor_source_text(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: AnchorReader,
    origin: Option<&str>,
) -> String {
    let Some(origin) = origin else {
        return strings.get(origin_code_key(AnchorOrigin::Athlete, reader), locale);
    };
    // Matched whatever its case, as the codes below are, so a "Coach" never
    // reads raw; ahead of them, since `coach` is no stored code and would
    // otherwise read as "from coach".
    if origin.eq_ignore_ascii_case(COACH_STATED_ORIGIN) {
        return strings.get(
            match reader {
                AnchorReader::Athlete => KEY_HUMAN_COACH_ANCHOR_SOURCE_FOR_ATHLETE,
                AnchorReader::Coach => KEY_HUMAN_COACH_ANCHOR_SOURCE_FOR_COACH,
            },
            locale,
        );
    }
    // The model writes a per-sport anchor's source as free text, so a code
    // can arrive as "Imported" or "PROMOTED"; the code's words are the same.
    // Only this reading folds case: stored data decodes exactly as written.
    AnchorOrigin::parse(&origin.to_ascii_lowercase()).map_or_else(
        || strings.render(KEY_ENVELOPE_SOURCE_FROM, locale, &[origin]),
        |code| strings.get(origin_code_key(code, reader), locale),
    )
}

/// The key of the words a stored origin code reads as for `reader`. The one
/// place an [`AnchorOrigin`] is given its words: a new origin does not
/// compile until it has them.
const fn origin_code_key(code: AnchorOrigin, reader: AnchorReader) -> &'static str {
    match code {
        AnchorOrigin::Athlete => match reader {
            AnchorReader::Athlete => KEY_ENVELOPE_SOURCE_OWN,
            AnchorReader::Coach => KEY_ENVELOPE_SOURCE_ATHLETE,
        },
        AnchorOrigin::Imported => KEY_ENVELOPE_SOURCE_IMPORTED,
        AnchorOrigin::Estimated => KEY_ENVELOPE_SOURCE_ESTIMATED,
        AnchorOrigin::Promoted => KEY_ENVELOPE_SOURCE_PROMOTED,
    }
}

/// A range's ends in the order they read: a pace range runs from the faster
/// pace (the higher speed) to the slower one.
fn ordered_for_display(unit: AnchorUnit, low: f32, high: f32) -> (f32, f32) {
    if unit == AnchorUnit::MetresPerSecond {
        (high, low)
    } else {
        (low, high)
    }
}

/// A value with its unit: watts, beats per minute, or a speed as a pace — per
/// 100 m in the pool, per kilometre or mile on land as `units` reads.
fn with_unit(unit: AnchorUnit, sport: SportFamily, units: UnitSystem, value: f32) -> String {
    let number = bare(unit, sport, units, value);
    match unit {
        AnchorUnit::Watts => format!("{number} W"),
        AnchorUnit::BeatsPerMinute => format!("{number} bpm"),
        AnchorUnit::MetresPerSecond if sport == SportFamily::Swim => format!("{number}/100 m"),
        AnchorUnit::MetresPerSecond => format!("{number}/{}", units.distance_symbol()),
    }
}

/// A value without its unit: a whole number, or a speed as `m:ss` of pace.
fn bare(unit: AnchorUnit, sport: SportFamily, units: UnitSystem, value: f32) -> String {
    if unit != AnchorUnit::MetresPerSecond || value <= 0.0 {
        return format!("{:.0}", value.round());
    }
    let per = if sport == SportFamily::Swim {
        SWIM_PACE_METRES
    } else {
        // A kilometre, or a mile: the metres in one of `units`' distance units.
        METERS_PER_KM / units.distance_from_km(1.0)
    };
    // A pace of at most a few minutes: far inside u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let seconds = (per / f64::from(value)).round() as u32;
    format!(
        "{}:{:02}",
        seconds / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::{ATHLETE_REPORTED_ORIGIN, SUPPORTED_LOCALES};

    /// Whether `text` carries `code` as a word of its own, the way an
    /// interpolated origin code would read ("from imported"): a word that
    /// merely starts with it ("the athlete's") is not the code.
    fn names_the_code(text: &str, code: &str) -> bool {
        text.split_whitespace()
            .any(|word| word.trim_matches(|c: char| c.is_ascii_punctuation()) == code)
    }

    fn point(value: f32, range: Option<(f32, f32)>, confidence: AnchorConfidence) -> AnchorValue {
        AnchorValue {
            value,
            low: range.map(|(low, _)| low),
            high: range.map(|(_, high)| high),
            origin: None,
            as_of: None,
            method: None,
            confidence,
        }
    }

    #[test]
    fn a_swim_speed_reads_as_pace_per_100_metres() {
        assert_eq!(
            with_unit(
                AnchorUnit::MetresPerSecond,
                SportFamily::Swim,
                UnitSystem::Metric,
                1.25
            ),
            "1:20/100 m",
            "bug: a pool speed reads as pace per 100 m"
        );
        assert_eq!(
            with_unit(
                AnchorUnit::MetresPerSecond,
                SportFamily::Run,
                UnitSystem::Metric,
                4.0
            ),
            "4:10/km",
            "bug: a run speed reads as pace per km"
        );
        assert_eq!(
            with_unit(
                AnchorUnit::BeatsPerMinute,
                SportFamily::Run,
                UnitSystem::Metric,
                171.6
            ),
            "172 bpm",
            "bug: a heart rate reads as whole beats per minute"
        );
    }

    #[test]
    fn an_imperial_reader_reads_run_pace_per_mile_and_pool_pace_per_100_metres() {
        assert_eq!(
            with_unit(
                AnchorUnit::MetresPerSecond,
                SportFamily::Run,
                UnitSystem::Imperial,
                4.0
            ),
            "6:42/mi",
            "bug: an imperial runner reads 4 m/s as pace per mile (1609.344 m / 4 m/s = 402 s)"
        );
        assert_eq!(
            with_unit(
                AnchorUnit::MetresPerSecond,
                SportFamily::Swim,
                UnitSystem::Imperial,
                1.25
            ),
            "1:20/100 m",
            "bug: there is no pool setting, so pool pace stays per 100 m on imperial"
        );
        assert_eq!(
            with_unit(
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Imperial,
                285.0
            ),
            "285 W",
            "bug: watts are watts in either system"
        );
    }

    #[test]
    fn a_low_confidence_value_reads_as_its_range_or_an_estimate_never_a_point() {
        let strings = MessagingStringsRegistry::new();
        let ranged = point(255.0, Some((245.0, 265.0)), AnchorConfidence::Low);
        assert_eq!(
            anchor_value_text(
                &strings,
                "en",
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &ranged,
                Some(MeasurementKind::Measured)
            ),
            "about 245–265 W (estimate)",
            "bug: a low-confidence anchor with a range reads as that range, measured or not"
        );
        let bare_point = point(285.0, None, AnchorConfidence::Low);
        assert_eq!(
            anchor_value_text(
                &strings,
                "fr",
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &bare_point,
                Some(MeasurementKind::Measured)
            ),
            "environ 285 W (estimation)",
            "bug: a low-confidence anchor with no range reads as an estimate, in the reader's words"
        );
        let measured = point(380.0, Some((365.0, 395.0)), AnchorConfidence::High);
        assert_eq!(
            anchor_value_text(
                &strings,
                "en",
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &measured,
                Some(MeasurementKind::Measured)
            ),
            "380 W",
            "bug: a measured high-confidence anchor reads as its value"
        );
    }

    #[test]
    fn an_estimated_anchor_reads_as_an_estimate_whatever_its_confidence() {
        let strings = MessagingStringsRegistry::new();
        let high = point(380.0, Some((365.0, 395.0)), AnchorConfidence::High);
        assert_eq!(
            anchor_value_text(
                &strings,
                "en",
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &high,
                Some(MeasurementKind::Estimated)
            ),
            "about 380 W (estimate)",
            "bug: an estimate stored at high confidence reads as a measured number"
        );
        let medium_pace = point(4.0, None, AnchorConfidence::Medium);
        assert_eq!(
            anchor_value_text(
                &strings,
                "fr",
                AnchorUnit::MetresPerSecond,
                SportFamily::Run,
                UnitSystem::Metric,
                &medium_pace,
                Some(MeasurementKind::Estimated)
            ),
            "environ 4:10/km (estimation)",
            "bug: an estimated pace at medium confidence reads as an estimate, in the reader's words"
        );
        assert_eq!(
            anchor_value_text(
                &strings,
                "en",
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &high,
                None
            ),
            "about 380 W (estimate)",
            "bug: an anchor recording no kind is no measurement, so it is never stated as one"
        );
        assert_eq!(
            anchor_value_text(
                &strings,
                "en",
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &high,
                Some(MeasurementKind::Measured)
            ),
            "380 W",
            "bug: a measured high-confidence anchor still reads as its value"
        );
    }

    #[test]
    fn a_stated_range_reads_from_its_faster_end_in_the_readers_units() {
        let pace = point(4.0, Some((3.92, 4.08)), AnchorConfidence::Medium);
        assert_eq!(
            anchor_range_text(
                AnchorUnit::MetresPerSecond,
                SportFamily::Run,
                UnitSystem::Metric,
                &pace
            )
            .as_deref(),
            Some("4:05–4:15/km"),
            "bug: a pace range opens on its faster end"
        );
        assert_eq!(
            anchor_range_text(
                AnchorUnit::MetresPerSecond,
                SportFamily::Run,
                UnitSystem::Imperial,
                &pace
            )
            .as_deref(),
            Some("6:34–6:51/mi"),
            "bug: an imperial reader reads both ends per mile"
        );
        let power = point(380.0, Some((365.0, 395.0)), AnchorConfidence::High);
        assert_eq!(
            anchor_range_text(
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &power
            )
            .as_deref(),
            Some("365–395 W"),
            "bug: a power range opens on its low end"
        );
        let half = AnchorValue {
            high: None,
            ..power
        };
        assert_eq!(
            anchor_range_text(
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &half
            ),
            None,
            "bug: one end is not a range"
        );
    }

    #[test]
    fn an_anchor_the_athlete_set_is_named_for_its_reader() {
        let strings = MessagingStringsRegistry::new();
        assert_eq!(
            anchor_source_text(&strings, "en", AnchorReader::Athlete, None),
            "your own value",
            "bug: an anchor naming no origin is the athlete's own"
        );
        assert_eq!(
            anchor_source_text(
                &strings,
                "en",
                AnchorReader::Coach,
                Some(ATHLETE_REPORTED_ORIGIN)
            ),
            "the athlete's own value",
            "bug: a coach reads an athlete-reported anchor as the athlete's"
        );
        assert_eq!(
            anchor_source_text(&strings, "en", AnchorReader::Athlete, Some("intervals.icu")),
            "from intervals.icu",
            "bug: an anchor someone else produced names them"
        );
    }

    #[test]
    fn an_imported_anchor_reads_as_another_app_never_as_the_code() {
        let strings = MessagingStringsRegistry::new();
        let imported = Some(AnchorOrigin::Imported.as_str());
        assert_eq!(
            anchor_source_text(&strings, "en", AnchorReader::Athlete, imported),
            "from another app",
            "bug: an imported anchor reads as the code 'imported'"
        );
        assert_eq!(
            anchor_source_text(&strings, "en", AnchorReader::Coach, imported),
            "from another app",
            "bug: a coach reads an imported anchor differently from the athlete"
        );
        assert_eq!(
            anchor_source_text(&strings, "fr", AnchorReader::Athlete, imported),
            "d'une autre app",
            "bug: an imported anchor is not worded in the reader's language"
        );
    }

    #[test]
    fn an_estimated_anchor_reads_as_not_yet_measured_never_as_the_code() {
        let strings = MessagingStringsRegistry::new();
        let estimated = Some(AnchorOrigin::Estimated.as_str());
        assert_eq!(
            anchor_source_text(&strings, "en", AnchorReader::Athlete, estimated),
            "not yet measured",
            "bug: an estimated anchor reads as the code 'estimated'"
        );
        assert_eq!(
            anchor_source_text(&strings, "es", AnchorReader::Coach, estimated),
            "aún sin medir",
            "bug: an estimated anchor is not worded in the reader's language"
        );
    }

    #[test]
    fn a_promoted_anchor_reads_as_recent_measurements_never_as_the_code() {
        let strings = MessagingStringsRegistry::new();
        let promoted = Some(AnchorOrigin::Promoted.as_str());
        assert_eq!(
            anchor_source_text(&strings, "en", AnchorReader::Athlete, promoted),
            "from recent measurements",
            "bug: a promoted anchor reads as the code 'promoted'"
        );
        assert_eq!(
            anchor_source_text(&strings, "de", AnchorReader::Coach, promoted),
            "aus aktuellen Messungen",
            "bug: a promoted anchor is not worded in the reader's language"
        );
    }

    #[test]
    fn an_origin_code_in_any_case_reads_in_words_and_a_name_keeps_its_case() {
        let strings = MessagingStringsRegistry::new();
        for (locale, origin, words) in [
            ("en", "Imported", "from another app"),
            ("en", "PROMOTED", "from recent measurements"),
            ("en", "Estimated", "not yet measured"),
            ("en", "Athlete", "your own value"),
            ("en", "Athlete-Reported", "your own value"),
            ("fr", "IMPORTED", "d'une autre app"),
            ("fr", "Promoted", "de mesures récentes"),
            ("fr", "eStImAtEd", "en attente d'une mesure"),
        ] {
            assert_eq!(
                anchor_source_text(&strings, locale, AnchorReader::Athlete, Some(origin)),
                words,
                "bug: the code written as '{origin}' does not read in its {locale} words"
            );
        }
        assert_eq!(
            anchor_source_text(&strings, "en", AnchorReader::Athlete, Some("Intervals.icu")),
            "from Intervals.icu",
            "bug: a provider's name is not shown exactly as stored"
        );
        assert_eq!(
            anchor_source_text(&strings, "fr", AnchorReader::Coach, Some("TrainingPeaks")),
            "de TrainingPeaks",
            "bug: a provider's name is not shown exactly as stored"
        );
    }

    #[test]
    fn no_origin_code_reaches_a_reader_raw_in_any_locale() {
        let strings = MessagingStringsRegistry::new();
        let spellings: Vec<(AnchorOrigin, &str)> = AnchorOrigin::ALL
            .into_iter()
            .map(|code| (code, code.as_str()))
            .chain([(AnchorOrigin::Athlete, ATHLETE_REPORTED_ORIGIN)])
            .collect();
        for (code, stored) in spellings {
            for locale in SUPPORTED_LOCALES {
                for reader in [AnchorReader::Athlete, AnchorReader::Coach] {
                    let text = anchor_source_text(&strings, locale, reader, Some(stored));
                    assert!(
                        !names_the_code(&text, stored),
                        "bug: the stored code '{stored}' reaches a {reader:?} raw in {locale}: '{text}'"
                    );
                    assert_eq!(
                        Some(text),
                        strings.get_exact(origin_code_key(code, reader), locale),
                        "bug: '{stored}' has no words of its own in {locale}, or reads through another key"
                    );
                }
            }
        }
    }

    #[test]
    fn an_anchor_a_coach_set_is_named_for_its_reader() {
        let strings = MessagingStringsRegistry::new();
        assert_eq!(
            anchor_source_text(
                &strings,
                "en",
                AnchorReader::Athlete,
                Some(COACH_STATED_ORIGIN)
            ),
            "set by your coach", // humanCoach.anchorSource.forAthlete
            "bug: the athlete reads an anchor their coach set as their coach's"
        );
        assert_eq!(
            anchor_source_text(
                &strings,
                "en",
                AnchorReader::Coach,
                Some(COACH_STATED_ORIGIN)
            ),
            "set by a coach",
            "bug: a coach reads an anchor a coach set as a coach's, never as the athlete's own"
        );
        assert_eq!(
            anchor_source_text(&strings, "en", AnchorReader::Athlete, Some("Coach")),
            "set by your coach", // humanCoach.anchorSource.forAthlete
            "bug: the coach origin in another case reads raw as \"from Coach\""
        );
    }
}
