// ABOUTME: Intervals.icu activity payload and its mapping onto the domain Activity — sensor fields, self-report, sport labels
// ABOUTME: Also the one parser for the timestamps Intervals.icu answers with, shared by the calendar and self-report readers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Intervals.icu activity wire shape and how it becomes an [`Activity`].
//!
//! The provider deserializes [`IntervalsIcuActivity`] from the list and detail
//! endpoints and hands it to [`map_activity`]; [`parse_local_dt`] reads the
//! timestamps back in the same dialect the provider's queries send.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use pierre_core::civil_time;
use serde::Deserialize;

use super::QUERY_DATETIME_FORMAT;
use crate::intervals_icu_self_report::{feel_from_icu, rpe_from_icu};
use crate::intervals_icu_source::upstream_source;
use crate::models::{Activity, ActivityBuilder, ActivityComment, SportType, TimeSeriesData};

/// Intervals.icu activity payload shape (subset we map to `Activity`).
///
/// Carries the athlete's self-report alongside the sensor fields: `feel`,
/// `icu_rpe` and the free-text `description`. `session_rpe` is not read — it
/// is `icu_rpe` × moving minutes, a load figure derived from two fields the
/// activity already carries, not a separate report.
#[derive(Debug, Deserialize)]
pub(super) struct IntervalsIcuActivity {
    /// The activity's id: what its streams and comment thread are read by,
    /// once [`IntervalsIcuProvider::fetch_activity`](super::IntervalsIcuProvider)
    /// has checked the activity belongs to the reader.
    pub(super) id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    activity_type: Option<String>,
    /// The instant the activity started, in UTC: what [`start_instant`] reads
    /// first.
    #[serde(default)]
    start_date: Option<String>,
    /// The start on the athlete's wall clock, with no offset.
    start_date_local: String,
    /// The athlete's zone as intervals.icu names it (`America/Toronto`): what
    /// places [`start_date_local`](Self::start_date_local) on the timeline
    /// when the payload carries no `start_date`.
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    elapsed_time: Option<u64>,
    /// Seconds moving: `elapsed_time` less the stops intervals.icu removed.
    #[serde(default)]
    moving_time: Option<u64>,
    #[serde(default)]
    distance: Option<f64>,
    #[serde(default)]
    total_elevation_gain: Option<f64>,
    #[serde(default)]
    average_heartrate: Option<f64>,
    #[serde(default)]
    max_heartrate: Option<f64>,
    #[serde(default)]
    average_speed: Option<f64>,
    #[serde(default)]
    max_speed: Option<f64>,
    #[serde(default)]
    calories: Option<u32>,
    #[serde(default)]
    average_cadence: Option<f64>,
    #[serde(default)]
    average_watts: Option<f64>,
    #[serde(default)]
    max_watts: Option<f64>,
    #[serde(default)]
    weighted_average_watts: Option<f64>,
    #[serde(default)]
    icu_ftp: Option<f64>,
    /// How the athlete felt, 1–5 with **1 as the best** — the reverse of most
    /// scales. Read only through [`feel_from_icu`].
    #[serde(default)]
    feel: Option<i32>,
    /// Rating of perceived exertion, 1–10.
    #[serde(default)]
    icu_rpe: Option<i32>,
    /// The athlete's free-text notes on the activity.
    #[serde(default)]
    description: Option<String>,
    /// Where intervals.icu got the activity: a connected service
    /// (`GARMIN_CONNECT`, `STRAVA`, …) or the athlete (`UPLOAD`, `MANUAL`).
    /// Read only through [`upstream_source`].
    #[serde(default)]
    source: Option<String>,
    /// The device that recorded the activity, as intervals.icu names it
    /// (`Garmin Forerunner 965`, …): its API terms identify Garmin-sourced
    /// data by this name containing "garmin". Read through
    /// [`upstream_source`] and carried on the activity.
    #[serde(default)]
    device_name: Option<String>,
    /// The athlete the activity belongs to: what keeps a delegated detail
    /// read inside its athlete.
    #[serde(default)]
    pub(super) icu_athlete_id: Option<String>,
}

