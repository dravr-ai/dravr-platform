// ABOUTME: The shapes of the records Garmin pushes, as much of each as Dravr reads, every field optional
// ABOUTME: Unknown fields are ignored and ids are read whether Garmin sends them as numbers or as text

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin push records.
//!
//! Every push body is one JSON object whose single key is the summary type and
//! whose value is the list of records. Each record names the Garmin user it
//! belongs to (`userId`); the rest is the summary. Every field is optional:
//! Garmin leaves out what a device did not record, and a missing field reads
//! as "not measured", never as zero. Fields Dravr does not read are ignored.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// An id Garmin sends as a JSON number in some summaries and as text in
/// others, read as its decimal text either way.
fn lenient_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(deserializer)? {
        Some(Value::String(id)) if !id.trim().is_empty() => Some(id.trim().to_owned()),
        Some(Value::Number(id)) => Some(id.to_string()),
        _ => None,
    })
}

/// A map from a time offset in seconds (as Garmin's text keys) to a reading.
pub(crate) type OffsetSeries = BTreeMap<String, f64>;

/// The fields every activity summary carries, in `activities` and as the
/// `summary` of an `activityDetails` record.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivitySummary {
    /// Garmin Connect's id for the activity: the key every copy of it shares.
    #[serde(default, deserialize_with = "lenient_id")]
    pub(crate) activity_id: Option<String>,
    /// The activity's name in Garmin Connect.
    #[serde(default)]
    pub(crate) activity_name: Option<String>,
    /// Garmin's activity type (`RUNNING`, `ROAD_BIKING`, …).
    #[serde(default)]
    pub(crate) activity_type: Option<String>,
    /// Start, unix seconds UTC.
    #[serde(default)]
    pub(crate) start_time_in_seconds: Option<i64>,
    /// Length of the activity, seconds.
    #[serde(default)]
    pub(crate) duration_in_seconds: Option<i64>,
    /// Average heart rate, bpm.
    #[serde(default)]
    pub(crate) average_heart_rate_in_beats_per_minute: Option<f64>,
    /// Maximum heart rate, bpm.
    #[serde(default)]
    pub(crate) max_heart_rate_in_beats_per_minute: Option<f64>,
    /// Average speed, m/s.
    #[serde(default)]
    pub(crate) average_speed_in_meters_per_second: Option<f64>,
    /// Maximum speed, m/s.
    #[serde(default)]
    pub(crate) max_speed_in_meters_per_second: Option<f64>,
    /// Average bike cadence, rpm.
    #[serde(default)]
    pub(crate) average_bike_cadence_in_rounds_per_minute: Option<f64>,
    /// Maximum bike cadence, rpm.
    #[serde(default)]
    pub(crate) max_bike_cadence_in_rounds_per_minute: Option<f64>,
    /// Average run cadence, steps per minute.
    #[serde(default)]
    pub(crate) average_run_cadence_in_steps_per_minute: Option<f64>,
    /// Maximum run cadence, steps per minute.
    #[serde(default)]
    pub(crate) max_run_cadence_in_steps_per_minute: Option<f64>,
    /// Active kilocalories.
    #[serde(default)]
    pub(crate) active_kilocalories: Option<f64>,
    /// The device that recorded it, named without the Garmin brand.
    #[serde(default)]
    pub(crate) device_name: Option<String>,
    /// Distance, meters.
    #[serde(default)]
    pub(crate) distance_in_meters: Option<f64>,
    /// Steps.
    #[serde(default)]
    pub(crate) steps: Option<f64>,
    /// Elevation gained, meters.
    #[serde(default)]
    pub(crate) total_elevation_gain_in_meters: Option<f64>,
    /// Where it started, degrees.
    #[serde(default)]
    pub(crate) starting_latitude_in_degree: Option<f64>,
    /// Where it started, degrees.
    #[serde(default)]
    pub(crate) starting_longitude_in_degree: Option<f64>,
    /// Whether this is the parent of a multisport activity's legs.
    #[serde(default)]
    pub(crate) is_parent: Option<bool>,
    /// Entered by hand rather than recorded on a device.
    #[serde(default)]
    pub(crate) manual: Option<bool>,
    /// Uploaded through the Garmin Connect web app.
    #[serde(default)]
    pub(crate) is_web_upload: Option<bool>,
}

