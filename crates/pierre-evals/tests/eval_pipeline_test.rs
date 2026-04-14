// ABOUTME: Integration test for the pierre-evals harness — loads fixtures, runs deterministic + multi-turn layers
// ABOUTME: Uses a synthetic judge so the test never makes outbound LLM calls and can run in offline CI
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_evals::deterministic::DeterministicReport;
use pierre_evals::fixtures::GoldenFixture;
use pierre_evals::judge::{JudgeVerdict, RubricScore};
use pierre_evals::multi_turn::MultiTurnEvaluator;
use pierre_evals::report::EvalSummary;
use pierre_evals::rubrics::Rubric;

fn synthetic_perfect_verdict(rubrics: &[Rubric]) -> JudgeVerdict {
    JudgeVerdict {
        rubrics: rubrics
            .iter()
            .map(|r| RubricScore {
                rubric_name: r.name.clone(),
                score: 5,
                rationale: "synthetic perfect".to_owned(),
                passed: true,
            })
            .collect(),
    }
}

#[test]
fn injury_triage_fixture_loads_and_evaluates() {
    let fixture = GoldenFixture::load_jsonl(format!(
        "{}/fixtures/injury_triage.jsonl",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture should load");

    assert!(
        !fixture.cases.is_empty(),
        "injury_triage fixture should have cases"
    );

    let rubrics = Rubric::defaults();
    let evaluator = MultiTurnEvaluator::default();
    let mut reports = Vec::new();

    for case in &fixture.cases {
        let report = evaluator.run(case, |_idx, _window| synthetic_perfect_verdict(&rubrics));
        // Synthetic judge always passes; deterministic checks should also
        // pass because the fixture's expected_coach contains the must_contain
        // substrings by construction.
        for tr in &report.turn_results {
            assert!(
                tr.deterministic.all_passed(),
                "deterministic checks should pass on the fixture's own expected reply: {:?}",
                tr.deterministic.checks
            );
            assert!(tr.verdict.all_passed());
        }
        reports.push(report);
    }

    let summary = EvalSummary::aggregate(&reports);
    assert_eq!(summary.case_count, fixture.cases.len());
    assert!(summary.turn_count > 0);
    assert_eq!(summary.fully_passing_turns, summary.turn_count);
    assert!((summary.mean_score - 5.0).abs() < f32::EPSILON);
}

#[test]
fn deterministic_layer_catches_missing_disclaimer() {
    let fixture = GoldenFixture::load_jsonl(format!(
        "{}/fixtures/injury_triage.jsonl",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture should load");

    // First turn of injury_002 requires "physiotherapist" — drop it and the
    // deterministic layer should flag it.
    let case = fixture
        .cases
        .iter()
        .find(|c| c.id == "injury_002")
        .expect("injury_002 case present");
    let turn = &case.turns[0];

    let bad_response = "Just rest a few days, you'll be fine.";
    let report = DeterministicReport::run(turn, bad_response, 5_000);
    assert!(!report.all_passed());
    assert!(report.failure_count() >= 1);
}
