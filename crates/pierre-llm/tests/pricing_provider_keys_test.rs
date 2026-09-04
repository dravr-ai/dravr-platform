// ABOUTME: Every pricing key and flat-rate entry must be a string a provider actually reports
// ABOUTME: A key spelled any other way is unreachable and its models silently bill at zero
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `llm_usage` rows carry `ChatProvider::name()`, which for a CLI provider is
//! the embacle runner's own `name()`. `lookup_pricing` matches that string by
//! equality, and `is_not_per_token_metered` by membership — so a table keyed on
//! any other spelling is not "slightly off", it is dead: every model under it
//! resolves to \$0.
//!
//! That is what happened to the three Claude Code rows, keyed `claude_code`
//! against a runner that reports `claude-code`. The spelling is not guessable
//! from the selector — `PIERRE_LLM_PROVIDER` accepts `claude_code`,
//! `CliRunnerType`'s `Display` prints `claude_code`, and only `name()` decides
//! what lands on the row.
//!
//! So the names here are *derived*, not restated: each one comes from
//! constructing the runner and asking it. An embacle bump that renames a runner
//! or adds one fails this file rather than silently unpricing a provider.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use embacle::types::LlmProvider as EmbacleLlmProvider;
use embacle::{
    ClaudeCodeRunner, CliRunnerType, ClineCliRunner, CodexCliRunner, ContinueCliRunner,
    CopilotHeadlessRunner, CopilotRunner, CursorAgentRunner, GeminiCliRunner, GooseCliRunner,
    KiloCliRunner, KiroCliRunner, OpenCodeRunner, RunnerConfig, WarpCliRunner,
};
use pierre_llm::config::LlmModelConfig;
use pierre_llm::pricing::{
    calculate_cost, is_not_per_token_metered, NOT_PER_TOKEN_METERED_PROVIDERS, PRICING_TABLE,
};
use pierre_llm::{
    CohereProvider, GeminiProvider, GroqProvider, LlmCapabilities, LlmProvider,
    OpenAiCompatibleConfig, OpenAiCompatibleProvider, OpenRouterProvider,
};

/// Every runner `CliLlmProvider::build_cli` can construct. The match below is
/// exhaustive, so an embacle release that adds a runner fails to compile here
/// instead of escaping the check.
const EVERY_CLI_RUNNER: [CliRunnerType; 13] = [
    CliRunnerType::ClaudeCode,
    CliRunnerType::CursorAgent,
    CliRunnerType::OpenCode,
    CliRunnerType::Copilot,
    CliRunnerType::GeminiCli,
    CliRunnerType::CodexCli,
    CliRunnerType::GooseCli,
    CliRunnerType::ClineCli,
    CliRunnerType::ContinueCli,
    CliRunnerType::WarpCli,
    CliRunnerType::KiroCli,
    CliRunnerType::KiloCli,
    CliRunnerType::CopilotHeadless,
];

/// The string this runner puts on a usage row.
fn cli_runner_name(kind: CliRunnerType) -> &'static str {
    let config = || RunnerConfig::new(PathBuf::from("/nonexistent"));
    match kind {
        CliRunnerType::ClaudeCode => ClaudeCodeRunner::new(config()).name(),
        CliRunnerType::CursorAgent => CursorAgentRunner::new(config()).name(),
        CliRunnerType::OpenCode => OpenCodeRunner::new(config()).name(),
        CliRunnerType::Copilot => CopilotRunner::new(config()).name(),
        CliRunnerType::GeminiCli => GeminiCliRunner::new(config()).name(),
        CliRunnerType::CodexCli => CodexCliRunner::new(config()).name(),
        CliRunnerType::GooseCli => GooseCliRunner::new(config()).name(),
        CliRunnerType::ClineCli => ClineCliRunner::new(config()).name(),
        CliRunnerType::ContinueCli => ContinueCliRunner::new(config()).name(),
        CliRunnerType::WarpCli => WarpCliRunner::new(config()).name(),
        CliRunnerType::KiroCli => KiroCliRunner::new(config()).name(),
        CliRunnerType::KiloCli => KiloCliRunner::new(config()).name(),
        CliRunnerType::CopilotHeadless => CopilotHeadlessRunner::from_env().name(),
    }
}

/// An `OpenAI`-compatible config whose `provider_name` decides which of the
/// four self-hosted names the provider reports.
fn local_config(provider_name: &str) -> OpenAiCompatibleConfig {
    OpenAiCompatibleConfig {
        base_url: "http://localhost:11434/v1".to_owned(),
        api_key: None,
        default_model: "test-model".to_owned(),
        fallback_model: "test-model".to_owned(),
        provider_name: provider_name.to_owned(),
        display_name: provider_name.to_owned(),
        capabilities: LlmCapabilities::STREAMING,
    }
}

