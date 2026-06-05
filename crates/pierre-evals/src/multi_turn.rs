// ABOUTME: Multi-turn evaluator — slides a 3–5 turn window across a fixture, scores each turn
// ABOUTME: Layer 3 of the eval harness; aggregates per-turn rubric verdicts into a fixture report
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Multi-Turn Sliding-Window Evaluator
//!
//! Per the gist (Part 2: Turn-by-turn conversational testing), each turn is
//! scored in the context of a bounded sliding window of recent turns rather
//! than the full transcript. This keeps the LLM judge cost predictable and
//! its judgments more reliable.
//!
//! The evaluator is **deliberately deterministic** — it does not call the
//! LLM provider itself. Callers pass a closure that resolves a
//! `(turn_index, window) -> JudgeVerdict` so unit tests can substitute a
//! synthetic judge and CI can plug in a real one.

use serde::{Deserialize, Serialize};

use crate::deterministic::DeterministicReport;
use crate::fixtures::{GoldenCase, Turn};
use crate::judge::JudgeVerdict;

/// Aggregated outcome for a single multi-turn fixture run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiTurnReport {
    /// Identifier of the case this report belongs to.
    pub case_id: String,
    /// Per-turn verdicts in conversation order.
    pub turn_results: Vec<TurnResult>,
    /// Average judge score across all turns.
    pub mean_score: f32,
    /// Number of turns where every rubric passed and deterministic checks were green.
    pub fully_passing_turns: usize,
}

/// Outcome for one turn within a multi-turn run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResult {
    /// Turn index inside the case.
    pub turn_index: usize,
    /// Layer 1 deterministic results.
    pub deterministic: DeterministicReport,
    /// Layer 2 LLM-judge verdict.
    pub verdict: JudgeVerdict,
}

/// Multi-turn evaluator parameters.
#[derive(Debug, Clone, Copy)]
pub struct MultiTurnEvaluator {
    /// Sliding window size (recommended 3–5 turns).
    pub window_size: usize,
    /// Character cap used by [`DeterministicReport`].
    pub max_chars: usize,
}

impl Default for MultiTurnEvaluator {
    fn default() -> Self {
        Self {
            window_size: 4,
            max_chars: 5_000,
        }
    }
}

impl MultiTurnEvaluator {
    /// Run the evaluator against one case.
    ///
    /// `judge_fn` receives the turn index plus the most recent `window_size`
    /// turns (including the one being scored) and must return a
    /// [`JudgeVerdict`] for the actual reply at that turn.
    pub fn run<F>(&self, case: &GoldenCase, mut judge_fn: F) -> MultiTurnReport
    where
        F: FnMut(usize, &[Turn]) -> JudgeVerdict,
    {
        let mut turn_results = Vec::with_capacity(case.turns.len());
        let mut total_score = 0.0_f32;
        let mut fully_passing = 0_usize;

        for (idx, turn) in case.turns.iter().enumerate() {
            let start = idx.saturating_sub(self.window_size.saturating_sub(1));
            let window = &case.turns[start..=idx];
            let verdict = judge_fn(idx, window);
            let det = DeterministicReport::run(turn, &turn.expected_coach, self.max_chars);

            total_score += verdict.mean_score();
            if verdict.all_passed() && det.all_passed() {
                fully_passing += 1;
            }
            turn_results.push(TurnResult {
                turn_index: idx,
                deterministic: det,
                verdict,
            });
        }

        let mean = if turn_results.is_empty() {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            {
                total_score / turn_results.len() as f32
            }
        };

        MultiTurnReport {
            case_id: case.id.clone(),
            turn_results,
            mean_score: mean,
            fully_passing_turns: fully_passing,
        }
    }
}
