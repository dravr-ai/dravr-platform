// ABOUTME: Intervals.icu sport settings — the FTP, threshold heart rate, max heart rate and threshold pace the athlete keeps per sport
// ABOUTME: Pure wire shape and mapping onto ProviderThresholds; the provider reads GET /api/v1/athlete/{id}/sport-settings and hands the array here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu sport settings
//!
//! Intervals.icu keeps one settings object per group of activity types
//! (`types: ["Ride", "VirtualRide", …]`), each holding the thresholds the
//! athlete set for those sports. A Dravr profile holds one value of each, so
//! the mapping decides whose settings each threshold comes from:
//!
//! - **FTP** from the cycling settings: the object listing `Ride`, else the
//!   first listing a cycling type;
//! - **threshold pace** from the running settings, the same way, turned from
//!   metres per second into seconds per kilometre;
//! - **threshold and maximum heart rate** together, from one object: the
//!   athlete's primary sport's settings when Dravr knows that sport, else
//!   cycling's, else running's, else the first object — the first of these
//!   that holds either heart rate, so a sport whose settings state none never
//!   hides another's. Taking both from one object keeps a running LTHR from
//!   pairing with a cycling max HR.
//!
//! Left out on purpose: `indoor_ftp` and `p_max`, which no profile field
//! holds; `w_prime`, because Intervals.icu's W′ belongs to its FTP-anchored
//! power model while Dravr's is the critical-power model's, and the two are
//! not interchangeable; and the zone arrays, since Dravr derives its own
//! zones from the thresholds.
//!
//! The OpenAPI spec states no unit for `threshold_pace`; it is read as metres
//! per second, the unit Intervals.icu's API uses for speed. A pace read in any
//! other unit lands far outside the threshold-pace range every saved value is
//! checked against, so it is skipped rather than stored.
//!
//! Decoding is lenient, as for every Intervals.icu array: each entry is
//! decoded on its own, and a field reads as absent when it is `null`, of
//! another type, or no usable threshold (zero or negative), so one odd entry
//! never costs the athlete the rest of their settings.

use chrono::NaiveDate;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use tracing::warn;

use crate::intervals_icu_calendar::event_sport;
use crate::intervals_icu_planned::lenient;
use crate::models::{
    sport_family_head, sport_matches_family, ProviderThreshold, ProviderThresholds, SportType,
};

/// The origin an imported threshold's provenance names.
const ORIGIN: &str = "intervals.icu";

/// Metres in a kilometre: what turns a speed in metres per second into a
/// pace in seconds per kilometre.
const METRES_PER_KM: f64 = 1_000.0;

/// Tenths in a second: a converted pace keeps one decimal, the precision a
/// pace is stated with, rather than the float division's tail.
const PACE_TENTHS_PER_SECOND: f64 = 10.0;

/// One entry of `GET /api/v1/athlete/{id}/sport-settings` (the
/// `SportSettings` schema of the Intervals.icu `OpenAPI` spec), the fields the
/// mapping uses. Every field is optional and read leniently.
#[derive(Debug, Deserialize)]
struct IntervalsIcuSportSettings {
    /// The activity types the settings apply to.
    #[serde(default, deserialize_with = "strings")]
    types: Option<Vec<String>>,
    /// Functional threshold power, watts.
    #[serde(default, deserialize_with = "lenient")]
    ftp: Option<i64>,
    /// Lactate threshold heart rate, bpm.
    #[serde(default, deserialize_with = "lenient")]
    lthr: Option<i64>,
    /// Maximum heart rate, bpm.
    #[serde(default, deserialize_with = "lenient")]
    max_hr: Option<i64>,
    /// Threshold pace, metres per second (`pace_units` is only how the app
    /// displays it).
    #[serde(default, deserialize_with = "lenient")]
    threshold_pace: Option<f64>,
    /// When the settings last changed, an ISO date-time.
    #[serde(default, deserialize_with = "lenient")]
    updated: Option<String>,
}

