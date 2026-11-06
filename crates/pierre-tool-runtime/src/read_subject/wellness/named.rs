// ABOUTME: A coach's read of an athlete's stored wellness, cut down to what their wellness grant names
// ABOUTME: Each stored record's metrics mapped onto GrantNamed; a metric no entry names, or a record left empty, is left out

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # What the wellness grant names
//!
//! An athlete grants their coach `coach_wellness_read` against a consent
//! that names what it shares: their sleep, HRV, resting heart rate, weight,
//! readiness and daily notes (`delegation.wellnessApps*`, five locales).
//! Their sources record more than that, and none of the rest was shared, so
//! a coach's read shows what [`GrantNamed`] lists and nothing else of their
//! health (Phil, 2026-11-06). The athlete's own read is unchanged.
//!
//! Each stored record maps its metrics onto the list in one place
//! ([`GrantedOnly`]): a metric no entry names is left out, and so is a
//! record left with none. A field added to one of these records upstream
//! stops the build until it is mapped here, so a new metric never reaches a
//! coach by default.
//!
//! The mapping runs on each stored row before the rows merge across sources
//! ([`readable_wellness`]): a day's primary is then picked among rows that
//! carry something named, so the merged record's id, source and time never
//! belong to a source that recorded only what the grant leaves out.

use pierre_core::models::{
    StoredDailyActivity, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
};
use pierre_services::stored_health::{self, RowProjection};
use pierre_tools_core::ToolResult;
use serde_json::json;

use super::{wellness_data, GrantNamed};
use crate::context::scoped_read_subject;
use crate::read_subject::READ_SUBJECT_ERROR_CODE;

/// The `reason` of a refusal of a read of what the grant does not name.
const UNNAMED_REASON: &str = "wellness_not_named";

/// What the athlete's sources may record that the grant does not name, as
/// a coach's read says it is left out.
const NOT_NAMED: &str = "blood pressure, glucose, body composition, blood oxygen, breathing \
     rate, skin temperature, stress, daily activity totals and intraday heart rate";

/// `value`, a metric the grant names as `_entry`: kept on a coach's read.
///
/// The entry is read by the type checker alone: it ties each kept field to
/// [`GrantNamed`], so an entry taken off the list stops the build until the
/// fields it kept are left out here too.
const fn named<T>(_entry: GrantNamed, value: Option<T>) -> Option<T> {
    value
}

/// A stored wellness record a coach's read shows only the named part of.
pub(crate) trait GrantedOnly: Sized {
    /// The record with only the metrics [`GrantNamed`] lists; `None` when it
    /// carries none of them.
    fn granted_only(self) -> Option<Self>;
}

impl GrantedOnly for StoredHealthMetrics {
    fn granted_only(self) -> Option<Self> {
        let Self {
            id,
            user_id,
            data_source_id,
            date,
            weight_kg,
            // Body composition, blood pressure, glucose and the watch's VO2max
            // estimate: none is named.
            body_fat_pct: _,
            muscle_mass_kg: _,
            bmi: _,
            bone_mass_kg: _,
            water_pct: _,
            systolic_bp: _,
            diastolic_bp: _,
            blood_glucose: _,
            vo2_max: _,
            source_name,
            recorded_at,
        } = self;
        let weight_kg = named(GrantNamed::Weight, weight_kg);
        weight_kg.is_some().then_some(Self {
            id,
            user_id,
            data_source_id,
            date,
            weight_kg,
            body_fat_pct: None,
            muscle_mass_kg: None,
            bmi: None,
            bone_mass_kg: None,
            water_pct: None,
            systolic_bp: None,
            diastolic_bp: None,
            blood_glucose: None,
            vo2_max: None,
            source_name,
            recorded_at,
        })
    }
}

