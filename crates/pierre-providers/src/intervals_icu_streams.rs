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
use tracing::warn;

use crate::core::keep_timed;
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
pub(crate) struct IntervalsIcuStream {
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
/// with no zero, repeated value or dropped entry standing in for it. A
/// sample whose time offset is `null` (`timed[i]` false) is dropped instead:
/// it has no instant to stand at.
fn gapped_channel<T>(
    stream: &IntervalsIcuStream,
    timed: &[bool],
    convert: impl Fn(f64) -> T,
) -> Vec<Option<T>> {
    keep_timed(
        stream
            .data
            .iter()
            .map(|sample| sample.map(&convert))
            .collect(),
        timed,
    )
}

/// The streams of one activity as a [`TimeSeriesData`], index-aligned and
/// with every unrecorded sample kept as a gap.
///
/// `timestamps` is intervals.icu's own `time` stream, seconds from the
/// start, which the streams answer carries beside the channels. A time offset
/// that is `null` leaves its sample with no instant, so that index is dropped
/// from every channel. An answer without a `time` stream has no time axis:
/// `timestamps` stays empty rather than counting sample indices as seconds,
/// and the channels are kept as served
/// ([`crate::core::stream_sample_count`] reads their length).
pub(crate) fn streams_to_time_series(streams: &[IntervalsIcuStream]) -> TimeSeriesData {
    let time = streams
        .iter()
        .find(|stream| stream.stream_type == "time" && !stream.all_null);
    // `timed[i]` is false where intervals.icu sent no time offset for sample `i`.
    let timed: Vec<bool> = time
        .map(|stream| stream.data.iter().map(Option::is_some).collect())
        .unwrap_or_default();
    let timestamps: Vec<u32> = time
        .map(|stream| stream.data.iter().flatten().map(|t| *t as u32).collect())
        .unwrap_or_default();
    let mut hr: Option<Vec<Option<u32>>> = None;
    let mut power: Option<Vec<Option<u32>>> = None;
    let mut cadence: Option<Vec<Option<u32>>> = None;
    let mut speed: Option<Vec<Option<f32>>> = None;
    let mut altitude: Option<Vec<Option<f32>>> = None;
    let mut latlng: Vec<Option<(f64, f64)>> = Vec::new();
    let mut sample_count = 0_usize;
    for stream in streams {
        sample_count = sample_count.max(stream.data.len());
        if stream.all_null {
            continue;
        }
        match stream.stream_type.as_str() {
            "heartrate" => hr = Some(gapped_channel(stream, &timed, |v| v as u32)),
            "watts" => power = Some(gapped_channel(stream, &timed, |v| v as u32)),
            "cadence" => cadence = Some(gapped_channel(stream, &timed, |v| v as u32)),
            "velocity_smooth" => speed = Some(gapped_channel(stream, &timed, |v| v as f32)),
            "altitude" => altitude = Some(gapped_channel(stream, &timed, |v| v as f32)),
            "latlng" => {
                // Latitudes in `data`, longitudes in `data2`, paired by index.
                // A latlng stream without `data2` carries no longitude at all:
                // it is read as no GPS, since no pairing of latitudes with each
                // other places a single point on the map. A sample without a
                // fix (either side `null`) keeps its slot as `None`, so every
                // fix after it stays on its own sample's time.
                if let Some(longitudes) = stream.data2.as_deref() {
                    let pairs: Vec<(Option<f64>, Option<f64>)> = stream
                        .data
                        .iter()
                        .copied()
                        .zip(longitudes.iter().copied())
                        .collect();
                    latlng = keep_timed(pairs, &timed)
                        .into_iter()
                        .map(|(lat, lng)| Some((lat?, lng?)))
                        .collect();
                }
            }
            _ => {}
        }
    }
    if time.is_none() && sample_count > 0 {
        warn!(
            sample_count,
            "intervals.icu streams answer carries no time stream; serving it without a time axis"
        );
    }
    TimeSeriesData {
        timestamps,
        heart_rate: hr,
        power,
        cadence,
        speed,
        altitude,
        temperature: None,
        gps_coordinates: latlng.iter().any(Option::is_some).then_some(latlng),
        distance: None,
    }
}
