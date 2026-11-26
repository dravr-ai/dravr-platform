// ABOUTME: The outcome evaluator's pure labelling rules — adherence, delta, ramp and consistency verdicts
// ABOUTME: Decide the clear cases from the data alone, without a DB or an LLM; ambiguous ones go to the judge
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pure labelling helpers of the outcome evaluator, unit-tested without a DB
//! or an LLM. The parent module re-exports them.

use dravr_cageux::periodization::load_ratios;
use pierre_memory::playbooks::OutcomeLabel;

/// Adherence verdict from whether a matching activity landed in the window.
/// `None` => no activity data at all, so we cannot tell (the caller expires it).
#[must_use]
pub fn activity_completed_label(matched: bool, any_activity: bool) -> Option<OutcomeLabel> {
    if matched {
        Some(OutcomeLabel::Success)
    } else if any_activity {
        Some(OutcomeLabel::Failure)
    } else {
        None
    }
}

/// Outcome of a delta-metric data heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaVerdict {
    /// `None` => not enough data to compute a delta (expire).
    pub label: Option<OutcomeLabel>,
    /// `true` => within the dead-band; escalate to the LLM judge.
    pub ambiguous: bool,
}

/// Label a "did the metric move favorably" delta.
///
/// A move smaller than `dead_band` is ambiguous (Neutral + escalate); a clear
/// favorable/unfavorable move is Success/Failure. Missing endpoints yield `None`
/// (expire).
#[must_use]
pub fn delta_label(
    before: Option<f64>,
    after: Option<f64>,
    higher_is_better: bool,
    dead_band: f64,
) -> DeltaVerdict {
    let (Some(b), Some(a)) = (before, after) else {
        return DeltaVerdict {
            label: None,
            ambiguous: false,
        };
    };
    let oriented = if higher_is_better { a - b } else { b - a };
    if oriented.abs() < dead_band {
        DeltaVerdict {
            label: Some(OutcomeLabel::Neutral),
            ambiguous: true,
        }
    } else if oriented > 0.0 {
        DeltaVerdict {
            label: Some(OutcomeLabel::Success),
            ambiguous: false,
        }
    } else {
        DeltaVerdict {
            label: Some(OutcomeLabel::Failure),
            ambiguous: false,
        }
    }
}

/// The highest EMA ATL/CTL a window reached, from its `(atl, ctl)` days, or
/// `None` when no day has a ratio (cageux's
/// [`atl_ctl_ratio`](load_ratios::atl_ctl_ratio): no chronic base, or no
/// reading).
#[must_use]
pub fn peak_atl_ctl_ratio(days: impl IntoIterator<Item = (f64, f64)>) -> Option<f64> {
    days.into_iter()
        .filter_map(|(atl, ctl)| load_ratios::atl_ctl_ratio(atl, ctl))
        .reduce(f64::max)
}

/// Did the window's peak EMA ATL/CTL stay within the ceiling? Inclusive at
/// the ceiling.
///
/// The ceiling arrives widened from the metric's `f32` (lossless), so the
/// comparison is exact. The caller maps over the `Option<f64>` peak from
/// [`peak_atl_ctl_ratio`].
#[must_use]
pub fn ramp_within_label(peak_ratio: f64, ceiling: f64) -> OutcomeLabel {
    if load_ratios::within_ramp_ceiling(peak_ratio, ceiling) {
        OutcomeLabel::Success
    } else {
        OutcomeLabel::Failure
    }
}

/// Label a session-count consistency window: enough sessions => Success, none =>
/// Failure, in-between => ambiguous (escalate). `expected` is the rough session
/// target for the window.
#[must_use]
pub fn consistency_label(session_count: u32, expected: u32) -> DeltaVerdict {
    if session_count >= expected.max(1) {
        DeltaVerdict {
            label: Some(OutcomeLabel::Success),
            ambiguous: false,
        }
    } else if session_count == 0 {
        DeltaVerdict {
            label: Some(OutcomeLabel::Failure),
            ambiguous: false,
        }
    } else {
        DeltaVerdict {
            label: Some(OutcomeLabel::Neutral),
            ambiguous: true,
        }
    }
}
