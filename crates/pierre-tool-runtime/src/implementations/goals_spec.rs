// ABOUTME: What a fitness goal is to the goal tools: the types, timeframes and sport they measure
// ABOUTME: Validates set_goal's arguments on the way in and reads a stored goal back out for tracking
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The vocabulary `set_goal` writes and `track_progress` reads.
//!
//! A goal is stored as JSON. Keeping both sides' vocabulary here means a goal
//! the tracker cannot measure is refused where it is written instead of being
//! saved and reported as `unknown` on every later reading.

use chrono::{DateTime, FixedOffset};
use serde_json::{from_value, Map, Value};

use pierre_core::errors::{AppError, AppResult, JsonResultExt};
use pierre_core::models::{resolve_sport_type, sport_matches_family, Activity, SportType};
use pierre_mcp_schema::json_schemas::SetGoalParams;

/// The goal types the goal tools measure, each with the unit its target is in.
pub(crate) const GOAL_TYPES: [(&str, &str); 3] = [
    ("distance", "km"),
    ("duration", "hours"),
    ("frequency", "activities"),
];

/// The windows a goal is counted over.
pub(crate) const GOAL_TIMEFRAMES: [&str; 4] = ["week", "month", "quarter", "year"];

/// The window a timeframe names, read case-insensitively and accepting the
/// adjective ("monthly") a model or an athlete says as readily as the noun.
fn canonical_timeframe(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "week" | "weekly" => Some("week"),
        "month" | "monthly" => Some("month"),
        "quarter" | "quarterly" => Some("quarter"),
        "year" | "yearly" | "annual" => Some("year"),
        _ => None,
    }
}

/// `goal_type: 'a' (unit), ...` for descriptions and refusals.
pub(crate) fn goal_type_vocabulary() -> String {
    GOAL_TYPES
        .iter()
        .map(|(kind, unit)| format!("'{kind}' ({unit})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A `set_goal` call the tracker can measure.
pub(crate) struct NewGoal {
    /// The arguments as given, with the serde defaults applied.
    pub(crate) params: SetGoalParams,
    /// The sport the goal counts, resolved to its canonical name.
    pub(crate) sport: Option<SportType>,
}

impl NewGoal {
    /// The sport as stored and echoed: the canonical `snake_case` name.
    pub(crate) fn sport_name(&self) -> Option<String> {
        self.sport.as_ref().and_then(sport_name)
    }
}

/// Parse and check `set_goal`'s arguments.
///
/// # Errors
///
/// Refuses a goal type or timeframe outside the vocabularies above, a target
/// that is not a positive number, and a sport name that resolves to no sport:
/// each would be saved and then never measured.
pub(crate) fn validated_new_goal(args: &Value) -> AppResult<NewGoal> {
    let mut params: SetGoalParams = from_value(args.clone())
        .json_context("set_goal parameters")
        .map_err(|e| AppError::invalid_input(e.to_string()))?;

    params.goal_type = params.goal_type.trim().to_ascii_lowercase();
    if !GOAL_TYPES.iter().any(|(kind, _)| *kind == params.goal_type) {
        return Err(AppError::invalid_input(format!(
            "goal_type '{}' is not one progress can be measured for; use one of {}",
            params.goal_type,
            goal_type_vocabulary()
        )));
    }
    let Some(timeframe) = canonical_timeframe(&params.timeframe) else {
        return Err(AppError::invalid_input(format!(
            "timeframe '{}' is not a goal window; use one of {}",
            params.timeframe,
            GOAL_TIMEFRAMES.join(", ")
        )));
    };
    timeframe.clone_into(&mut params.timeframe);
    if !(params.target_value.is_finite() && params.target_value > 0.0) {
        return Err(AppError::invalid_input(format!(
            "target_value must be a positive number, got {}",
            params.target_value
        )));
    }
    let sport = match params.sport.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => Some(resolve_sport_type(raw).ok_or_else(|| {
            AppError::invalid_input(format!(
                "sport '{raw}' is not a sport name; name one such as run, ride or swim, \
                 or omit it to count every activity"
            ))
        })?),
    };
    Ok(NewGoal { params, sport })
}

/// The canonical `snake_case` name of a sport, as the goal stores it.
fn sport_name(sport: &SportType) -> Option<String> {
    serde_json::to_value(sport)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
}

/// Goal details extracted from storage.
pub(crate) struct GoalDetails {
    pub(crate) goal_type: String,
    pub(crate) goal_target: f64,
    pub(crate) timeframe: String,
    pub(crate) created_at: Option<DateTime<FixedOffset>>,
    /// The sport the goal counts; `None` counts every activity.
    pub(crate) sport: Option<SportType>,
}

impl GoalDetails {
    /// The sport as the tracker echoes it.
    pub(crate) fn sport_name(&self) -> Option<String> {
        self.sport.as_ref().and_then(sport_name)
    }
}

/// Read a stored goal back; `None` when it carries no numeric target.
///
/// A goal written before the timeframe was defaulted reads as monthly, and one
/// written before a sport could be named counts every activity.
pub(crate) fn extract_goal_details(goal: &Map<String, Value>) -> Option<GoalDetails> {
    let goal_type = goal
        .get("goal_type")
        .and_then(Value::as_str)
        .unwrap_or("distance")
        .to_owned();
    let goal_target = goal.get("target_value").and_then(Value::as_f64)?;
    let timeframe = goal
        .get("timeframe")
        .and_then(Value::as_str)
        .unwrap_or("month")
        .to_owned();
    let created_at = goal
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok());
    let sport = goal.get("sport").and_then(Value::as_str).and_then(|s| {
        from_value::<SportType>(Value::String(s.to_owned()))
            .ok()
            .or_else(|| resolve_sport_type(s))
    });

    Some(GoalDetails {
        goal_type,
        goal_target,
        timeframe,
        created_at,
        sport,
    })
}

/// The activities that count toward a goal.
///
/// Those started after it was set, and of its sport's family when it names
/// one, so trail and treadmill runs count toward a running goal and a ride
/// never does.
#[must_use]
pub fn activities_toward_goal<'a>(
    activities: &'a [Activity],
    created_at: Option<DateTime<FixedOffset>>,
    sport: Option<&SportType>,
) -> Vec<&'a Activity> {
    activities
        .iter()
        .filter(|a| created_at.is_none_or(|created| a.start_date() > created))
        .filter(|a| sport.is_none_or(|goal_sport| sport_matches_family(a.sport_type(), goal_sport)))
        .collect()
}
