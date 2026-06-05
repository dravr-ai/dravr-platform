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
