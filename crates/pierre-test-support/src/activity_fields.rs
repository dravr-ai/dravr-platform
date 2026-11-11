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
//! does not know is reported as missing (`unknown:<name>`), so a field added
//! to the dictionary cannot pass a guard before its getter is here.

use pierre_core::models::{Activity, TimeSeriesData};

/// Whether `activity` carries the field the dictionary calls `name`; `None`
/// for a name the dictionary does not define.
#[must_use]
pub fn carries_field(activity: &Activity, name: &str) -> Option<bool> {
    let carried = match name {
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
        _ => return None,
    };
    Some(carried)
}

/// Whether `activity`'s streams carry the channel the dictionary calls
/// `name`; `None` for a name the dictionary does not define.
#[must_use]
pub fn carries_channel(activity: &Activity, name: &str) -> Option<bool> {
    activity.time_series_data().map_or_else(
        || CHANNELS.contains(&name).then_some(false),
        |series| channel(series, name),
    )
}

/// Every channel name the dictionary defines.
const CHANNELS: [&str; 9] = [
    "timestamps",
    "heart_rate",
    "power",
    "cadence",
    "speed",
    "altitude",
    "gps_coordinates",
    "distance",
    "temperature",
];

fn channel(series: &TimeSeriesData, name: &str) -> Option<bool> {
    Some(match name {
        "timestamps" => !series.timestamps.is_empty(),
        "heart_rate" => series.heart_rate.is_some(),
        "power" => series.power.is_some(),
        "cadence" => series.cadence.is_some(),
        "speed" => series.speed.is_some(),
        "altitude" => series.altitude.is_some(),
        "gps_coordinates" => series.gps_coordinates.is_some(),
        "distance" => series.distance.is_some(),
        "temperature" => series.temperature.is_some(),
        _ => return None,
    })
}

/// The names among `fields` and `channels` that `activity` does not carry.
///
/// Fields come first, then channels (as `streams.<name>`), and a name this
/// table does not know as `unknown:<name>`: what a guard reports when a
/// mapper falls short of its declaration.
#[must_use]
pub fn missing(activity: &Activity, fields: &[&str], channels: &[&str]) -> Vec<String> {
    let fields = fields
        .iter()
        .filter_map(|name| match carries_field(activity, name) {
            Some(true) => None,
            Some(false) => Some((*name).to_owned()),
            None => Some(format!("unknown:{name}")),
        });
    let channels = channels
        .iter()
        .filter_map(|name| match carries_channel(activity, name) {
            Some(true) => None,
            Some(false) => Some(format!("streams.{name}")),
            None => Some(format!("unknown:streams.{name}")),
        });
    fields.chain(channels).collect()
}
