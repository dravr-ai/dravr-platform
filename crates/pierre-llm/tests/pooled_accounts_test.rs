// ABOUTME: Tests EmbacleProvider::pooled_accounts — the primary's further accounts read from
// ABOUTME: CLAUDE_CODE_OAUTH_TOKEN_2..N become named tiers, stopping at the first unset number
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One Claude account's quota must never dead-end a turn (carnet#480). The
//! platform reads each further account's token from a numbered variable and
//! builds it as its own tier, named `<runner>#N` so a health line, an alert
//! label or a usage row says which account served.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::env;
use std::sync::{Mutex, MutexGuard, PoisonError};

use embacle::CliRunnerType;
use pierre_llm::{EmbacleProvider, LlmProvider};

/// The tests in this file share process environment; each holds this for its
/// whole body, so they run one at a time.
static ENV: Mutex<()> = Mutex::new(());

fn exclusive() -> MutexGuard<'static, ()> {
    ENV.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Every variable this test touches, cleared before and after.
const VARS: [&str; 4] = [
    "CLAUDE_CODE_OAUTH_TOKEN_2",
    "CLAUDE_CODE_OAUTH_TOKEN_3",
    "CLAUDE_CODE_OAUTH_TOKEN_4",
    "CLI_LLM_BINARY",
];

fn clear() {
    for var in VARS {
        env::remove_var(var);
    }
}

fn set(var: &str, value: &str) {
    env::set_var(var, value);
}

#[test]
fn numbered_tokens_become_named_tiers_in_order() {
    let _env = exclusive();
    clear();
    set("CLI_LLM_BINARY", "/bin/sh");
    set("CLAUDE_CODE_OAUTH_TOKEN_2", "tok-2");
    set("CLAUDE_CODE_OAUTH_TOKEN_3", "tok-3");

    let accounts = EmbacleProvider::pooled_accounts(CliRunnerType::ClaudeCode, None);
    let names: Vec<&str> = accounts.iter().map(LlmProvider::name).collect();
    assert_eq!(names, ["claude-code#2", "claude-code#3"]);
    clear();
}

#[test]
fn the_first_gap_ends_the_pool() {
    let _env = exclusive();
    clear();
    set("CLI_LLM_BINARY", "/bin/sh");
    set("CLAUDE_CODE_OAUTH_TOKEN_2", "tok-2");
    set("CLAUDE_CODE_OAUTH_TOKEN_4", "tok-4");

    let accounts = EmbacleProvider::pooled_accounts(CliRunnerType::ClaudeCode, None);
    assert_eq!(accounts.len(), 1, "_4 behind an unset _3 is not read");
    clear();
}

#[test]
fn no_numbered_token_means_no_further_account() {
    let _env = exclusive();
    clear();
    set("CLI_LLM_BINARY", "/bin/sh");
    assert!(EmbacleProvider::pooled_accounts(CliRunnerType::ClaudeCode, None).is_empty());
    clear();
}

#[test]
fn a_runner_without_a_credential_variable_never_pools() {
    let _env = exclusive();
    clear();
    set("CLI_LLM_BINARY", "/bin/sh");
    set("CLAUDE_CODE_OAUTH_TOKEN_2", "tok-2");
    assert!(EmbacleProvider::pooled_accounts(CliRunnerType::WarpCli, None).is_empty());
    clear();
}
