// ABOUTME: Garmin Connect response shapes the native Garmin provider deserializes
// ABOUTME: List rows are flat; detail metrics sit under summaryDTO; laps and splits share lapDTOs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::Deserialize;

/// Garmin's `GET user/id` answer: the user id the access token belongs to,
/// which is all the Health API says about the user.
#[derive(Debug, Deserialize)]
pub(super) struct GarminUserIdResponse {
    #[serde(rename = "userId")]
    pub(super) user_id: String,
}

/// Garmin Connect API activity-type DTO. Returned at multiple nesting depths
/// (`activityTypeDTO` on detail; `activityType` on list rows).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(super) struct GarminActivityType {
    #[serde(rename = "typeKey")]
    pub(super) type_key: Option<String>,
    #[serde(rename = "parentTypeId")]
    pub(super) parent_type_id: Option<u32>,
    #[serde(rename = "typeId")]
    pub(super) type_id: Option<u32>,
    /// The broader family (`cycling` for `road_biking`); list rows fall back to it
    /// when `typeKey` is absent.
    #[serde(rename = "parentTypeKey")]
    pub(super) parent_type_key: Option<String>,
}

/// One row of `activitylist-service/activities/search/activities`.
///
/// List rows are FLAT: the type is `activityType` (not `activityTypeDTO`), the
/// metrics sit at the top level (not under `summaryDTO`), and the start time is
/// `startTimeGMT`. The key set is the one dravr-sciotte's `providers/garmin.toml`
/// list extractor reads against live Garmin Connect traffic, so a key listed here
/// is one a real response was seen to carry — except the two power figures
/// below, whose names come from public Garmin Connect clients.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct GarminListRow {
    pub(super) activity_id: u64,
    pub(super) activity_name: Option<String>,
    pub(super) activity_type: GarminActivityType,
    #[serde(rename = "startTimeGMT")]
    pub(super) start_time_gmt: Option<String>,
    pub(super) distance: Option<f64>,
    pub(super) duration: Option<f64>,
    pub(super) elevation_gain: Option<f64>,
    pub(super) average_speed: Option<f64>,
    pub(super) max_speed: Option<f64>,
    #[serde(rename = "averageHR")]
    pub(super) average_hr: Option<f32>,
    #[serde(rename = "maxHR")]
    pub(super) max_hr: Option<f32>,
    pub(super) calories: Option<f64>,
    pub(super) average_running_cadence_in_steps_per_minute: Option<f32>,
    pub(super) average_biking_cadence_in_rev_per_minute: Option<f32>,
    /// Garmin has sent the average power under both names.
    pub(super) avg_power: Option<f32>,
    pub(super) average_power: Option<f32>,
    pub(super) max_power: Option<f32>,
    /// List rows abbreviate it `normPower`; the detail summary says
    /// `normalizedPower`.
    pub(super) norm_power: Option<f32>,
}

/// Garmin's `summaryDTO` — top-level summary fields nested inside the
/// detail response. Numeric durations are seconds (f64); HR/cadence values
/// are f32 because Garmin's API emits floating-point HR for partial-second
/// averages.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct GarminSummary {
    pub(super) distance: Option<f64>,
    pub(super) duration: Option<f64>,
    pub(super) moving_duration: Option<f64>,
    pub(super) elapsed_duration: Option<f64>,
    pub(super) elevation_gain: Option<f64>,
    pub(super) elevation_loss: Option<f64>,
    pub(super) average_speed: Option<f64>,
    pub(super) max_speed: Option<f64>,
    pub(super) average_moving_speed: Option<f64>,
    #[serde(rename = "averageHR")]
    pub(super) average_hr: Option<f32>,
    #[serde(rename = "maxHR")]
    pub(super) max_hr: Option<f32>,
    #[serde(rename = "minHR")]
    pub(super) min_hr: Option<f32>,
    pub(super) average_run_cadence: Option<f32>,
    pub(super) average_biking_cadence: Option<f32>,
    pub(super) average_power: Option<f32>,
    pub(super) max_power: Option<f32>,
    /// LIMITATION(registre#105): the power key names (`normalizedPower` / `maxPower` here, `normPower` / `maxPower` on list rows) come from public Garmin Connect clients, not a live capture: no power-meter ride was available to record.
    ///
    /// The clients agree: python-garminconnect's `Activity` (list), go-garmin's
    /// `ActivitySummaryDTO` (detail), connect-personal-api's
    /// `SummarizedActivityDTO` / `SummaryDTO`, and `ConnectStats`.
    pub(super) normalized_power: Option<f32>,
    pub(super) calories: Option<f64>,
    pub(super) bmr_calories: Option<f64>,
    pub(super) average_temperature: Option<f64>,
    pub(super) max_temperature: Option<f64>,
    pub(super) min_temperature: Option<f64>,
    pub(super) avg_elevation: Option<f64>,
    pub(super) min_elevation: Option<f64>,
    pub(super) max_elevation: Option<f64>,
    pub(super) start_latitude: Option<f64>,
    pub(super) start_longitude: Option<f64>,
    pub(super) end_latitude: Option<f64>,
    pub(super) end_longitude: Option<f64>,
    #[serde(rename = "startTimeGMT")]
    pub(super) start_time_gmt: Option<String>,
    pub(super) start_time_local: Option<String>,
    pub(super) training_effect: Option<f32>,
    pub(super) anaerobic_training_effect: Option<f32>,
    pub(super) activity_training_load: Option<f32>,
}

