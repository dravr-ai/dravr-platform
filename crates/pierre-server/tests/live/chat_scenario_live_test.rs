// ABOUTME: Live half of the chat conversation eval — drives every YAML scenario against a real LLM
// ABOUTME: Built only with the live-e2e feature; needs a running Ollama serving PIERRE_LLM_MODEL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live execution of `tests/scenarios/*.yaml`.
//!
//! Each scenario runs against a live LLM-backed fixture
//! (`helpers::chat_scenario::live_driver`), which wires the canonical
//! `PIERRE_SYSTEM_PROMPT`, the registered tool catalog, and an in-memory
//! provider store seeded from `provider_state`. The chat-eval workflow's
//! nightly and on-demand `live-llm` job provisions a local Ollama and runs
//! this target as the drift detector; the offline structural pass is
//! `tests/chat_scenario_test.rs`.
//!
//! ## Running
//!
//! A live test: it is built only with the `live-e2e` feature, so a plain
//! `cargo test` never compiles it. Built, it never skips (carnet#805): an
//! unset `PIERRE_LLM_MODEL`, an unreachable Ollama, or a selection that
//! executes zero scenarios fails the test.
//!
//! ```sh
//! PIERRE_LLM_MODEL=dravr-chateval \
//!   cargo test --features live-e2e --test chat_scenario_live_test -- --nocapture
//! ```
//!
//! `CHAT_SCENARIO_SHARD` (`"<index>/<total>"`) restricts the run to one
//! shard's slice, and `CHAT_SCENARIO_INCLUDE_ONDEMAND=1` adds the
//! `nightly_gate: false` scenarios. Both are selection, not skips.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[path = "../common.rs"]
mod common;
#[path = "../helpers/mod.rs"]
mod helpers;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::thread::sleep as thread_sleep;
use std::time::Duration;

use helpers::chat_scenario::{
    live_driver::RealExecution, load_scenario, run_scenario, LiveScenarioDriver,
    VocabularyContractRegistry,
};
use tokio::runtime::Builder as TokioRuntimeBuilder;

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

/// Seed an athlete on a real server for a scenario that grades written state.
///
/// Built fresh per attempt rather than shared: a retry must start from an
/// athlete with no plan, or the second attempt grades rows the first one
/// wrote and a model that saved nothing passes on its predecessor's work.
///
/// Synchronous because the live-scenario test is a plain `#[test]` — the
/// driver already spins a nested runtime per turn for the same reason.
fn real_execution_fixture() -> Result<RealExecution, String> {
    let rt = TokioRuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("build runtime: {e}"))?;
    rt.block_on(async {
        common::init_server_config();
        common::init_test_http_clients();
        let resources = common::create_test_server_resources()
            .await
            .map_err(|e| format!("server resources: {e}"))?;
        let (user_id, _user) = common::create_test_user(&resources.agent.database)
            .await
            .map_err(|e| format!("test user: {e}"))?;
        let tenant_id = resources
            .common
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| format!("tenants: {e}"))?
            .first()
            .ok_or_else(|| "the seeded athlete owns no tenant".to_owned())?
            .id;
        Ok(RealExecution {
            resources,
            user_id,
            tenant_id,
        })
    })
}

/// Resolve the `tests/scenarios/` directory at compile time so the test
/// works regardless of `cargo test` invocation directory.
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

/// Partition the selected scenarios for the current CI shard.
///
/// The chat-eval `live-llm` job fans out across parallel runners because a
/// single scenario's multi-turn conversation costs minutes of CPU inference
/// on the local Ollama model; sharding keeps each runner inside its
/// wall-clock cap. `CHAT_SCENARIO_SHARD` is `"<index>/<total>"` (1-based,
/// e.g. `"2/3"`); scenarios are assigned round-robin over the sorted list so
/// the split is deterministic, stable across runs, and spreads the heavier
/// multi-locale scenarios rather than clustering them on one runner. Unset
/// or blank runs every scenario — the default for local dev and non-sharded
/// dispatch. A malformed value panics rather than silently running all
/// scenarios on every runner (which would mask a shard misconfiguration).
///
/// It partitions what the on-demand carve-out already SELECTED, not the raw
/// file list: sharding the files first left nightly shards holding only
/// `nightly_gate: false` scenarios, which then graded nothing and passed.
fn shard_selected<T>(selected: Vec<T>) -> Vec<T> {
    let spec = match env::var("CHAT_SCENARIO_SHARD") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return selected,
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
    selected
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % total == index - 1)
        .map(|(_, item)| item)
        .collect()
}