impl GrantedOnly for StoredRecoveryMetrics {
    fn granted_only(self) -> Option<Self> {
        let Self {
            id,
            user_id,
            data_source_id,
            date,
            recovery_score,
            readiness_score,
            hrv_ms,
            hrv_rmssd,
            resting_heart_rate,
            // The source's stress and energy scores, blood oxygen, breathing
            // rate, skin temperature and day strain: none is named.
            stress_score: _,
            body_battery: _,
            spo2: _,
            respiratory_rate: _,
            skin_temp_deviation: _,
            daily_strain: _,
            athlete_note,
            source_name,
            recorded_at,
        } = self;
        let kept = Self {
            id,
            user_id,
            data_source_id,
            date,
            recovery_score: named(GrantNamed::Readiness, recovery_score),
            readiness_score: named(GrantNamed::Readiness, readiness_score),
            hrv_ms: named(GrantNamed::Hrv, hrv_ms),
            hrv_rmssd: named(GrantNamed::Hrv, hrv_rmssd),
            resting_heart_rate: named(GrantNamed::RestingHeartRate, resting_heart_rate),
            stress_score: None,
            body_battery: None,
            spo2: None,
            respiratory_rate: None,
            skin_temp_deviation: None,
            daily_strain: None,
            athlete_note: named(GrantNamed::DailyNotes, athlete_note),
            source_name,
            recorded_at,
        };
        let carries_any = kept.recovery_score.is_some()
            || kept.readiness_score.is_some()
            || kept.daily_hrv().is_some()
            || kept.resting_heart_rate.is_some()
            || kept.athlete_note.is_some();
        carries_any.then_some(kept)
    }
}

impl GrantedOnly for StoredSleepSession {
    fn granted_only(self) -> Option<Self> {
        let Self {
            id,
            user_id,
            data_source_id,
            is_nap,
            start_datetime,
            end_datetime,
            total_sleep_seconds,
            deep_sleep_seconds,
            light_sleep_seconds,
            rem_sleep_seconds,
            awake_seconds,
            sleep_efficiency,
            // The night's average heart rate and blood oxygen: vitals, neither
            // named.
            avg_heart_rate: _,
            min_heart_rate,
            avg_hrv,
            sleep_score,
            avg_spo2: _,
            stages,
            source_name,
        } = self;
        // The sleep itself is named, so a night always stays.
        Some(Self {
            id,
            user_id,
            data_source_id,
            is_nap,
            start_datetime,
            end_datetime,
            total_sleep_seconds: named(GrantNamed::Sleep, total_sleep_seconds),
            deep_sleep_seconds: named(GrantNamed::Sleep, deep_sleep_seconds),
            light_sleep_seconds: named(GrantNamed::Sleep, light_sleep_seconds),
            rem_sleep_seconds: named(GrantNamed::Sleep, rem_sleep_seconds),
            awake_seconds: named(GrantNamed::Sleep, awake_seconds),
            sleep_efficiency: named(GrantNamed::Sleep, sleep_efficiency),
            avg_heart_rate: None,
            // The night's lowest heart rate is the resting rate it reads.
            min_heart_rate: named(GrantNamed::RestingHeartRate, min_heart_rate),
            avg_hrv: named(GrantNamed::Hrv, avg_hrv),
            sleep_score: named(GrantNamed::Sleep, sleep_score),
            avg_spo2: None,
            stages,
            source_name,
        })
    }
}

impl GrantedOnly for StoredDailyActivity {
    /// A day's steps, energy and intensity minutes: the grant names none of
    /// a day's activity totals.
    fn granted_only(self) -> Option<Self> {
        None
    }
}

/// What the in-flight read may show of each of the athlete's stored rows,
/// for the merge to run on: each row whole on a caller's own read; on a
/// coach's read, each with only what the grant names ([`GrantNamed`]), a row
/// left with none of it dropped before it can stand for its day.
pub(crate) fn readable_wellness<T: GrantedOnly>() -> RowProjection<T> {
    if scoped_read_subject().is_none() {
        stored_health::as_stored
    } else {
        T::granted_only
    }
}

