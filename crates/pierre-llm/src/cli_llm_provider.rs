// ABOUTME: Embacle-based LLM provider facade wrapping CLI subprocess and SDK runners
// ABOUTME: Handles environment configuration, auto-detection, readiness checking, and RunnerError bridging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use embacle::auth::check_readiness;
use embacle::config::parse_timeout;
use embacle::{
    ClaudeCodeRunner, CliRunnerType, ClineCliRunner, CodexCliRunner, ContinueCliRunner,
    CopilotHeadlessConfig, CopilotHeadlessRunner, CopilotRunner, CursorAgentRunner,
    GeminiCliRunner, GooseCliRunner, KiloCliRunner, KiroCliRunner, OpenAiApiConfig,
    OpenAiApiRunner, OpenCodeRunner, RunnerConfig, WarpCliRunner,
};
use futures_util::StreamExt;
use pierre_core::http_client::llm_inner_client;
use tracing::{debug, info, warn};

use embacle::types::{
    ChatStream as EmbacleChatStream, LlmProvider as EmbacleLlmProvider, RunnerError,
};

use super::{ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider};
use crate::errors::AppError;

/// Re-export embacle runner readiness status for external use
pub use embacle::auth::ProviderReadiness;

const READINESS_UNKNOWN: u8 = 0;
const READINESS_READY: u8 = 1;
const READINESS_NOT_READY: u8 = 2;

