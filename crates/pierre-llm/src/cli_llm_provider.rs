// ABOUTME: Embache-based LLM provider facade wrapping CLI subprocess and SDK runners
// ABOUTME: Handles environment configuration, auto-detection, readiness checking, and RunnerError bridging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use embache::auth::check_readiness;
use embache::config::parse_timeout;
use embache::{
    ClaudeCodeRunner, CliRunnerType, CopilotRunner, CursorAgentRunner, OpenCodeRunner, RunnerConfig,
};
use futures_util::StreamExt;
use tracing::{debug, info, warn};

use embache::types::LlmProvider as EmbacheLlmProvider;

use super::{ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider};
use crate::config::LlmProviderType;
use crate::errors::AppError;

/// Re-export embache runner readiness status for external use
pub use embache::auth::ProviderReadiness;

const READINESS_UNKNOWN: u8 = 0;
const READINESS_READY: u8 = 1;
const READINESS_NOT_READY: u8 = 2;

/// Embache-based LLM provider facade
///
/// Wraps any embache runner (CLI subprocess runners or SDK runners) behind the
/// platform [`LlmProvider`] trait. Supports environment-based configuration,
/// auto-detection of installed CLI tools, and non-blocking readiness checks.
///
/// All embache providers — Claude Code, Copilot CLI, Cursor Agent, `OpenCode`,
/// and Copilot SDK — are handled uniformly through this single facade.
pub struct CliLlmProvider {
    runner: Box<dyn EmbacheLlmProvider>,
    /// CLI runner binary path (None for SDK runners)
    binary_path: Option<PathBuf>,
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
    /// Returns `AppError` if no runner can be detected or the binary
    /// cannot be resolved.
    pub fn from_env() -> Result<Self, AppError> {
        let provider_env = env::var("PIERRE_LLM_PROVIDER").unwrap_or_default();
        match provider_env.to_lowercase().as_str() {
            "claude_code" | "claude-code" => {
                let config = build_runner_config(CliRunnerType::ClaudeCode)?;
                Ok(Self::build_cli(CliRunnerType::ClaudeCode, config))
            }
            "cursor_agent" | "cursor-agent" => {
                let config = build_runner_config(CliRunnerType::CursorAgent)?;
                Ok(Self::build_cli(CliRunnerType::CursorAgent, config))
            }
            "opencode" | "open_code" => {
                let config = build_runner_config(CliRunnerType::OpenCode)?;
                Ok(Self::build_cli(CliRunnerType::OpenCode, config))
            }
            "copilot" | "github_copilot" | "github-copilot" => {
                let config = build_runner_config(CliRunnerType::Copilot)?;
                Ok(Self::build_cli(CliRunnerType::Copilot, config))
            }
            "copilot_sdk" | "copilot-sdk" => Ok(Self::build_sdk()),
            "cli" => {
                debug!("PIERRE_LLM_PROVIDER=cli, auto-detecting installed CLI runner");
                let (runner_type, base_config) = embache::discover_runner()?;
                let config = merge_env_overrides(base_config);
                Ok(Self::build_cli(runner_type, config))
            }
            _ => Err(AppError::config(format!(
                "PIERRE_LLM_PROVIDER={provider_env} is not an embache runner type; \
                 expected one of: claude_code, copilot, cursor_agent, opencode, copilot_sdk, cli"
            ))),
        }
    }

    /// Create a provider for a specific CLI runner type with an explicit config
    #[must_use]
    pub fn from_runner_type(runner_type: CliRunnerType, config: RunnerConfig) -> Self {
        Self::build_cli(runner_type, config)
    }

    /// Build a CLI subprocess runner
    fn build_cli(runner_type: CliRunnerType, config: RunnerConfig) -> Self {
        let binary_path = config.binary_path.clone();
        info!(runner = %runner_type, path = %binary_path.display(), "Creating CLI LLM runner");

        let runner: Box<dyn EmbacheLlmProvider> = match runner_type {
            CliRunnerType::ClaudeCode => Box::new(ClaudeCodeRunner::new(config)),
            CliRunnerType::Copilot => Box::new(CopilotRunner::new(config)),
            CliRunnerType::CursorAgent => Box::new(CursorAgentRunner::new(config)),
            CliRunnerType::OpenCode => Box::new(OpenCodeRunner::new(config)),
        };

        let provider = Self {
            runner,
            binary_path: Some(binary_path.clone()),
            readiness: Arc::new(AtomicU8::new(READINESS_UNKNOWN)),
        };

        spawn_readiness_check(runner_type, binary_path, Arc::clone(&provider.readiness));
        provider
    }