/// The string items of an array, else `None`: a non-string type name is left
/// out rather than costing the entry every other type it lists.
fn strings<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Value::deserialize(deserializer)? {
        Value::Array(items) => Some(
            items
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
        ),
        _ => None,
    })
}

/// One sport's settings, with the sports its types name and the day it last
/// changed.
struct SportSettings {
    sports: Vec<SportType>,
    as_of: Option<NaiveDate>,
    settings: IntervalsIcuSportSettings,
}

impl SportSettings {
    fn of(settings: IntervalsIcuSportSettings) -> Self {
        let sports = settings
            .types
            .iter()
            .flatten()
            .map(|name| event_sport(Some(name)))
            .collect();
        let as_of = settings
            .updated
            .as_deref()
            .and_then(|updated| updated.get(..10))
            .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok());
        Self {
            sports,
            as_of,
            settings,
        }
    }

    /// Whether these settings hold a usable threshold or maximum heart rate.
    fn holds_heart_rate(&self) -> bool {
        whole::<u16>(self.settings.lthr).is_some() || whole::<u16>(self.settings.max_hr).is_some()
    }

    /// `value` with the day these settings last changed.
    const fn threshold<T>(&self, value: T) -> ProviderThreshold<T> {
        ProviderThreshold {
            value,
            as_of: self.as_of,
        }
    }
}

/// The settings `sport` is trained under: the object listing `sport` itself,
/// else the one listing its family's head (`Run` for a trail run), else the
/// first listing any sport of that family.
fn settings_for<'a>(all: &'a [SportSettings], sport: &SportType) -> Option<&'a SportSettings> {
    let head = sport_family_head(sport).unwrap_or_else(|| sport.clone());
    all.iter()
        .find(|entry| entry.sports.contains(sport))
        .or_else(|| all.iter().find(|entry| entry.sports.contains(&head)))
        .or_else(|| {
            all.iter().find(|entry| {
                entry
                    .sports
                    .iter()
                    .any(|listed| sport_matches_family(listed, &head))
            })
        })
}

/// A whole-number threshold, when the provider states a positive one that
/// fits the profile's field.
fn whole<T: TryFrom<i64>>(value: Option<i64>) -> Option<T> {
    value
        .filter(|value| *value > 0)
        .and_then(|value| T::try_from(value).ok())
}

/// A threshold pace in seconds per kilometre, from a positive speed in metres
/// per second.
fn pace_sec_per_km(metres_per_second: Option<f64>) -> Option<f64> {
    let speed = metres_per_second.filter(|speed| speed.is_finite() && *speed > 0.0)?;
    Some((METRES_PER_KM / speed * PACE_TENTHS_PER_SECOND).round() / PACE_TENTHS_PER_SECOND)
}

