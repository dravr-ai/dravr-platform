// ABOUTME: Argument parsing for the nutrition tools: biometrics, the day's training and the session
// ABOUTME: Turns calculate_daily_nutrition and get_nutrient_timing arguments into cageux's inputs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_cageux::nutrition_calculator::{
    ActivityLevel, DailyNutritionParams, Gender, PlannedSession, TrainingDay, TrainingGoal,
    WorkoutIntensity,
};
use dravr_cageux::zones::HrIntensity;
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

/// The longest a session, or a day's training, can last: one day, in minutes.
const MAX_MINUTES: f64 = 24.0 * 60.0;

/// The longest gap between eating and a session that a pre-workout plan
/// covers, hours.
const MAX_HOURS_BEFORE: f64 = 24.0;

fn parse_gender(s: &str) -> AppResult<Gender> {
    match s.to_lowercase().as_str() {
        "male" => Ok(Gender::Male),
        "female" => Ok(Gender::Female),
        _ => Err(AppError::invalid_input("Gender must be 'male' or 'female'")),
    }
}

fn parse_activity_level(s: &str) -> AppResult<ActivityLevel> {
    match s.to_lowercase().as_str() {
        "sedentary" => Ok(ActivityLevel::Sedentary),
        "lightly_active" => Ok(ActivityLevel::LightlyActive),
        "moderately_active" => Ok(ActivityLevel::ModeratelyActive),
        "very_active" => Ok(ActivityLevel::VeryActive),
        "extra_active" => Ok(ActivityLevel::ExtraActive),
        _ => Err(AppError::invalid_input(
            "Invalid activity_level. Must be one of: sedentary, lightly_active, \
             moderately_active, very_active, extra_active",
        )),
    }
}

fn parse_training_goal(s: &str) -> AppResult<TrainingGoal> {
    match s.to_lowercase().as_str() {
        "maintenance" => Ok(TrainingGoal::Maintenance),
        "weight_loss" => Ok(TrainingGoal::WeightLoss),
        "muscle_gain" => Ok(TrainingGoal::MuscleGain),
        "endurance_performance" => Ok(TrainingGoal::EndurancePerformance),
        _ => Err(AppError::invalid_input(
            "Invalid training_goal. Must be one of: maintenance, weight_loss, \
             muscle_gain, endurance_performance",
        )),
    }
}

pub(super) fn parse_workout_intensity(s: &str) -> AppResult<WorkoutIntensity> {
    match s.to_lowercase().as_str() {
        "low" => Ok(WorkoutIntensity::Low),
        "moderate" => Ok(WorkoutIntensity::Moderate),
        "high" => Ok(WorkoutIntensity::High),
        _ => Err(AppError::invalid_input(
            "Invalid workout_intensity. Must be one of: low, moderate, high",
        )),
    }
}

/// The session a pre-workout plan is for: `session_minutes` (1-1440) and
/// `hours_before` (0-24), both required.
pub(super) fn parse_planned_session(args: &Value) -> AppResult<PlannedSession> {
    let minutes = args
        .get("session_minutes")
        .and_then(Value::as_f64)
        .filter(|m| (1.0..=MAX_MINUTES).contains(m))
        .ok_or_else(|| {
            AppError::invalid_input(
                "Missing or invalid required parameter: session_minutes (1-1440)",
            )
        })?;
    let hours_before = args
        .get("hours_before")
        .and_then(Value::as_f64)
        .filter(|h| (0.0..=MAX_HOURS_BEFORE).contains(h))
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: hours_before (0-24)")
        })?;
    // Bounded above by the filter: 1440 minutes fits u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let minutes = minutes.round() as u32;
    Ok(PlannedSession {
        minutes,
        hours_before,
    })
}

pub(super) fn parse_nutrition_params(args: &Value) -> AppResult<DailyNutritionParams> {
    let weight_kg = args
        .get("weight_kg")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: weight_kg")
        })?;

    let height_cm = args
        .get("height_cm")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: height_cm")
        })?;

    let age_u64 = args
        .get("age")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::invalid_input("Missing or invalid required parameter: age"))?;

    #[allow(clippy::cast_possible_truncation)]
    let age = if age_u64 <= 150 {
        age_u64 as u32
    } else {
        return Err(AppError::invalid_input(
            "Age must be between 0 and 150 years",
        ));
    };

    let gender_str = args
        .get("gender")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing or invalid required parameter: gender"))?;
    let gender = parse_gender(gender_str)?;

    let activity_level_str = args
        .get("activity_level")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: activity_level")
        })?;
    let activity_level = parse_activity_level(activity_level_str)?;

    let training_goal_str = args
        .get("training_goal")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: training_goal")
        })?;
    let training_goal = parse_training_goal(training_goal_str)?;

    Ok(DailyNutritionParams {
        weight_kg,
        height_cm,
        age,
        gender,
        activity_level,
        training_goal,
        training: parse_training_day(args)?,
    })
}

