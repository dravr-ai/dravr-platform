// ABOUTME: Pins that the Strava API mapper fills every field the Strava descriptor declares
// ABOUTME: A list row from the summary shape, a detail read with all nine stream channels
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::StravaProvider;
    use crate::field_support::tests::assert_mapper_fills;
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
            "laps": [{ "id": 1, "elapsed_time": 7_400, "moving_time": 7_020, "distance": 61_200.0 }]
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
            "latlng": { "data": [[45.5, -73.6], [45.5001, -73.6001]] }
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
}
