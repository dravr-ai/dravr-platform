// ABOUTME: Tests EmbacleProvider::pooled_accounts — a tier's further accounts read from CLAUDE_CODE_OAUTH_TOKEN_N
// ABOUTME: or ANTHROPIC_API_KEY_N become named tiers, stopping at the first unset number
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

use pierre_llm::config::{ChainEntry, LlmChain, LlmProviderType};
use pierre_llm::{EmbacleProvider, LlmProvider};

/// The tests in this file share process environment; each holds this for its
/// whole body, so they run one at a time.
static ENV: Mutex<()> = Mutex::new(());

fn exclusive() -> MutexGuard<'static, ()> {
    ENV.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Every variable this test touches, cleared before and after.
const VARS: [&str; 9] = [
    "CLAUDE_CODE_OAUTH_TOKEN_2",
    "CLAUDE_CODE_OAUTH_TOKEN_3",
    "CLAUDE_CODE_OAUTH_TOKEN_4",
    "CLI_LLM_BINARY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_API_KEY_2",
    "ANTHROPIC_API_KEY_3",
    "ANTHROPIC_EFFORT",
    "ANTHROPIC_DEFAULT_MODEL",
];

fn clear() {
    for var in VARS {
        env::remove_var(var);
    }
}

fn set(var: &str, value: &str) {
    env::set_var(var, value);
}

fn entry(provider: LlmProviderType, model: Option<&str>) -> ChainEntry {
    ChainEntry::new(provider, model)
}

#[test]
fn numbered_tokens_become_named_tiers_in_order() {
    let _env = exclusive();
    clear();
    set("CLI_LLM_BINARY", "/bin/sh");
    set("CLAUDE_CODE_OAUTH_TOKEN_2", "tok-2");
    set("CLAUDE_CODE_OAUTH_TOKEN_3", "tok-3");

    let accounts = EmbacleProvider::pooled_accounts(&entry(LlmProviderType::ClaudeCode, None));
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

    let accounts = EmbacleProvider::pooled_accounts(&entry(LlmProviderType::ClaudeCode, None));
    assert_eq!(accounts.len(), 1, "_4 behind an unset _3 is not read");
    clear();
}

#[test]
fn no_numbered_token_means_no_further_account() {
    let _env = exclusive();
    clear();
    set("CLI_LLM_BINARY", "/bin/sh");
    assert!(EmbacleProvider::pooled_accounts(&entry(LlmProviderType::ClaudeCode, None)).is_empty());
    clear();
}

#[test]
fn a_runner_without_a_credential_variable_never_pools() {
    let _env = exclusive();
    clear();
    set("CLI_LLM_BINARY", "/bin/sh");
    set("CLAUDE_CODE_OAUTH_TOKEN_2", "tok-2");
    assert!(EmbacleProvider::pooled_accounts(&entry(LlmProviderType::WarpCli, None)).is_empty());
    clear();
}

#[test]
fn numbered_anthropic_keys_become_named_tiers_on_the_entry_model() {
    let _env = exclusive();
    clear();
    set("ANTHROPIC_API_KEY", "sk-ant-1");
    set("ANTHROPIC_API_KEY_2", "sk-ant-2");
    set("ANTHROPIC_API_KEY_3", "sk-ant-3");

    let accounts = EmbacleProvider::pooled_accounts(&entry(
        LlmProviderType::Anthropic,
        Some("claude-sonnet-5-5"),
    ));
    let names: Vec<&str> = accounts.iter().map(LlmProvider::name).collect();
    assert_eq!(
        names,
        ["anthropic#2", "anthropic#3"],
        "the first key is the tier itself; each further organization is its own tier"
    );
    for account in &accounts {
        assert_eq!(
            account.default_model(),
            "claude-sonnet-5-5",
            "every account runs the model the chain entry names"
        );
        assert!(
            account.capabilities().supports_function_calling(),
            "a pooled account keeps the API loop"
        );
    }
    clear();
}

#[test]
fn an_anthropic_pool_without_its_first_key_is_empty() {
    let _env = exclusive();
    clear();
    set("ANTHROPIC_API_KEY_2", "sk-ant-2");
    assert!(
        EmbacleProvider::pooled_accounts(&entry(LlmProviderType::Anthropic, None)).is_empty(),
        "the tier itself fails without ANTHROPIC_API_KEY, and its accounts go with it"
    );
    clear();
}

#[test]
fn an_invalid_shared_anthropic_setting_pools_nothing() {
    let _env = exclusive();
    clear();
    set("ANTHROPIC_API_KEY", "sk-ant-1");
    set("ANTHROPIC_API_KEY_2", "sk-ant-2");
    set("ANTHROPIC_EFFORT", "extreme");
    assert!(EmbacleProvider::pooled_accounts(&entry(LlmProviderType::Anthropic, None)).is_empty());
    clear();
}

#[test]
fn the_copilot_runtime_never_pools() {
    let _env = exclusive();
    clear();
    set("CLAUDE_CODE_OAUTH_TOKEN_2", "tok-2");
    set("ANTHROPIC_API_KEY", "sk-ant-1");
    set("ANTHROPIC_API_KEY_2", "sk-ant-2");
    assert!(EmbacleProvider::pooled_accounts(&entry(LlmProviderType::CopilotSdk, None)).is_empty());
    clear();
}

#[test]
fn an_anthropic_account_entry_puts_that_organization_on_its_own_model() {
    let _env = exclusive();
    clear();
    set("ANTHROPIC_API_KEY", "sk-ant-1");
    set("ANTHROPIC_API_KEY_2", "sk-ant-2");
    set("ANTHROPIC_API_KEY_3", "sk-ant-3");

    let chain = LlmChain::parse("anthropic=claude-opus-5-5,anthropic#2=claude-haiku-5-5");
    let accounts = EmbacleProvider::pooled_accounts(chain.head());
    let models: Vec<(&str, &str)> = accounts
        .iter()
        .map(|account| (account.name(), account.default_model()))
        .collect();
    assert_eq!(
        models,
        [
            ("anthropic#2", "claude-haiku-5-5"),
            ("anthropic#3", "claude-opus-5-5"),
        ],
        "#2 runs its own model; #3 has none and runs the tier's"
    );
    clear();
}

#[test]
fn a_cli_account_entry_puts_that_account_on_its_own_model() {
    let _env = exclusive();
    clear();
    set("CLI_LLM_BINARY", "/bin/sh");
    set("CLAUDE_CODE_OAUTH_TOKEN_2", "tok-2");

    let chain = LlmChain::parse("claude_code=sonnet,claude_code#2=haiku");
    let accounts = EmbacleProvider::pooled_accounts(chain.head());
    let models: Vec<(&str, &str)> = accounts
        .iter()
        .map(|account| (account.name(), account.default_model()))
        .collect();
    assert_eq!(models, [("claude-code#2", "haiku")]);
    clear();
}
