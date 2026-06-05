// ABOUTME: External tests for the aggregated eval summary report (report.rs)
// ABOUTME: Covers empty aggregation, turn-weighted mean, and rubric-kind distinctness
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::deterministic::DeterministicReport;
use pierre_evals::fixtures::Turn;
use pierre_evals::judge::{JudgeVerdict, RubricScore};
use pierre_evals::multi_turn::{MultiTurnReport, TurnResult};
use pierre_evals::report::{EvalSummary, RubricKind};

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
    let turn_results: Vec<TurnResult> = scores.iter().map(|(s, p)| turn_result(*s, *p)).collect();
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
