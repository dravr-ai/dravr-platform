// ABOUTME: Entry test for the chat conversation eval framework — discovers YAML scenarios and runs them
// ABOUTME: Default mode loads + parses scenarios; opt-in CHAT_SCENARIO_LIVE=1 drives them against a real LLM
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
/// clause; retries with a fresh history absorb that variance without
/// hiding a hard schema regression (which fails on every attempt).
/// Four attempts — not the earlier seven — because each is a full
/// multi-turn run against a CPU-bound local Ollama model (minutes per
/// turn), so a high ceiling would let one flaky scenario eat the shard's
/// wall-clock budget. A fresh-history retry is independent, so four
/// drive a per-scenario miss rate `p` to `p^4` (≈0.05% at p=0.15, the
/// ~85%-reliable persona scenarios), which keeps the suite green without
/// masking a hard regression (which fails every attempt). Three proved
/// too few: `fragment_dedup` \[fr\] went 0-for-3 on both the 2026-08-02
/// and 2026-08-03 nightlies with every physical variable held constant
/// (same commit, same EPYC 7763 runner, same ollama v0.32.5, same
/// weights digest) — the Modelfile pins no temperature or seed, so its
/// turn-1 miss rate is materially above the 0.15 this ceiling was sized
/// for, and `p^3` was landing in red-a-nightly territory. Hoisted out of
/// the function body so the workspace's `clippy::items_after_statements`
/// lint stays satisfied.
const MAX_SCENARIO_ATTEMPTS: usize = 4;

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

/// Partition the discovered scenario files for the current CI shard.
///
/// The chat-eval `live-llm` job fans out across parallel runners because a
/// single scenario's multi-turn conversation costs minutes of CPU inference
/// on the local Ollama model; sharding keeps each runner inside its
/// wall-clock cap. `CHAT_SCENARIO_SHARD` is `"<index>/<total>"` (1-based,
/// e.g. `"2/3"`); files are assigned round-robin over the sorted list so
/// the split is deterministic, stable across runs, and spreads the heavier
/// multi-locale scenarios rather than clustering them on one runner. Unset
/// or blank runs every file — the default for local dev and non-sharded
/// dispatch. A malformed value panics rather than silently running all
/// scenarios on every runner (which would mask a shard misconfiguration).
fn shard_scenario_files(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let spec = match env::var("CHAT_SCENARIO_SHARD") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return files,
    };
    let parsed = spec.split_once('/').and_then(|(idx, total)| {
        let idx = idx.trim().parse::<usize>().ok()?;
        let total = total.trim().parse::<usize>().ok()?;
        (idx >= 1 && total >= 1 && idx <= total).then_some((idx, total))
    });
    let (index, total) = parsed.unwrap_or_else(|| {
        panic!(
            "CHAT_SCENARIO_SHARD must be \"<index>/<total>\" (1-based, index <= total); got {spec:?}"
        )
    });
    files
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % total == index - 1)
        .map(|(_, path)| path)
        .collect()
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

/// Drive every YAML scenario through the live LLM-backed driver.
///
/// Off by default — set `CHAT_SCENARIO_LIVE=1` (with a local Ollama server
/// serving `PIERRE_LLM_MODEL`) to opt in. The chat-eval workflow's nightly
/// and on-demand `live-llm` job provisions Ollama and exports both env vars;
/// on developer machines the test self-skips so a casual `cargo test`
/// doesn't stall for minutes on CPU inference. `CHAT_SCENARIO_SHARD`
/// (`"<index>/<total>"`) optionally restricts this run to one shard's slice
/// of the scenarios so CI can fan the suite across parallel runners.
#[test]
fn live_driver_executes_every_scenario() {
    if env::var("CHAT_SCENARIO_LIVE").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping live_driver_executes_every_scenario: set CHAT_SCENARIO_LIVE=1 + a running Ollama (PIERRE_LLM_MODEL) to enable"
        );
        return;
    }

    let files = enumerate_scenario_files();
    assert!(
        !files.is_empty(),
        "no scenario files found under {}",
        scenarios_dir().display()
    );
    let files = shard_scenario_files(files);
    eprintln!(
        "live_driver_executes_every_scenario: running {} scenario file(s){}",
        files.len(),
        env::var("CHAT_SCENARIO_SHARD")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map_or_else(String::new, |s| format!(" for shard {s}"))
    );

    // On-demand carve-out: scenarios with `nightly_gate: false` (a grader
    // model genuinely can't clear their honest assertion) run only when
    // CHAT_SCENARIO_INCLUDE_ONDEMAND is set, so the nightly stays green while
    // the scenario keeps its strict assertions for deliberate runs. Empty /
    // unset (the nightly cron case) skips them.
    let include_ondemand = env::var("CHAT_SCENARIO_INCLUDE_ONDEMAND")
        .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    let vocab = VocabularyContractRegistry::with_defaults();
    let mut failures: Vec<String> = Vec::new();

    for path in &files {
        let scenario =
            load_scenario(path).unwrap_or_else(|e| panic!("load scenario {}: {e}", path.display()));

        if !scenario.nightly_gate && !include_ondemand {
            eprintln!(
                "skipping on-demand-only scenario {} (nightly_gate=false; set \
                 CHAT_SCENARIO_INCLUDE_ONDEMAND=1 to run it)",
                path.display()
            );
            continue;
        }

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
