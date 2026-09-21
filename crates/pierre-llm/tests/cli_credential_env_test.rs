// ABOUTME: A CLI runner's sandbox allowlist carries the runner's own credential variable
// ABOUTME: Pins that Claude Code gets CLAUDE_CODE_OAUTH_TOKEN — without it every call is "Not logged in"
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use embacle::config::default_allowed_env_keys;
use embacle::CliRunnerType;
use pierre_llm::{cli_credential_env_keys, cli_runner_config};
use std::env;

#[test]
fn every_deployed_cli_runner_names_its_credential() {
    assert!(cli_credential_env_keys(CliRunnerType::ClaudeCode).contains(&"CLAUDE_CODE_OAUTH_TOKEN"));
    assert!(cli_credential_env_keys(CliRunnerType::Copilot).contains(&"COPILOT_GITHUB_TOKEN"));
    assert!(cli_credential_env_keys(CliRunnerType::GeminiCli).contains(&"GEMINI_API_KEY"));
    assert!(cli_credential_env_keys(CliRunnerType::CodexCli).contains(&"OPENAI_API_KEY"));
}

/// The premise of the allowlist extension: embacle's default passes no
/// credential, so a runner built on the default alone cannot log in.
#[test]
fn the_sandbox_default_does_not_carry_the_claude_token() {
    assert!(!default_allowed_env_keys()
        .iter()
        .any(|key| key == "CLAUDE_CODE_OAUTH_TOKEN"));
}

#[test]
fn a_claude_code_config_lets_its_token_through_the_sandbox() {
    // Any executable resolves the binary; nothing here runs it.
    env::set_var("CLI_LLM_BINARY", "/bin/sh");
    let config = cli_runner_config(CliRunnerType::ClaudeCode, None).expect("config builds");
    env::remove_var("CLI_LLM_BINARY");

    assert!(
        config
            .allowed_env_keys
            .iter()
            .any(|key| key == "CLAUDE_CODE_OAUTH_TOKEN"),
        "{:?}",
        config.allowed_env_keys
    );
    for key in default_allowed_env_keys() {
        assert!(
            config.allowed_env_keys.contains(&key),
            "the default {key} stays allowed"
        );
    }
}
