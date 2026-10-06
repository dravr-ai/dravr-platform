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

use chrono::{DateTime, Utc};
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
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    activity_type: Option<String>,
    start_date_local: String,
    #[serde(default)]
    elapsed_time: Option<u64>,
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
    let start_date = parse_local_dt(&raw.start_date_local)?;
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

pub fn parse_local_dt(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, QUERY_DATETIME_FORMAT)
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
