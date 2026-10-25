// ABOUTME: Chat conversation eval framework — multi-turn YAML scenarios with locale matrix + drift detection
// ABOUTME: Public API: load scenario, build driver, run, assert. See tests/scenarios/*.yaml
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Shared helpers in `tests/helpers/` compile into every integration-test
// binary, but only the chat_scenario targets consume this submodule — so
// every other binary sees these symbols as dead. This is the documented
// Rust pattern for scoped test helpers (Rust Book ch11.3) — the only
// alternative is to move the tree out of `helpers/`, which costs more
// than the lint silencing buys.
#![allow(dead_code, unused_imports)]

//! Conversation eval framework.
//!
//! Realizes the P1-P4 gap-analysis architecture from
//! `Claude Outputs/2026-05-18 Chat Conversation Test Gap Analysis.md`.
//! Scenarios live as YAML in `tests/scenarios/`; the test entry point
//! `tests/chat_scenario_test.rs` discovers them, executes each against
//! a driver, and asserts the run passes.
//!
//! Two layers:
//!
//! 1. **Format + Asserter catalog** ([`format`], [`asserters`]) — the
//!    YAML schema and the per-assertion dispatch. Adding a new
//!    assertion shape requires a variant in
//!    [`format::AssertionSpec`] + a dispatch arm in [`asserters`].
//! 2. **Runner + Driver** ([`runner`]) — orchestrates the turn loop,
//!    tracks aggregate claims across turns for drift detection, and
//!    returns a structured [`runner::ScenarioReport`].
//!
//! Vocabulary contracts ([`vocabulary_contract`]) and drift detection
//! ([`drift`]) ship as siblings so the same primitives can be used
//! from places other than the runner (e.g., the P3 Telegram-trace
//! replay harness).

pub(crate) mod asserters;
pub(crate) mod drift;
pub(crate) mod format;
pub(crate) mod live_driver;
pub(crate) mod reply_tap;
pub(crate) mod runner;
pub(crate) mod telegram_trace;
pub(crate) mod vocabulary_contract;

pub(crate) use format::{load as load_scenario, ChatScenario};
pub(crate) use live_driver::LiveScenarioDriver;
pub(crate) use runner::{run_scenario, MockScenarioDriver, TurnContext};
pub(crate) use telegram_trace::load_trace;
pub(crate) use vocabulary_contract::VocabularyContractRegistry;