/// The thresholds a raw `GET /sport-settings` array holds, chosen per
/// threshold as the module documentation describes. `primary_sport` is the
/// athlete's main sport when Dravr knows it.
///
/// An entry that is not a settings object is left out with a warning naming
/// only its position.
pub(crate) fn thresholds_from_settings(
    entries: Vec<Value>,
    primary_sport: Option<&SportType>,
) -> ProviderThresholds {
    let all: Vec<SportSettings> = entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            serde_json::from_value::<IntervalsIcuSportSettings>(entry).map_or_else(
                |_| {
                    warn!(
                        index,
                        "intervals_icu sport-settings read left out an entry that is not a settings object"
                    );
                    None
                },
                |settings| Some(SportSettings::of(settings)),
            )
        })
        .collect();

    let cycling = settings_for(&all, &SportType::Ride);
    let running = settings_for(&all, &SportType::Run);
    let heart = primary_sport
        .and_then(|sport| settings_for(&all, sport))
        .into_iter()
        .chain(cycling)
        .chain(running)
        .chain(&all)
        .find(|entry| entry.holds_heart_rate());

    ProviderThresholds {
        origin: ORIGIN,
        ftp_watts: cycling
            .and_then(|entry| whole(entry.settings.ftp).map(|ftp| entry.threshold(ftp))),
        threshold_hr: heart
            .and_then(|entry| whole(entry.settings.lthr).map(|lthr| entry.threshold(lthr))),
        max_hr: heart
            .and_then(|entry| whole(entry.settings.max_hr).map(|max_hr| entry.threshold(max_hr))),
        threshold_pace_sec_per_km: running.and_then(|entry| {
            pace_sec_per_km(entry.settings.threshold_pace).map(|pace| entry.threshold(pace))
        }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn day(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(y, m, d)
    }

    /// An athlete who rides and runs, each sport with its own settings.
    fn ride_and_run() -> Vec<Value> {
        vec![
            json!({
                "types": ["Ride", "VirtualRide", "GravelRide"],
                "ftp": 285, "indoor_ftp": 270, "w_prime": 21000, "p_max": 1100,
                "lthr": 168, "max_hr": 186,
                "power_zones": [55, 75, 90, 105, 120, 150, 999],
                "updated": "2026-09-30T08:12:44.120+00:00"
            }),
            json!({
                "types": ["Run", "TrailRun", "VirtualRun"],
                "lthr": 174, "max_hr": 192, "threshold_pace": 4.0, "pace_units": "MINS_KM",
                "updated": "2026-08-14T19:01:02Z"
            }),
        ]
    }

    #[test]
    fn ftp_comes_from_cycling_and_pace_from_running() {
        let read = thresholds_from_settings(ride_and_run(), None);
        assert_eq!(read.origin, "intervals.icu");
        assert_eq!(
            read.ftp_watts,
            Some(ProviderThreshold {
                value: 285,
                as_of: day(2026, 9, 30)
            })
        );
        // 4.0 m/s is a 4:10/km threshold.
        assert_eq!(
            read.threshold_pace_sec_per_km,
            Some(ProviderThreshold {
                value: 250.0,
                as_of: day(2026, 8, 14)
            })
        );
    }

    #[test]
    fn heart_rate_follows_the_primary_sport_and_otherwise_cycling() {
        let runner = thresholds_from_settings(ride_and_run(), Some(&SportType::TrailRunning));
        assert_eq!(runner.threshold_hr.map(|t| t.value), Some(174));
        assert_eq!(runner.max_hr.map(|t| t.value), Some(192));
        assert_eq!(runner.max_hr.and_then(|t| t.as_of), day(2026, 8, 14));

        let unknown = thresholds_from_settings(ride_and_run(), None);
        assert_eq!(unknown.threshold_hr.map(|t| t.value), Some(168));
        assert_eq!(unknown.max_hr.map(|t| t.value), Some(186));

        // A swimmer with no swim settings reads cycling's.
        let swimmer = thresholds_from_settings(ride_and_run(), Some(&SportType::Swim));
        assert_eq!(swimmer.threshold_hr.map(|t| t.value), Some(168));
    }

    #[test]
    fn heart_rate_falls_back_to_running_then_to_the_first_entry() {
        let run_only = vec![ride_and_run().remove(1)];
        let read = thresholds_from_settings(run_only, Some(&SportType::Swim));
        assert_eq!(read.threshold_hr.map(|t| t.value), Some(174));
        assert_eq!(read.ftp_watts, None);

        let swim_only = vec![json!({ "types": ["Swim"], "lthr": 150, "max_hr": 175 })];
        let read = thresholds_from_settings(swim_only, None);
        assert_eq!(read.threshold_hr.map(|t| t.value), Some(150));
        assert_eq!(read.max_hr.map(|t| t.value), Some(175));
        assert_eq!(read.ftp_watts, None);
        assert_eq!(read.threshold_pace_sec_per_km, None);
    }

    #[test]
    fn a_primary_sport_whose_settings_state_no_heart_rate_never_hides_another_sports() {
        // The running settings hold only a pace: a runner's heart rates come
        // from the next object that states them, both from that one object.
        let entries = vec![
            json!({ "types": ["Ride"], "ftp": 285, "lthr": 168, "max_hr": 186,
                    "updated": "2026-09-30T08:00:00Z" }),
            json!({ "types": ["Run"], "threshold_pace": 4.0, "lthr": null, "max_hr": 0 }),
        ];
        let read = thresholds_from_settings(entries, Some(&SportType::Run));
        assert_eq!(read.threshold_hr.map(|t| t.value), Some(168));
        assert_eq!(read.max_hr.map(|t| t.value), Some(186));
        assert_eq!(read.max_hr.and_then(|t| t.as_of), day(2026, 9, 30));
        assert_eq!(read.threshold_pace_sec_per_km.map(|t| t.value), Some(250.0));

        // Settings stating one heart rate are the pair's object: the other
        // reads as absent rather than borrowed from another sport.
        let one = vec![
            json!({ "types": ["Ride"], "lthr": 168, "max_hr": 186 }),
            json!({ "types": ["Run"], "max_hr": 192 }),
        ];
        let read = thresholds_from_settings(one, Some(&SportType::Run));
        assert_eq!(read.max_hr.map(|t| t.value), Some(192));
        assert_eq!(read.threshold_hr, None);
    }

    #[test]
    fn a_cycling_type_other_than_ride_still_carries_the_ftp() {
        let indoor = vec![json!({ "types": ["VirtualRide"], "ftp": 250 })];
        let read = thresholds_from_settings(indoor, None);
        assert_eq!(read.ftp_watts.map(|t| t.value), Some(250));
        assert_eq!(read.ftp_watts.and_then(|t| t.as_of), None);
    }

    #[test]
    fn w_prime_indoor_ftp_and_the_zones_are_never_read() {
        let only_ignored = vec![json!({
            "types": ["Ride"], "indoor_ftp": 270, "w_prime": 21000, "p_max": 1100,
            "power_zones": [55, 75, 90], "hr_zones": [140, 155, 168]
        })];
        let read = thresholds_from_settings(only_ignored, None);
        assert_eq!(
            read,
            ProviderThresholds {
                origin: "intervals.icu",
                ftp_watts: None,
                threshold_hr: None,
                max_hr: None,
                threshold_pace_sec_per_km: None,
            }
        );
    }

    #[test]
    fn nulls_odd_types_and_unusable_values_read_as_absent() {
        let entries = vec![
            json!({
                "types": null, "ftp": null, "lthr": "170", "max_hr": 0,
                "threshold_pace": null, "updated": null
            }),
            json!({
                "types": ["Ride", 7], "ftp": -5, "lthr": 70000, "max_hr": [190],
                "updated": "yesterday"
            }),
            json!({ "types": ["Run"], "threshold_pace": 0.0 }),
        ];
        let read = thresholds_from_settings(entries, None);
        assert_eq!(read.ftp_watts, None);
        assert_eq!(read.threshold_hr, None);
        assert_eq!(read.max_hr, None);
        assert_eq!(read.threshold_pace_sec_per_km, None);
    }

    #[test]
    fn one_odd_entry_never_costs_the_athlete_the_rest() {
        let mut entries = ride_and_run();
        entries.insert(1, json!("not a settings object"));
        entries.insert(0, json!(42));
        let read = thresholds_from_settings(entries, Some(&SportType::Run));
        assert_eq!(read.ftp_watts.map(|t| t.value), Some(285));
        assert_eq!(read.threshold_hr.map(|t| t.value), Some(174));
        assert_eq!(read.threshold_pace_sec_per_km.map(|t| t.value), Some(250.0));
    }

    #[test]
    fn a_converted_pace_keeps_one_decimal() {
        let entries = vec![json!({ "types": ["Run"], "threshold_pace": 3.7 })];
        let read = thresholds_from_settings(entries, None);
        assert_eq!(read.threshold_pace_sec_per_km.map(|t| t.value), Some(270.3));
    }

    #[test]
    fn no_settings_at_all_hold_no_thresholds() {
        let read = thresholds_from_settings(Vec::new(), Some(&SportType::Ride));
        assert_eq!(read.ftp_watts, None);
        assert_eq!(read.threshold_hr, None);
    }
}
