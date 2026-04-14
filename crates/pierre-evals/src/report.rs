// ABOUTME: Aggregated rubric-driven report — sums multi-turn outcomes across an entire fixture set
// ABOUTME: Used by the test runner to print a CI-friendly summary table
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

use crate::multi_turn::MultiTurnReport;

/// Stable identifier for the rubric category a row in the summary covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RubricKind {
    /// Relevance dimension.
    Relevance,
    /// Safety / faithfulness dimension.
    Safety,
    /// Persona adherence dimension.
    PersonaAdherence,
}

/// Aggregated summary across a list of [`MultiTurnReport`]s.
///
/// Built by the CI test runner via [`Self::aggregate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSummary {
    /// Total cases evaluated.
    pub case_count: usize,
    /// Total turns evaluated across all cases.
    pub turn_count: usize,
    /// Number of turns that passed every check (deterministic + judge).
    pub fully_passing_turns: usize,
    /// Average rubric score across every turn evaluated.
    pub mean_score: f32,
}

impl EvalSummary {
    /// Aggregate per-case reports into a single summary.
    #[must_use]
    pub fn aggregate(reports: &[MultiTurnReport]) -> Self {
        let case_count = reports.len();
        let turn_count: usize = reports.iter().map(|r| r.turn_results.len()).sum();
        let fully_passing_turns: usize = reports.iter().map(|r| r.fully_passing_turns).sum();

        let mean_score = if turn_count == 0 {
            0.0
        } else {
            let total: f32 = reports
                .iter()
                .map(|r| {
                    #[allow(clippy::cast_precision_loss)]
                    let weight = r.turn_results.len() as f32;
                    r.mean_score * weight
                })
                .sum();
            #[allow(clippy::cast_precision_loss)]
            {
                total / turn_count as f32
            }
        };

        Self {
            case_count,
            turn_count,
            fully_passing_turns,
            mean_score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EvalSummary, RubricKind};
    use crate::deterministic::DeterministicReport;
    use crate::fixtures::Turn;
    use crate::judge::{JudgeVerdict, RubricScore};
    use crate::multi_turn::{MultiTurnReport, TurnResult};

    fn turn_result(score: u8, passed: bool) -> TurnResult {
        TurnResult {
            turn_index: 0,
            deterministic: DeterministicReport::run(
                &Turn {
                    user: String::new(),
                    expected_coach: "x".into(),
                    must_contain: vec![],
                    must_not_contain: vec![],
                },
                "x",
                100,
            ),
            verdict: JudgeVerdict {
                rubrics: vec![RubricScore {
                    rubric_name: "relevance".into(),
                    score,
                    rationale: "test".into(),
                    passed,
                }],
            },
        }
    }

    fn report(scores: &[(u8, bool)]) -> MultiTurnReport {
        let turn_results: Vec<TurnResult> =
            scores.iter().map(|(s, p)| turn_result(*s, *p)).collect();
        #[allow(clippy::cast_precision_loss)]
        let mean =
            scores.iter().map(|(s, _)| f32::from(*s)).sum::<f32>() / (scores.len() as f32).max(1.0);
        let passing = scores.iter().filter(|(_, p)| *p).count();
        MultiTurnReport {
            case_id: "c".into(),
            turn_results,
            mean_score: mean,
            fully_passing_turns: passing,
        }
    }

    #[test]
    fn aggregate_zero_reports_is_zero() {
        let s = EvalSummary::aggregate(&[]);
        assert_eq!(s.case_count, 0);
        assert_eq!(s.turn_count, 0);
        assert!((s.mean_score - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn aggregate_average_is_turn_weighted() {
        let r1 = report(&[(5, true), (5, true)]);
        let r2 = report(&[(3, false)]);
        let s = EvalSummary::aggregate(&[r1, r2]);
        assert_eq!(s.case_count, 2);
        assert_eq!(s.turn_count, 3);
        assert_eq!(s.fully_passing_turns, 2);
        // (5+5+3)/3 = 4.333...
        assert!((s.mean_score - 13.0_f32 / 3.0).abs() < 0.001);
    }

    #[test]
    fn rubric_kind_is_distinct() {
        assert_ne!(RubricKind::Relevance, RubricKind::Safety);
        assert_ne!(RubricKind::Safety, RubricKind::PersonaAdherence);
    }
}
