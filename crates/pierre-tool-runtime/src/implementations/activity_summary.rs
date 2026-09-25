// ABOUTME: ActivitySummary — the scalar per-activity shape the agent model reads
// ABOUTME: Split out of fitness_support.rs so that file stays within its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The agent-facing activity DTO.
//
// `mode=summary` renders a list of these. The shape is the contract the model
// reasons over, so a sensor the provider reports but this struct omits is
// invisible to the agent no matter how faithfully it was fetched.

use pierre_core::json_value::to_value_as_written;
use pierre_core::models::{Activity, Feel, SportType, ZoneDistribution};
use pierre_core::untrusted::{display_line, fence_athlete_text, ACTIVITY_NAME_MAX_CHARS};
use serde::Serialize;
use serde_json::{Map, Value};

/// Longest athlete description `mode=detailed` carries, in characters — and
/// the cap every other provider free-text description is fenced to.
pub(crate) const MAX_DESCRIPTION_CHARS: usize = 600;

/// Longest single comment `mode=detailed` carries, in characters.
const MAX_COMMENT_CHARS: usize = 300;

/// Most comments `mode=detailed` carries per activity — the newest ones.
const MAX_COMMENTS: usize = 10;

/// Longest comment author name, in characters.
const MAX_AUTHOR_CHARS: usize = 60;

/// The serialized [`Activity`] key holding the provider's encoded route
/// overview, which [`detail_json`] never hands the model.
const ROUTE_OVERVIEW_KEY: &str = "summary_polyline";

/// Activity summary with scalar sensor fields for efficient list queries.
///
/// Used when `mode=summary`. Carries the full set of scalar fields every
/// agent persona needs for basic reasoning (HR zones, elevation load,
/// calorie estimate, cadence, power) without the arrays (splits, laps,
/// segments, HR zones, power zones, time-series data) that only a deep
/// per-activity analysis agent needs. All sensor fields are `Option<T>`
/// and `#[serde(skip_serializing_if = "Option::is_none")]` so activities
/// recorded without an HRM or on indoor trainers render cleanly without
/// null noise.
#[derive(Debug, Clone, Serialize)]
pub struct ActivitySummary {
    /// Unique activity identifier
    pub id: String,
    /// Activity name/title, as one defanged line of at most
    /// [`ACTIVITY_NAME_MAX_CHARS`]: whoever can write to the athlete's
    /// provider account typed it.
    pub name: String,
    /// Activity sport type (e.g., "run", "ride", "cross\_country\_skiing")
    pub sport_type: SportType,
    /// Start date/time in ISO 8601 format (UTC). Kept UTC so day-windowing,
    /// sorting, and fragment detection stay timezone-stable.
    pub start_date: String,
    /// Start time rendered in the user's local IANA timezone (RFC3339 with
    /// offset, e.g. `2026-05-29T08:36:07-04:00`), when the user has a timezone
    /// on file. This is the field to DISPLAY to the user — `start_date` is the
    /// raw UTC instant. `None` when the user has no timezone configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date_local: Option<String>,
    /// Distance in meters (0.0 if not available)
    pub distance_meters: f64,
    /// Duration in seconds
    pub duration_seconds: u64,
    /// Total elevation gained in meters, when the provider reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_gain_meters: Option<f64>,
    /// Average heart rate in BPM over the activity, when the user wore an HRM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_heart_rate: Option<u32>,
    /// Maximum heart rate in BPM over the activity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_heart_rate: Option<u32>,
    /// Provider-reported calorie estimate, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calories: Option<u32>,
    /// Average cadence (rpm for cycling, spm for running).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_cadence: Option<u32>,
    /// Average power output in watts (cycling / rowing / running power meters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_power: Option<u32>,
    /// Normalized Power in watts, as the provider computed it over the ride's
    /// own power samples — Strava calls this "weighted average power",
    /// intervals.icu reports it under the same name. `None` for activities
    /// recorded without a power meter, and for providers that omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_power: Option<u32>,
    /// Strava's "Suffer Score" (Relative Effort) when available. Surrogate for
    /// perceived exertion grounded in HR-in-zone time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffer_score: Option<u32>,
    /// The athlete's own rating of perceived exertion (1–10 CR-10 scale), when
    /// their provider records one. Self-reported, unlike `suffer_score`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perceived_exertion: Option<f32>,
    /// How the athlete said they felt, named on a best-to-worst scale
    /// (`strong` … `weak`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feel: Option<Feel>,
    /// Average ambient temperature in Celsius when the provider reports it.
    /// Outdoor activities from Strava and Garmin OAuth surface this when the
    /// recording device captured ambient temp; Coros does too if its watch
    /// reported it. Whoop / Terra don't expose ambient temperature
    /// on workouts (skin temp on Whoop Recovery is recorded separately, on
    /// the recovery record, not the activity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Endurance Intensity Factor — `normalized_power / ftp` (Coggan).
    /// Populated by the Endurance latest-snapshot pipeline; `None` for
    /// activities without a power stream or for users without an FTP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity_factor: Option<f64>,
    /// Endurance Efficiency Factor — `normalized_power / average_heart_rate`.
    /// `None` when either input is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efficiency_factor: Option<f64>,
    /// Endurance Variability Index — `normalized_power / average_power`.
    /// `None` when the activity has no power stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variability_index: Option<f64>,
    /// Endurance aerobic decoupling percentage. `None` when the activity
    /// has fewer than 20 paired HR+speed samples (Coggan threshold).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoupling_pct: Option<f64>,
    /// Endurance time-in-zone distribution computed against the user's
    /// configured `HrZoneSet`. `None` when no HR stream or no user zones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_distribution: Option<ZoneDistribution>,
}

