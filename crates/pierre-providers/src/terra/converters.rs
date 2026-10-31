// ABOUTME: Converters from Terra API models to Pierre unified data models
// ABOUTME: Maps Terra's standardized JSON schemas to Activity, SleepSession, HealthMetrics, etc.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Terra to Pierre model converters
//!
//! This module provides conversion functions from Terra's standardized data models
//! to Pierre's unified fitness data models.

use crate::constants::oauth_providers::TERRA;
use crate::models::{
    Activity, ActivityBuilder, SportType, StoredDailyActivity, StoredHealthMetrics,
    StoredRecoveryMetrics, StoredSleepSession,
};
use chrono::{DateTime, FixedOffset, Utc};
use dravr_equilibre::{SleepStage, SleepStageType};
use uuid::Uuid;

use super::constants::{
    TERRA_ACTIVITY_ALPINE_SKI, TERRA_ACTIVITY_BASKETBALL, TERRA_ACTIVITY_CROSSFIT,
    TERRA_ACTIVITY_CROSS_COUNTRY_SKI, TERRA_ACTIVITY_EBIKE_RIDE, TERRA_ACTIVITY_GOLF,
    TERRA_ACTIVITY_GRAVEL_RIDE, TERRA_ACTIVITY_HIKE, TERRA_ACTIVITY_INDOOR_CYCLING,
    TERRA_ACTIVITY_INDOOR_RUN, TERRA_ACTIVITY_INLINE_SKATING, TERRA_ACTIVITY_KAYAKING,
    TERRA_ACTIVITY_MOUNTAIN_BIKE, TERRA_ACTIVITY_OPEN_WATER_SWIM, TERRA_ACTIVITY_PADDLEBOARD,
    TERRA_ACTIVITY_PILATES, TERRA_ACTIVITY_POOL_SWIM, TERRA_ACTIVITY_RIDE,
    TERRA_ACTIVITY_ROCK_CLIMBING, TERRA_ACTIVITY_ROWING, TERRA_ACTIVITY_RUN,
    TERRA_ACTIVITY_SKATEBOARDING, TERRA_ACTIVITY_SNOWBOARD, TERRA_ACTIVITY_SNOWSHOE,
    TERRA_ACTIVITY_SOCCER, TERRA_ACTIVITY_STRENGTH_TRAINING, TERRA_ACTIVITY_SURFING,
    TERRA_ACTIVITY_SWIM, TERRA_ACTIVITY_TENNIS, TERRA_ACTIVITY_TRAIL_RUN, TERRA_ACTIVITY_TREADMILL,
    TERRA_ACTIVITY_WALK, TERRA_ACTIVITY_YOGA, TERRA_SLEEP_STAGE_AWAKE, TERRA_SLEEP_STAGE_DEEP,
    TERRA_SLEEP_STAGE_REM,
};
use super::models::{
    TerraActivity, TerraBody, TerraDaily, TerraSleep, TerraSleepDurations, TerraSleepStage,
    TerraUser,
};

/// Converter utilities for Terra to Pierre models
#[derive(Debug)]
pub struct TerraConverters;

/// Extracted activity metrics from Terra data
struct ActivityMetrics {
    distance_meters: Option<f64>,
    elevation_gain: Option<f64>,
    steps: Option<u32>,
    calories: Option<u32>,
    average_heart_rate: Option<u32>,
    max_heart_rate: Option<u32>,
    hrv_score: Option<f64>,
    average_speed: Option<f64>,
    max_speed: Option<f64>,
    average_cadence: Option<u32>,
    max_cadence: Option<u32>,
    average_power: Option<u32>,
    max_power: Option<u32>,
    normalized_power: Option<u32>,
    training_stress_score: Option<f32>,
    intensity_factor: Option<f32>,
    ftp: Option<u32>,
    spo2: Option<f32>,
    average_altitude: Option<f32>,
    suffer_score: Option<u32>,
    start_latitude: Option<f64>,
    start_longitude: Option<f64>,
}

