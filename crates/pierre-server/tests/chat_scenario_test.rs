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
//!   LLM-backed fixture (`helpers::chat_scenario::live_driver`). Wires
//!   the canonical `PIERRE_SYSTEM_PROMPT`, the registered tool catalog,
//!   and an in-memory provider store seeded from `provider_state`.
//!   Cost-bounded, gated to nightly + on-demand `workflow_dispatch` in
//!   the eval workflow; locally a missing env self-skips the test.
//!
//! The default-mode pass is the per-push gate; the live-mode pass is
//! the nightly drift detector.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::thread::sleep as thread_sleep;
use std::time::Duration;

/// Scenario-level retry budget for the live driver. Real LLMs are
/// non-deterministic and occasionally drop a digit on multi-step
/// arithmetic or pick a sibling phrasing that misses an `any_of`
/// clause; one retry with a fresh history absorbs the variance
/// without hiding a hard schema regression (which would fail on every
/// attempt). Hoisted out of the function body so the workspace's
/// `clippy::items_after_statements` lint stays satisfied.
const MAX_SCENARIO_ATTEMPTS: usize = 2;

use helpers::chat_scenario::{
    format::{AssertionSpec, ProviderState},
    load_scenario, load_trace, run_scenario, ChatScenario, LiveScenarioDriver, MockScenarioDriver,
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
/// framework wiring works. The companion `live_driver_executes_every_scenario`
/// test below exercises the same runner against a real LLM when
/// `CHAT_SCENARIO_LIVE=1` is set.
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

/// Drive every YAML scenario through the live LLM-backed driver.
///
/// Off by default — set `CHAT_SCENARIO_LIVE=1` (and supply a
/// `GEMINI_API_KEY`) to opt in. The chat-eval workflow's nightly +
/// on-demand `live-llm` job exports both env vars; on developer
/// machines the test self-skips so a casual `cargo test` doesn't
/// blow through someone's Gemini quota.
#[test]
fn live_driver_executes_every_scenario() {
    if env::var("CHAT_SCENARIO_LIVE").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping live_driver_executes_every_scenario: set CHAT_SCENARIO_LIVE=1 + GEMINI_API_KEY to enable"
        );
        return;
    }

    let files = enumerate_scenario_files();
    assert!(
        !files.is_empty(),
        "no scenario files found under {}",
        scenarios_dir().display()
    );

    let vocab = VocabularyContractRegistry::with_defaults();
    let mut failures: Vec<String> = Vec::new();

    for (idx, path) in files.iter().enumerate() {
        // Pace the run: Gemini free tier is ~10 RPM. Five scenarios with
        // 2-3 turns + tool loop each easily bursts past that, and 429s
        // bleed into retry storms that swallow real signal. A 6 s spacer
        // between scenarios keeps us under the budget without the live
        // driver itself having to track wall-clock state.
        if idx > 0 {
            thread_sleep(Duration::from_secs(6));
        }
        let scenario =
            load_scenario(path).unwrap_or_else(|e| panic!("load scenario {}: {e}", path.display()));

        let mut last_attempt_failures: Vec<String> = Vec::new();
        let mut scenario_passed = false;
        for attempt in 0..MAX_SCENARIO_ATTEMPTS {
            if attempt > 0 {
                eprintln!(
                    "scenario {} did not pass on attempt {}; retrying with fresh driver state",
                    path.display(),
                    attempt - 1
                );
                thread_sleep(Duration::from_secs(6));
            }
            let mut driver = LiveScenarioDriver::from_env().unwrap_or_else(|e| {
                panic!("LiveScenarioDriver::from_env failed: {e}");
            });
            driver.reset_history();
            let reports = run_scenario(&scenario, &mut driver, &vocab);
            let attempt_failures: Vec<String> = reports
                .iter()
                .filter(|r| !r.passed())
                .map(|r| format!("{}: {}", path.display(), r.failure_summary()))
                .collect();
            if attempt_failures.is_empty() {
                scenario_passed = true;
                break;
            }
            last_attempt_failures = attempt_failures;
        }
        if !scenario_passed {
            failures.extend(last_attempt_failures);
        }
    }

    assert!(
        failures.is_empty(),
        "live scenario failures (after {MAX_SCENARIO_ATTEMPTS} attempts each):\n{}",
        failures.join("\n\n")
    );
}