/// The activity carries intervals.icu's `device_name`, which names the
/// Garmin device model its attribution shows, whichever service relayed it
/// (carnet#521).
pub(super) fn map_activity(
    raw: IntervalsIcuActivity,
    streams: Option<TimeSeriesData>,
    comments: Option<Vec<ActivityComment>>,
) -> Option<Activity> {
    let start_date = start_instant(&raw)?;
    let sport = sport_for(raw.activity_type.as_deref());
    let name = raw
        .name
        .unwrap_or_else(|| format!("Intervals.icu {}", raw.id));
    let source = upstream_source(raw.source.as_deref(), raw.device_name.as_deref());
    let mut builder = ActivityBuilder::new(
        raw.id,
        name,
        sport,
        start_date,
        raw.elapsed_time.unwrap_or(0),
        "intervals_icu".to_owned(),
    )
    .source_opt(source)
    .moving_time_seconds_opt(raw.moving_time.filter(|seconds| *seconds > 0))
    .device_name_opt(
        raw.device_name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty()),
    );
    if let Some(d) = raw.distance {
        builder = builder.distance_meters(d);
    }
    if let Some(g) = raw.total_elevation_gain {
        builder = builder.elevation_gain(g);
    }
    if let Some(hr) = raw.average_heartrate {
        builder = builder.average_heart_rate(hr as u32);
    }
    if let Some(hr) = raw.max_heartrate {
        builder = builder.max_heart_rate(hr as u32);
    }
    if let Some(s) = raw.average_speed {
        builder = builder.average_speed(s);
    }
    if let Some(s) = raw.max_speed {
        builder = builder.max_speed(s);
    }
    if let Some(c) = raw.calories {
        builder = builder.calories(c);
    }
    if let Some(c) = raw.average_cadence {
        builder = builder.average_cadence(c as u32);
    }
    if let Some(p) = raw.average_watts {
        builder = builder.average_power(p as u32);
    }
    if let Some(p) = raw.max_watts {
        builder = builder.max_power(p as u32);
    }
    if let Some(p) = raw.weighted_average_watts {
        builder = builder.normalized_power(p as u32);
    }
    if let Some(ftp) = raw.icu_ftp {
        builder = builder.ftp(ftp as u32);
    }
    builder = builder
        .feel_opt(raw.feel.and_then(feel_from_icu))
        .perceived_exertion_opt(raw.icu_rpe.and_then(rpe_from_icu))
        .description_opt(raw.description.filter(|d| !d.trim().is_empty()))
        // An empty thread is a thread with nothing to say; `None` keeps it
        // off the wire rather than serializing an empty list.
        .comments_opt(comments.filter(|c| !c.is_empty()))
        .time_series_data_opt(streams);
    Some(builder.build())
}

/// When the activity started, as a UTC instant.
///
/// Intervals.icu answers with both `start_date`, the instant, and
/// `start_date_local`, the athlete's wall clock with no offset. Read as UTC,
/// the wall clock files an activity the athlete's UTC offset away from its
/// Strava, Garmin or WHOOP copy (four or five hours in Québec), far outside
/// the start window `merge_duplicates` ties copies by, and one ride stands as
/// two sessions in training load. The wall clock is read only for a payload
/// without `start_date`: in the athlete's `timezone` when it names one, as
/// UTC when it names none.
fn start_instant(raw: &IntervalsIcuActivity) -> Option<DateTime<Utc>> {
    raw.start_date
        .as_deref()
        .and_then(parse_local_dt)
        .or_else(|| wall_clock_in_zone(&raw.start_date_local, raw.timezone.as_deref()))
        .or_else(|| parse_local_dt(&raw.start_date_local))
}

/// An offset-less wall clock placed in a named IANA zone; `None` when the
/// zone is absent or unknown, the clock does not parse, or the clock falls in
/// the zone's spring-forward gap.
fn wall_clock_in_zone(wall_clock: &str, zone: Option<&str>) -> Option<DateTime<Utc>> {
    let zone = civil_time::parse_zone(zone)?;
    let naive = NaiveDateTime::parse_from_str(wall_clock, QUERY_DATETIME_FORMAT).ok()?;
    zone.from_local_datetime(&naive)
        .earliest()
        .map(|local| local.with_timezone(&Utc))
}

pub(crate) fn parse_local_dt(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, QUERY_DATETIME_FORMAT)
                .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
                .ok()
        })
}