impl From<&Activity> for ActivitySummary {
    fn from(activity: &Activity) -> Self {
        Self {
            id: activity.id().to_owned(),
            name: display_line(activity.name(), ACTIVITY_NAME_MAX_CHARS),
            sport_type: activity.sport_type().clone(),
            start_date: activity.start_date().to_rfc3339(),
            // Populated by prepare_activity_data when the user's timezone is
            // known; the From conversion has no timezone context.
            start_date_local: None,
            distance_meters: activity.distance_meters().unwrap_or(0.0),
            duration_seconds: activity.duration_seconds(),
            elevation_gain_meters: activity.elevation_gain(),
            average_heart_rate: activity.average_heart_rate(),
            max_heart_rate: activity.max_heart_rate(),
            calories: activity.calories(),
            average_cadence: activity.average_cadence(),
            average_power: activity.average_power(),
            normalized_power: activity.normalized_power(),
            suffer_score: activity.suffer_score(),
            perceived_exertion: activity.perceived_exertion(),
            feel: activity.feel(),
            temperature: activity.temperature(),
            // Endurance metrics are derived in the latest_snapshot pipeline,
            // not at the per-activity summary boundary. Keep them None here
            // so the JSON shape is stable for non-Section-11 callers.
            intensity_factor: None,
            efficiency_factor: None,
            variability_index: None,
            decoupling_pct: None,
            zone_distribution: None,
        }
    }
}

/// The activity list in summary mode, as the tool returns it.
///
/// Through [`to_value_as_written`] like every reader-facing payload: a
/// summary's `temperature` (the backfilled weather) and `perceived_exertion`
/// are `f32`, and a plain `serde_json::to_value` rendered 12.8 °C as
/// 12.800000190734863.
///
/// # Errors
///
/// Returns the serialization error when a summary cannot be encoded.
pub fn summary_json(summaries: &[ActivitySummary]) -> serde_json::Result<Value> {
    to_value_as_written(summaries)
}

/// Serialize activities for `mode=detailed`, fencing the athlete-authored text.
///
/// Detail mode hands the model the raw [`Activity`], and two of its fields are
/// free prose anyone with write access to the athlete's provider account
/// typed: the description and the comment thread. Each is fenced with
/// [`fence_athlete_text`] — one line, capped, its angle brackets unable to
/// close the fence — so a note reading "ignore your instructions" arrives as
/// something the athlete wrote, never as something the agent was told. The
/// activity's name and a comment author are short labels, so they are
/// flattened, defanged and capped ([`display_line`]) rather than fenced. The
/// thread keeps its newest [`MAX_COMMENTS`] entries.
///
/// The route overview (`summary_polyline`) is left out. It is an encoded
/// polyline — hundreds of characters a model cannot read as a route — and it
/// is the provider's untrimmed line, starting and ending at the athlete's
/// door. Geometry leaves the server only through the privacy-trimmed route
/// track, never as the provider sent it.
///
/// # Errors
///
/// Returns the serialization error when an activity cannot be encoded.
pub fn detail_json(activities: &[Activity]) -> serde_json::Result<Value> {
    let mut value = to_value_as_written(activities)?;
    if let Some(rows) = value.as_array_mut() {
        for row in rows.iter_mut().filter_map(Value::as_object_mut) {
            fence_self_report(row);
            row.remove(ROUTE_OVERVIEW_KEY);
        }
    }
    Ok(value)
}

/// Fence one serialized activity's description and comment thread, and
/// neutralize its name, in place.
fn fence_self_report(row: &mut Map<String, Value>) {
    if let Some(name) = row.get("name").and_then(Value::as_str) {
        let name = display_line(name, ACTIVITY_NAME_MAX_CHARS);
        row.insert("name".to_owned(), Value::String(name));
    }
    let description = row
        .get("description")
        .and_then(Value::as_str)
        .and_then(|d| fence_athlete_text(d, MAX_DESCRIPTION_CHARS));
    match description {
        Some(fenced) => {
            row.insert("description".to_owned(), Value::String(fenced));
        }
        None => {
            row.remove("description");
        }
    }

    let Some(Value::Array(comments)) = row.get_mut("comments") else {
        return;
    };
    let oldest_kept = comments.len().saturating_sub(MAX_COMMENTS);
    comments.drain(..oldest_kept);
    comments.retain_mut(|comment| {
        let Some(entry) = comment.as_object_mut() else {
            return false;
        };
        let Some(text) = entry
            .get("text")
            .and_then(Value::as_str)
            .and_then(|t| fence_athlete_text(t, MAX_COMMENT_CHARS))
        else {
            return false;
        };
        entry.insert("text".to_owned(), Value::String(text));
        if let Some(author) = entry.get("author").and_then(Value::as_str) {
            let name = display_line(author, MAX_AUTHOR_CHARS);
            entry.insert("author".to_owned(), Value::String(name));
        }
        true
    });
    if comments.is_empty() {
        row.remove("comments");
    }
}
