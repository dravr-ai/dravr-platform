// ABOUTME: External tests for the multi-turn sliding-window evaluator (multi_turn.rs)
// ABOUTME: Verifies per-turn scoring and that the sliding-window size is respected
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::fixtures::{GoldenCase, Turn};
use pierre_evals::judge::{JudgeVerdict, RubricScore};
use pierre_evals::multi_turn::{MultiTurnEvaluator, MultiTurnReport};

fn case_with(turns: Vec<Turn>) -> GoldenCase {
    GoldenCase {
        id: "c1".into(),
        label: "test".into(),
        persona: "marathon_coach".into(),
        turns,
    }
}

fn turn(text: &str) -> Turn {
    Turn {
        user: "u".into(),
        expected_coach: text.into(),
        must_contain: vec![],
        must_not_contain: vec![],
    }
}

fn perfect_verdict() -> JudgeVerdict {
    JudgeVerdict {
        rubrics: vec![
            RubricScore {
                rubric_name: "relevance".into(),
                score: 5,
                rationale: "perfect".into(),
                passed: true,
            },
            RubricScore {
                rubric_name: "safety".into(),
                score: 5,
                rationale: "safe".into(),
                passed: true,
            },
        ],
    }
}

#[test]
fn run_scores_every_turn() {
    let case = case_with(vec![turn("hello"), turn("world"), turn("how are you?")]);
    let evaluator = MultiTurnEvaluator::default();
    let report: MultiTurnReport = evaluator.run(&case, |_idx, _window| perfect_verdict());
    assert_eq!(report.case_id, "c1");
    assert_eq!(report.turn_results.len(), 3);
    assert!((report.mean_score - 5.0).abs() < f32::EPSILON);
    assert_eq!(report.fully_passing_turns, 3);
}

#[test]
fn sliding_window_size_respected() {
    let case = case_with((0..6).map(|i| turn(&format!("t{i}"))).collect());
    let evaluator = MultiTurnEvaluator {
        window_size: 3,
        max_chars: 5_000,
    };
    let mut sizes = Vec::new();
    evaluator.run(&case, |_idx, window| {
        sizes.push(window.len());
        perfect_verdict()
    });
    // Indices 0,1 see windows of 1 and 2, then everything else sees 3.
    assert_eq!(sizes, vec![1, 2, 3, 3, 3, 3]);
}
