// ABOUTME: Google Health API provider — the cloud replacement for the Fitbit Web API (turndown Sep 2026).
// ABOUTME: Parses intraday heart-rate dataPoints from health.googleapis.com/v4 into riviere time-series points.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Google Health API Provider
//!
//! Targets the Google Health API (`https://health.googleapis.com/v4`), the
//! cloud successor to the Fitbit Web API. Heart-rate data is retrieved from the
//! unified data-point endpoint:
//!
//! ```text
//! GET /v4/users/me/dataTypes/heart-rate/dataPoints
//! Authorization: Bearer {google_oauth_access_token}
//! ```
//!
//! Auth is Google OAuth 2.0. Heart rate is a health metric, not an activity
//! metric, so it is authorised by
//! `https://www.googleapis.com/auth/googlehealth.health_metrics_and_measurements.readonly`
//! — `activity_and_fitness` covers steps and active-zone-minutes and is rejected
//! on this endpoint.
//! Each heart-rate `dataPoint` carries a `sampleTime.physicalTime` (an absolute
//! RFC3339 UTC instant) and a `beatsPerMinute` value, which map directly to
//! riviere `(timestamp, value)` time-series points.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use serde::Deserialize;
use serde_json::from_str;

/// Base URL for the Google Health API (v4).
pub const GOOGLE_HEALTH_API_BASE: &str = "https://health.googleapis.com/v4";

/// OAuth scope required to read the health-metrics bundle, which is the one
/// carrying `heart-rate` (alongside variability, oxygen saturation, ECG and
/// resting heart rate).
///
/// LIMITATION(registre#324): `GOOGLE_HEALTH_API_BASE` and
/// `GOOGLE_HEALTH_METRICS_SCOPE` have no production consumer — no
/// `FitnessProvider` implementation, no `spi::ProviderDescriptor` and no
/// `registry.rs` registration exists for Google Health, so nothing in a
/// shipping build can read them.
pub const GOOGLE_HEALTH_METRICS_SCOPE: &str =
    "https://www.googleapis.com/auth/googlehealth.health_metrics_and_measurements.readonly";

/// Response envelope for `users.dataTypes.dataPoints.list`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleHealthDataPointsResponse {
    #[serde(default)]
    data_points: Vec<GoogleHealthDataPoint>,
    #[serde(default)]
    next_page_token: Option<String>,
}

/// A single data point. Only the heart-rate payload is decoded here; other
/// data types carry sibling fields the heart-rate path ignores.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleHealthDataPoint {
    #[serde(default)]
    heart_rate: Option<GoogleHealthHeartRate>,
}

/// Heart-rate payload: a beats-per-minute value (int64 serialized as a string)
/// observed at a sample time.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleHealthHeartRate {
    sample_time: GoogleHealthSampleTime,
    beats_per_minute: String,
}

/// Observation sample time. `physicalTime` is the absolute UTC instant of the
/// reading (`civilTime`/`utcOffset` carry the local rendering, unused here).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleHealthSampleTime {
    physical_time: String,
}

/// Parse a Google Health API heart-rate `dataPoints.list` JSON body into
/// `(timestamp, bpm)` points, dropping any non-heart-rate points.
///
/// LIMITATION(registre#324): `parse_intraday_heart_rate` and `next_page_token`
/// are called only by this module's doctest — the `GoogleHealthProvider` that
/// would fetch the bodies they decode does not exist, so no shipping build
/// reaches either. They also do not yet reject an API error envelope (a body
/// carrying `error` decodes to zero points) nor range-check the parsed bpm.
///
/// Timestamps come from `heartRate.sampleTime.physicalTime`, an absolute UTC
/// instant, so no timezone correction is required (unlike Fitbit's local times).
/// `nextPageToken`, when present, signals the caller to fetch the next page.
///
/// # Examples
///
/// ```
/// use pierre_providers::google_health_provider::parse_intraday_heart_rate;
///
/// let body = r#"{
///   "dataPoints": [
///     {
///       "name": "users/me/dataTypes/heart-rate/dataPoints/1",
///       "dataSource": { "recordingMethod": "PASSIVELY_MEASURED", "platform": "FITBIT" },
///       "heartRate": {
///         "sampleTime": { "physicalTime": "2026-06-02T13:10:05Z", "utcOffset": "-18000s" },
///         "beatsPerMinute": "61"
///       }
///     },
///     {
///       "name": "users/me/dataTypes/heart-rate/dataPoints/2",
///       "heartRate": {
///         "sampleTime": { "physicalTime": "2026-06-02T13:10:10Z" },
///         "beatsPerMinute": "63"
///       }
///     }
///   ],
///   "nextPageToken": ""
/// }"#;
/// let points = parse_intraday_heart_rate(body).unwrap();
/// assert_eq!(points.len(), 2);
/// assert_eq!(points[0].1, 61.0);
/// assert_eq!(points[1].1, 63.0);
/// ```
///
/// # Errors
///
/// Returns an error if the body is not valid `dataPoints` JSON, a `physicalTime`
/// is not RFC3339, or a `beatsPerMinute` is not an integer.
pub fn parse_intraday_heart_rate(body: &str) -> AppResult<Vec<(DateTime<Utc>, f64)>> {
    let response: GoogleHealthDataPointsResponse = from_str(body).map_err(|e| {
        AppError::external_service("GoogleHealth", format!("Failed to parse dataPoints: {e}"))
    })?;

    let mut points = Vec::with_capacity(response.data_points.len());
    for point in &response.data_points {
        let Some(hr) = &point.heart_rate else {
            continue;
        };
        let timestamp = DateTime::parse_from_rfc3339(&hr.sample_time.physical_time)
            .map_err(|e| {
                AppError::external_service(
                    "GoogleHealth",
                    format!("invalid sampleTime '{}': {e}", hr.sample_time.physical_time),
                )
            })?
            .with_timezone(&Utc);
        let bpm: f64 = hr.beats_per_minute.parse().map_err(|e| {
            AppError::external_service(
                "GoogleHealth",
                format!("invalid beatsPerMinute '{}': {e}", hr.beats_per_minute),
            )
        })?;
        points.push((timestamp, bpm));
    }
    Ok(points)
}

/// The `nextPageToken` from a `dataPoints.list` body, if pagination should
/// continue. Empty tokens are treated as "no more pages".
///
/// # Errors
///
/// Returns an error if the body is not valid `dataPoints` JSON.
pub fn next_page_token(body: &str) -> AppResult<Option<String>> {
    let response: GoogleHealthDataPointsResponse = from_str(body).map_err(|e| {
        AppError::external_service("GoogleHealth", format!("Failed to parse dataPoints: {e}"))
    })?;
    Ok(response.next_page_token.filter(|token| !token.is_empty()))
}
