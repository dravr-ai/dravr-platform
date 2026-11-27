// ABOUTME: The thresholds Coach Home's attention queue raises a reason at, read from the environment once at startup
// ABOUTME: Product thresholds (quiet days, fatigue mornings, new members, missed planned days, the break baseline), never science
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};
use std::env;

/// Days without a session before the queue raises `nothing_received`.
pub const DEFAULT_DAYS_WITHOUT_ACTIVITY: u32 = 5;

/// Consecutive mornings in the deep-fatigue band, today included, before the
/// queue raises `deep_fatigue_streak`.
pub const DEFAULT_FATIGUE_STREAK_MORNINGS: u32 = 2;

/// Days after joining one of the coach's groups during which an athlete with
/// no plan is a new member to plan for.
pub const DEFAULT_NEW_MEMBER_DAYS: u32 = 14;

/// Consecutive missed planned days before `planned_sessions_missed` (carnet#1792).
///
/// A planned day is missed when none of its sessions was done. Rest days
/// between two planned days do not break the run, and one missed day never
/// raises it: planned and executed training diverge as a matter of course.
pub const DEFAULT_MISSED_PLANNED_DAYS: u32 = 2;

/// Days before today the missed-session check reads planned sessions over.
pub const DEFAULT_MISSED_LOOKBACK_DAYS: u32 = 14;

/// Days of training before today the break baseline reads the athlete's own
/// pattern over (carnet#1792, P3b): eight weeks.
///
/// Fixed, never read from the environment: the routine-detection consent the
/// athlete grants names "your last 8 weeks", so changing it changes what they
/// agreed to and the catalogue strings with it (Phil, 2026-11-27).
pub const BASELINE_WINDOW_DAYS: u32 = 56;

/// Days the athlete's first training day in the window must lie before today
/// for the baseline to read anything: four weeks.
///
/// Fixed like [`BASELINE_WINDOW_DAYS`]: the consent names "at least 4 weeks".
pub const BASELINE_MIN_HISTORY_DAYS: u32 = 28;

/// The shortest break, in whole days without a session, that the baseline
/// calls unusual, however short the athlete's earlier breaks were.
pub const DEFAULT_BASELINE_MIN_BREAK_DAYS: u32 = 3;

/// When Coach Home's attention queue raises a reason (carnet#1252).
///
/// These say when a coach should look, not what a body is doing: the band an
/// athlete's form falls in is the sports-science engine's, and the queue only
/// counts how many mornings in a row it held. Each value is positive; an
/// unset, unparseable or zero variable keeps the default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoachHomeConfig {
    /// `COACH_HOME_DAYS_WITHOUT_ACTIVITY`: days without a session before
    /// `nothing_received` is raised.
    pub days_without_activity: u32,
    /// `COACH_HOME_FATIGUE_STREAK_MORNINGS`: consecutive deep-fatigue
    /// mornings, today included, before `deep_fatigue_streak` is raised.
    pub fatigue_streak_mornings: u32,
    /// `COACH_HOME_NEW_MEMBER_DAYS`: days after joining during which an
    /// athlete without a plan raises `new_member_no_plan`.
    pub new_member_days: u32,
    /// `COACH_HOME_MISSED_PLANNED_DAYS`: consecutive planned days with none
    /// of their sessions done before `planned_sessions_missed` is raised.
    pub missed_planned_days: u32,
    /// `COACH_HOME_MISSED_LOOKBACK_DAYS`: days before today the
    /// missed-session check reads planned sessions over.
    pub missed_lookback_days: u32,
    /// `COACH_HOME_BASELINE_MIN_BREAK_DAYS`: the shortest break the baseline
    /// calls unusual.
    pub baseline_min_break_days: u32,
}

impl Default for CoachHomeConfig {
    fn default() -> Self {
        Self {
            days_without_activity: DEFAULT_DAYS_WITHOUT_ACTIVITY,
            fatigue_streak_mornings: DEFAULT_FATIGUE_STREAK_MORNINGS,
            new_member_days: DEFAULT_NEW_MEMBER_DAYS,
            missed_planned_days: DEFAULT_MISSED_PLANNED_DAYS,
            missed_lookback_days: DEFAULT_MISSED_LOOKBACK_DAYS,
            baseline_min_break_days: DEFAULT_BASELINE_MIN_BREAK_DAYS,
        }
    }
}

impl CoachHomeConfig {
    /// Read the thresholds from the environment, each falling back to its
    /// default.
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            days_without_activity: positive("COACH_HOME_DAYS_WITHOUT_ACTIVITY")
                .unwrap_or(defaults.days_without_activity),
            fatigue_streak_mornings: positive("COACH_HOME_FATIGUE_STREAK_MORNINGS")
                .unwrap_or(defaults.fatigue_streak_mornings),
            new_member_days: positive("COACH_HOME_NEW_MEMBER_DAYS")
                .unwrap_or(defaults.new_member_days),
            missed_planned_days: positive("COACH_HOME_MISSED_PLANNED_DAYS")
                .unwrap_or(defaults.missed_planned_days),
            missed_lookback_days: positive("COACH_HOME_MISSED_LOOKBACK_DAYS")
                .unwrap_or(defaults.missed_lookback_days),
            baseline_min_break_days: positive("COACH_HOME_BASELINE_MIN_BREAK_DAYS")
                .unwrap_or(defaults.baseline_min_break_days),
        }
    }
}

/// The positive number `key` holds, `None` when unset, not a number or zero.
fn positive(key: &str) -> Option<u32> {
    env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .filter(|value| *value > 0)
}