/// Embacle-based LLM provider facade
///
/// Wraps any embacle runner (CLI subprocess runners or SDK runners) behind the
/// platform [`LlmProvider`] trait. Supports environment-based configuration,
/// auto-detection of installed CLI tools, and non-blocking readiness checks.
///
/// All embacle providers — Claude Code, Copilot CLI, Cursor Agent, `OpenCode`,
/// Copilot Headless (ACP), and Warp — are handled uniformly through this single facade.
pub struct CliLlmProvider {
    runner: Box<dyn EmbacleLlmProvider>,
    readiness: Arc<AtomicU8>,
    /// Typed reference to the headless runner for `converse()` access (ACP only)
    headless_runner: Option<Arc<CopilotHeadlessRunner>>,
    /// Cached display name (embacle returns `&str`, pierre trait needs `&'static str`)
    cached_display_name: &'static str,
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
    pub async fn from_env() -> Result<Self, AppError> {
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
            "copilot_headless" | "copilot-headless" => Ok(Self::build_headless()),
            "gemini_cli" | "gemini-cli" => {
                let config = build_runner_config(CliRunnerType::GeminiCli)?;
                Ok(Self::build_cli(CliRunnerType::GeminiCli, config))
            }
            "codex_cli" | "codex-cli" | "codex" => {
                let config = build_runner_config(CliRunnerType::CodexCli)?;
                Ok(Self::build_cli(CliRunnerType::CodexCli, config))
            }
            "goose_cli" | "goose-cli" | "goose" => {
                let config = build_runner_config(CliRunnerType::GooseCli)?;
                Ok(Self::build_cli(CliRunnerType::GooseCli, config))
            }
            "cline_cli" | "cline-cli" | "cline" => {
                let config = build_runner_config(CliRunnerType::ClineCli)?;
                Ok(Self::build_cli(CliRunnerType::ClineCli, config))
            }
            "continue_cli" | "continue-cli" | "continue" => {
                let config = build_runner_config(CliRunnerType::ContinueCli)?;
                Ok(Self::build_cli(CliRunnerType::ContinueCli, config))
            }
            "warp_cli" | "warp-cli" | "warp" | "oz" => {
                let config = build_runner_config(CliRunnerType::WarpCli)?;
                Ok(Self::build_cli(CliRunnerType::WarpCli, config))
            }
            "kiro_cli" | "kiro-cli" | "kiro" => {
                let config = build_runner_config(CliRunnerType::KiroCli)?;
                Ok(Self::build_cli(CliRunnerType::KiroCli, config))
            }
            "kilo_cli" | "kilo-cli" | "kilo" => {
                let config = build_runner_config(CliRunnerType::KiloCli)?;
                Ok(Self::build_cli(CliRunnerType::KiloCli, config))
            }
            "openai_api" | "openai-api" | "openai" => Ok(Self::build_openai_api().await?),
            "cli" => {
                debug!("PIERRE_LLM_PROVIDER=cli, auto-detecting installed CLI runner");
                let (runner_type, base_config) = embacle::discover_runner()?;
                let config = merge_env_overrides(base_config);
                Ok(Self::build_cli(runner_type, config))
            }
            _ => Err(AppError::config(format!(
                "PIERRE_LLM_PROVIDER={provider_env} is not an embacle runner type; \
                 expected one of: claude_code, copilot, cursor_agent, opencode, copilot_headless, \
                 gemini_cli, codex_cli, goose_cli, cline_cli, continue_cli, warp_cli, kiro_cli, kilo_cli, openai_api, cli"
            ))),
        }
    }

    /// Create a provider for a specific CLI runner type with an explicit config
    #[must_use]
    pub fn from_runner_type(runner_type: CliRunnerType, config: RunnerConfig) -> Self {
        Self::build_cli(runner_type, config)
    }

    /// Create a provider for a specific CLI runner type, resolving the binary
    /// via the standard `embacle` discovery and applying an optional explicit
    /// model override.
    ///
    /// Bypasses `PIERRE_LLM_PROVIDER` env-var dispatch in [`Self::from_env`]
    /// — needed when the caller already knows the runner type (e.g. the
    /// runtime fallback chain in [`crate::provider::ChatProvider`] builds
    /// the secondary from `PIERRE_LLM_FALLBACK_PROVIDER`, not the primary
    /// `PIERRE_LLM_PROVIDER`).
    ///
    /// When `model_override` is `Some`, the value wins over `PIERRE_LLM_MODEL`
    /// for this runner. Use it to give the fallback secondary a different
    /// model name than the primary (see `PIERRE_LLM_FALLBACK_PROVIDER_MODEL`).
    ///
    /// # Errors
    ///
    /// Returns `AppError` if the binary cannot be resolved for `runner_type`.
    pub fn from_runner_type_with_model(
        runner_type: CliRunnerType,
        model_override: Option<&str>,
    ) -> Result<Self, AppError> {
        let config = build_runner_config_with_model(runner_type, model_override)?;
        Ok(Self::build_cli(runner_type, config))
    }

    /// Build a CLI subprocess runner
    fn build_cli(runner_type: CliRunnerType, config: RunnerConfig) -> Self {
        let binary_path = config.binary_path.clone();

        let runner: Box<dyn EmbacleLlmProvider> = match runner_type {
            CliRunnerType::ClaudeCode => Box::new(ClaudeCodeRunner::new(config)),
            CliRunnerType::Copilot => Box::new(CopilotRunner::new(config)),
            CliRunnerType::CursorAgent => Box::new(CursorAgentRunner::new(config)),
            CliRunnerType::OpenCode => Box::new(OpenCodeRunner::new(config)),
            CliRunnerType::GeminiCli => Box::new(GeminiCliRunner::new(config)),
            CliRunnerType::CodexCli => Box::new(CodexCliRunner::new(config)),
            CliRunnerType::GooseCli => Box::new(GooseCliRunner::new(config)),
            CliRunnerType::ClineCli => Box::new(ClineCliRunner::new(config)),
            CliRunnerType::ContinueCli => Box::new(ContinueCliRunner::new(config)),
            CliRunnerType::WarpCli => Box::new(WarpCliRunner::new(config)),
            CliRunnerType::KiroCli => Box::new(KiroCliRunner::new(config)),
            CliRunnerType::KiloCli => Box::new(KiloCliRunner::new(config)),
            CliRunnerType::CopilotHeadless => {
                return Self::build_headless();
            }
        };

        info!(
            runner = %runner_type,
            path = %binary_path.display(),
            model = runner.default_model(),
            available_models = ?runner.available_models(),
            "Creating CLI LLM runner"
        );

        let display_name = runner_display_name(runner_type);
        let provider = Self {
            runner,
            readiness: Arc::new(AtomicU8::new(READINESS_UNKNOWN)),
            headless_runner: None,
            cached_display_name: display_name,
        };

        spawn_readiness_check(runner_type, binary_path, Arc::clone(&provider.readiness));
        provider
    }

    /// Build a Copilot Headless (ACP) runner (NDJSON JSON-RPC via `copilot --acp`)
    ///
    /// `PIERRE_LLM_MODEL` overrides the headless-specific `COPILOT_HEADLESS_MODEL` env var.
    ///
    /// LIMITATION(registre#104): `build_headless` binds one `CopilotHeadlessConfig::model` for every call in the turn — tool-loop iterations and the athlete-facing draft run on the same model, with no per-stage routing to a cheaper one.
    fn build_headless() -> Self {
        let mut config = CopilotHeadlessConfig::from_env();

        // PIERRE_LLM_MODEL is the unified model override (highest priority)
        if let Ok(model) = env::var("PIERRE_LLM_MODEL") {
            if !model.is_empty() {
                config.model = model;
            }
        }

        info!(model = %config.model, "Creating Copilot Headless runner (copilot --acp)");

        let headless = Arc::new(CopilotHeadlessRunner::with_config(config));

        Self {
            runner: Box::new(HeadlessRunnerAdapter(Arc::clone(&headless))),
            readiness: Arc::new(AtomicU8::new(READINESS_UNKNOWN)),
            headless_runner: Some(headless),
            cached_display_name: "GitHub Copilot (Headless)",
        }
    }

    /// Build an `OpenAI`-compatible HTTP API runner (via embacle `OpenAiApiRunner`)
    ///
    /// Reads configuration from `OPENAI_API_*` env vars. `PIERRE_LLM_MODEL`
    /// overrides `OPENAI_API_MODEL` when set.
    async fn build_openai_api() -> Result<Self, AppError> {
        let mut config = OpenAiApiConfig::from_env();

        // PIERRE_LLM_MODEL is the unified model override (highest priority)
        if let Ok(model) = env::var("PIERRE_LLM_MODEL") {
            if !model.is_empty() {
                config.model = model;
            }
        }

        info!(
            base_url = %config.base_url,
            model = %config.model,
            "Creating OpenAI API runner"
        );

        let client = llm_inner_client().clone();
        let runner = OpenAiApiRunner::with_client(config, client).await;

        Ok(Self {
            runner: Box::new(runner),
            readiness: Arc::new(AtomicU8::new(READINESS_READY)),
            headless_runner: None,
            cached_display_name: "OpenAI API",
        })
    }

    /// Access the inner `CopilotHeadlessRunner` if this provider wraps one.
    ///
    /// Returns `Some` only for Copilot Headless (ACP) providers.
    /// Used by the headless tool loop to access `converse()`.
    #[must_use]
    pub fn as_headless_runner(&self) -> Option<&CopilotHeadlessRunner> {
        self.headless_runner.as_deref()
    }
}

