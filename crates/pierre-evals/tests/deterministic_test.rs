// ABOUTME: External tests for the Layer 1 deterministic checks (deterministic.rs)
// ABOUTME: Covers empty/missing/forbidden/too-long failures and the all-pass path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::deterministic::{DeterministicCheck, DeterministicReport};
use pierre_evals::fixtures::Turn;

fn turn(must: Vec<&str>, must_not: Vec<&str>) -> Turn {
    Turn {
        user: String::new(),
        expected_coach: String::new(),
        must_contain: must.into_iter().map(str::to_owned).collect(),
        must_not_contain: must_not.into_iter().map(str::to_owned).collect(),
    }
}

#[test]
fn empty_response_fails() {
    let report = DeterministicReport::run(&turn(vec![], vec![]), "", 5_000);
    assert_eq!(report.checks, vec![DeterministicCheck::Empty]);
    assert!(!report.all_passed());
}

#[test]
fn missing_required_substring_fails() {
    let t = turn(vec!["disclaimer"], vec![]);
    let report = DeterministicReport::run(&t, "your training looks fine", 5_000);
    assert!(matches!(
        report.checks[0],
        DeterministicCheck::MissingRequired { .. }
    ));
    assert!(!report.all_passed());
}

#[test]
fn forbidden_substring_fails() {
    let t = turn(vec![], vec!["definitely cured"]);
    let report = DeterministicReport::run(&t, "you are definitely cured", 5_000);
    assert!(matches!(
        report.checks[0],
        DeterministicCheck::ForbiddenPresent { .. }
    ));
}

#[test]
fn too_long_fails() {
    let t = turn(vec![], vec![]);
    let response = "x".repeat(10);
    let report = DeterministicReport::run(&t, &response, 5);
    assert!(matches!(
        report.checks[0],
        DeterministicCheck::TooLong { .. }
    ));
}

#[test]
fn all_pass() {
    let t = turn(vec!["plan"], vec!["never"]);
    let report = DeterministicReport::run(&t, "Here is your training plan.", 5_000);
    assert_eq!(report.checks, vec![DeterministicCheck::Ok]);
    assert!(report.all_passed());
    assert_eq!(report.failure_count(), 0);
}
