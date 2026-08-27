// ABOUTME: Endurance Phase 5 workout-template + prescribed-workout shapes for the prescription library
// ABOUTME: Backs workout_templates + prescribed_workouts tables; toml seed format mirrors WorkoutTemplate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// An intensity label relative to the athlete's thresholds.
///
/// This is the closed grammar Dravr understands, in the coach's own
/// vocabulary, when it turns a step's `target_zone` (or a plan day's
/// `intensity`) into a calendar target.
///
/// Kept relative on purpose: a provider resolves `Zone(2)` against the
/// athlete's own zones, so an FTP retest never invalidates a pushed session.
/// A label outside this grammar is not an error; it is prose, and the entry
/// goes out timed and un-targeted rather than with a target the coach never
/// stated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativeIntensity {
    /// Training zone 1–7 in the sport's primary target family.
    Zone(u8),
    /// Training zone 1–7, explicitly on heart rate.
    HeartRateZone(u8),
    /// Sweet spot, the 88–94 % FTP band.
    SweetSpot,
    /// A percentage band of threshold; `high == low` for a single value.
    Percent {
        /// Lower bound, percent of threshold.
        low: u16,
        /// Upper bound, percent of threshold.
        high: u16,
    },
}

impl RelativeIntensity {
    /// Highest zone number the grammar admits.
    const MAX_ZONE: u8 = 7;
    /// Highest percentage the grammar admits (sprint work sits above 200 %).
    const MAX_PERCENT: u16 = 300;

    /// Parse a coach-vocabulary label: `Z2`, `zone 4`, `Z2 HR`, `tempo`,
    /// `threshold`, `VO2max`, `sweet spot`, `75%`, `88-93% FTP`. Case and
    /// surrounding whitespace do not matter. Returns `None` for anything else.
    #[must_use]
    pub fn parse(label: &str) -> Option<Self> {
        let lowered = label.trim().to_lowercase();
        let label = lowered.strip_prefix("zone ").unwrap_or(&lowered).trim();
        if let Some(rest) = label.strip_suffix(" hr") {
            return Self::zone_number(rest.trim()).map(Self::HeartRateZone);
        }
        if matches!(label, "sweet spot" | "sweetspot" | "sweet-spot") {
            return Some(Self::SweetSpot);
        }
        if let Some(zone) = Self::zone_number(label) {
            return Some(Self::Zone(zone));
        }
        Self::percent_band(label)
    }

    /// `z2` / `2` after a `zone ` prefix, or a named zone, → its number.
    fn zone_number(label: &str) -> Option<u8> {
        if let Some(digits) = label.strip_prefix('z') {
            let n: u8 = digits.trim().parse().ok()?;
            return (1..=Self::MAX_ZONE).contains(&n).then_some(n);
        }
        if let Ok(n) = label.parse::<u8>() {
            return (1..=Self::MAX_ZONE).contains(&n).then_some(n);
        }
        match label {
            "recovery" | "active recovery" => Some(1),
            "endurance" | "aerobic" => Some(2),
            "tempo" => Some(3),
            "threshold" | "lactate threshold" | "ftp" => Some(4),
            "vo2max" | "vo2 max" | "vo2" => Some(5),
            "anaerobic" | "anaerobic capacity" => Some(6),
            "neuromuscular" | "sprint" => Some(7),
            _ => None,
        }
    }

    /// `NN%` / `NN-MM%` / `NN-MM% FTP` → the band; anything else `None`.
    fn percent_band(label: &str) -> Option<Self> {
        let body = label
            .strip_suffix(" ftp")
            .or_else(|| label.strip_suffix("ftp"))
            .unwrap_or(label)
            .trim()
            .strip_suffix('%')?
            .trim();
        let (low, high) = match body.split_once('-') {
            Some((low, high)) => (low.trim(), Some(high.trim())),
            None => (body, None),
        };
        let low: u16 = low.parse().ok()?;
        if !(1..=Self::MAX_PERCENT).contains(&low) {
            return None;
        }
        let high = match high {
            Some(high) => high.parse::<u16>().ok()?,
            None => low,
        };
        (low..=Self::MAX_PERCENT)
            .contains(&high)
            .then_some(Self::Percent { low, high })
    }
}