/// One `activityDetails` record: the summary, its samples and its laps.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityDetailsRecord {
    /// Garmin Connect's id for the activity.
    #[serde(default, deserialize_with = "lenient_id")]
    pub(crate) activity_id: Option<String>,
    /// The activity's summary.
    #[serde(default)]
    pub(crate) summary: ActivitySummary,
    /// Every recorded sample, oldest first.
    #[serde(default)]
    pub(crate) samples: Vec<ActivitySample>,
    /// Each lap, by its start.
    #[serde(default)]
    pub(crate) laps: Vec<ActivityLap>,
}

/// One recorded sample of an activity.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivitySample {
    /// When it was recorded, unix seconds UTC.
    #[serde(default)]
    pub(crate) start_time_in_seconds: Option<i64>,
    /// Position, degrees.
    #[serde(default)]
    pub(crate) latitude_in_degree: Option<f64>,
    /// Position, degrees.
    #[serde(default)]
    pub(crate) longitude_in_degree: Option<f64>,
    /// Elevation, meters.
    #[serde(default)]
    pub(crate) elevation_in_meters: Option<f64>,
    /// Air temperature, Celsius (Garmin's spelling).
    #[serde(default, rename = "airTemperatureCelcius")]
    pub(crate) air_temperature_celsius: Option<f64>,
    /// Heart rate, bpm.
    #[serde(default)]
    pub(crate) heart_rate: Option<f64>,
    /// Speed, m/s.
    #[serde(default)]
    pub(crate) speed_meters_per_second: Option<f64>,
    /// Run cadence, steps per minute.
    #[serde(default)]
    pub(crate) steps_per_minute: Option<f64>,
    /// Distance covered since the start, meters.
    #[serde(default)]
    pub(crate) total_distance_in_meters: Option<f64>,
    /// Power, watts.
    #[serde(default)]
    pub(crate) power_in_watts: Option<f64>,
    /// Bike cadence, rpm.
    #[serde(default, rename = "bikeCadenceInRPM")]
    pub(crate) bike_cadence_in_rpm: Option<f64>,
    /// Swim cadence, strokes per minute.
    #[serde(default)]
    pub(crate) swim_cadence_in_strokes_per_minute: Option<f64>,
}

/// One lap of an activity: Garmin sends its start only.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityLap {
    /// When the lap started, unix seconds UTC.
    #[serde(default)]
    pub(crate) start_time_in_seconds: Option<i64>,
}

/// One `activityFiles` record: where the activity's raw file can be fetched,
/// once.
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityFileRecord {
    /// Garmin Connect's id for the activity.
    #[serde(default, deserialize_with = "lenient_id")]
    pub(crate) activity_id: Option<String>,
    /// `FIT`, `TCX` or `GPX`.
    #[serde(default)]
    pub(crate) file_type: Option<String>,
    /// The one-shot download URL.
    #[serde(default, rename = "callbackURL")]
    pub(crate) callback_url: Option<String>,
    /// Entered by hand rather than recorded on a device.
    #[serde(default)]
    pub(crate) manual: Option<bool>,
}

/// Prints whether a callback URL is present, never the URL: it carries the
/// one-shot download token.
impl fmt::Debug for ActivityFileRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActivityFileRecord")
            .field("activity_id", &self.activity_id)
            .field("file_type", &self.file_type)
            .field(
                "callback_url",
                &self.callback_url.as_ref().map(|_| "[redacted]"),
            )
            .field("manual", &self.manual)
            .finish()
    }
}

/// One `dailies` record: the athlete's day.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DailyRecord {
    /// The day it is shown on in Garmin Connect, `yyyy-mm-dd`.
    #[serde(default)]
    pub(crate) calendar_date: Option<String>,
    /// Start of the day, unix seconds UTC.
    #[serde(default)]
    pub(crate) start_time_in_seconds: Option<i64>,
    /// Resting heart rate, bpm.
    #[serde(default)]
    pub(crate) resting_heart_rate_in_beats_per_minute: Option<f64>,
    /// Average stress, 1–100; negative when not enough data.
    #[serde(default)]
    pub(crate) average_stress_level: Option<f64>,
    /// Heart rate samples by offset from the start, bpm.
    #[serde(default)]
    pub(crate) time_offset_heart_rate_samples: OffsetSeries,
}

/// One time range of a sleep level.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LevelRange {
    /// Start, unix seconds UTC.
    #[serde(default)]
    pub(crate) start_time_in_seconds: Option<i64>,
    /// End, unix seconds UTC.
    #[serde(default)]
    pub(crate) end_time_in_seconds: Option<i64>,
}

