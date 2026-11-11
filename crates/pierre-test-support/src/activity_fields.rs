// ABOUTME: Whether a mapped Activity carries a field or stream channel, by the name the field dictionary uses
// ABOUTME: The one getter table the field-dictionary guards read, so every mapper is checked the same way
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Field presence on a mapped [`Activity`], by dictionary name.
//!
//! A backend's descriptor declares, by name, the fields its mapper fills
//! (`pierre_providers::field_support`). Each guard maps a fixture through
//! that mapper and checks every declared name here. The names are the keys an
//! [`Activity`] and its [`TimeSeriesData`] serialize under. A name this table
//! does not know panics, so a field added to the dictionary cannot pass a
//! guard before its getter is here.

use pierre_core::models::{Activity, TimeSeriesData};

/// Whether `activity` carries the field the dictionary calls `name`.
///
/// # Panics
///
/// On a name the dictionary does not define.
#[must_use]
pub fn carries_field(activity: &Activity, name: &str) -> bool {
    match name {
        "description" => activity.description().is_some(),
        "private_note" => activity.private_note().is_some(),
        "comments" => activity.comments().is_some_and(|c| !c.is_empty()),
        "workout_type" => activity.workout_type().is_some(),
        "trainer" => activity.trainer().is_some(),
        "commute" => activity.commute().is_some(),
        "average_heart_rate" => activity.average_heart_rate().is_some(),
        "max_heart_rate" => activity.max_heart_rate().is_some(),
        "average_power" => activity.average_power().is_some(),
        "normalized_power" => activity.normalized_power().is_some(),
        "moving_time_seconds" => activity.moving_time_seconds().is_some(),
        "calories" => activity.calories().is_some(),
        "average_cadence" => activity.average_cadence().is_some(),
        "gear" => activity.gear().is_some(),
        "device_name" => activity.device_name().is_some(),
        "laps" => activity.laps().is_some_and(|l| !l.is_empty()),
        "splits" => activity.splits().is_some_and(|s| !s.is_empty()),
        "segment_efforts" => activity.segment_efforts().is_some_and(|s| !s.is_empty()),
        "summary_polyline" => activity.summary_polyline().is_some(),
        "sport_type_detail" => activity.sport_type_detail().is_some(),
        "perceived_exertion" => activity.perceived_exertion().is_some(),
        other => panic!("the field dictionary names `{other}`, which this table has no getter for"),
    }
}

/// Whether `activity`'s streams carry the channel the dictionary calls `name`.
///
/// # Panics
///
/// On a name the dictionary does not define.
#[must_use]
pub fn carries_channel(activity: &Activity, name: &str) -> bool {
    activity
        .time_series_data()
        .is_some_and(|series| channel(series, name))
}

fn channel(series: &TimeSeriesData, name: &str) -> bool {
    match name {
        "timestamps" => !series.timestamps.is_empty(),
        "heart_rate" => series.heart_rate.is_some(),
        "power" => series.power.is_some(),
        "cadence" => series.cadence.is_some(),
        "speed" => series.speed.is_some(),
        "altitude" => series.altitude.is_some(),
        "gps_coordinates" => series.gps_coordinates.is_some(),
        "distance" => series.distance.is_some(),
        "temperature" => series.temperature.is_some(),
        other => panic!(
            "the field dictionary names channel `{other}`, which this table has no reader for"
        ),
    }
}

/// The names among `fields` and `channels` that `activity` does not carry,
/// fields first and then channels (as `streams.<name>`): what a guard reports
/// when a mapper falls short of its declaration.
#[must_use]
pub fn missing(activity: &Activity, fields: &[&str], channels: &[&str]) -> Vec<String> {
    fields
        .iter()
        .filter(|name| !carries_field(activity, name))
        .map(|name| (*name).to_owned())
        .chain(
            channels
                .iter()
                .filter(|name| !carries_channel(activity, name))
                .map(|name| format!("streams.{name}")),
        )
        .collect()
}
