// ABOUTME: CLI-based LLM provider facade wrapping subprocess runners (Claude Code, Cursor, OpenCode)
// ABOUTME: Handles environment configuration, auto-detection, and readiness checking
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, info, warn};

use super::{ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider};
use crate::errors::AppError;
use cli_llm_runners::{
    auth::check_readiness, config::parse_timeout, ClaudeCodeRunner, CliRunnerType, CopilotRunner,
    CursorAgentRunner, OpenCodeRunner, ProviderReadiness, RunnerConfig,
};

const READINESS_UNKNOWN: u8 = 0;
const READINESS_READY: u8 = 1;
const READINESS_NOT_READY: u8 = 2;

/// CLI-based LLM provider facade
///
/// Wraps a `cli-llm-runners` runner (Claude Code, Cursor Agent, or `OpenCode`)
/// behind the `LlmProvider` trait. Supports environment-based configuration,
/// auto-detection of installed CLI tools, and non-blocking readiness checks.
pub struct CliLlmProvider {
    runner: Box<dyn LlmProvider>,
    runner_type: CliRunnerType,
    binary_path: PathBuf,
    readiness: Arc<AtomicU8>,
}

impl CliLlmProvider {
    /// Create a provider by reading environment variables
    ///
    /// Reads `PIERRE_LLM_PROVIDER` to select the runner type, then builds
    /// a `RunnerConfig` from `CLI_LLM_*` env vars. Falls back to auto-detection
    /// when the provider is set to `"cli"`.
    ///
    /// # Errors
    ///
    /// Returns `AppError` if no CLI runner can be detected or the binary
    /// cannot be resolved.
    pub fn from_env() -> Result<Self, AppError> {
        let provider_env = env::var("PIERRE_LLM_PROVIDER").unwrap_or_default();
        match provider_env.to_lowercase().as_str() {
            "claude_code" | "claude-code" => {
                let config = build_runner_config(CliRunnerType::ClaudeCode)?;
                Ok(Self::build(CliRunnerType::ClaudeCode, config))
            }
            "cursor_agent" | "cursor-agent" => {
                let config = build_runner_config(CliRunnerType::CursorAgent)?;
                Ok(Self::build(CliRunnerType::CursorAgent, config))
            }
            "opencode" | "open_code" => {
                let config = build_runner_config(CliRunnerType::OpenCode)?;
                Ok(Self::build(CliRunnerType::OpenCode, config))
            }
            "copilot" | "github_copilot" | "github-copilot" => {
                let config = build_runner_config(CliRunnerType::Copilot)?;
                Ok(Self::build(CliRunnerType::Copilot, config))
            }
            "cli" => {
                debug!("PIERRE_LLM_PROVIDER=cli, auto-detecting installed CLI runner");
                let (runner_type, base_config) = cli_llm_runners::discover_runner()?;
                let config = merge_env_overrides(base_config);
                Ok(Self::build(runner_type, config))
            }
            _ => Err(AppError::config(format!(
                "PIERRE_LLM_PROVIDER={provider_env} is not a CLI runner type; \
                 expected one of: claude_code, copilot, cursor_agent, opencode, cli"
            ))),
        }
    }

    /// Create a provider for a specific runner type with an explicit config
    #[must_use]
    pub fn from_runner_type(runner_type: CliRunnerType, config: RunnerConfig) -> Self {
        Self::build(runner_type, config)
    }

    /// Internal constructor: create runner, store binary path, spawn readiness check
    fn build(runner_type: CliRunnerType, config: RunnerConfig) -> Self {
        let binary_path = config.binary_path.clone();
        info!(runner = %runner_type, path = %binary_path.display(), "Creating CLI LLM runner");

        let runner: Box<dyn LlmProvider> = match runner_type {
            CliRunnerType::ClaudeCode => Box::new(ClaudeCodeRunner::new(config)),
            CliRunnerType::Copilot => Box::new(CopilotRunner::new(config)),
            CliRunnerType::CursorAgent => Box::new(CursorAgentRunner::new(config)),
            CliRunnerType::OpenCode => Box::new(OpenCodeRunner::new(config)),
        };

        let provider = Self {
            runner,
            runner_type,
            binary_path,
            readiness: Arc::new(AtomicU8::new(READINESS_UNKNOWN)),
        };

        provider.spawn_readiness_check();
        provider
    }

