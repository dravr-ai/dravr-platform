// ABOUTME: Training-plan domain model — coach-authored plan outlines + weekly microcycles
// ABOUTME: Two-level periodization (outline blocks / day-by-day weeks); supersession, never mutation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Training plans
//!
//! A training plan is the coach's strategic answer to a dated goal, split in
//! two levels that mirror endurance periodization:
//!
//! - [`TrainingPlan`] — the **outline** (macrocycle): a goal-race snapshot,
//!   the ordered [`PlanBlock`]s (base/build/peak/taper…), and a prose
//!   strategy. One `active` outline per (tenant, user, coach).
//! - [`PlanWeek`] — one **microcycle**: seven [`PlannedDay`] rows with
//!   intensity expressed *relative to thresholds* (zones, %FTP) so an FTP
//!   retest never invalidates a stored plan, and — when the coach states
//!   it — the session's structure as [`WorkoutStep`]s.
//!
//! Plans are captured by explicit coach tool calls, never post-hoc
//! extraction — Tier-2 extraction minting coach prescriptions as `user_facts`
//! is the failure this entity replaces. Adjustments are **prospective-only
//! whole-row supersession**: a re-save creates a new row pointing at the old
//! one via `supersedes_id`; nothing is edited in place, past weeks stay
//! immutable, and the chain is the audit trail.
//!
//! Boundary versus the sibling coach-memory surfaces: a plan is a
//! forward-looking structured prescription; a followup
//! ([`crate::followups::CoachFollowup`]) is a one-off commitment to check in;
//! a playbook ([`crate::playbooks::Playbook`]) is a learned
//! trigger→intervention pattern. A plan is never a followup (no due-at
//! semantics) and never a playbook (not evidence-scored).

use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::models::periodization::serde_num::{whole_u32_opt, whole_u8};
use pierre_core::models::{FuelingProtocol, WorkoutStep};
use serde::{Deserialize, Serialize};

/// Lifecycle of a [`TrainingPlan`] outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// The plan currently guiding this athlete (at most one per coach).
    Active,
    /// Replaced by a newer outline (`supersedes_id` chain).
    Superseded,
    /// The goal race happened; kept for history.
    Completed,
    /// Dropped without completing (goal abandoned, athlete left).
    Abandoned,
}

impl PlanStatus {
    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }

    /// Parse the DB representation. Unknown strings return `None` so a
    /// migration gap surfaces as an error, not a silent default.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "completed" => Some(Self::Completed),
            "abandoned" => Some(Self::Abandoned),
            _ => None,
        }
    }
}

/// Lifecycle of a [`PlanWeek`]. Past-ness is derived from `week_start` at
/// read time — a week that elapsed is simply in the past, not a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeekStatus {
    /// The current prescription for its calendar week.
    Active,
    /// Replaced by an adjusted re-save of the same week.
    Superseded,
}

impl WeekStatus {
    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
        }
    }

    /// Parse the DB representation.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

/// Race priority in the athlete's calendar, borrowed from standard coaching
/// nomenclature: A = the goal, B = tune-up raced for a result, C = training
/// race / no taper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RacePriority {
    /// Goal race the plan builds toward.
    A,
    /// Secondary race worth a mini-taper.
    B,
    /// Training race ridden through.
    C,
}

impl RacePriority {
    /// Stable string identifier — byte-for-byte what serde emits for this
    /// variant, so renderers can label a race without a JSON round-trip.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
        }
    }
}

/// A race in the plan's calendar.
///
/// The outline's `goal_race` is a **snapshot** taken at plan time; the living
/// source of truth for what the athlete wants is the pillar `Goal` user fact
/// linked via `goal_fact_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalRace {
    /// Race name as the athlete calls it ("Big Red").
    pub name: String,
    /// Civil race date, `YYYY-MM-DD` (validated by [`parse_plan_date`]).
    pub date: String,
    /// Discipline ("gravel", "xco", "road", "trail run", …). Freeform.
    pub discipline: String,
    /// Priority in the athlete's calendar.
    pub priority: RacePriority,
}

/// Phase of a training block within the outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockPhase {
    /// Recovery / reset week(s).
    Rest,
    /// Aerobic volume foundation.
    Base,
    /// Progressive load with race-specific intensity.
    Build,
    /// Sharpening at race demands.
    Peak,
    /// Pre-race freshening: shorter, sharper, more rest.
    Taper,
}