impl TerraConverters {
    /// Extract activity metrics from Terra activity data
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn extract_activity_metrics(terra: &TerraActivity) -> ActivityMetrics {
        let distance_data = terra.distance_data.as_ref();
        let hr_data = terra.heart_rate_data.as_ref();
        let movement = terra.movement_data.as_ref();
        let power = terra.power_data.as_ref();
        let tss = terra.tss_data.as_ref();
        let oxygen = terra.oxygen_data.as_ref();
        let position = terra.position_data.as_ref();

        let (start_latitude, start_longitude) = position
            .and_then(|p| p.start_pos_lat_lng_deg)
            .map_or((None, None), |[lat, lng]| (Some(lat), Some(lng)));

        ActivityMetrics {
            distance_meters: distance_data.and_then(|d| d.distance_meters),
            elevation_gain: distance_data.and_then(|d| d.elevation_gain_metres),
            steps: distance_data.and_then(|d| d.steps).map(|s| s as u32),
            calories: terra
                .calories_data
                .as_ref()
                .and_then(|c| c.total_burned_calories)
                .map(|c| c as u32),
            average_heart_rate: hr_data.and_then(|h| h.avg_hr_bpm).map(|h| h as u32),
            max_heart_rate: hr_data.and_then(|h| h.max_hr_bpm).map(|h| h as u32),
            hrv_score: hr_data.and_then(|h| h.avg_hrv_rmssd),
            average_speed: movement.and_then(|m| m.avg_speed_metres_per_second),
            max_speed: movement.and_then(|m| m.max_speed_metres_per_second),
            average_cadence: movement.and_then(|m| m.avg_cadence).map(|c| c as u32),
            max_cadence: movement.and_then(|m| m.max_cadence).map(|c| c as u32),
            average_power: power.and_then(|p| p.avg_watts).map(|w| w as u32),
            max_power: power.and_then(|p| p.max_watts).map(|w| w as u32),
            normalized_power: power.and_then(|p| p.normalized_watts).map(|w| w as u32),
            training_stress_score: tss.and_then(|t| t.tss).map(|t| t as f32),
            intensity_factor: tss.and_then(|t| t.intensity_factor).map(|i| i as f32),
            ftp: tss.and_then(|t| t.ftp).map(|f| f as u32),
            spo2: oxygen
                .and_then(|o| o.avg_saturation_percentage)
                .map(|s| s as f32),
            average_altitude: distance_data
                .and_then(|d| d.avg_elevation_metres)
                .map(|e| e as f32),
            suffer_score: terra
                .strain_data
                .as_ref()
                .and_then(|s| s.strain_level)
                .map(|s| s as u32),
            start_latitude,
            start_longitude,
        }
    }

    /// Convert Terra activity to Pierre `Activity`.
    ///
    /// `None` without a start time or a summary id: a workout is never dated
    /// by the time it happened to arrive, and one with no id of Terra's own
    /// would be stored again on every redelivery of the event.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn activity_from_terra(terra: &TerraActivity, terra_user: &TerraUser) -> Option<Activity> {
        let metadata = terra.metadata.as_ref()?;
        let start_date = metadata.start_time?.with_timezone(&Utc);
        let summary_id = metadata.summary_id.clone()?;
        let end_time = metadata.end_time;

        let duration_seconds = end_time
            .map(|end| {
                (end.with_timezone(&Utc) - start_date)
                    .num_seconds()
                    .unsigned_abs()
            })
            .or_else(|| {
                terra
                    .active_durations_data
                    .as_ref()
                    .and_then(|d| d.activity_seconds)
                    .map(|s| s as u64)
            })
            .unwrap_or(0);

        let metrics = Self::extract_activity_metrics(terra);
        let sport_type = Self::map_terra_activity_type(metadata.activity_type.unwrap_or(0));