/// A sleep's overall score.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct SleepScore {
    /// The score, 0–100.
    #[serde(default)]
    pub(crate) value: Option<f64>,
}

/// One `sleeps` record.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SleepRecord {
    /// Start of the sleep window, unix seconds UTC: the key an update keeps.
    #[serde(default)]
    pub(crate) start_time_in_seconds: Option<i64>,
    /// Time asleep, seconds (awake time excluded).
    #[serde(default)]
    pub(crate) duration_in_seconds: Option<i64>,
    /// Deep sleep, seconds.
    #[serde(default)]
    pub(crate) deep_sleep_duration_in_seconds: Option<i64>,
    /// Light sleep, seconds.
    #[serde(default)]
    pub(crate) light_sleep_duration_in_seconds: Option<i64>,
    /// REM sleep, seconds.
    #[serde(default)]
    pub(crate) rem_sleep_in_seconds: Option<i64>,
    /// Awake during the window, seconds.
    #[serde(default)]
    pub(crate) awake_duration_in_seconds: Option<i64>,
    /// Each level's time ranges (`deep`, `light`, `rem`, `awake`).
    #[serde(default)]
    pub(crate) sleep_levels_map: BTreeMap<String, Vec<LevelRange>>,
    /// The overall sleep score.
    #[serde(default)]
    pub(crate) overall_sleep_score: Option<SleepScore>,
}

/// One `hrv` record: the night's heart-rate variability.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HrvRecord {
    /// The day it is shown on, `yyyy-mm-dd`.
    #[serde(default)]
    pub(crate) calendar_date: Option<String>,
    /// Start of the measurement, unix seconds UTC.
    #[serde(default)]
    pub(crate) start_time_in_seconds: Option<i64>,
    /// The night's average rMSSD, ms.
    #[serde(default)]
    pub(crate) last_night_avg: Option<f64>,
    /// rMSSD readings by offset from the start, ms.
    #[serde(default)]
    pub(crate) hrv_values: OffsetSeries,
}

/// One `userMetrics` record.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserMetricsRecord {
    /// The day it is shown on, `yyyy-mm-dd`.
    #[serde(default)]
    pub(crate) calendar_date: Option<String>,
    /// `VO2max` estimate, ml/kg/min.
    #[serde(default)]
    pub(crate) vo2_max: Option<f64>,
    /// Cycling `VO2max` estimate, ml/kg/min.
    #[serde(default)]
    pub(crate) vo2_max_cycling: Option<f64>,
}

/// One `bodyComps` record.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BodyCompRecord {
    /// When it was measured, unix seconds UTC.
    #[serde(default)]
    pub(crate) measurement_time_in_seconds: Option<i64>,
    /// Offset to the device's local time, seconds.
    #[serde(default)]
    pub(crate) measurement_time_offset_in_seconds: Option<i64>,
    /// Weight, grams.
    #[serde(default)]
    pub(crate) weight_in_grams: Option<f64>,
    /// Body fat, percent.
    #[serde(default)]
    pub(crate) body_fat_in_percent: Option<f64>,
    /// Body mass index.
    #[serde(default)]
    pub(crate) body_mass_index: Option<f64>,
    /// Muscle mass, grams.
    #[serde(default)]
    pub(crate) muscle_mass_in_grams: Option<f64>,
    /// Bone mass, grams.
    #[serde(default)]
    pub(crate) bone_mass_in_grams: Option<f64>,
    /// Body water, percent.
    #[serde(default)]
    pub(crate) body_water_in_percent: Option<f64>,
}

/// One `stressDetails` record.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StressDetailsRecord {
    /// The day it is shown on, `yyyy-mm-dd`.
    #[serde(default)]
    pub(crate) calendar_date: Option<String>,
    /// Body Battery levels by offset.
    #[serde(default)]
    pub(crate) time_offset_body_battery_values: OffsetSeries,
}

/// One `skinTemp` record.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkinTempRecord {
    /// The day it is shown on, `yyyy-mm-dd`.
    #[serde(default)]
    pub(crate) calendar_date: Option<String>,
    /// The night's average deviation from the athlete's baseline, Celsius.
    #[serde(default)]
    pub(crate) avg_deviation_celsius: Option<f64>,
}

/// One `userPermissionsChange` record: the permissions the athlete now grants.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PermissionsRecord {
    /// Every permission the athlete grants now.
    #[serde(default)]
    pub(crate) permissions: Vec<String>,
}