/// The name a self-hosted endpoint reports under this `provider_name`.
fn local_name(provider_name: &str) -> &'static str {
    OpenAiCompatibleProvider::new(local_config(provider_name))
        .expect("an OpenAI-compatible provider builds from a plain config")
        .name()
}

/// The providers that call a vendor API directly, asked for their own names.
fn native_provider_names() -> Vec<&'static str> {
    let models = LlmModelConfig {
        default_model: "test-model".to_owned(),
        fallback_model: "test-model".to_owned(),
    };
    vec![
        GeminiProvider::with_config("test-key", &models).name(),
        GroqProvider::new("test-key".to_owned()).name(),
        CohereProvider::new("test-key".to_owned()).name(),
        OpenRouterProvider::new("test-key".to_owned()).name(),
        local_name("ollama"),
        local_name("vllm"),
        local_name("localai"),
        // Anything the config does not name explicitly reports as "local".
        local_name("some-self-hosted-endpoint"),
        // `OpenAiApiRunner::new` is async and probes `/v1/models` on
        // construction, so its name is named rather than built.
        "openai_api",
    ]
}

/// Every string a provider can put on a usage row.
fn every_provider_name() -> BTreeSet<&'static str> {
    EVERY_CLI_RUNNER
        .into_iter()
        .map(cli_runner_name)
        .chain(native_provider_names())
        .collect()
}

#[tokio::test]
async fn every_pricing_key_is_a_name_a_provider_reports() {
    let reported = every_provider_name();
    for (provider, model_prefix, _) in PRICING_TABLE {
        assert!(
            reported.contains(provider),
            "PRICING_TABLE keys ({provider}, {model_prefix}) on a string no provider reports; \
             lookup_pricing matches by equality, so that row is unreachable and every model \
             under it bills $0. Known names: {reported:?}"
        );
    }
}

#[tokio::test]
async fn every_not_metered_entry_is_a_name_a_provider_reports() {
    let reported = every_provider_name();
    for provider in NOT_PER_TOKEN_METERED_PROVIDERS {
        assert!(
            reported.contains(provider),
            "NOT_PER_TOKEN_METERED_PROVIDERS lists {provider}, which no provider reports; \
             the suppression it is meant to apply never fires. Known names: {reported:?}"
        );
    }
}

#[tokio::test]
async fn the_claude_code_rows_are_reachable_from_the_name_the_runner_reports() {
    let reported = cli_runner_name(CliRunnerType::ClaudeCode);
    assert_eq!(
        reported, "claude-code",
        "the pricing rows are keyed on this exact string"
    );

    // One million input tokens at the Sonnet rate is $3.00 exactly.
    let sonnet = calculate_cost(reported, "claude-sonnet-4.5", 1_000_000, 0);
    assert!(
        (sonnet - 3.0).abs() < 1e-9,
        "claude-sonnet must price at $3/M input; got {sonnet}"
    );
    let opus = calculate_cost(reported, "claude-opus-4-1", 0, 1_000_000);
    assert!(
        (opus - 75.0).abs() < 1e-9,
        "claude-opus-4 must price at $75/M output; got {opus}"
    );
    let haiku = calculate_cost(reported, "claude-haiku-4-5", 1_000_000, 0);
    assert!(
        (haiku - 0.80).abs() < 1e-9,
        "claude-haiku-4 must price at $0.80/M input; got {haiku}"
    );
}

#[tokio::test]
async fn claude_code_is_priced_rather_than_suppressed() {
    assert!(
        !is_not_per_token_metered("claude-code"),
        "claude-code carries real PRICING_TABLE rows; suppressing it would zero its shadow COGS \
         and hide the miss behind the by-design $0 path"
    );
    assert!(
        !is_not_per_token_metered("copilot_headless"),
        "copilot_headless is priced for the same reason"
    );
}

#[tokio::test]
async fn every_flat_rate_runner_the_table_does_not_price_is_suppressed() {
    // The complement of the rule above: a runner with no PRICING_TABLE rows
    // bills $0, and that $0 must be classified as correct rather than logged as
    // an undercount.
    let priced: BTreeSet<&str> = PRICING_TABLE.iter().map(|(p, _, _)| *p).collect();
    for kind in EVERY_CLI_RUNNER {
        let name = cli_runner_name(kind);
        if priced.contains(name) {
            continue;
        }
        assert!(
            is_not_per_token_metered(name),
            "{name} has no pricing rows, so every one of its turns bills $0 — it must be \
             classified as subscription-billed or the cost path logs a false undercount"
        );
    }
}
