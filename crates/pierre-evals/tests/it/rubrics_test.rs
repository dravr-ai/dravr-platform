// ABOUTME: External tests for the LLM-as-judge rubric definitions (rubrics.rs)
// ABOUTME: Checks the default rubric set and the relative safety threshold
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::rubrics::Rubric;

#[test]
fn defaults_returns_three_rubrics() {
    let r = Rubric::defaults();
    assert_eq!(r.len(), 3);
    assert!(r.iter().any(|x| x.name == "relevance"));
    assert!(r.iter().any(|x| x.name == "safety"));
    assert!(r.iter().any(|x| x.name == "persona_adherence"));
}

#[test]
fn safety_threshold_strictest() {
    assert!(Rubric::safety().passing_score >= Rubric::relevance().passing_score);
}
