// ABOUTME: Compares a plan's opening week against the athlete's actual recent load
// ABOUTME: The one enforced rail on plan difficulty — warns, never blocks, and says when it cannot measure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Plan-save ramp check.
//!
//! The progression guidance in the coach prompt is guidance to a model this
//! repo has documented ignoring directives. This check is the only thing that
//! actually looks at a saved plan and measures it, so it must not overstate
//! what it saw.
//!
//! It compares the plan's **first** week against the athlete's real six-week
//! average, not week two against week one. The dangerous plan is not the one
//! that ramps internally — a coach writing +5% a week is doing the right thing
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq)]
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
/// week where the coach simply omitted the field everywhere is not measurable
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

#[cfg(test)]
mod tests {
    use super::{assess_ramp, planned_week_minutes, RampUnmeasurable, RampVerdict};
    use pierre_core::models::LoadSnapshot;

    fn baseline(weekly_hours: f64) -> LoadSnapshot {
        LoadSnapshot {
            weekly_hours,
            sessions_per_week: 4.0,
            longest_session_min: 120,
            weeks: 6,
        }
    }

    #[test]
    fn a_plan_opening_far_above_recent_load_warns() {
        // 12h planned against a 7h baseline is +71% — the case the rail exists
        // for, and the one a week-over-week check never sees.
        let week = vec![Some(180), Some(180), Some(180), Some(180)];
        let verdict = assess_ramp(&week, Some(&baseline(7.0)));
        let increase = match &verdict {
            RampVerdict::Exceeded { increase, .. } => *increase,
            _ => 0.0,
        };
        assert!(
            increase > 0.70,
            "a 12h opening week on a 7h baseline must warn, got {verdict:?}"
        );
    }

    #[test]
    fn a_plan_opening_near_recent_load_does_not_warn() {
        let week = vec![Some(120), Some(120), Some(90), Some(90)];
        assert!(matches!(
            assess_ramp(&week, Some(&baseline(7.0))),
            RampVerdict::WithinThreshold { .. }
        ));
    }

    #[test]
    fn the_threshold_boundary_does_not_warn() {
        // Exactly +20% is within; the warning is for exceeding it.
        let week = vec![Some(504)]; // 8.4h == 7h * 1.20
        assert!(
            matches!(
                assess_ramp(&week, Some(&baseline(7.0))),
                RampVerdict::WithinThreshold { .. }
            ),
            "the boundary itself must not fire, or every compliant plan warns"
        );
    }

    #[test]
    fn a_uniformly_hard_bundle_is_caught_by_the_first_week() {
        // Every week identical at 11h on a 6h baseline: no internal ramp at
        // all. This is precisely the plan a week-over-week check passes.
        let flat_week = vec![Some(165), Some(165), Some(165), Some(165)];
        assert!(matches!(
            assess_ramp(&flat_week, Some(&baseline(6.0))),
            RampVerdict::Exceeded { .. }
        ));
    }

    #[test]
    fn an_athlete_with_no_cached_activity_is_reported_not_passed() {
        assert_eq!(
            assess_ramp(&[Some(120)], None),
            RampVerdict::Unmeasurable(RampUnmeasurable::NoBaseline)
        );
    }

    #[test]
    fn a_plan_with_no_durations_is_reported_not_passed() {
        // `duration_min` is optional in the save schema. Summing `None`s to
        // zero would make every unquantified plan look like a rest week and
        // pass silently — the exact way a rail becomes decorative.
        assert_eq!(
            assess_ramp(&[None, None, None], Some(&baseline(7.0))),
            RampVerdict::Unmeasurable(RampUnmeasurable::NoPlannedDurations)
        );
    }

    #[test]
    fn a_partially_quantified_week_is_still_measured() {
        let week = vec![Some(180), None, Some(180)];
        assert!(matches!(
            assess_ramp(&week, Some(&baseline(2.0))),
            RampVerdict::Exceeded { .. }
        ));
    }

    #[test]
    fn a_zero_baseline_cannot_produce_a_ratio() {
        assert_eq!(
            assess_ramp(&[Some(120)], Some(&baseline(0.0))),
            RampVerdict::Unmeasurable(RampUnmeasurable::NoBaseline),
            "dividing by a zero baseline would report an infinite increase"
        );
    }

    #[test]
    fn a_real_rest_week_is_measurable_but_an_unquantified_one_is_not() {
        assert_eq!(planned_week_minutes(&[Some(0), Some(0)]), Some(0));
        assert_eq!(planned_week_minutes(&[None, None]), None);
        assert_eq!(planned_week_minutes(&[]), None);
    }

    #[test]
    fn a_lighter_plan_never_warns() {
        let week = vec![Some(60), Some(60)];
        assert!(matches!(
            assess_ramp(&week, Some(&baseline(7.0))),
            RampVerdict::WithinThreshold { .. }
        ));
    }

    #[test]
    fn unmeasurable_reasons_are_distinguishable_in_the_event() {
        assert_eq!(RampUnmeasurable::NoBaseline.as_str(), "no_baseline");
        assert_eq!(
            RampUnmeasurable::NoPlannedDurations.as_str(),
            "no_planned_durations"
        );
    }
}