    /// Get the runner type backing this provider
    #[must_use]
    pub fn runner_type(&self) -> &CliRunnerType {
        &self.runner_type
    }

    /// Check whether the CLI runner is authenticated and available
    ///
    /// Performs a non-cached check. Updates the internal readiness state.
    pub async fn check_readiness(&self) -> ProviderReadiness {
        let result = check_readiness(&self.runner_type, &self.binary_path)
            .await
            .unwrap_or_else(|e| ProviderReadiness::Unknown {
                reason: format!("Readiness check failed: {e}"),
            });

        let state = if result.is_ready() {
            READINESS_READY
        } else {
            READINESS_NOT_READY
        };
        self.readiness.store(state, Ordering::Relaxed);
        result
    }

    /// Spawn a background task to check readiness without blocking construction
    fn spawn_readiness_check(&self) {
        let runner_type = self.runner_type;
        let binary_path = self.binary_path.clone();
        let readiness = Arc::clone(&self.readiness);

        tokio::spawn(async move {
            let result = check_readiness(&runner_type, &binary_path)
                .await
                .unwrap_or_else(|e| ProviderReadiness::Unknown {
                    reason: format!("Background readiness check failed: {e}"),
                });

            let state = if result.is_ready() {
                READINESS_READY
            } else {
                warn!(
                    runner = %runner_type,
                    status = %result,
                    "CLI LLM runner is not ready"
                );
                READINESS_NOT_READY
            };
            readiness.store(state, Ordering::Relaxed);
        });
    }
}

#[async_trait]
impl LlmProvider for CliLlmProvider {
    fn name(&self) -> &'static str {
        match self.runner_type {
            CliRunnerType::ClaudeCode => "claude_code",
            CliRunnerType::Copilot => "copilot",
            CliRunnerType::CursorAgent => "cursor_agent",
            CliRunnerType::OpenCode => "opencode",
        }
    }

    fn display_name(&self) -> &'static str {
        match self.runner_type {
            CliRunnerType::ClaudeCode => "Claude Code (CLI)",
            CliRunnerType::Copilot => "GitHub Copilot (CLI)",
            CliRunnerType::CursorAgent => "Cursor Agent (CLI)",
            CliRunnerType::OpenCode => "OpenCode (CLI)",
        }
    }

    fn capabilities(&self) -> LlmCapabilities {
        self.runner.capabilities()
    }

    fn default_model(&self) -> &str {
        self.runner.default_model()
    }

    fn available_models(&self) -> &'static [&'static str] {
        self.runner.available_models()
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.runner.complete(request).await
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        self.runner.complete_stream(request).await
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        self.runner.health_check().await
    }
}

/// Build a `RunnerConfig` for a specific runner type from environment variables
///
/// Resolves the binary path via `CLI_LLM_BINARY` env var override or `which`
/// discovery, then applies `CLI_LLM_*` overrides for model, timeout, and args.
fn build_runner_config(runner_type: CliRunnerType) -> Result<RunnerConfig, AppError> {
    let binary_override = env::var("CLI_LLM_BINARY").ok();
    let binary_path =
        cli_llm_runners::resolve_binary(runner_type.binary_name(), binary_override.as_deref())?;

    let mut config = RunnerConfig::new(binary_path);
    config = apply_env_overrides(config);
    Ok(config)
}

/// Merge `CLI_LLM_*` environment overrides into a discovered `RunnerConfig`
fn merge_env_overrides(config: RunnerConfig) -> RunnerConfig {
    apply_env_overrides(config)
}

/// Apply `CLI_LLM_*` environment variable overrides to a `RunnerConfig`
fn apply_env_overrides(mut config: RunnerConfig) -> RunnerConfig {
    if let Ok(model) = env::var("CLI_LLM_MODEL") {
        if !model.is_empty() {
            config = config.with_model(model);
        }
    }

    if let Ok(timeout_str) = env::var("CLI_LLM_TIMEOUT_SECS") {
        if let Ok(timeout) = parse_timeout(&timeout_str) {
            config = config.with_timeout(timeout);
        }
    }

    if let Ok(extra) = env::var("CLI_LLM_EXTRA_ARGS") {
        let args: Vec<String> = extra
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        if !args.is_empty() {
            config = config.with_extra_args(args);
        }
    }

    if let Ok(workdir) = env::var("CLI_LLM_WORKING_DIR") {
        if !workdir.is_empty() {
            config = config.with_working_directory(PathBuf::from(workdir));
        }
    }

    config
}