        let activity = ActivityBuilder::new(
            summary_id,
            metadata
                .name
                .clone()
                .unwrap_or_else(|| sport_type.display_name().to_owned()),
            sport_type,
            start_date,
            duration_seconds,
            TERRA,
        )
        .source_opt(terra_user.provider.as_ref().map(|p| p.to_lowercase()))
        .distance_meters_opt(metrics.distance_meters)
        .elevation_gain_opt(metrics.elevation_gain)
        .average_heart_rate_opt(metrics.average_heart_rate)
        .max_heart_rate_opt(metrics.max_heart_rate)
        .average_speed_opt(metrics.average_speed)
        .max_speed_opt(metrics.max_speed)
        .calories_opt(metrics.calories)
        .steps_opt(metrics.steps)
        .average_power_opt(metrics.average_power)
        .max_power_opt(metrics.max_power)
        .normalized_power_opt(metrics.normalized_power)
        .ftp_opt(metrics.ftp)
        .average_cadence_opt(metrics.average_cadence)
        .max_cadence_opt(metrics.max_cadence)
        .hrv_score_opt(metrics.hrv_score)
        .average_altitude_opt(metrics.average_altitude)
        .spo2_opt(metrics.spo2)
        .training_stress_score_opt(metrics.training_stress_score)
        .intensity_factor_opt(metrics.intensity_factor)
        .suffer_score_opt(metrics.suffer_score)
        .start_latitude_opt(metrics.start_latitude)
        .start_longitude_opt(metrics.start_longitude)
        .city_opt(metadata.city.clone())
        .country_opt(metadata.country.clone())
        .sport_type_detail_opt(metadata.name.clone())
        .build();
        Some(activity)
    }

    /// The night a Terra sleep summary describes, as the wellness store
    /// keeps nights: unstamped (no user or data source yet, the writer files
    /// it), named `terra`.
    ///
    /// `None` without both a start and an end: a night is never dated by the
    /// time it happened to arrive.
    #[must_use]
    pub fn night_from_terra(terra: &TerraSleep) -> Option<StoredSleepSession> {
        let metadata = terra.metadata.as_ref()?;
        let start = metadata.start_time?.with_timezone(&Utc);
        let end = metadata.end_time?.with_timezone(&Utc);
        let durations = terra.sleep_durations_data.as_ref();
        let seconds = |field: fn(&TerraSleepDurations) -> Option<f64>| {
            durations.and_then(field).and_then(whole_seconds)
        };
        let hr = terra.heart_rate_data.as_ref();
        let total_sleep_seconds = seconds(|d| d.asleep_seconds);
        let sleep_efficiency = durations.and_then(|d| d.sleep_efficiency).or_else(|| {
            let in_bed = durations.and_then(|d| d.in_bed_seconds)?;
            let asleep = durations.and_then(|d| d.asleep_seconds)?;
            (in_bed > 0.0).then(|| asleep / in_bed * 100.0)
        });
        Some(StoredSleepSession {
            id: Uuid::new_v4().to_string(),
            user_id: String::new(),
            data_source_id: String::new(),
            is_nap: false,
            start_datetime: start,
            end_datetime: end,
            total_sleep_seconds,
            deep_sleep_seconds: seconds(|d| d.deep_sleep_seconds),
            light_sleep_seconds: seconds(|d| d.light_sleep_seconds),
            rem_sleep_seconds: seconds(|d| d.rem_sleep_seconds),
            awake_seconds: seconds(|d| d.awake_seconds),
            sleep_efficiency,
            avg_heart_rate: hr.and_then(|h| h.avg_hr_bpm),
            min_heart_rate: hr.and_then(|h| h.min_hr_bpm).and_then(score),
            avg_hrv: hr.and_then(|h| h.avg_hrv_rmssd),
            sleep_score: None,
            avg_spo2: None,
            stages: durations
                .and_then(|d| d.sleep_stages.as_ref())
                .map(|stages| {
                    stages
                        .iter()
                        .filter_map(Self::convert_sleep_stage)
                        .collect()
                })
                .unwrap_or_default(),
            source_name: TERRA.to_owned(),
        })
    }

    /// Convert a Terra sleep stage to a stored night's stage.
    fn convert_sleep_stage(terra_stage: &TerraSleepStage) -> Option<SleepStage> {
        let start = terra_stage.start_time?;
        let end = terra_stage.end_time?;
        Some(SleepStage {
            stage_type: Self::map_terra_sleep_stage(terra_stage.stage.unwrap_or(0)),
            start,
            end,
            duration_seconds: u32::try_from((end - start).num_seconds()).ok()?,
        })
    }

    /// Map Terra sleep stage code to a stored stage type
    ///
    /// Terra sleep stage codes:
    /// - 1: Awake
    /// - 2: Light sleep (default for unknown)
    /// - 3: Deep sleep
    /// - 4: REM sleep
    const fn map_terra_sleep_stage(stage: i32) -> SleepStageType {
        match stage {
            TERRA_SLEEP_STAGE_AWAKE => SleepStageType::Awake,
            TERRA_SLEEP_STAGE_DEEP => SleepStageType::Deep,
            TERRA_SLEEP_STAGE_REM => SleepStageType::Rem,
            // TERRA_SLEEP_STAGE_LIGHT (2) and unknown values default to Light
            _ => SleepStageType::Light,
        }
    }

    /// The recovery readings a Terra sleep summary carries, on the local date
    /// the night ended: resting heart rate, HRV, respiration, the skin
    /// temperature deviation, and the readiness and recovery scores the
    /// relayed service computed.
    ///
    /// `None` without an end time, or when the summary carries none of them.
    #[must_use]
    pub fn night_recovery_from_terra(terra: &TerraSleep) -> Option<StoredRecoveryMetrics> {
        let end = terra.metadata.as_ref()?.end_time?;
        let hr = terra.heart_rate_data.as_ref();
        let readiness = terra.readiness_data.as_ref();
        let day = StoredRecoveryMetrics {
            recovery_score: readiness.and_then(|r| r.recovery_score).and_then(score),
            readiness_score: readiness.and_then(|r| r.readiness_score).and_then(score),
            hrv_rmssd: hr.and_then(|h| h.avg_hrv_rmssd),
            resting_heart_rate: hr.and_then(|h| h.resting_hr_bpm).and_then(score),
            respiratory_rate: terra
                .respiration_data
                .as_ref()
                .and_then(|r| r.avg_breaths_per_minute),
            skin_temp_deviation: terra
                .temperature_data
                .as_ref()
                .and_then(|t| t.delta_temperature_celsius),
            ..empty_recovery(end)
        };
        has_recovery_reading(&day).then_some(day)
    }

    /// The recovery readings a Terra daily summary carries, on its local
    /// date: resting heart rate, HRV, stress and the recovery score.
    ///
    /// `None` without a start time, or when the summary carries none of them.
    #[must_use]
    pub fn day_recovery_from_terra(terra: &TerraDaily) -> Option<StoredRecoveryMetrics> {
        let start = terra.metadata.as_ref()?.start_time?;
        let hr = terra.heart_rate_data.as_ref();
        let day = StoredRecoveryMetrics {
            recovery_score: terra
                .scores
                .as_ref()
                .and_then(|s| s.recovery_score)
                .and_then(score),
            hrv_rmssd: hr.and_then(|h| h.avg_hrv_rmssd),
            resting_heart_rate: hr.and_then(|h| h.resting_hr_bpm).and_then(score),
            stress_score: terra
                .stress_data
                .as_ref()
                .and_then(|s| s.avg_stress_level)
                .and_then(score),
            ..empty_recovery(start)
        };
        has_recovery_reading(&day).then_some(day)
    }

    /// A Terra daily summary's activity totals, on its local date: steps,
    /// active and total calories, and the minutes at moderate or vigorous
    /// intensity.
    ///
    /// `None` without a start time, or when the summary carries none of them.
    #[must_use]
    pub fn day_activity_from_terra(terra: &TerraDaily) -> Option<StoredDailyActivity> {
        let start = terra.metadata.as_ref()?.start_time?;
        let calories = terra.calories_data.as_ref();
        let durations = terra.active_durations_data.as_ref();
        let intensity_minutes = durations.and_then(|d| {
            let moderate = d.moderate_intensity_seconds;
            let vigorous = d.vigorous_intensity_seconds;
            if moderate.is_none() && vigorous.is_none() {
                return None;
            }
            whole_seconds((moderate.unwrap_or(0.0) + vigorous.unwrap_or(0.0)) / 60.0)
        });
        let day = StoredDailyActivity {
            id: Uuid::new_v4().to_string(),
            user_id: String::new(),
            data_source_id: String::new(),
            date: start.date_naive(),
            steps: terra
                .distance_data
                .as_ref()
                .and_then(|d| d.steps)
                .and_then(|s| u32::try_from(s).ok()),
            active_kcal: calories.and_then(|c| c.activity_burned_calories),
            total_kcal: calories.and_then(|c| c.total_burned_calories),
            intensity_minutes,
            source_name: TERRA.to_owned(),
            recorded_at: start.with_timezone(&Utc),
        };
        (day.steps.is_some()
            || day.active_kcal.is_some()
            || day.total_kcal.is_some()
            || day.intensity_minutes.is_some())
        .then_some(day)
    }

    /// A Terra body summary's measurements, on the local date it was taken:
    /// weight, composition, BMI, blood pressure and glucose.
    ///
    /// `None` without a start time, or when the summary carries none of them.
    #[must_use]
    pub fn body_from_terra(terra: &TerraBody) -> Option<StoredHealthMetrics> {
        let start = terra.metadata.as_ref()?.start_time?;
        let m = terra.measurements_data.as_ref()?;
        let (systolic_bp, diastolic_bp) =
            match (m.blood_pressure_systolic, m.blood_pressure_diastolic) {
                (Some(sys), Some(dia)) => (score(sys), score(dia)),
                _ => (None, None),
            };
        let body = StoredHealthMetrics {
            id: Uuid::new_v4().to_string(),
            user_id: String::new(),
            data_source_id: String::new(),
            date: start.date_naive(),
            weight_kg: m.weight_kg,
            body_fat_pct: m.body_fat_percentage,
            muscle_mass_kg: m.muscle_mass_kg,
            bmi: m.bmi,
            bone_mass_kg: m.bone_mass_kg,
            water_pct: m.body_water_percentage,
            systolic_bp,
            diastolic_bp,
            blood_glucose: m.blood_glucose_mg_per_dl,
            vo2_max: None,
            source_name: TERRA.to_owned(),
            recorded_at: start.with_timezone(&Utc),
        };
        (body.weight_kg.is_some()
            || body.body_fat_pct.is_some()
            || body.muscle_mass_kg.is_some()
            || body.bmi.is_some()
            || body.bone_mass_kg.is_some()
            || body.water_pct.is_some()
            || body.systolic_bp.is_some()
            || body.blood_glucose.is_some())
        .then_some(body)
    }

    /// Map Terra activity type code to Pierre `SportType`
    ///
    /// Terra activity types: <https://docs.tryterra.co/reference/activity-types>
    #[must_use]
    pub const fn map_terra_activity_type(activity_type: i32) -> SportType {
        match activity_type {
            // Running variants (1-4)
            // Both outdoor and indoor running map to Run
            TERRA_ACTIVITY_RUN | TERRA_ACTIVITY_INDOOR_RUN => SportType::Run,
            TERRA_ACTIVITY_TRAIL_RUN => SportType::TrailRunning,
            TERRA_ACTIVITY_TREADMILL => SportType::VirtualRun,

            // Cycling variants (5-9)
            TERRA_ACTIVITY_RIDE => SportType::Ride,
            TERRA_ACTIVITY_INDOOR_CYCLING => SportType::VirtualRide,
            TERRA_ACTIVITY_MOUNTAIN_BIKE => SportType::MountainBike,
            TERRA_ACTIVITY_GRAVEL_RIDE => SportType::GravelRide,
            TERRA_ACTIVITY_EBIKE_RIDE => SportType::EbikeRide,

            // Swimming (10-12) - all variants map to Swim
            TERRA_ACTIVITY_SWIM | TERRA_ACTIVITY_POOL_SWIM | TERRA_ACTIVITY_OPEN_WATER_SWIM => {
                SportType::Swim
            }

            // Walking/Hiking (13-14)
            TERRA_ACTIVITY_WALK => SportType::Walk,
            TERRA_ACTIVITY_HIKE => SportType::Hike,

            // Winter sports (15-18)
            TERRA_ACTIVITY_CROSS_COUNTRY_SKI => SportType::CrossCountrySkiing,
            TERRA_ACTIVITY_ALPINE_SKI => SportType::AlpineSkiing,
            TERRA_ACTIVITY_SNOWBOARD => SportType::Snowboarding,
            TERRA_ACTIVITY_SNOWSHOE => SportType::Snowshoe,

            // Water sports (19-22)
            TERRA_ACTIVITY_ROWING => SportType::Rowing,
            TERRA_ACTIVITY_KAYAKING => SportType::Kayaking,
            TERRA_ACTIVITY_PADDLEBOARD => SportType::Paddleboarding,
            TERRA_ACTIVITY_SURFING => SportType::Surfing,

            // Gym/fitness (30-33)
            TERRA_ACTIVITY_STRENGTH_TRAINING => SportType::StrengthTraining,
            TERRA_ACTIVITY_CROSSFIT => SportType::Crossfit,
            TERRA_ACTIVITY_YOGA => SportType::Yoga,
            TERRA_ACTIVITY_PILATES => SportType::Pilates,

            // Team sports (40-43)
            TERRA_ACTIVITY_SOCCER => SportType::Soccer,
            TERRA_ACTIVITY_BASKETBALL => SportType::Basketball,
            TERRA_ACTIVITY_TENNIS => SportType::Tennis,
            TERRA_ACTIVITY_GOLF => SportType::Golf,

            // Other activities (50-52)
            TERRA_ACTIVITY_ROCK_CLIMBING => SportType::RockClimbing,
            TERRA_ACTIVITY_SKATEBOARDING => SportType::Skateboarding,
            TERRA_ACTIVITY_INLINE_SKATING => SportType::InlineSkating,

            // Unknown activity types default to generic Workout
            _ => SportType::Workout,
        }
    }
}

/// A non-negative duration or count as a whole number, rounded; `None` when
/// it is negative, not finite, or past `u32`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole_seconds(value: f64) -> Option<u32> {
    let rounded = value.round();
    (rounded.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&rounded))
        .then_some(rounded as u32)
}

/// A score, a heart rate or a pressure as the whole number the stores keep.
fn score(value: f64) -> Option<u32> {
    whole_seconds(value)
}

/// A recovery day on `at`'s local date, holding no reading yet: unstamped,
/// named `terra`.
fn empty_recovery(at: DateTime<FixedOffset>) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: Uuid::new_v4().to_string(),
        user_id: String::new(),
        data_source_id: String::new(),
        date: at.date_naive(),
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: None,
        resting_heart_rate: None,
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: TERRA.to_owned(),
        recorded_at: at.with_timezone(&Utc),
    }
}

/// Whether a recovery day holds any reading.
const fn has_recovery_reading(day: &StoredRecoveryMetrics) -> bool {
    day.recovery_score.is_some()
        || day.readiness_score.is_some()
        || day.hrv_rmssd.is_some()
        || day.resting_heart_rate.is_some()
        || day.stress_score.is_some()
        || day.respiratory_rate.is_some()
        || day.skin_temp_deviation.is_some()
}