impl BlockPhase {
    /// Stable string identifier — byte-for-byte what serde emits for this
    /// variant, so renderers can label a block without a JSON round-trip.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Base => "base",
            Self::Build => "build",
            Self::Peak => "peak",
            Self::Taper => "taper",
        }
    }
}

/// One block of the outline (mesocycle): a phase with a start date, a length
/// in weeks, and the coach's intent for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanBlock {
    /// What kind of block this is.
    pub phase: BlockPhase,
    /// Civil start date of the block, `YYYY-MM-DD` (a Monday by convention,
    /// not enforced — athletes' weeks start where their lives allow).
    pub start: String,
    /// Block length in weeks.
    #[serde(deserialize_with = "whole_u8")]
    pub weeks: u8,
    /// The coach's intent, in coach voice ("rebuild volume, one moderate
    /// day/week").
    pub intent: String,
    /// Target weekly volume in hours, when the coach prescribes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_hours: Option<f32>,
}

/// One prescribed day inside a [`PlanWeek`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedDay {
    /// Civil date, `YYYY-MM-DD`.
    pub date: String,
    /// Sport ("mtb", "gravel", "run", …) or `"rest"` for a rest day.
    pub sport: String,
    /// What to do, in coach voice ("2h endurance, low HR on climbs").
    pub workout: String,
    /// Planned duration in minutes; `None` for rest days.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub duration_min: Option<u32>,
    /// Intensity relative to the athlete's thresholds ("Z2", "3x8min @
    /// 88-93% FTP"). Empty for rest days. Stored relative — never absolute
    /// watts — so the plan survives an FTP retest. With `steps`, the day's
    /// summary label.
    #[serde(default)]
    pub intensity: String,
    /// The session's structure, when the coach states it as steps rather
    /// than prose: the same [`WorkoutStep`] vocabulary a single prescription
    /// carries, so a plan day reaches a provider calendar as workout-builder
    /// steps with a computable planned load. Empty for a steady day, a rest
    /// day, or a day the coach described in words only — `intensity` is then
    /// the only structure the day has.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkoutStep>,
    /// What to take in during the session, when the coach prescribes it.
    ///
    /// The builder coaches have emitted a `fueling_protocol` on every long
    /// session since the structured-workout schema defined one; without a
    /// field to land in, that prescription was discarded on the way to
    /// storage and the athlete never saw it. Absent for a session short
    /// enough not to need fuelling, and for a rest day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fueling: Option<FuelingProtocol>,
}

impl PlannedDay {
    /// `true` when the day prescribes no training.
    #[must_use]
    pub fn is_rest(&self) -> bool {
        self.sport.eq_ignore_ascii_case("rest")
    }
}

/// The plan outline (macrocycle): goal-race snapshot, block structure, and
/// strategy. Tenant-scoped; at most one `active` per (tenant, user, coach).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingPlan {
    /// Stable identifier.
    pub id: String,
    /// Tenant that owns the plan.
    pub tenant_id: String,
    /// Athlete the plan is for.
    pub user_id: String,
    /// Coach persona slug that authored it, or `None` for coach-agnostic.
    pub coach_slug: Option<String>,
    /// Pillar `Goal` user-fact this plan serves, when linked. The fact is the
    /// living goal; this plan snapshots it. A superseded fact flags the plan
    /// stale on read.
    pub goal_fact_id: Option<String>,
    /// Snapshot of the goal race taken at plan time.
    pub goal_race: GoalRace,
    /// Secondary races on the calendar (B/C priorities).
    pub races: Vec<GoalRace>,
    /// The coach's strategy in prose — what the athlete sees as "what the
    /// coach has in mind".
    pub strategy: String,
    /// Ordered mesocycle blocks from now to the goal race.
    pub blocks: Vec<PlanBlock>,
    /// Lifecycle status.
    pub status: PlanStatus,
    /// Outline this one replaced, if any (audit chain).
    pub supersedes_id: Option<String>,
    /// Conversation the plan was agreed in, for provenance.
    pub source_conversation_id: Option<String>,
    /// When this outline row was created.
    pub created_at: DateTime<Utc>,
    /// When this outline row last changed status.
    pub updated_at: DateTime<Utc>,
}

