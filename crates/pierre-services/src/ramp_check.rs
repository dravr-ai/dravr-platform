// ABOUTME: Compares a plan's opening week against the athlete's actual recent load
// ABOUTME: The one enforced rail on plan difficulty — warns, never blocks, and says when it cannot measure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Plan-save ramp check.
//!
//! The progression guidance in the agent prompt is guidance to a model this
//! repo has documented ignoring directives. This check is the only thing that
//! actually looks at a saved plan and measures it, so it must not overstate
//! what it saw.
//!
//! It compares the plan's **first** week against the athlete's real six-week
//! average, not week two against week one. The dangerous plan is not the one
//! that ramps internally — an agent writing +5% a week is doing the right thing
//! — it is the one that opens far above what the athlete actually does. A
//! four-week bundle that is uniformly 60% too hard has no internal jump at all
//! and would pass a week-over-week check silently.
//!
//! When the comparison cannot be made — no cached activity, or a plan whose
//! days carry no `duration_min` (the field is optional, and omitted for rest
//! days) — the check reports that explicitly instead of passing quietly.
//! Absence of a warning has to mean "measured and fine", or the warning's
//! presence tells you nothing.

use pierre_core::models::LoadSnapshot;
use serde::Serialize;

/// Fractional increase over the athlete's recent weekly hours that trips the
/// warning.
///
/// Twenty percent, deliberately looser than the +10% the guidance recommends:
/// this rail is also the measurement instrument for whether calibration steers
/// generation at all, and a threshold set at the guidance line would fire on
/// nearly every plan and teach nobody anything. Tighten it toward +10% with
/// data, not before.
pub const RAMP_WARN_THRESHOLD: f64 = 0.20;

/// Slack on the threshold comparison.
///
/// Both sides of the ratio come from integer minutes through a division, and
/// the resulting hour counts are not exactly representable — 504 minutes is
/// "8.4" hours only approximately. Without this, a plan sitting exactly on the
/// +20% line lands a few units in the last place above it and warns, which
/// would make the rail fire on plans that followed the guidance precisely.
const THRESHOLD_EPSILON: f64 = 1e-9;

/// Why a ramp check could not produce a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RampUnmeasurable {
    /// No cached activity in the window — no provider connected, a new
    /// account, or a genuine layoff. There is no baseline to compare against.
    NoBaseline,
    /// The plan's first week carries no `duration_min` on any day, so its
    /// planned hours cannot be summed.
    NoPlannedDurations,
}

impl RampUnmeasurable {
    /// Stable reason string for the notify event.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoBaseline => "no_baseline",
            Self::NoPlannedDurations => "no_planned_durations",
        }
    }
}

/// What the ramp check concluded about a saved plan.
///
/// Serialised because the save reports it to the agent: the numbers are
/// descriptive magnitudes — planned hours, the athlete's recent average, the
/// fractional increase — and never a risk claim, which is the CI-enforced
/// framing rule for load ratios. The agent writes the sentence; this carries
/// only what was measured.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RampVerdict {
    /// The opening week sits within the threshold of recent load.
    WithinThreshold {
        /// Planned hours in the plan's first week.
        planned_hours: f64,
        /// The athlete's recent weekly-hours average.
        baseline_hours: f64,
    },
    /// The opening week exceeds recent load by more than the threshold.
    Exceeded {
        /// Planned hours in the plan's first week.
        planned_hours: f64,
        /// The athlete's recent weekly-hours average.
        baseline_hours: f64,
        /// Fractional increase, e.g. `0.45` for +45%.
        increase: f64,
    },
    /// The check could not run.
    Unmeasurable(RampUnmeasurable),
}

/// Total planned minutes for one week, or `None` when no day declares a
/// duration.
///
/// A week of pure rest days legitimately sums to zero minutes across days that
/// *do* carry `duration_min: 0`; that is measurable and returns `Some(0)`. A
/// week where the agent simply omitted the field everywhere is not measurable
/// and returns `None` — the two cases must not collapse, or an unquantified
/// plan would read as a rest week and always pass.
#[must_use]
pub fn planned_week_minutes(durations: &[Option<u32>]) -> Option<u32> {
    if durations.iter().all(Option::is_none) {
        return None;
    }
    Some(durations.iter().flatten().sum())
}

/// Compare a plan's opening week against the athlete's recent load.
#[must_use]
pub fn assess_ramp(
    first_week_durations: &[Option<u32>],
    baseline: Option<&LoadSnapshot>,
) -> RampVerdict {
    let Some(baseline) = baseline else {
        return RampVerdict::Unmeasurable(RampUnmeasurable::NoBaseline);
    };
    let Some(planned_minutes) = planned_week_minutes(first_week_durations) else {
        return RampVerdict::Unmeasurable(RampUnmeasurable::NoPlannedDurations);
    };
    // A baseline of zero hours cannot produce a meaningful ratio, and dividing
    // by it would yield infinity. An athlete with a cached window that sums to
    // nothing has no measurable baseline either.
    if baseline.weekly_hours <= 0.0 {
        return RampVerdict::Unmeasurable(RampUnmeasurable::NoBaseline);
    }

    let planned_hours = f64::from(planned_minutes) / 60.0;
    let increase = (planned_hours - baseline.weekly_hours) / baseline.weekly_hours;

    if increase > RAMP_WARN_THRESHOLD + THRESHOLD_EPSILON {
        RampVerdict::Exceeded {
            planned_hours,
            baseline_hours: baseline.weekly_hours,
            increase,
        }
    } else {
        RampVerdict::WithinThreshold {
            planned_hours,
            baseline_hours: baseline.weekly_hours,
        }
    }
}