#[async_trait]
impl LlmProvider for CliLlmProvider {
    fn name(&self) -> &'static str {
        self.runner.name()
    }

    fn display_name(&self) -> &'static str {
        self.cached_display_name
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
        EmbacleLlmProvider::complete(&*self.runner, request)
            .await
            .map_err(AppError::from)
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let embacle_stream = EmbacleLlmProvider::complete_stream(&*self.runner, request)
            .await
            .map_err(AppError::from)?;

        Ok(Box::pin(
            embacle_stream.map(|result| result.map_err(AppError::from)),
        ))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        EmbacleLlmProvider::health_check(&*self.runner)
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
    build_runner_config_with_model(runner_type, None)
}

/// Like [`build_runner_config`] but uses an explicit model instead of
/// reading `PIERRE_LLM_MODEL`. Used to construct the secondary in a runtime
/// fallback chain when its model differs from the primary's (e.g. Copilot
/// `claude-opus-4.7` primary → Anthropic CLI `claude-opus-4-7` secondary).
fn build_runner_config_with_model(
    runner_type: CliRunnerType,
    model_override: Option<&str>,
) -> Result<RunnerConfig, AppError> {
    let binary_override = env::var("CLI_LLM_BINARY").ok();
    let binary_path =
        embacle::resolve_binary(runner_type.binary_name(), binary_override.as_deref())?;

    let mut config = RunnerConfig::new(binary_path);
    config = apply_env_overrides(config, model_override);
    Ok(config)
}

/// Merge `CLI_LLM_*` environment overrides into a discovered `RunnerConfig`
fn merge_env_overrides(config: RunnerConfig) -> RunnerConfig {
    apply_env_overrides(config, None)
}

