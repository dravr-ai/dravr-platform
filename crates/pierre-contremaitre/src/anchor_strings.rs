// ABOUTME: How one of an athlete's anchors reads: its value (a range or an estimate at low confidence), its pace, who set it
// ABOUTME: The envelope notice and the physiology screen print an anchor through these, so both read the same words
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How an anchor reads (carnet#1495, carnet#1516).
//!
//! A low-confidence anchor is never shown as one precise number: it reads as
//! the range it states, or as an estimate when it states none. Watts and
//! beats per minute read as whole numbers. A speed reads as a pace: per
//! 100 m in the pool in either unit system, since the app has no pool
//! setting and the activity view writes pool pace per 100 m too; per
//! kilometre or per mile on land, as the reader's setting says. Who set it
//! reads as the reader's own value, as the athlete's own in a coach's view,
//! or as "from" whoever produced it.
//!
//! The envelope notice names the anchor a session sits on with these, and
//! the athlete's physiology screen lists every stored anchor with them, so
//! the same anchor never reads two ways.

use pierre_core::constants::units::METERS_PER_KM;
use pierre_core::models::periodization::{AnchorConfidence, AnchorValue, SportFamily};
use pierre_core::models::{AnchorOrigin, AnchorUnit, UnitSystem, ATHLETE_REPORTED_ORIGIN};

use crate::messaging_strings::{
    MessagingStringsRegistry, KEY_ENVELOPE_ANCHOR_ESTIMATE, KEY_ENVELOPE_ANCHOR_RANGE,
    KEY_ENVELOPE_SOURCE_ATHLETE, KEY_ENVELOPE_SOURCE_FROM, KEY_ENVELOPE_SOURCE_OWN,
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

/// `point`'s value as it reads in `locale`.
///
/// At low confidence it is the range the anchor states ("about 245–265 W
/// (estimate)"), or an estimate when it states none; at medium or high
/// confidence, the value itself. A speed reads as a pace in `sport`, per
/// 100 m in the pool and per kilometre or mile on land as `units` reads.
///
/// Empty when the catalogue holds no string for a low-confidence form; the
/// caller decides whether to show the anchor without it.
#[must_use]
pub fn anchor_value_text(
    strings: &MessagingStringsRegistry,
    locale: &str,
    unit: AnchorUnit,
    sport: SportFamily,
    units: UnitSystem,
    point: &AnchorValue,
) -> String {
    if point.confidence != AnchorConfidence::Low {
        return with_unit(unit, sport, units, point.value);
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

/// Who set an anchor whose origin is `origin`, as `reader` reads it.
///
/// An anchor the athlete stated reads as the reader's own value, or as the
/// athlete's own in a coach's view; an anchor naming no origin is the
/// athlete's. Any other reads as "from" its origin, in `locale`.
#[must_use]
pub fn anchor_source_text(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: AnchorReader,
    origin: Option<&str>,
) -> String {
    let origin = origin.unwrap_or(AnchorOrigin::Athlete.as_str());
    if origin == AnchorOrigin::Athlete.as_str() || origin == ATHLETE_REPORTED_ORIGIN {
        strings.get(
            match reader {
                AnchorReader::Athlete => KEY_ENVELOPE_SOURCE_OWN,
                AnchorReader::Coach => KEY_ENVELOPE_SOURCE_ATHLETE,
            },
            locale,
        )
    } else {
        strings.render(KEY_ENVELOPE_SOURCE_FROM, locale, &[origin])
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
                &ranged
            ),
            "about 245–265 W (estimate)",
            "bug: a low-confidence anchor with a range reads as that range"
        );
        let bare_point = point(285.0, None, AnchorConfidence::Low);
        assert_eq!(
            anchor_value_text(
                &strings,
                "fr",
                AnchorUnit::Watts,
                SportFamily::Ride,
                UnitSystem::Metric,
                &bare_point
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
                &measured
            ),
            "380 W",
            "bug: a high-confidence anchor reads as its value"
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
}
