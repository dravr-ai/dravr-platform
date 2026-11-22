// ABOUTME: Sunrise and sunset for a place and day, computed locally with NOAA's solar position equations
// ABOUTME: Pure: no vendor, no clock; a forecast reads it at the coordinates it was fetched for

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Daylight (carnet#1486).
//!
//! A plan for an easy loop before dark needs the sunset as a fetched figure,
//! not a guess. This is the NOAA Solar Calculator's algorithm (the Meeus form
//! NOAA publishes in its spreadsheet), with the standard refraction-corrected
//! zenith of 90.833°: within a minute of the published times between ±72°
//! latitude.
//!
//! The day is the place's **solar** date: an instant shifted by the place's
//! longitude / 15 hours. It is right to the day everywhere but a few hours
//! either side of a political zone line, and needs no zone database.

use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};

/// Zenith of sunrise and sunset: the Sun's centre 0.833° below the horizon,
/// for its radius and the atmosphere's refraction.
const ZENITH_DEGREES: f64 = 90.833;

/// Refinement passes: each recomputes the Sun's position at the event's
/// previous estimate.
const REFINEMENTS: usize = 3;

/// Minutes in a day.
const DAY_MINUTES: f64 = 1_440.0;

/// Sunrise and sunset of one solar day at one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Daylight {
    /// The Sun rises and sets.
    Day {
        /// When the Sun's upper limb clears the horizon.
        sunrise: Timestamp,
        /// When it goes below.
        sunset: Timestamp,
    },
    /// The Sun never sets that day.
    PolarDay,
    /// The Sun never rises that day.
    PolarNight,
}

/// The place's solar date at `instant`: the civil date at its mean solar
/// time, `longitude / 15` hours from UTC.
#[must_use]
pub fn solar_date(instant: Timestamp, longitude: f64) -> CivilDate {
    // Four minutes a degree, rounded to the second: the shift is ±12 h.
    #[allow(clippy::cast_possible_truncation)] // |longitude| ≤ 180, so |seconds| ≤ 43 200
    let seconds = (longitude * 240.0).round() as i64;
    (instant + Elapsed::seconds(seconds)).utc_date()
}

