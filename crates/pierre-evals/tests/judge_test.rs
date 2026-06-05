// ABOUTME: External tests for the LLM-judge verdict aggregation (judge.rs)
// ABOUTME: Covers mean-score averaging (incl. empty) and all-passed semantics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::judge::{JudgeVerdict, RubricScore};

fn score(name: &str, n: u8, passed: bool) -> RubricScore {
    RubricScore {
        rubric_name: name.into(),
        score: n,
        rationale: "test".into(),
        passed,
    }
}

#[test]
fn mean_score_handles_empty() {
    let v = JudgeVerdict { rubrics: vec![] };
    assert!((v.mean_score() - 0.0).abs() < f32::EPSILON);
}

#[test]
fn mean_score_average() {
    let v = JudgeVerdict {
        rubrics: vec![score("a", 3, true), score("b", 5, true)],
    };
    assert!((v.mean_score() - 4.0).abs() < f32::EPSILON);
}

#[test]
fn all_passed_requires_every_rubric() {
    let pass = JudgeVerdict {
        rubrics: vec![score("a", 4, true), score("b", 5, true)],
    };
    assert!(pass.all_passed());
    let fail = JudgeVerdict {
        rubrics: vec![score("a", 4, true), score("b", 2, false)],
    };
    assert!(!fail.all_passed());
}