/// Apply environment variable overrides to a `RunnerConfig`
///
/// Model resolution order (highest priority first):
///   1. `model_override` argument (used by the runtime fallback chain to
///      give the secondary a different model than the primary)
///   2. `PIERRE_LLM_MODEL`
///   3. `CLI_LLM_MODEL`
fn apply_env_overrides(mut config: RunnerConfig, model_override: Option<&str>) -> RunnerConfig {
    let model = model_override
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            env::var("PIERRE_LLM_MODEL")
                .or_else(|_| env::var("CLI_LLM_MODEL"))
                .ok()
                .filter(|m| !m.is_empty())
        });
    if let Some(model) = model {
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

/// Map a CLI runner type to its static display name string
const fn runner_display_name(runner_type: CliRunnerType) -> &'static str {
    match runner_type {
        CliRunnerType::ClaudeCode => "Claude Code",
        CliRunnerType::Copilot => "GitHub Copilot (CLI)",
        CliRunnerType::CursorAgent => "Cursor Agent",
        CliRunnerType::OpenCode => "OpenCode",
        CliRunnerType::GeminiCli => "Gemini (CLI)",
        CliRunnerType::CodexCli => "Codex (CLI)",
        CliRunnerType::GooseCli => "Goose (CLI)",
        CliRunnerType::ClineCli => "Cline (CLI)",
        CliRunnerType::ContinueCli => "Continue (CLI)",
        CliRunnerType::WarpCli => "Warp (CLI)",
        CliRunnerType::KiroCli => "Kiro (CLI)",
        CliRunnerType::KiloCli => "Kilo Code (CLI)",
        CliRunnerType::CopilotHeadless => "GitHub Copilot (Headless)",
    }
}

// ============================================================================
// Headless Runner Adapter
// ============================================================================

/// Adapter wrapping `Arc<CopilotHeadlessRunner>` as `Box<dyn EmbacleLlmProvider>`.
///
/// This allows the headless runner to be stored both as a trait object (for the
/// unified `runner` field) and as a typed `Arc` (for `converse()` access).
///
/// LIMITATION(registre#102): nothing on this path marks the system-prompt +
/// tool-surface prefix cacheable, and nothing can. `cache_control` is settable
/// only on a request we build ourselves; here the ACP agent builds it. The gap
/// is not an omission on our side — the ACP schema defines the two cache counts
/// on `Usage` and no way to influence them (no `cache_control`, no breakpoint,
/// no ephemeral marker anywhere in the protocol), and Copilot CLI 1.0.81
/// advertises `loadSession`, `mcpCapabilities`, `promptCapabilities` and
/// `sessionCapabilities{close,list}`, with no caching capability at all.
///
/// Copilot does cache, and since embacle 0.22.0 the counts are reported and
/// billed — but it caches its OWN preamble, never our prompt. Measured
/// 2026-08-29 by `examples/acp_cache_boundary_probe.rs` in dravr-embacle, on
/// CLI 1.0.81 / claude-sonnet-5: prefixes of 32, 10k, 20k and 40k tokens were
/// each served exactly 13,964 cached tokens on the following turn — identical
/// to the token across a 1,250x change in what we send — while the cache
/// *write* tracked our prompt size (14,214 / 26,276 / 38,371 / 62,564). We pay
/// the write premium on the whole prompt every turn and are served none of it
/// back.
///
/// So prompt LAYOUT is not a lever on this path: no ordering of our blocks can
/// move `cachedReadTokens`, because our bytes are never in the cached region.
/// Prompt SIZE is the only thing on our side of the boundary. Re-run the probe
/// before believing otherwise; a vendor that began honouring our prefix would
/// show the cached read growing with it.
struct HeadlessRunnerAdapter(Arc<CopilotHeadlessRunner>);

#[async_trait]
impl EmbacleLlmProvider for HeadlessRunnerAdapter {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn display_name(&self) -> &str {
        self.0.display_name()
    }

    fn capabilities(&self) -> super::LlmCapabilities {
        self.0.capabilities()
    }

    fn default_model(&self) -> &str {
        self.0.default_model()
    }

    fn available_models(&self) -> &[String] {
        self.0.available_models()
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.0.complete(request).await
    }

    async fn complete_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<EmbacleChatStream, RunnerError> {
        self.0.complete_stream(request).await
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        self.0.health_check().await
    }
}