/// One lap from Garmin's `/activity-service/activity/{id}/laps` endpoint.
/// Field names are camelCase in the source JSON; `serde(rename_all)` maps
/// them. All metric fields are `Option` because Garmin omits them when the
/// recording device didn't capture the corresponding sensor.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct GarminLap {
    pub(super) lap_index: Option<u32>,
    pub(super) message_index: Option<u32>,
    pub(super) start_time_gmt: Option<String>,
    pub(super) distance: Option<f64>,
    pub(super) duration: Option<f64>,
    pub(super) elapsed_duration: Option<f64>,
    pub(super) moving_duration: Option<f64>,
    pub(super) elevation_gain: Option<f64>,
    pub(super) elevation_loss: Option<f64>,
    pub(super) max_elevation: Option<f64>,
    pub(super) min_elevation: Option<f64>,
    pub(super) average_speed: Option<f64>,
    pub(super) max_speed: Option<f64>,
    pub(super) average_moving_speed: Option<f64>,
    pub(super) max_vertical_speed: Option<f64>,
    #[serde(rename = "averageHR")]
    pub(super) average_hr: Option<f32>,
    #[serde(rename = "maxHR")]
    pub(super) max_hr: Option<f32>,
    pub(super) average_running_cadence_in_steps_per_minute: Option<f32>,
    pub(super) average_biking_cadence_in_rev_per_minute: Option<f32>,
    pub(super) average_power: Option<f32>,
    pub(super) max_power: Option<f32>,
    pub(super) calories: Option<f64>,
    pub(super) bmr_calories: Option<f64>,
    pub(super) average_temperature: Option<f64>,
    pub(super) max_temperature: Option<f64>,
    pub(super) min_temperature: Option<f64>,
    pub(super) start_latitude: Option<f64>,
    pub(super) start_longitude: Option<f64>,
    pub(super) end_latitude: Option<f64>,
    pub(super) end_longitude: Option<f64>,
}

/// Wrapper for the `/activity/{id}/laps` endpoint response.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct GarminLapsResponse {
    pub(super) activity_id: Option<u64>,
    #[serde(rename = "lapDTOs")]
    pub(super) lap_dtos: Vec<GarminLap>,
}

/// One split from `/activity/{id}/splits` (typically per-km or per-mile slices).
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct GarminSplit {
    pub(super) message_index: Option<u32>,
    pub(super) start_time_gmt: Option<String>,
    pub(super) distance: Option<f64>,
    pub(super) duration: Option<f64>,
    pub(super) moving_duration: Option<f64>,
    pub(super) elevation_gain: Option<f64>,
    pub(super) elevation_loss: Option<f64>,
    pub(super) average_speed: Option<f64>,
    pub(super) max_speed: Option<f64>,
    #[serde(rename = "averageHR")]
    pub(super) average_hr: Option<f32>,
    #[serde(rename = "maxHR")]
    pub(super) max_hr: Option<f32>,
    pub(super) average_power: Option<f32>,
    pub(super) calories: Option<f64>,
    pub(super) start_latitude: Option<f64>,
    pub(super) start_longitude: Option<f64>,
}

/// Wrapper for the `/activity/{id}/splits` endpoint response. Garmin returns
/// a top-level `lapDTOs` array (sic — they reuse the field name) for splits.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct GarminSplitsResponse {
    #[serde(rename = "lapDTOs")]
    pub(super) splits: Vec<GarminSplit>,
}

/// Top-level Garmin API response for `/activity-service/activity/{id}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GarminActivityResponse {
    #[serde(rename = "activityId")]
    pub(super) activity_id: u64,
    #[serde(default, rename = "activityName")]
    pub(super) activity_name: Option<String>,
    #[serde(default, rename = "activityTypeDTO")]
    pub(super) activity_type_dto: GarminActivityType,
    #[serde(default, rename = "summaryDTO")]
    pub(super) summary_dto: GarminSummary,
}

/// Garmin API response for summary stats
#[derive(Debug, Deserialize)]
pub(super) struct GarminStatsResponse {
    #[serde(rename = "totalActivities")]
    pub(super) activities_count: Option<u64>,
    #[serde(rename = "totalDistance")]
    pub(super) distance: Option<f64>,
    #[serde(rename = "totalDuration")]
    pub(super) duration: Option<f64>,
    #[serde(rename = "totalElevationGain")]
    pub(super) elevation_gain: Option<f64>,
}
