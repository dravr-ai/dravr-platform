// ABOUTME: Pins that the Strava API mapper fills every field the Strava descriptor declares
// ABOUTME: A list row from the summary shape, a detail read with its efforts and all eleven stream channels
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::StravaProvider;
    use crate::core::stream_sample_count;
    use crate::field_support::tests::assert_mapper_fills;
    use crate::field_support::StreamChannels;
    use crate::spi::{ProviderDescriptor, StravaDescriptor};
    use crate::strava_types::{DetailedActivityResponse, StravaActivityResponse, StravaStreamSet};

    /// A ride as `GET /athlete/activities` returns it: no calories, no
    /// description, the summary polyline under `map`.
    fn summary() -> serde_json::Value {
        json!({
            "id": 16_001,
            "name": "Sortie du samedi",
            "type": "Ride",
            "sport_type": "GravelRide",
            "start_date": "2026-11-07T13:00:00Z",
            "elapsed_time": 7_400,
            "moving_time": 7_020,
            "distance": 61_200.0,
            "total_elevation_gain": 540.0,
            "average_heartrate": 141.2,
            "max_heartrate": 176.0,
            "average_cadence": 84.1,
            "average_watts": 196.4,
            "weighted_average_watts": 214.0,
            "workout_type": 10,
            "trainer": false,
            "commute": false,
            "map": { "summary_polyline": "_p~iF~ps|U_ulLnnqC_mqNvxq`@" }
        })
    }

    #[test]
    fn the_strava_mapper_fills_what_its_descriptor_declares() {
        let list_row = StravaProvider::convert_strava_activity(
            serde_json::from_value::<StravaActivityResponse>(summary()).unwrap(),
        )
        .unwrap();

        let mut detail = summary();
        for (key, value) in json!({
            "description": "Mal aux bras !",
            "private_note": "Selle trop haute",
            "calories": 1_380.0,
            "device_name": "Garmin Edge 840",
            "gear": { "id": "b123", "name": "Gravel" },
            "perceived_exertion": 6.0,
            "splits_metric": [{ "distance": 1_000.0, "elapsed_time": 120, "moving_time": 118, "split": 1 }],
            "laps": [{ "id": 1, "elapsed_time": 7_400, "moving_time": 7_020, "distance": 61_200.0 }],
            "segment_efforts": [{
                "id": 9_001,
                "name": "Camillien-Houde",
                "elapsed_time": 412,
                "start_date": "2026-11-07T13:10:00Z",
                "distance": 1_820.5,
                "segment": { "id": 611_413, "name": "Camillien-Houde" }
            }]
        })
        .as_object()
        .unwrap()
        {
            detail[key] = value.clone();
        }
        let streams: StravaStreamSet = serde_json::from_value(json!({
            "time": { "data": [0, 1] },
            "heartrate": { "data": [120, 121] },
            "watts": { "data": [180, 190] },
            "cadence": { "data": [80, 82] },
            "velocity_smooth": { "data": [7.1, 7.2] },
            "altitude": { "data": [44.0, 44.2] },
            "temp": { "data": [11.0, 11.0] },
            "distance": { "data": [0.0, 7.2] },
            "latlng": { "data": [[45.5, -73.6], [45.5001, -73.6001]] },
            "grade_smooth": { "data": [3.2, 3.4] },
            "moving": { "data": [true, true] }
        }))
        .unwrap();
        let detailed = StravaProvider::convert_detailed_strava_activity(
            serde_json::from_value::<DetailedActivityResponse>(detail).unwrap(),
            StravaProvider::streams_to_time_series(streams),
        )
        .unwrap();

        assert_mapper_fills(
            "strava",
            StravaDescriptor.activity_fields(),
            &list_row,
            &detailed,
        );
    }

    /// Strava claims the two provider-computed channels its mapper fills.
    #[test]
    fn the_strava_descriptor_claims_grade_smooth_and_moving() {
        let streams = StravaDescriptor.activity_fields().streams;
        assert!(
            streams.contains(StreamChannels::GRADE_SMOOTH | StreamChannels::MOVING),
            "bug: Strava serves grade_smooth and moving"
        );
    }

    /// Strava's own `grade_smooth` and `moving` streams reach the two
    /// channels as served: values index-aligned with the time axis, a `null`
    /// sample a gap, a sample with no time offset dropped from both.
    #[test]
    fn grade_smooth_and_moving_map_into_their_channels() {
        let streams: StravaStreamSet = serde_json::from_value(json!({
            "time": { "data": [0, 1, null, 3] },
            "grade_smooth": { "data": [1.5, null, 9.9, -2.0] },
            "moving": { "data": [false, true, true, null] }
        }))
        .unwrap();

        let series = StravaProvider::streams_to_time_series(streams).expect("a stream set");

        assert_eq!(
            series.timestamps,
            vec![0, 1, 3],
            "bug: the time axis changed"
        );
        assert_eq!(
            series.grade_smooth,
            Some(vec![Some(1.5), None, Some(-2.0)]),
            "bug: the grade channel did not cross"
        );
        assert_eq!(
            series.moving,
            Some(vec![Some(false), Some(true), None]),
            "bug: the moving channel did not cross"
        );
    }

    /// A stream set carrying only the two provider-computed channels is still
    /// a stream set.
    #[test]
    fn a_set_with_only_grade_and_moving_is_not_empty() {
        let streams: StravaStreamSet = serde_json::from_value(json!({
            "grade_smooth": { "data": [1.0, 2.0] },
            "moving": { "data": [true, false] }
        }))
        .unwrap();

        let series = StravaProvider::streams_to_time_series(streams).expect("a stream set");

        assert_eq!(
            series.timestamps,
            Vec::<u32>::new(),
            "bug: a series with no time axis grew one"
        );
        assert_eq!(
            stream_sample_count(&series),
            2,
            "bug: the channel samples were miscounted"
        );
    }
}