/// What kind of calendar entry a [`PlannedSession`] becomes on the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedSessionKind {
    /// A training session on one date.
    Workout,
    /// A note pinned to a whole calendar week (a plan week's focus).
    WeekNote,
}

/// One calendar entry as a provider write sees it.
///
/// Both write paths converge here: a prescription renders its
/// [`WorkoutTemplate`] into one, and a plan push renders each planned day into
/// one. A provider turns it into its own event shape — Intervals.icu into a
/// `WORKOUT` event whose description carries the step DSL, a structured-workout
/// API into its own JSON — so the reconciler that diffs sessions never learns
/// a provider's vocabulary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedSession {
    /// Dravr's key for this entry on the provider (`dravr:rx:{prescription}`
    /// or `dravr:plan:{user}:{date}:{ordinal}`). Stable across re-pushes, so
    /// the provider-side `external_id` and the ledger agree on identity.
    pub external_id: String,
    /// Workout or week note.
    pub kind: PlannedSessionKind,
    /// Civil date of the entry (a week note's is the week's first day).
    pub date: NaiveDate,
    /// Sport the entry is for; carried but unused for a week note.
    pub sport: SportType,
    /// Title shown on the calendar.
    pub name: String,
    /// Planned duration, when the entry has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u32>,
    /// Coach prose, verbatim — what the athlete reads on the entry.
    pub notes: String,
    /// Structured steps when Dravr has structure; empty means a timed entry
    /// carrying only the prose.
    #[serde(default)]
    pub steps: Vec<WorkoutStep>,
}

impl PlannedSession {
    /// Render a prescription's template as the calendar entry for `date`.
    #[must_use]
    pub fn from_template(template: &WorkoutTemplate, date: NaiveDate, external_id: String) -> Self {
        Self {
            external_id,
            kind: PlannedSessionKind::Workout,
            date,
            sport: template.sport.clone(),
            name: template.name.clone(),
            duration_seconds: Some(template.duration_minutes.saturating_mul(60)),
            notes: String::new(),
            steps: template.structure.clone(),
        }
    }

    /// Content hash of the entry, so a re-push can tell an unchanged entry
    /// from a changed one without a provider round trip. The JSON form is the
    /// canonical one (struct field order is fixed), hashed with SHA-256.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when the session cannot be encoded —
    /// which no well-formed session produces.
    pub fn payload_hash(&self) -> Result<String, serde_json::Error> {
        let bytes = serde_json::to_vec(self)?;
        let digest = Sha256::digest(&bytes);
        Ok(format!("{digest:x}"))
    }
}

/// Dravr's `external_id` keys for provider calendar entries.
///
/// A key names the calendar *slot*, not the row that filled it: a plan day's
/// key is (athlete, date, ordinal), so plan and week ids — which change on
/// every supersession — never churn the calendar, and a re-push updates the
/// same entry. A prescription's key is its own ledger id, because two
/// prescriptions on one date are two entries by design.
pub struct CalendarKey;

impl CalendarKey {
    /// Prefix shared by every plan-derived key.
    pub const PLAN_PREFIX: &'static str = "dravr:plan:";
    /// Prefix of every single-prescription key.
    pub const PRESCRIPTION_PREFIX: &'static str = "dravr:rx:";

    /// Key of a single prescription.
    #[must_use]
    pub fn prescription(prescription_id: Uuid) -> String {
        format!("{}{prescription_id}", Self::PRESCRIPTION_PREFIX)
    }

    /// Prefix of every plan-derived key for one athlete — what a reconcile
    /// matches against when it adopts an entry the ledger lost track of.
    #[must_use]
    pub fn plan_prefix(user_id: Uuid) -> String {
        format!("{}{user_id}:", Self::PLAN_PREFIX)
    }

    /// Key of the `ordinal`-th session on `date` of an athlete's plan.
    #[must_use]
    pub fn plan_day(user_id: Uuid, date: NaiveDate, ordinal: usize) -> String {
        format!(
            "{}{}:{ordinal}",
            Self::plan_prefix(user_id),
            date.format("%Y-%m-%d")
        )
    }

    /// Key of the week-level note of the plan week starting `week_start`.
    #[must_use]
    pub fn plan_week_note(user_id: Uuid, week_start: NaiveDate) -> String {
        format!(
            "{}week:{}",
            Self::plan_prefix(user_id),
            week_start.format("%Y-%m-%d")
        )
    }
}

