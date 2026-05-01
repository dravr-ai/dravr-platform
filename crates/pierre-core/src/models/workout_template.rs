// ABOUTME: Endurance Phase 5 workout-template + prescribed-workout shapes for the prescription library
// ABOUTME: Backs workout_templates + prescribed_workouts tables; toml seed format mirrors WorkoutTemplate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::SportType;

/// Intensity distribution model for a workout (Seiler-influenced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntensityDistribution {
    /// Predominantly low intensity (>80 % Z1+Z2).
    Polarized,
    /// Mostly threshold work (Z3 dominant).
    Threshold,
    /// VO2max-dominant (Z4-Z5).
    Vo2max,
    /// Recovery (Z1 only).
    Recovery,
    /// Mixed pyramid distribution.
    Pyramid,
}

/// Single step inside a structured workout (warmup, interval, recovery, cool-down).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkoutStep {
    /// Human-readable label ("Warm-up", "Interval", "Recovery", "Cool-down").
    pub label: String,
    /// Duration in seconds (use the lower bound for distance-based intervals).
    pub duration_seconds: u32,
    /// Optional distance in metres for distance-based intervals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_meters: Option<f64>,
    /// Target zone label (`"Z1"`, `"Z2"`, `"Threshold"`, `"VO2max"`, `"Recovery"`).
    pub target_zone: String,
    /// Optional integer repeat count when this step is part of a set
    /// (e.g. `repeat = 4` for 4×8 min threshold). Defaults to 1.
    #[serde(default = "default_repeat")]
    pub repeat: u32,
    /// Optional free-form note for the coach to surface in the prescription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

const fn default_repeat() -> u32 {
    1
}

/// Per-template target zone overlay (applied on top of the user's own
/// `HrZoneSet` / `PowerZoneSet` when prescribing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkoutTargetZones {
    /// Optional HR percentages (of LT2) for the workout's zones, in
    /// ascending order matching `Z1..Z5`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hr_pct_of_lt2: Option<[f64; 5]>,
    /// Optional power percentages (of FTP) for the workout's zones, in
    /// ascending order matching `Z1..Z5`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_pct_of_ftp: Option<[f64; 5]>,
}

/// Endurance workout template — declarative, repeatable structured session.
///
/// The compiled-in cornerstones (`workout_templates/*.toml` at the repo
/// root) deserialize into this struct via `WorkoutTemplate::from_toml`.
/// User-authored overrides live in the `workout_templates` database
/// table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkoutTemplate {
    /// Stable identifier (UUID for DB rows, deterministic UUID for compiled-in templates).
    pub id: Uuid,
    /// Tenant scope for user-authored templates; `None` for compiled-in cornerstones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<Uuid>,
    /// User scope for user-authored templates; `None` for compiled-in cornerstones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    /// URL-safe slug (`long_run_z2`, `threshold_4x8`, …) — unique per scope.
    pub slug: String,
    /// Human-readable workout name.
    pub name: String,
    /// Sport this template applies to.
    pub sport: SportType,
    /// Total expected duration in minutes (sum of step durations).
    pub duration_minutes: u32,
    /// High-level intensity distribution for downstream coaching cues.
    pub intensity_distribution: IntensityDistribution,
    /// Ordered list of steps that make up the workout.
    pub structure: Vec<WorkoutStep>,
    /// Per-template target-zone overlay.
    pub target_zones: WorkoutTargetZones,
    /// Whether this template is one of the compiled-in cornerstones (read-only).
    #[serde(default)]
    pub is_compiled_in: bool,
    /// Last-modified timestamp.
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

impl WorkoutTemplate {
    /// Parse a TOML byte slice into a [`WorkoutTemplate`].
    ///
    /// # Errors
    ///
    /// Returns an error string when the TOML cannot be deserialised.
    pub fn from_toml(bytes: &[u8]) -> Result<Self, String> {
        let s = str::from_utf8(bytes).map_err(|e| format!("template not valid UTF-8: {e}"))?;
        toml::from_str(s).map_err(|e| format!("template parse failed: {e}"))
    }
}

/// Audit row for a single prescription pushed to a provider calendar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrescribedWorkout {
    /// Audit row id.
    pub id: Uuid,
    /// Tenant scope.
    pub tenant_id: Uuid,
    /// Athlete the workout was prescribed for.
    pub user_id: Uuid,
    /// Optional coach id that triggered the prescription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coach_id: Option<String>,
    /// Slug of the template that was pushed.
    pub template_slug: String,
    /// Sport label.
    pub sport: SportType,
    /// Calendar date the workout is scheduled for.
    pub prescribed_for_date: NaiveDate,
    /// Provider the workout was pushed to (e.g. `intervals_icu`).
    pub provider: String,
    /// Provider-side event id when known (returned by the push call).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_event_id: Option<String>,
    /// JSON payload that was pushed (kept for debugging + replay).
    pub payload_json: String,
    /// Lifecycle status (`pushed`, `cancelled`, `completed`).
    pub status: String,
    /// Timestamp when the prescription was recorded.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
}
