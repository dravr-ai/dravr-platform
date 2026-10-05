// ABOUTME: Intervals.icu activity streams — the streams.json wire shape and its mapping onto TimeSeriesData
// ABOUTME: Pure functions; an unrecorded sample stays a gap, and latlng pairs latitudes with longitudes by index
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// Clippy allowances for this module:
// - cast_possible_truncation: Intervals.icu returns f64 for HR/power/cadence; truncating to u32 is safe within sensor ranges
// - cast_sign_loss: same — HR/power/cadence are always non-negative
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

//! # Intervals.icu activity streams
//!
//! The per-second samples `GET /api/v1/activity/{id}/streams.json` returns,
//! read into the platform's [`TimeSeriesData`].

use serde::Deserialize;

use crate::models::TimeSeriesData;

/// One stream of `GET /api/v1/activity/{id}/streams.json` (the `ActivityStream`
/// schema of the intervals.icu `OpenAPI` spec).
///
/// Every stream carries its samples in `data`, index-aligned with the
/// recording; a sample the device did not record is `null`. `latlng` is the
/// one stream with a second array: `data` holds the latitudes and `data2` the
/// longitudes, index by index, so its sample count is `data.len()`.
/// `allNull` marks a stream with no recorded sample at all.
#[derive(Debug, Deserialize)]
pub struct IntervalsIcuStream {
    #[serde(rename = "type")]
    stream_type: String,
    #[serde(default)]
    data: Vec<Option<f64>>,
    /// The longitudes of a `latlng` stream; absent on every other stream.
    #[serde(default)]
    data2: Option<Vec<Option<f64>>>,
    /// Set when no sample of the stream was recorded.
    #[serde(default, rename = "allNull")]
    all_null: bool,
}

/// A scalar channel, index-aligned with `timestamps`, its `null` samples
/// kept as gaps.
///
/// [`TimeSeriesData`] marks an unrecorded entry as `None`, and every consumer
/// skips a gap rather than reading it (zone time, normalized power and
/// decoupling count recorded samples only). A `null` therefore stays a gap:
/// the recorded samples around it keep their values and their instants,
/// with no zero, repeated value or dropped entry standing in for it.
fn gapped_channel<T>(stream: &IntervalsIcuStream, convert: impl Fn(f64) -> T) -> Vec<Option<T>> {
    stream
        .data
        .iter()
        .map(|sample| sample.map(&convert))
        .collect()
}

/// The streams of one activity as a [`TimeSeriesData`], index-aligned and
/// with every unrecorded sample kept as a gap.
pub fn streams_to_time_series(streams: &[IntervalsIcuStream]) -> TimeSeriesData {
    let mut hr: Option<Vec<Option<u32>>> = None;
    let mut power: Option<Vec<Option<u32>>> = None;
    let mut cadence: Option<Vec<Option<u32>>> = None;
    let mut speed: Option<Vec<Option<f32>>> = None;
    let mut altitude: Option<Vec<Option<f32>>> = None;
    let mut latlng: Vec<(f64, f64)> = Vec::new();
    let mut max_len = 0_usize;
    for stream in streams {
        max_len = max_len.max(stream.data.len());
        if stream.all_null {
            continue;
        }
        match stream.stream_type.as_str() {
            "heartrate" => hr = Some(gapped_channel(stream, |v| v as u32)),
            "watts" => power = Some(gapped_channel(stream, |v| v as u32)),
            "cadence" => cadence = Some(gapped_channel(stream, |v| v as u32)),
            "velocity_smooth" => speed = Some(gapped_channel(stream, |v| v as f32)),
            "altitude" => altitude = Some(gapped_channel(stream, |v| v as f32)),
            "latlng" => {
                // Latitudes in `data`, longitudes in `data2`, paired by index.
                // A latlng stream without `data2` carries no longitude at all:
                // it is read as no GPS, since no pairing of latitudes with each
                // other places a single point on the map. A sample without a
                // fix (either side `null`) is left out of the track, which is
                // a line through the recorded points and not indexed by time.
                if let Some(longitudes) = stream.data2.as_deref() {
                    latlng = stream
                        .data
                        .iter()
                        .zip(longitudes)
                        .filter_map(|(lat, lng)| Some(((*lat)?, (*lng)?)))
                        .collect();
                }
            }
            _ => {}
        }
    }
    let timestamps: Vec<u32> = (0..max_len)
        .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
        .collect();
    TimeSeriesData {
        timestamps,
        heart_rate: hr,
        power,
        cadence,
        speed,
        altitude,
        temperature: None,
        gps_coordinates: if latlng.is_empty() {
            None
        } else {
            Some(latlng)
        },
        distance: None,
    }
}
