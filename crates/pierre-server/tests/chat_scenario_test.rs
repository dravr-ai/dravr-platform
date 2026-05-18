// ABOUTME: Entry test for the chat conversation eval framework — discovers YAML scenarios and runs them
// ABOUTME: Default mode loads + parses scenarios; opt-in CHAT_SCENARIO_LIVE=1 routes through the real chat pipeline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Discovery + execution test for `tests/scenarios/*.yaml`.
//!
//! Two execution modes:
//!
//! - **Default (no env var):** every scenario file is loaded and
//!   parsed; structural invariants are asserted (at least one locale,
//!   at least one turn, assertions reference known asserter kinds).
//!   Fast, deterministic, runs in CI on every push.
//! - **`CHAT_SCENARIO_LIVE=1`:** scenarios are executed against a live
//!   chat-pipeline fixture using a real LLM. Cost-bounded, gated to
//!   nightly + on-demand `workflow_dispatch` in the eval workflow.
//!   The live driver lives in `helpers::chat_scenario::live_driver`
//!   (P3 follow-up — for this PR the mode is opt-in but the runner
//!   still walks every YAML so the framework gates itself against
//!   format drift).
//!
//! The default-mode pass is the per-push gate; the live-mode pass is
//! the nightly drift detector.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

use std::env;
use std::fs;
use std::path::PathBuf;

use helpers::chat_scenario::{
    format::{AssertionSpec, ProviderState},
    load_scenario, load_trace, run_scenario, ChatScenario, MockScenarioDriver,
    VocabularyContractRegistry,
};

/// Resolve the `tests/scenarios/` directory relative to this file at
/// compile time so the test works regardless of `cargo test` invocation
/// directory.
fn scenarios_dir() -> PathBuf {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(crate_dir).join("tests").join("scenarios")
}

fn enumerate_scenario_files() -> Vec<PathBuf> {
    let dir = scenarios_dir();
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read scenarios dir {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
        .collect();
    files.sort();
    files
}

#[test]
fn every_scenario_file_parses_and_meets_structural_invariants() {
    let files = enumerate_scenario_files();
    assert!(
        !files.is_empty(),
        "no scenario files found under {}",
        scenarios_dir().display()
    );
    let mut failures: Vec<String> = Vec::new();
    for path in files {
        match load_scenario(&path) {
            Ok(s) => {
                if let Err(e) = check_invariants(&s) {
                    failures.push(format!("{}: {e}", path.display()));
                }
            }
            Err(e) => failures.push(format!("{}: load failed: {e}", path.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "scenario validation:\n{}",
        failures.join("\n")
    );
}

fn enumerate_trace_files() -> Vec<PathBuf> {
    let dir = scenarios_dir().join("telegram_traces");
    if !dir.exists() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read telegram_traces dir {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
}

#[test]
fn every_telegram_trace_parses_and_projects_to_a_valid_scenario() {
    let files = enumerate_trace_files();
    assert!(
        !files.is_empty(),
        "no telegram traces under {}",
        scenarios_dir().join("telegram_traces").display()
    );
    let mut failures: Vec<String> = Vec::new();
    for path in files {
        match load_trace(&path) {
            Ok(s) => {
                if let Err(e) = check_invariants(&s) {
                    failures.push(format!("{}: {e}", path.display()));
                }
            }
            Err(e) => failures.push(format!("{}: load failed: {e}", path.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "telegram trace validation:\n{}",
        failures.join("\n")
    );
}

fn check_invariants(s: &ChatScenario) -> Result<(), String> {
    if s.name.trim().is_empty() {
        return Err("name is empty".to_owned());
    }
    if s.locales.is_empty() {
        return Err("locales is empty (omit field to default to [\"en\"])".to_owned());
    }
    if s.turns.is_empty() {
        return Err("scenario has no turns".to_owned());
    }
    for (i, t) in s.turns.iter().enumerate() {
        if t.user.trim().is_empty() {
            return Err(format!("turn {}: user message is empty", i + 1));
        }
        for (j, a) in t.assertions.iter().enumerate() {
            check_assertion(a).map_err(|e| format!("turn {} assertion {}: {e}", i + 1, j + 1))?;
        }
    }
    Ok(())
}

fn check_assertion(spec: &AssertionSpec) -> Result<(), String> {
    match spec {
        AssertionSpec::ReplyContains { value } if value.trim().is_empty() => {
            Err("reply_contains value is empty".to_owned())
        }
        AssertionSpec::NoSubstring { values } if values.is_empty() => {
            Err("no_substring values list is empty".to_owned())
        }
        AssertionSpec::AnyOf { values } if values.is_empty() => {
            Err("any_of values list is empty".to_owned())
        }
        AssertionSpec::ToolCalled { name, .. } if name.trim().is_empty() => {
            Err("tool_called name is empty".to_owned())
        }
        AssertionSpec::VocabularyContract { coach_id } if coach_id.trim().is_empty() => {
            Err("vocabulary_contract coach_id is empty".to_owned())
        }
        AssertionSpec::DistanceMentioned { tolerance_km, .. } if *tolerance_km < 0.0 => {
            Err("distance_mentioned tolerance_km must be >= 0".to_owned())
        }
        _ => Ok(()),
    }
}

/// Smoke-test the runner against the mock driver to prove the
/// framework wiring works. Live execution against the real chat
/// pipeline lands in P3 and runs only when `CHAT_SCENARIO_LIVE=1`.
#[test]
fn runner_executes_a_scenario_against_the_mock_driver() {
    let scenario = ChatScenario {
        name: "Mock-driver smoke".to_owned(),
        locales: vec!["en".to_owned()],
        notes: String::new(),
        provider_state: ProviderState::default(),
        turns: vec![helpers::chat_scenario::format::TurnSpec {
            user: "ping".to_owned(),
            trigger_sync_before_turn: false,
            assertions: vec![AssertionSpec::ReplyContains {
                value: "pong".to_owned(),
            }],
        }],
    };
    let mut driver = MockScenarioDriver::new(vec!["pong!".to_owned()], vec![vec![]]);
    let vocab = VocabularyContractRegistry::with_defaults();
    let reports = run_scenario(&scenario, &mut driver, &vocab);
    assert_eq!(reports.len(), 1);
    assert!(reports[0].passed(), "{}", reports[0].failure_summary());
}

#[test]
#[ignore = "opt-in live driver lands in P3"]
fn live_driver_executes_every_scenario() {
    if env::var("CHAT_SCENARIO_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    // P3 follow-up: boot the test server fixture, wire a
    // LiveScenarioDriver, execute every YAML scenario against the
    // real chat pipeline using the configured LLM.
    panic!("CHAT_SCENARIO_LIVE=1 set but LiveScenarioDriver is not yet implemented");
}
