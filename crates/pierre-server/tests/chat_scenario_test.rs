// ABOUTME: Entry test for the chat conversation eval framework — discovers YAML scenarios and runs them
// ABOUTME: Offline half: loads + parses scenarios and smoke-tests the runner against the mock driver
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Discovery + offline validation for `tests/scenarios/*.yaml`.
//!
//! Every scenario file and Telegram trace is loaded and parsed;
//! structural invariants are asserted (at least one locale, at least
//! one turn, assertions reference known asserter kinds), and the runner
//! is smoke-tested against the mock driver. Fast, deterministic, runs
//! in CI on every push.
//!
//! Executing the scenarios against a real LLM is the live half,
//! `tests/live/chat_scenario_live_test.rs`, built only with the
//! `live-e2e` feature — the per-push gate lives here, the nightly drift
//! detector there.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

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
            // An assertion that grades written state needs somewhere to write.
            // Caught here, at parse time, rather than at run time: the
            // asserter does fail without a platform, but a scenario that can
            // never pass should be rejected when it is read, not after an LLM
            // has been paid for.
            if matches!(a, AssertionSpec::PlanWeekWritten { .. }) && !s.real_execution {
                return Err(format!(
                    "turn {} assertion {}: plan_week_written grades what the turn \
                     SAVED, which requires `real_execution: true` on the scenario",
                    i + 1,
                    j + 1
                ));
            }
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
        AssertionSpec::VocabularyContract { agent_id } if agent_id.trim().is_empty() => {
            Err("vocabulary_contract coach_id is empty".to_owned())
        }
        AssertionSpec::DistanceMentioned { tolerance_km, .. } if *tolerance_km < 0.0 => {
            Err("distance_mentioned tolerance_km must be >= 0".to_owned())
        }
        // Every turn is already graded on being written in its run locale, so
        // a bare `reply_language` asserts what the runner asserts anyway. It
        // is rejected rather than ignored because writing it means believing
        // the check is opt-in, and that belief is what let
        // `intent_cote_course_combien_fr` turn 1 ship with `assertions: []`
        // and a licence to answer in English (carnet#159, carnet#162).
        AssertionSpec::ReplyLanguage { locale: None } => Err(
            "reply_language without a locale: restates the automatic per-turn check — delete it. \
             Name a locale: only to assert a language OTHER than the run's, or set \
             skip_language_check: true on the turn if its reply cannot be judged"
                .to_owned(),
        ),
        AssertionSpec::ReplyLanguage { locale: Some(l) } if l.trim().is_empty() => {
            Err("reply_language locale is empty".to_owned())
        }
        _ => Ok(()),
    }
}

/// The redundant-assertion guard must actually fire.
///
/// A guard that only ever sees valid input passes vacuously. Feed it the
/// exact shape it exists to reject — a `reply_language` with no locale,
/// which is what an author writes when they think the check is opt-in.
#[test]
fn a_bare_reply_language_assertion_is_rejected() {
    let err = check_assertion(&AssertionSpec::ReplyLanguage { locale: None })
        .expect_err("a bare reply_language must be rejected");
    assert!(
        err.contains("automatic per-turn check"),
        "the message must tell the author why it is redundant: {err}"
    );
    assert!(
        err.contains("skip_language_check"),
        "the message must name the real escape hatch: {err}"
    );

    // The override form stays legal — that is the only reason to write one.
    assert!(check_assertion(&AssertionSpec::ReplyLanguage {
        locale: Some("en".to_owned()),
    })
    .is_ok());
}

/// Smoke-test the runner against the mock driver to prove the
/// framework wiring works. The companion `live_driver_executes_every_scenario`
/// test in `tests/live/chat_scenario_live_test.rs` exercises the same runner
/// against a real LLM.
#[test]
fn runner_executes_a_scenario_against_the_mock_driver() {
    let scenario = ChatScenario {
        real_execution: false,
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
            skip_language_check: false,
        }],
        skip_drift: false,
        nightly_gate: true,
        current_date: None,
    };
    // English prose, not a bare "pong!": every turn is language-checked
    // against its run locale, and whatlang does not return a reliable verdict
    // until roughly 130 characters. A fixture shorter than that fails on the
    // detector rather than on the wiring this test is here to prove.
    let mut driver = MockScenarioDriver::new(
        vec![
            "Pong! The server is up and answering normally, dispatching this turn \
              through the mock driver and returning a canned reply long enough to \
              read a language from."
                .to_owned(),
        ],
        vec![vec![]],
    );
    let vocab = VocabularyContractRegistry::with_defaults();
    let reports = run_scenario(&scenario, &mut driver, &vocab);
    assert_eq!(reports.len(), 1);
    assert!(reports[0].passed(), "{}", reports[0].failure_summary());
}