/// A calendar event as the provider reports it back — the identity and
/// freshness a reconcile needs, nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEventRef {
    /// The provider's own id for the event.
    pub provider_event_id: String,
    /// The `external_id` the event carries, when its writer set one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Civil date of the event.
    pub date: NaiveDate,
    /// When the provider last saw the event change, when it reports that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

/// Which write path produced a calendar ledger row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarEventSource {
    /// A single session written by `prescribe_workout`.
    #[default]
    Prescription,
    /// One planned day of the athlete's active training plan.
    PlanDay,
    /// The week-level note of a plan week (its focus and adjustment reason).
    PlanWeekNote,
}

impl CalendarEventSource {
    /// Stable string identifier — byte-for-byte what serde emits, and the
    /// `source` column value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prescription => "prescription",
            Self::PlanDay => "plan_day",
            Self::PlanWeekNote => "plan_week_note",
        }
    }

    /// Parse a `source` column value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "prescription" => Some(Self::Prescription),
            "plan_day" => Some(Self::PlanDay),
            "plan_week_note" => Some(Self::PlanWeekNote),
            _ => None,
        }
    }

    /// Whether this row was rendered from the athlete's training plan.
    #[must_use]
    pub const fn is_plan(self) -> bool {
        matches!(self, Self::PlanDay | Self::PlanWeekNote)
    }
}

/// Ledger row for one calendar entry Dravr wrote to a provider.
///
/// Every provider write — a single prescription or one entry of a plan push —
/// leaves exactly one row per attempt, so the ledger can answer "what is on
/// the athlete's calendar because of Dravr, and which provider event is it?"
/// A row is never edited into a different entry: a re-push of the same key
/// writes a new `pushed` row pointing back at the old one via `replaces_id`
/// and moves the old one to `replaced`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrescribedWorkout {
    /// Ledger row id.
    pub id: Uuid,
    /// Tenant scope.
    pub tenant_id: Uuid,
    /// Athlete the entry was written for.
    pub user_id: Uuid,
    /// Optional coach id that triggered the write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coach_id: Option<String>,
    /// Slug of the template a prescription pushed; `None` for a plan entry,
    /// which has no template behind it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_slug: Option<String>,
    /// Sport label.
    pub sport: SportType,
    /// Calendar date the entry is scheduled for.
    pub prescribed_for_date: NaiveDate,
    /// Provider the entry was written to (e.g. `intervals_icu`).
    pub provider: String,
    /// Provider-side event id when known (returned by the push call).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_event_id: Option<String>,
    /// Dravr's key for the event on the provider — see
    /// [`PlannedSession::external_id`]. `None` only on rows written before
    /// the ledger carried keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Which write path produced this row.
    #[serde(default)]
    pub source: CalendarEventSource,
    /// The plan week the entry was rendered from, for plan sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_week_id: Option<String>,
    /// The row this one superseded on the provider — same calendar entry,
    /// new content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces_id: Option<Uuid>,
    /// [`PlannedSession::payload_hash`] of what was pushed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_hash: Option<String>,
    /// JSON payload that was pushed (kept for debugging + replay).
    pub payload_json: String,
    /// Outcome: [`Self::STATUS_PUSHED`] while the entry is live on the
    /// provider, [`Self::STATUS_FAILED`] when the provider refused,
    /// [`Self::STATUS_REPLACED`] once a newer row took over the same entry,
    /// [`Self::STATUS_WITHDRAWN`] once the entry was deleted from the provider.
    pub status: String,
    /// Timestamp when the row was recorded.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    /// Timestamp of the last status change.
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

impl PrescribedWorkout {
    /// The entry is live on the provider and `provider_event_id` names it.
    pub const STATUS_PUSHED: &'static str = "pushed";
    /// The provider refused the write; no event exists.
    pub const STATUS_FAILED: &'static str = "failed";
    /// A newer row now owns the same calendar entry.
    pub const STATUS_REPLACED: &'static str = "replaced";
    /// The entry was deleted from the provider.
    pub const STATUS_WITHDRAWN: &'static str = "withdrawn";

    /// Whether the row's entry is live on the provider.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.status == Self::STATUS_PUSHED
    }
}