fn sport_for(label: Option<&str>) -> SportType {
    match label.unwrap_or("").to_ascii_lowercase().as_str() {
        "ride" | "virtualride" | "ebikeride" | "cycling" => SportType::Ride,
        "run" | "trailrun" | "virtualrun" | "running" => SportType::Run,
        "swim" => SportType::Swim,
        "walk" | "hike" => SportType::Walk,
        "yoga" => SportType::Yoga,
        "weighttraining" | "workout" => SportType::Workout,
        other if !other.is_empty() => SportType::Other(other.to_owned()),
        _ => SportType::Other("intervals_icu".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::{json, Value};

    use super::{map_activity, IntervalsIcuActivity};
    use crate::deduplication::{self, DedupConfig};
    use crate::models::{Activity, ActivityBuilder, SportType};

    /// A 17:00 ride in Québec on 2026-09-20 (EDT, UTC−4) is 21:00 UTC.
    fn ride_at_21_utc() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 20, 21, 0, 0).unwrap()
    }

    fn mapped(payload: Value) -> Activity {
        let raw: IntervalsIcuActivity = serde_json::from_value(payload).unwrap();
        map_activity(raw, None, None).unwrap()
    }

    #[test]
    fn start_date_is_the_instant_and_wins_over_the_wall_clock() {
        let activity = mapped(json!({
            "id": "i1",
            "type": "Ride",
            "start_date": "2026-09-20T21:00:00Z",
            "start_date_local": "2026-09-20T17:00:00",
            "timezone": "America/Toronto",
        }));
        assert_eq!(
            activity.start_date(),
            ride_at_21_utc(),
            "bug: the payload's UTC `start_date` must be the start, not the wall clock"
        );
    }

    #[test]
    fn moving_time_sits_beside_the_elapsed_duration() {
        let activity = mapped(json!({
            "id": "i9",
            "type": "Ride",
            "start_date": "2026-09-20T21:00:00Z",
            "start_date_local": "2026-09-20T17:00:00",
            "elapsed_time": 5_400,
            "moving_time": 4_800,
        }));
        assert_eq!(
            activity.duration_seconds(),
            5_400,
            "bug: duration is elapsed time"
        );
        assert_eq!(
            activity.moving_time_seconds(),
            Some(4_800),
            "bug: moving time is carried beside it"
        );
    }

    #[test]
    fn wall_clock_without_start_date_is_read_in_the_athletes_zone() {
        let activity = mapped(json!({
            "id": "i2",
            "type": "Ride",
            "start_date_local": "2026-09-20T17:00:00",
            "timezone": "America/Toronto",
        }));
        assert_eq!(
            activity.start_date(),
            ride_at_21_utc(),
            "bug: without `start_date`, the wall clock must be read in the payload's `timezone`"
        );
    }

    #[test]
    fn wall_clock_in_an_unknown_zone_reads_as_utc() {
        let activity = mapped(json!({
            "id": "i3",
            "type": "Ride",
            "start_date_local": "2026-09-20T17:00:00",
            "timezone": "Not/AZone",
        }));
        assert_eq!(
            activity.start_date(),
            Utc.with_ymd_and_hms(2026, 9, 20, 17, 0, 0).unwrap(),
            "bug: a wall clock in a zone that does not parse must still map, read as UTC"
        );
    }

    /// Intervals.icu's copy of a ride Strava also holds merges into one
    /// session, so the ride's training load counts once.
    #[test]
    fn intervals_copy_of_a_strava_ride_merges_into_one_session() {
        let intervals = mapped(json!({
            "id": "i4",
            "type": "Ride",
            "start_date": "2026-09-20T21:00:00Z",
            "start_date_local": "2026-09-20T17:00:00",
            "timezone": "America/Toronto",
            "elapsed_time": 5_400,
            "distance": 45_000.0,
            "source": "GARMIN_CONNECT",
            "device_name": "Garmin Edge 840",
        }));
        let strava = ActivityBuilder::new(
            "s4".to_owned(),
            "Afternoon Ride".to_owned(),
            SportType::Ride,
            ride_at_21_utc(),
            5_400,
            "strava".to_owned(),
        )
        .distance_meters(45_100.0)
        .build();

        let (merged, _) =
            deduplication::merge_duplicates(vec![intervals, strava], &DedupConfig::default());

        assert_eq!(
            merged.len(),
            1,
            "bug: intervals.icu's copy must merge with Strava's, one ride, one session: {merged:?}"
        );
    }
}