/// Sunrise and sunset at (`latitude`, `longitude`), in decimal degrees, on
/// the solar `date` there.
#[must_use]
pub fn daylight(latitude: f64, longitude: f64, date: CivilDate) -> Daylight {
    let noon = solar_position(julian_century(date, 4.0f64.mul_add(-longitude, 720.0)));
    match hour_angle(latitude, noon.declination) {
        HourAngle::Never => Daylight::PolarNight,
        HourAngle::Always => Daylight::PolarDay,
        HourAngle::Degrees(_) => {
            match (
                event(latitude, longitude, date, Event::Rise),
                event(latitude, longitude, date, Event::Set),
            ) {
                (Some(sunrise), Some(sunset)) => Daylight::Day { sunrise, sunset },
                // A refinement crossed into a day with no such event: the
                // place is at the polar edge, and the noon reading decides.
                _ if noon.declination * latitude > 0.0 => Daylight::PolarDay,
                _ => Daylight::PolarNight,
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Event {
    Rise,
    Set,
}

enum HourAngle {
    Degrees(f64),
    Always,
    Never,
}

struct SolarPosition {
    /// Declination, degrees.
    declination: f64,
    /// Equation of time, minutes.
    equation_of_time: f64,
}

/// The `kind` event at the place on its solar `date`, refined until the
/// Sun's position is read at the event itself.
fn event(latitude: f64, longitude: f64, date: CivilDate, kind: Event) -> Option<Timestamp> {
    let mut minutes = 4.0f64.mul_add(-longitude, 720.0);
    for _ in 0..=REFINEMENTS {
        let sun = solar_position(julian_century(date, minutes));
        let HourAngle::Degrees(angle) = hour_angle(latitude, sun.declination) else {
            return None;
        };
        let signed = match kind {
            Event::Rise => angle,
            Event::Set => -angle,
        };
        minutes = 4.0f64.mul_add(-(longitude + signed), 720.0) - sun.equation_of_time;
    }
    let midnight = date.at_midnight().and_utc().ok()?;
    #[allow(clippy::cast_possible_truncation)] // within a few days of the date, in milliseconds
    let millis = (minutes * 60_000.0).round() as i64;
    Some(midnight + Elapsed::milliseconds(millis))
}

/// The hour angle of sunrise at `latitude` for a Sun at `declination`.
fn hour_angle(latitude: f64, declination: f64) -> HourAngle {
    let (lat, decl) = (latitude.to_radians(), declination.to_radians());
    let cos = lat.tan().mul_add(
        -decl.tan(),
        ZENITH_DEGREES.to_radians().cos() / (lat.cos() * decl.cos()),
    );
    if cos > 1.0 {
        HourAngle::Never
    } else if cos < -1.0 {
        HourAngle::Always
    } else {
        HourAngle::Degrees(cos.acos().to_degrees())
    }
}

/// Julian centuries since J2000.0 at `minutes` past UTC midnight of `date`.
fn julian_century(date: CivilDate, minutes: f64) -> f64 {
    let epoch = CivilDate::from_ymd(2000, 1, 1).ok().unwrap_or(date);
    let days = (date - epoch).num_days();
    // J2000.0 is 2000-01-01 12:00 UTC.
    #[allow(clippy::cast_precision_loss)] // a few hundred thousand days
    let since_epoch = days as f64 + minutes / DAY_MINUTES - 0.5;
    since_epoch / 36_525.0
}

/// The Sun's declination and the equation of time at `t` Julian centuries.
///
/// NOAA's spreadsheet polynomials, written as fused multiply-adds.
fn solar_position(t: f64) -> SolarPosition {
    let mean_longitude = t
        .mul_add(t.mul_add(0.000_303_2, 36_000.769_83), 280.466_46)
        .rem_euclid(360.0);
    let mean_anomaly = t.mul_add(0.000_153_7f64.mul_add(-t, 35_999.050_29), 357.529_11);
    let eccentricity = t.mul_add(-0.000_000_126_7f64.mul_add(t, 0.000_042_037), 0.016_708_634);
    let anomaly = mean_anomaly.to_radians();
    let center = (3.0 * anomaly).sin().mul_add(
        0.000_289,
        anomaly.sin().mul_add(
            t.mul_add(-0.000_014f64.mul_add(t, 0.004_817), 1.914_602),
            (2.0 * anomaly).sin() * 0.000_101f64.mul_add(-t, 0.019_993),
        ),
    );
    let true_longitude = mean_longitude + center;
    let omega = 1_934.136f64.mul_add(-t, 125.04).to_radians();
    let apparent_longitude = 0.004_78f64.mul_add(-omega.sin(), true_longitude - 0.005_69);
    let arc_seconds = t.mul_add(-t.mul_add(t.mul_add(-0.001_813, 0.000_59), 46.815), 21.448);
    let mean_obliquity = 23.0 + (26.0 + arc_seconds / 60.0) / 60.0;
    let obliquity = 0.002_56f64
        .mul_add(omega.cos(), mean_obliquity)
        .to_radians();
    let declination = (obliquity.sin() * apparent_longitude.to_radians().sin())
        .asin()
        .to_degrees();
    let half_tan = (obliquity / 2.0).tan().powi(2);
    let longitude = mean_longitude.to_radians();
    let terms = [
        half_tan * (2.0 * longitude).sin(),
        -2.0 * eccentricity * anomaly.sin(),
        4.0 * eccentricity * half_tan * anomaly.sin() * (2.0 * longitude).cos(),
        -0.5 * half_tan * half_tan * (4.0 * longitude).sin(),
        -1.25 * eccentricity * eccentricity * (2.0 * anomaly).sin(),
    ];
    let equation: f64 = terms.iter().sum();
    SolarPosition {
        declination,
        equation_of_time: 4.0 * equation.to_degrees(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tolerance against the published times.
    const TOLERANCE_SECONDS: i64 = 120;

    fn utc(text: &str) -> Timestamp {
        Timestamp::parse_from_rfc3339(text).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> CivilDate {
        CivilDate::from_ymd(y, m, d).unwrap()
    }

    /// Assert the day at the place rises and sets within the tolerance of
    /// `sunrise` and `sunset`, the NOAA algorithm's times for it.
    fn assert_day(
        place: &str,
        latitude: f64,
        longitude: f64,
        on: CivilDate,
        sunrise: &str,
        sunset: &str,
    ) {
        let Daylight::Day {
            sunrise: rose,
            sunset: set,
        } = daylight(latitude, longitude, on)
        else {
            panic!("bug: {place} has a sunrise and a sunset on {on}");
        };
        for (what, got, want) in [
            ("sunrise", rose, utc(sunrise)),
            ("sunset", set, utc(sunset)),
        ] {
            assert!(
                (got - want).num_seconds().abs() <= TOLERANCE_SECONDS,
                "bug: {place} {on} {what} {got} is more than 2 min from {want}"
            );
        }
    }

    // Expected times are the NOAA Solar Calculator's algorithm on each
    // place's solar date, as astral 3.2 implements it; Montréal's match the
    // calculator's published 05:05 / 20:46 EDT for the solstice.

    #[test]
    fn montreal_at_both_solstices() {
        assert_day(
            "Montréal",
            45.50,
            -73.57,
            date(2026, 6, 21),
            "2026-06-21T09:05:52Z",
            "2026-06-22T00:46:26Z",
        );
        assert_day(
            "Montréal",
            45.50,
            -73.57,
            date(2026, 12, 21),
            "2026-12-21T12:31:38Z",
            "2026-12-21T21:13:16Z",
        );
    }

    #[test]
    fn quito_at_the_equinox() {
        assert_day(
            "Quito",
            -0.18,
            -78.47,
            date(2026, 3, 20),
            "2026-03-20T11:18:10Z",
            "2026-03-20T23:24:20Z",
        );
    }

    #[test]
    fn auckland_across_the_date_line() {
        assert_day(
            "Auckland",
            -36.85,
            174.76,
            date(2026, 1, 1),
            "2025-12-31T17:05:16Z",
            "2026-01-01T07:43:09Z",
        );
    }

    #[test]
    fn tromso_has_polar_day_and_polar_night() {
        assert_eq!(
            daylight(69.65, 18.96, date(2026, 6, 21)),
            Daylight::PolarDay,
            "bug: Tromsø's midsummer sun never sets"
        );
        assert_eq!(
            daylight(69.65, 18.96, date(2026, 12, 21)),
            Daylight::PolarNight,
            "bug: Tromsø's midwinter sun never rises"
        );
    }

    #[test]
    fn a_late_evening_in_montreal_is_already_tomorrow_in_paris() {
        let late_evening = utc("2026-11-19T04:00:00Z"); // 23:00 in Montréal on the 18th
        let paris_day = solar_date(late_evening, 2.35);
        assert_eq!(
            paris_day,
            date(2026, 11, 19),
            "bug: Paris is already on the 19th"
        );
        assert_eq!(
            solar_date(late_evening, -73.57),
            date(2026, 11, 18),
            "bug: Montréal is still on the 18th"
        );
        assert_day(
            "Paris",
            48.86,
            2.35,
            paris_day,
            "2026-11-19T07:05:54Z",
            "2026-11-19T16:05:32Z",
        );
    }
}