/// What a coach's read of `athlete`'s stored wellness says of the rest of
/// their health: what the grant names, and that nothing else is read here,
/// never whether anything else exists.
fn not_named_note(athlete: &str) -> String {
    format!(
        "{athlete} shares their {} with you, and nothing else of their health: their \
         {NOT_NAMED} are not read here, whether or not their sources record them. Say so rather \
         than estimating them.",
        wellness_data()
    )
}

/// What the in-flight read says of the athlete's health the grant does not
/// name: the note on a coach's read, `None` on the caller's own.
pub(crate) fn unnamed_wellness_note() -> Option<String> {
    scoped_read_subject().map(|subject| not_named_note(&subject.access.display_name))
}

/// The refusal, on a coach's read, of `tool_name` asked for `asked` (an
/// argument and its value, as the call sent them), which reads `what`, a
/// part of the athlete's health the grant does not name; `instead` says what
/// to ask for. In the shape of every read-subject refusal, as a tool body
/// answers it. `None` on the caller's own read.
pub(crate) fn unnamed_refusal(
    tool_name: &str,
    asked: &str,
    what: &str,
    instead: &str,
) -> Option<ToolResult> {
    let subject = scoped_read_subject()?;
    let note = not_named_note(&subject.access.display_name);
    Some(ToolResult::error(json!({
        "error_code": READ_SUBJECT_ERROR_CODE,
        "reason": UNNAMED_REASON,
        "error": format!("'{tool_name}' with {asked} reads the athlete's {what}. {note} {instead}"),
    })))
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Utc};

    use super::*;

    #[test]
    fn the_note_names_what_is_shared_and_never_that_anything_is_missing() {
        let note = not_named_note("Mira").to_lowercase();
        assert!(
            note.starts_with("mira shares their sleep, hrv")
                && note.contains("blood pressure")
                && note.contains("daily activity totals"),
            "bug: the note says what the grant names and what it does not: {note}"
        );
        for missing in ["no data", "not saved", "missing", "none was"] {
            assert!(
                !note.contains(missing),
                "bug: what is not shared never reads as absent ({missing}): {note}"
            );
        }
    }

    fn body() -> StoredHealthMetrics {
        StoredHealthMetrics {
            id: "day".to_owned(),
            user_id: "athlete".to_owned(),
            data_source_id: "ds".to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
            weight_kg: Some(61.5),
            body_fat_pct: Some(17.0),
            muscle_mass_kg: Some(48.0),
            bmi: Some(21.1),
            bone_mass_kg: Some(2.6),
            water_pct: Some(58.0),
            systolic_bp: Some(118),
            diastolic_bp: Some(76),
            blood_glucose: Some(92.0),
            vo2_max: Some(55.0),
            source_name: "withings".to_owned(),
            recorded_at: Utc::now(),
        }
    }

    #[test]
    fn a_body_day_keeps_its_weight_alone() {
        let kept = body().granted_only().unwrap();
        let shown = serde_json::to_value(&kept).unwrap();
        assert_eq!(shown["weight_kg"], 61.5, "weight is named: {shown}");
        for unnamed in [
            "body_fat_pct",
            "muscle_mass_kg",
            "bmi",
            "bone_mass_kg",
            "water_pct",
            "systolic_bp",
            "diastolic_bp",
            "blood_glucose",
            "vo2_max",
        ] {
            assert!(
                shown[unnamed].is_null(),
                "bug: {unnamed} is not named: {shown}"
            );
        }
        let no_weight = StoredHealthMetrics {
            weight_kg: None,
            ..body()
        };
        assert!(
            no_weight.granted_only().is_none(),
            "bug: a day with only unnamed metrics is left out, not shown empty"
        );
    }

    #[test]
    fn a_recovery_day_keeps_readiness_hrv_resting_rate_and_the_note() {
        let day = StoredRecoveryMetrics {
            id: "day".to_owned(),
            user_id: "athlete".to_owned(),
            data_source_id: "ds".to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
            recovery_score: Some(71),
            readiness_score: Some(68),
            hrv_ms: None,
            hrv_rmssd: Some(68.4),
            resting_heart_rate: Some(46),
            stress_score: Some(22),
            body_battery: Some(80),
            spo2: Some(97.0),
            respiratory_rate: Some(14.5),
            skin_temp_deviation: Some(0.3),
            daily_strain: Some(12.0),
            athlete_note: Some("legs heavy".to_owned()),
            source_name: "garmin".to_owned(),
            recorded_at: Utc::now(),
        };
        let shown = serde_json::to_value(day.clone().granted_only().unwrap()).unwrap();
        for named in [
            "recovery_score",
            "readiness_score",
            "hrv_rmssd",
            "resting_heart_rate",
            "athlete_note",
        ] {
            assert!(!shown[named].is_null(), "bug: {named} is named: {shown}");
        }
        for unnamed in [
            "stress_score",
            "body_battery",
            "spo2",
            "respiratory_rate",
            "skin_temp_deviation",
            "daily_strain",
        ] {
            assert!(
                shown[unnamed].is_null(),
                "bug: {unnamed} is not named: {shown}"
            );
        }
        let unnamed_only = StoredRecoveryMetrics {
            recovery_score: None,
            readiness_score: None,
            hrv_rmssd: None,
            resting_heart_rate: None,
            athlete_note: None,
            ..day
        };
        assert!(
            unnamed_only.granted_only().is_none(),
            "bug: a day with only unnamed metrics is left out"
        );
    }

    #[test]
    fn a_night_keeps_its_sleep_hrv_and_lowest_heart_rate() {
        let night = StoredSleepSession {
            id: "night".to_owned(),
            user_id: "athlete".to_owned(),
            data_source_id: "ds".to_owned(),
            is_nap: false,
            start_datetime: Utc::now(),
            end_datetime: Utc::now(),
            total_sleep_seconds: Some(25_200),
            deep_sleep_seconds: Some(5_400),
            light_sleep_seconds: Some(12_600),
            rem_sleep_seconds: Some(7_200),
            awake_seconds: Some(900),
            sleep_efficiency: Some(93.0),
            avg_heart_rate: Some(52.5),
            min_heart_rate: Some(46),
            avg_hrv: Some(71.0),
            sleep_score: Some(84),
            avg_spo2: Some(96.0),
            stages: Vec::new(),
            source_name: "garmin".to_owned(),
        };
        let shown = serde_json::to_value(night.granted_only().unwrap()).unwrap();
        assert_eq!(shown["total_sleep_seconds"], 25_200, "{shown}");
        assert_eq!(shown["min_heart_rate"], 46, "{shown}");
        assert_eq!(shown["avg_hrv"], 71.0, "{shown}");
        assert!(
            shown["avg_heart_rate"].is_null() && shown["avg_spo2"].is_null(),
            "bug: the night's average heart rate and blood oxygen are not named: {shown}"
        );
    }

    #[test]
    fn a_day_of_activity_totals_is_never_named() {
        let day = StoredDailyActivity {
            id: "day".to_owned(),
            user_id: "athlete".to_owned(),
            data_source_id: "ds".to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
            steps: Some(12_000),
            active_kcal: Some(640.0),
            total_kcal: Some(2_300.0),
            intensity_minutes: Some(55),
            source_name: "garmin".to_owned(),
            recorded_at: Utc::now(),
        };
        assert!(day.granted_only().is_none(), "bug: steps are not named");
    }

    #[test]
    fn a_callers_own_read_keeps_each_row_whole() {
        let kept = readable_wellness::<StoredHealthMetrics>()(body()).unwrap();
        assert_eq!(
            (kept.body_fat_pct, kept.systolic_bp, kept.weight_kg),
            (Some(17.0), Some(118), Some(61.5)),
            "bug: outside a coach's read no row is cut down before the merge"
        );
    }

    #[test]
    fn a_callers_own_read_gets_no_note() {
        assert!(
            unnamed_wellness_note().is_none(),
            "bug: outside a coach's read nothing is said to be left out"
        );
    }
}
