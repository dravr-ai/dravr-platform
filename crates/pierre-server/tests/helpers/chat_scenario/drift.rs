// ABOUTME: Drift asserter — flags numeric claims that change across turns without acknowledgement
// ABOUTME: Catches Bug 3 class: bot admits "I have new data now" but the previously stated totals never update
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Cross-turn drift detection.
//!
//! Bug 3 from the 2026-05-18 Telegram session: the bot's turn 2 said
//! "Côté course: 25.10 km from 3 trails". Turn 4 acknowledged
//! "actually you had 10 activities, including yesterday's 8 km road
//! run and 25 km road bike" — but the running km total was never
//! re-stated as 33.10 km. The user had to do the math themselves.
//!
//! [`AggregateClaim`] enumerates the claim categories the scenario
//! runner tracks across turns. After every turn, the runner appends
//! every claim it extracts from the reply to a per-scenario
//! [`ClaimTimeline`]. At the end of the scenario the drift asserter
//! walks each category and flags inconsistencies.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};

/// Categories of numeric claim tracked across turns.
///
/// The category is the comparison axis: two claims of the same
/// category at different turns are comparable; claims in different
/// categories are not (a `Distance { sport: "run" }` is not
/// comparable to a `Distance { sport: "ride" }`).
#[derive(Debug, Clone, Hash, PartialEq, Eq, Ord, PartialOrd)]
pub enum AggregateClaim {
    /// Distance total for a specific sport.
    Distance { sport: String },
    /// Total count of activities (optionally scoped to a window).
    ActivityCount { scope: String },
}

/// One numeric value the bot stated at a given turn.
#[derive(Debug, Clone)]
pub struct ClaimRecord {
    /// 1-based turn index this claim was found in.
    pub turn_index: usize,
    /// Numeric value the bot stated.
    pub value: f64,
}

/// Per-scenario record of every numeric claim per category.
#[derive(Debug, Clone, Default)]
pub struct ClaimTimeline {
    entries: BTreeMap<AggregateClaim, Vec<ClaimRecord>>,
}

impl ClaimTimeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, claim: AggregateClaim, turn_index: usize, value: f64) {
        self.entries
            .entry(claim)
            .or_default()
            .push(ClaimRecord { turn_index, value });
    }

    /// Walk every category and flag inconsistencies.
    ///
    /// A category drifts when the most recent value differs from an
    /// earlier value by more than `tolerance`. We flag the *first*
    /// drift per category — the goal is to surface the bug, not to
    /// drown the operator in noise.
    #[must_use]
    pub fn detect_drift(&self, tolerance: f64) -> Vec<DriftFinding> {
        let mut findings = Vec::new();
        for (claim, records) in &self.entries {
            if records.len() < 2 {
                continue;
            }
            let last = records.last().expect("len >= 2 guarded above");
            for prev in &records[..records.len() - 1] {
                if (prev.value - last.value).abs() > tolerance {
                    findings.push(DriftFinding {
                        claim: claim.clone(),
                        earlier_turn: prev.turn_index,
                        earlier_value: prev.value,
                        latest_turn: last.turn_index,
                        latest_value: last.value,
                    });
                    break;
                }
            }
        }
        findings
    }
}

/// One drift event surfaced by [`ClaimTimeline::detect_drift`].
#[derive(Debug, Clone)]
pub struct DriftFinding {
    pub claim: AggregateClaim,
    pub earlier_turn: usize,
    pub earlier_value: f64,
    pub latest_turn: usize,
    pub latest_value: f64,
}

impl Display for DriftFinding {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} drifted: turn {} stated {}, turn {} stated {} — bot acknowledged new data without recomputing",
            self.claim, self.earlier_turn, self.earlier_value, self.latest_turn, self.latest_value
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_drift_when_values_stable() {
        let mut t = ClaimTimeline::new();
        t.record(
            AggregateClaim::Distance {
                sport: "run".to_owned(),
            },
            1,
            25.10,
        );
        t.record(
            AggregateClaim::Distance {
                sport: "run".to_owned(),
            },
            2,
            25.10,
        );
        assert!(t.detect_drift(0.5).is_empty());
    }

    #[test]
    fn bug_3_regression_is_caught() {
        // Reproduces the 2026-05-18 Telegram scenario:
        // - turn 2: "25.10 km from 3 trails"
        // - turn 4: bot acknowledges new activities but running km
        //   total is never re-stated → if it were re-stated as 33.10
        //   km, this asserter would flag the 8 km drift.
        let mut t = ClaimTimeline::new();
        t.record(
            AggregateClaim::Distance {
                sport: "run".to_owned(),
            },
            2,
            25.10,
        );
        t.record(
            AggregateClaim::Distance {
                sport: "run".to_owned(),
            },
            4,
            33.10,
        );
        let findings = t.detect_drift(0.5);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].earlier_turn, 2);
        assert_eq!(findings[0].latest_turn, 4);
        assert!((findings[0].earlier_value - 25.10).abs() < f64::EPSILON);
        assert!((findings[0].latest_value - 33.10).abs() < f64::EPSILON);
    }

    #[test]
    fn drift_within_tolerance_is_ignored() {
        let mut t = ClaimTimeline::new();
        t.record(
            AggregateClaim::ActivityCount {
                scope: "last_7d".to_owned(),
            },
            1,
            7.0,
        );
        t.record(
            AggregateClaim::ActivityCount {
                scope: "last_7d".to_owned(),
            },
            3,
            7.4,
        );
        // Tolerance 0.5 → 7.0 vs 7.4 is within budget, no finding.
        assert!(t.detect_drift(0.5).is_empty());
    }

    #[test]
    fn different_categories_do_not_compare() {
        let mut t = ClaimTimeline::new();
        t.record(
            AggregateClaim::Distance {
                sport: "run".to_owned(),
            },
            1,
            25.0,
        );
        t.record(
            AggregateClaim::Distance {
                sport: "ride".to_owned(),
            },
            2,
            150.0,
        );
        assert!(t.detect_drift(0.5).is_empty());
    }
}