/// One microcycle: the day-by-day prescription for a single calendar week of
/// a [`TrainingPlan`]. At most one `active` per (plan, `week_start`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanWeek {
    /// Stable identifier.
    pub id: String,
    /// Tenant that owns the week.
    pub tenant_id: String,
    /// Athlete the week is for (denormalized for tenant-scoped reads).
    pub user_id: String,
    /// Outline this week belongs to.
    pub plan_id: String,
    /// Civil date of the week's first day, `YYYY-MM-DD`.
    pub week_start: String,
    /// The week's intent in coach voice ("volume back up, one moderate day").
    pub focus: String,
    /// The day rows, in date order. At most seven.
    pub days: Vec<PlannedDay>,
    /// Lifecycle status.
    pub status: WeekStatus,
    /// Week row this one replaced, if any (adjustment audit chain).
    pub supersedes_id: Option<String>,
    /// Why the coach re-saved this week ("legs heavy after Buckland, moved
    /// tempo to Wednesday"). Empty on first save.
    pub adjustment_reason: String,
    /// When this week row was created.
    pub created_at: DateTime<Utc>,
    /// When this week row last changed status.
    pub updated_at: DateTime<Utc>,
}

/// Maximum day rows in one week — a microcycle is a calendar week.
pub const MAX_DAYS_PER_WEEK: usize = 7;

/// Parse a civil plan date, accepting only canonical zero-padded
/// `YYYY-MM-DD`.
///
/// Stored dates sort lexically (`ORDER BY week_start`), so a non-padded
/// `2026-8-8` — which chrono would happily parse — must be rejected or it
/// would sort after every August entry. Plan dates are athlete-visible and
/// must never be guessed.
#[must_use]
pub fn parse_plan_date(s: &str) -> Option<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    (parsed.format("%Y-%m-%d").to_string() == s).then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_plan_date, BlockPhase, GoalRace, PlanBlock, PlanStatus, PlannedDay, RacePriority,
        WeekStatus,
    };

    #[test]
    fn statuses_roundtrip_through_db_strings() {
        for status in [
            PlanStatus::Active,
            PlanStatus::Superseded,
            PlanStatus::Completed,
            PlanStatus::Abandoned,
        ] {
            assert_eq!(PlanStatus::parse(status.as_str()), Some(status));
        }
        for status in [WeekStatus::Active, WeekStatus::Superseded] {
            assert_eq!(WeekStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(PlanStatus::parse("bogus"), None);
        assert_eq!(WeekStatus::parse("completed"), None);
    }

    #[test]
    fn goal_race_and_blocks_serialize_stably() {
        let race = GoalRace {
            name: "Big Red".to_owned(),
            date: "2026-08-08".to_owned(),
            discipline: "gravel".to_owned(),
            priority: RacePriority::A,
        };
        let json = serde_json::to_string(&race).unwrap_or_default();
        assert!(json.contains("\"priority\":\"A\""));
        let back = serde_json::from_str::<GoalRace>(&json).ok();
        assert_eq!(back, Some(race));

        let block = PlanBlock {
            phase: BlockPhase::Build,
            start: "2026-07-13".to_owned(),
            weeks: 2,
            intent: "volume back, one moderate day/week".to_owned(),
            target_hours: Some(9.5),
        };
        let json = serde_json::to_string(&block).unwrap_or_default();
        assert!(json.contains("\"phase\":\"build\""));
        let back = serde_json::from_str::<PlanBlock>(&json).ok();
        assert_eq!(back.as_ref().map(|b| b.phase), Some(BlockPhase::Build));
        assert_eq!(back.and_then(|b| b.target_hours), Some(9.5));
    }

    #[test]
    fn rest_day_detection_and_optional_fields() {
        let rest = PlannedDay {
            date: "2026-07-13".to_owned(),
            sport: "Rest".to_owned(),
            workout: "off — legs up".to_owned(),
            duration_min: None,
            intensity: String::new(),
            steps: Vec::new(),
            fueling: None,
        };
        assert!(rest.is_rest());
        // duration_min: None must not serialize a null (schema hygiene for
        // the LLM-facing tool payloads).
        let json = serde_json::to_string(&rest).unwrap_or_default();
        assert!(!json.is_empty());
        assert!(!json.contains("duration_min"));

        let ride = serde_json::from_str::<PlannedDay>(
            r#"{"date":"2026-07-14","sport":"mtb","workout":"2h endurance","duration_min":120,"intensity":"Z2"}"#,
        )
        .ok();
        assert_eq!(ride.as_ref().map(PlannedDay::is_rest), Some(false));
        assert_eq!(ride.and_then(|d| d.duration_min), Some(120));
    }

    #[test]
    fn plan_dates_parse_strictly() {
        assert!(parse_plan_date("2026-08-08").is_some());
        assert!(parse_plan_date("08/08/2026").is_none());
        assert!(parse_plan_date("2026-8-8").is_none());
        assert!(parse_plan_date("tomorrow").is_none());
    }
}