/// Drive every YAML scenario through the live LLM-backed driver.
///
/// Needs a local Ollama server serving `PIERRE_LLM_MODEL`; the chat-eval
/// workflow's nightly and on-demand `live-llm` job provisions both.
/// `CHAT_SCENARIO_SHARD` (`"<index>/<total>"`) optionally restricts this run
/// to one shard's slice of the selected scenarios so CI can fan the suite
/// across parallel runners. A run that executes zero scenarios fails.
#[test]
fn live_driver_executes_every_scenario() {
    // Fail before any scenario is loaded: a missing model name is a
    // provisioning error, and naming it up front beats a panic deep in the
    // first scenario's driver construction.
    let model = env::var("PIERRE_LLM_MODEL").unwrap_or_default();
    assert!(
        !model.trim().is_empty(),
        "PIERRE_LLM_MODEL must name the model a running Ollama serves (e.g. dravr-chateval)"
    );

    let files = enumerate_scenario_files();
    assert!(
        !files.is_empty(),
        "no scenario files found under {}",
        scenarios_dir().display()
    );
    let discovered = files.len();

    // On-demand carve-out: scenarios with `nightly_gate: false` (a grader
    // model genuinely can't clear their honest assertion) run only when
    // CHAT_SCENARIO_INCLUDE_ONDEMAND is set, so the nightly stays green while
    // the scenario keeps its strict assertions for deliberate runs. Empty /
    // unset (the nightly cron case) leaves them out of the selection.
    let include_ondemand = env::var("CHAT_SCENARIO_INCLUDE_ONDEMAND")
        .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let mut selected = Vec::new();
    for path in files {
        let scenario = load_scenario(&path)
            .unwrap_or_else(|e| panic!("load scenario {}: {e}", path.display()));
        if !scenario.nightly_gate && !include_ondemand {
            eprintln!(
                "not selected: on-demand-only scenario {} (nightly_gate=false; set \
                 CHAT_SCENARIO_INCLUDE_ONDEMAND=1 to run it)",
                path.display()
            );
            continue;
        }
        selected.push((path, scenario));
    }
    let selected_total = selected.len();
    let selected = shard_selected(selected);
    eprintln!(
        "live_driver_executes_every_scenario: running {} of {selected_total} selected \
         scenario(s) ({discovered} discovered){}",
        selected.len(),
        env::var("CHAT_SCENARIO_SHARD")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map_or_else(String::new, |s| format!(" for shard {s}"))
    );
    assert!(
        !selected.is_empty(),
        "this run selected zero of {discovered} scenario file(s) ({selected_total} before \
         sharding): a live run that grades nothing must not pass — check \
         CHAT_SCENARIO_SHARD against the number of selected scenarios, and \
         CHAT_SCENARIO_INCLUDE_ONDEMAND"
    );

    let vocab = VocabularyContractRegistry::with_defaults();
    let mut failures: Vec<String> = Vec::new();

    for (path, scenario) in &selected {
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
            // A scenario grading written state gets a real platform: tools
            // that write commit, and the runner reads the rows back after
            // each turn. Built per attempt so a retry starts from a clean
            // athlete rather than inheriting the previous attempt's plan.
            if scenario.real_execution {
                driver = driver.with_real_execution(real_execution_fixture().unwrap_or_else(|e| {
                    panic!("seed a real platform for {}: {e}", path.display())
                }));
            }
            driver.reset_history();
            let reports = run_scenario(scenario, &mut driver, &vocab);
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
