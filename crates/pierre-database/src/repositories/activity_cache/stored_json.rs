// ABOUTME: The data_json a cached activity row stores: the activity as served, minus its per-sample streams
// ABOUTME: Samples live in one store only, so a provider row never carries them into cached_activities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::Activity;
use serde_json::Value;

/// The `Activity` key that holds its per-sample streams.
const TIME_SERIES_KEY: &str = "time_series_data";

/// The `data_json` a `cached_activities` row stores for `activity`.
///
/// It is the activity as its provider served it, without
/// `time_series_data`. A provider that reads samples on its list path
/// (sciotte with `PIERRE_SCIOTTE_ENRICH_DETAILS=true` attaches the route
/// track) would otherwise persist them in every summary row; samples belong
/// to the one per-activity sample store, and the live providers read them on
/// demand. `Activity` has no setter for the field, so the strip happens on
/// the serialized object: one extra `Value` pass per upserted row.
///
/// # Errors
/// Returns a database error when the activity does not serialize, or does
/// not serialize to a JSON object.
pub(crate) fn stored_activity_json(activity: &Activity) -> AppResult<String> {
    let mut value = serde_json::to_value(activity)
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))?;
    let Value::Object(object) = &mut value else {
        return Err(AppError::database(
            "Failed to serialize activity: not a JSON object",
        ));
    };
    object.remove(TIME_SERIES_KEY);
    serde_json::to_string(&value)
        .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use pierre_core::models::{ActivityBuilder, SportType, TimeSeriesData};

    fn ride(streams: Option<TimeSeriesData>) -> Activity {
        ActivityBuilder::new(
            "ride-1",
            "Morning ride",
            SportType::Ride,
            Utc.with_ymd_and_hms(2026, 10, 27, 7, 0, 0)
                .single()
                .expect("a valid instant"),
            3_600,
            "strava",
        )
        .distance_meters(30_000.0)
        .time_series_data_opt(streams)
        .build()
    }

    fn streams() -> TimeSeriesData {
        TimeSeriesData {
            timestamps: vec![0, 1, 2],
            heart_rate: Some(vec![Some(120), Some(121), Some(122)]),
            power: None,
            cadence: None,
            speed: None,
            altitude: None,
            temperature: None,
            gps_coordinates: Some(vec![
                Some((45.5, -73.5)),
                Some((45.6, -73.6)),
                Some((45.7, -73.7)),
            ]),
            distance: None,
        }
    }

    #[test]
    fn a_streams_bearing_activity_is_stored_without_its_samples() {
        let json = stored_activity_json(&ride(Some(streams()))).expect("serializes");

        assert!(
            !json.contains(TIME_SERIES_KEY),
            "bug: the stored row still carries samples: {json}"
        );
        let stored: Activity = serde_json::from_str(&json).expect("an Activity");
        assert!(
            stored.time_series_data().is_none(),
            "bug: samples read back from the stored row"
        );
        assert_eq!(stored.id(), "ride-1", "bug: the summary lost its id");
        assert_eq!(
            stored.distance_meters(),
            Some(30_000.0),
            "bug: the summary lost its distance"
        );
    }

    #[test]
    fn an_activity_without_samples_is_stored_as_served() {
        let activity = ride(None);

        let json = stored_activity_json(&activity).expect("serializes");

        let stored: Value = serde_json::from_str(&json).expect("JSON");
        assert_eq!(
            stored,
            serde_json::to_value(&activity).expect("serializes"),
            "bug: an activity without samples must be stored unchanged"
        );
    }
}