    /// Build a Copilot SDK runner (persistent JSON-RPC, no binary path needed)
    ///
    /// `PIERRE_LLM_MODEL` overrides the SDK-specific `COPILOT_SDK_MODEL` env var.
    fn build_sdk() -> Self {
        let mut config = embache::CopilotSdkConfig::from_env();

        // PIERRE_LLM_MODEL is the unified model override (highest priority)
        if let Ok(model) = env::var("PIERRE_LLM_MODEL") {
            if !model.is_empty() {
                config.model = model;
            }
        }

        info!(model = %config.model, "Creating Copilot SDK runner (copilot --headless)");

        Self {
            runner: Box::new(embache::CopilotSdkRunner::with_config(config)),
            binary_path: None,
            readiness: Arc::new(AtomicU8::new(READINESS_UNKNOWN)),
        }
    }

    /// Get the `LlmProviderType` for this runner
    #[must_use]
    pub fn provider_type(&self) -> LlmProviderType {
        match self.runner.name() {
            "copilot" => LlmProviderType::Copilot,
            "cursor_agent" => LlmProviderType::CursorAgent,
            "opencode" => LlmProviderType::OpenCode,
            "copilot_sdk" => LlmProviderType::CopilotSdk,
            // "claude_code" and any future runners default here
            _ => LlmProviderType::ClaudeCode,
        }
    }

    /// Downcast the inner runner to a `CopilotSdkRunner` reference.
    ///
    /// Returns `Some` only when the underlying runner is the Copilot SDK provider.
    /// Used by the SDK tool loop to access `execute_with_tools()`.
    #[must_use]
    pub fn as_copilot_sdk_runner(&self) -> Option<&embache::CopilotSdkRunner> {
        self.runner
            .as_any()
            .downcast_ref::<embache::CopilotSdkRunner>()
    }

    /// Check whether the CLI runner is authenticated and available
    ///
    /// Performs a non-cached check. Updates the internal readiness state.
    /// SDK runners always return `Ready` since they handle auth internally.
    pub async fn check_readiness(&self) -> ProviderReadiness {
        let Some(ref binary_path) = self.binary_path else {
            // SDK runners handle authentication internally via the SDK client
            return ProviderReadiness::Ready;
        };

        let runner_name = self.runner.name();
        let runner_type = match runner_name {
            "claude_code" => CliRunnerType::ClaudeCode,
            "copilot" => CliRunnerType::Copilot,
            "cursor_agent" => CliRunnerType::CursorAgent,
            "opencode" => CliRunnerType::OpenCode,
            _ => return ProviderReadiness::Ready,
        };

        let result = check_readiness(&runner_type, binary_path)
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
}

#[async_trait]
impl LlmProvider for CliLlmProvider {
    fn name(&self) -> &'static str {
        self.runner.name()
    }

    fn display_name(&self) -> &'static str {
        self.runner.display_name()
    }

    fn capabilities(&self) -> LlmCapabilities {
        self.runner.capabilities()
    }

    fn default_model(&self) -> &str {
        self.runner.default_model()
    }

    fn available_models(&self) -> &[String] {
        self.runner.available_models()
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        EmbacheLlmProvider::complete(&*self.runner, request)
            .await
            .map_err(AppError::from)
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let embache_stream = EmbacheLlmProvider::complete_stream(&*self.runner, request)
            .await
            .map_err(AppError::from)?;

        Ok(Box::pin(
            embache_stream.map(|result| result.map_err(AppError::from)),
        ))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        EmbacheLlmProvider::health_check(&*self.runner)
            .await
            .map_err(AppError::from)
    }
}

/// Spawn a background task to check CLI runner readiness without blocking construction
fn spawn_readiness_check(
    runner_type: CliRunnerType,
    binary_path: PathBuf,
    readiness: Arc<AtomicU8>,
) {
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

/// Build a `RunnerConfig` for a specific runner type from environment variables
///
/// Resolves the binary path via `CLI_LLM_BINARY` env var override or `which`
/// discovery, then applies `CLI_LLM_*` overrides for model, timeout, and args.
fn build_runner_config(runner_type: CliRunnerType) -> Result<RunnerConfig, AppError> {
    let binary_override = env::var("CLI_LLM_BINARY").ok();
    let binary_path =
        embache::resolve_binary(runner_type.binary_name(), binary_override.as_deref())?;

    let mut config = RunnerConfig::new(binary_path);
    config = apply_env_overrides(config);
    Ok(config)
}

/// Merge `CLI_LLM_*` environment overrides into a discovered `RunnerConfig`
fn merge_env_overrides(config: RunnerConfig) -> RunnerConfig {
    apply_env_overrides(config)
}

/// Apply environment variable overrides to a `RunnerConfig`
///
/// `PIERRE_LLM_MODEL` is the unified model override (highest priority).
/// `CLI_LLM_MODEL` is the runner-specific fallback.
fn apply_env_overrides(mut config: RunnerConfig) -> RunnerConfig {
    let model_override = env::var("PIERRE_LLM_MODEL")
        .or_else(|_| env::var("CLI_LLM_MODEL"))
        .ok()
        .filter(|m| !m.is_empty());
    if let Some(model) = model_override {
        config = config.with_model(model);
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