/// The day's training from `training_minutes` and `training_intensity`, given
/// together; neither, or zero minutes, is a day without training.
fn parse_training_day(args: &Value) -> AppResult<Option<TrainingDay>> {
    let minutes = args.get("training_minutes").and_then(Value::as_f64);
    let intensity = args.get("training_intensity").and_then(Value::as_str);
    match (minutes, intensity) {
        (None, None) => Ok(None),
        (Some(minutes), _) if !(0.0..=MAX_MINUTES).contains(&minutes) => Err(
            AppError::invalid_input("training_minutes must be between 0 and 1440"),
        ),
        (Some(minutes), _) if minutes <= 0.0 => Ok(None),
        (Some(minutes), Some(intensity)) => {
            // Bounded above: 24 h of minutes fits u32 seconds exactly.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let duration_seconds = (minutes * 60.0).round() as u32;
            Ok(Some(TrainingDay {
                duration_seconds,
                intensity: parse_training_intensity(intensity)?,
            }))
        }
        _ => Err(AppError::invalid_input(
            "Give training_minutes and training_intensity together, or neither for a day \
             without training",
        )),
    }
}

/// The day's training intensity on the athlete's heart-rate zones: easy is
/// zones 1-2, moderate zone 3, hard zone 4, very hard zone 5 and above.
fn parse_training_intensity(s: &str) -> AppResult<HrIntensity> {
    match s.to_lowercase().as_str() {
        "easy" => Ok(HrIntensity::Endurance),
        "moderate" => Ok(HrIntensity::Moderate),
        "hard" => Ok(HrIntensity::Hard),
        "very_hard" => Ok(HrIntensity::VeryHard),
        _ => Err(AppError::invalid_input(
            "Invalid training_intensity. Must be one of: easy, moderate, hard, very_hard",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::errors::ErrorCode;
    use serde_json::json;

    /// The message of an argument the parser refused as invalid input.
    fn refusal<T>(parsed: AppResult<T>) -> String {
        match parsed {
            Ok(_) => panic!("the parser accepted an argument it should refuse"),
            Err(error) => {
                assert_eq!(
                    error.code,
                    ErrorCode::InvalidInput,
                    "a bad argument is the caller's input error: {}",
                    error.message
                );
                error.message
            }
        }
    }

    #[test]
    fn a_training_day_carries_its_minutes_as_seconds_and_its_intensity() {
        let day = parse_training_day(&json!({
            "training_minutes": 90, "training_intensity": "hard"
        }))
        .unwrap();
        assert_eq!(
            day,
            Some(TrainingDay {
                duration_seconds: 5_400,
                intensity: HrIntensity::Hard,
            }),
            "90 minutes at threshold reads as 5400 s of zone 4"
        );
        let easy = parse_training_day(&json!({
            "training_minutes": 45, "training_intensity": "easy"
        }))
        .unwrap();
        assert_eq!(
            easy.map(|day| day.intensity),
            Some(HrIntensity::Endurance),
            "easy is the zones 1-2 row"
        );
    }

    #[test]
    fn no_training_or_zero_minutes_is_a_day_without_training() {
        assert_eq!(
            parse_training_day(&json!({})).unwrap(),
            None,
            "neither argument is a rest day"
        );
        assert_eq!(
            parse_training_day(&json!({ "training_minutes": 0 })).unwrap(),
            None,
            "zero minutes is a rest day"
        );
    }

    #[test]
    fn a_training_day_out_of_range_or_half_given_is_refused() {
        for minutes in [-30.0, 1_441.0] {
            let message = refusal(parse_training_day(&json!({
                "training_minutes": minutes, "training_intensity": "hard"
            })));
            assert!(
                message.contains("between 0 and 1440"),
                "{minutes} minutes is outside a day: {message}"
            );
        }
        for half in [
            json!({ "training_minutes": 60 }),
            json!({ "training_intensity": "hard" }),
        ] {
            let message = refusal(parse_training_day(&half));
            assert!(
                message.contains("together"),
                "one of the pair alone is refused: {half} -> {message}"
            );
        }
        let message = refusal(parse_training_day(&json!({
            "training_minutes": 60, "training_intensity": "brutal"
        })));
        assert!(
            message.contains("training_intensity"),
            "an unknown intensity names the argument: {message}"
        );
    }

    #[test]
    fn a_planned_session_reads_its_length_and_gap() {
        assert_eq!(
            parse_planned_session(&json!({ "session_minutes": 90, "hours_before": 2.5 })).unwrap(),
            PlannedSession {
                minutes: 90,
                hours_before: 2.5,
            },
            "the session's length and the athlete's own gap pass through"
        );
    }

    #[test]
    fn a_planned_session_out_of_range_or_incomplete_is_refused() {
        for args in [
            json!({ "hours_before": 2 }),
            json!({ "session_minutes": 0.4, "hours_before": 2 }),
            json!({ "session_minutes": 1_441, "hours_before": 2 }),
        ] {
            let message = refusal(parse_planned_session(&args));
            assert!(
                message.contains("session_minutes"),
                "{args} names session_minutes: {message}"
            );
        }
        for args in [
            json!({ "session_minutes": 90 }),
            json!({ "session_minutes": 90, "hours_before": -1 }),
            json!({ "session_minutes": 90, "hours_before": 25 }),
        ] {
            let message = refusal(parse_planned_session(&args));
            assert!(
                message.contains("hours_before"),
                "{args} names hours_before: {message}"
            );
        }
    }
}
