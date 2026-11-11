// ABOUTME: Pins that the Wahoo mapper fills every field the Wahoo descriptor declares
// ABOUTME: The Cloud API reference's workout sample, and its FIT file's samples on the detail read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{TimeZone, Utc};
use pierre_test_support::fit::FitWorkout;

use super::{time_series_from_fit, WahooWorkout};
use crate::field_support::assert_mapper_fills;
use crate::spi::ProviderDescriptor;
use crate::wahoo_descriptor::WahooDescriptor;

/// The Cloud API reference's workout sample, decimals as strings.
const COMPLETED: &str = r#"{
    "id": 56519, "starts": "2015-08-12T09:00:00.000Z", "minutes": 12,
    "name": "Friday Fun", "workout_token": "123", "workout_type_id": 40,
    "workout_summary": {
        "id": 8297, "name": "Easy Ride", "ascent_accum": "450.00",
        "cadence_avg": "50.00", "calories_accum": "1500.00",
        "distance_accum": "24909.71", "duration_active_accum": "179.00",
        "duration_total_accum": "275.00", "heart_rate_avg": "100.00",
        "power_bike_np_last": "150.00", "power_bike_tss_last": "304.90",
        "power_avg": "94.59", "speed_avg": "10.75"
    }
}"#;

#[test]
fn the_wahoo_mapper_fills_what_its_descriptor_declares() {
    let workout: WahooWorkout = serde_json::from_str(COMPLETED).unwrap();
    let summary = workout.workout_summary.clone().unwrap();
    let list_row = workout.activity_builder(&summary).unwrap().build();
    let fit = FitWorkout::ride(Utc.with_ymd_and_hms(2026, 9, 20, 21, 0, 0).unwrap(), 30)
        .with_power()
        .encode();
    let detailed = workout
        .activity_builder(&summary)
        .unwrap()
        .time_series_data(time_series_from_fit(&fit).unwrap())
        .build();

    assert_mapper_fills(
        "wahoo",
        WahooDescriptor.activity_fields(),
        &list_row,
        &detailed,
    );
}
