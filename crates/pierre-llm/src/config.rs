// ABOUTME: LLM configuration types for provider selection and model settings
// ABOUTME: Contains LlmProviderType enum and LlmModelConfig for environment-based configuration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use embacle::CopilotHeadlessConfig;
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt::{Display, Formatter, Result as FmtResult};

/// LLM provider selection for chat functionality
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProviderType {
    /// Groq provider - LPU-accelerated inference for Llama/Mixtral models
    Groq,
    /// Google Gemini provider - full-featured with vision support (default)
    #[default]
    Gemini,
    /// Local LLM provider - `OpenAI`-compatible endpoint (Ollama, vLLM, `LocalAI`)
    Local,
    /// Claude Code CLI provider - subprocess-based Anthropic Claude
    ClaudeCode,
    /// Cursor Agent CLI provider - subprocess-based Cursor AI
    CursorAgent,
    /// `OpenCode` CLI provider - subprocess-based open-source coding agent
    OpenCode,
    /// GitHub Copilot CLI provider - subprocess-based GitHub Copilot
    Copilot,
    /// GitHub Copilot Headless provider - ACP via `copilot --acp`
    CopilotHeadless,
    /// Warp terminal `oz` CLI provider
    WarpCli,
    /// Gemini CLI provider - subprocess-based Google Gemini
    GeminiCli,
    /// Codex CLI provider - subprocess-based `OpenAI` Codex
    CodexCli,
    /// Goose CLI provider - subprocess-based Goose coding agent
    GooseCli,
    /// Cline CLI provider - subprocess-based Cline coding agent
    ClineCli,
    /// Continue CLI provider - subprocess-based Continue coding agent
    ContinueCli,
    /// Kiro CLI provider - subprocess-based AWS Kiro coding agent
    KiroCli,
    /// Kilo Code CLI provider - subprocess-based Kilo coding agent
    KiloCli,
    /// `OpenAI`-compatible HTTP API provider via embacle (any `OpenAI`-compatible endpoint)
    OpenAiApi,
}

impl LlmProviderType {
    /// Environment variable name for LLM provider selection
    pub const ENV_VAR: &'static str = "PIERRE_LLM_PROVIDER";

    /// Parse from string with fallback to default
    #[must_use]
    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "groq" => Self::Groq,
            "local" | "ollama" | "vllm" | "localai" => Self::Local,
            "claude_code" | "claude-code" => Self::ClaudeCode,
            "cursor_agent" | "cursor-agent" => Self::CursorAgent,
            "opencode" | "open_code" => Self::OpenCode,
            "copilot" | "github_copilot" | "github-copilot" => Self::Copilot,
            "copilot_headless" | "copilot-headless" => Self::CopilotHeadless,
            "warp_cli" | "warp-cli" | "warp" | "oz" => Self::WarpCli,
            "gemini_cli" | "gemini-cli" => Self::GeminiCli,
            "codex_cli" | "codex-cli" | "codex" => Self::CodexCli,
            "goose_cli" | "goose-cli" | "goose" => Self::GooseCli,
            "cline_cli" | "cline-cli" | "cline" => Self::ClineCli,
            "continue_cli" | "continue-cli" | "continue" => Self::ContinueCli,
            "kiro_cli" | "kiro-cli" | "kiro" => Self::KiroCli,
            "kilo_cli" | "kilo-cli" | "kilo" => Self::KiloCli,
            "openai_api" | "openai-api" | "openai" => Self::OpenAiApi,
            _ => Self::Gemini, // Default fallback (including "gemini", "google")
        }
    }

    /// Load from environment variable
    #[must_use]
    pub fn from_env() -> Self {
        env::var(Self::ENV_VAR)
            .map(|s| Self::from_str_or_default(&s))
            .unwrap_or_default()
    }
}

impl LlmProviderType {
    /// Environment variable for model/version selection
    pub const MODEL_ENV_VAR: &'static str = "PIERRE_LLM_MODEL";

    /// Environment variable for enabling fallback
    pub const FALLBACK_ENABLED_ENV_VAR: &'static str = "PIERRE_LLM_FALLBACK_ENABLED";

    /// Environment variable for fallback provider selection
    pub const FALLBACK_PROVIDER_ENV_VAR: &'static str = "PIERRE_LLM_FALLBACK_PROVIDER";

    /// Environment variable for fallback model selection
    pub const FALLBACK_MODEL_ENV_VAR: &'static str = "PIERRE_LLM_FALLBACK_MODEL";

    /// Environment variable for fallback wait time in seconds
    pub const FALLBACK_WAIT_SECS_ENV_VAR: &'static str = "PIERRE_LLM_FALLBACK_WAIT_SECS";

    /// Environment variable for opting into per-request runtime fallback.
    ///
    /// Distinct from [`Self::FALLBACK_ENABLED_ENV_VAR`], which only governs
    /// boot-time selection: with `RUNTIME_FALLBACK_ENV_VAR=true`, the
    /// `ChatProvider` keeps both primary and fallback initialized and
    /// retries individual requests against the fallback on retryable
    /// upstream errors (auth, 5xx, transient network).
    pub const RUNTIME_FALLBACK_ENV_VAR: &'static str = "PIERRE_LLM_RUNTIME_FALLBACK";

    /// Default wait time before attempting fallback (10 seconds, matches Gemini retry)
    pub const DEFAULT_FALLBACK_WAIT_SECS: u64 = 10;

    /// Get model from environment, respecting the active provider
    ///
    /// For API-based providers (Gemini, Groq, Local), reads `PIERRE_LLM_MODEL`.
    /// For embacle-based providers, reads the runner-specific env var
    /// (e.g. `COPILOT_SDK_MODEL`) or uses the runner's built-in default.
    /// Returns None only if no model can be determined.
    #[must_use]
    pub fn model_from_env() -> Option<String> {
        let provider = Self::from_env();

        // Embacle-based providers have their own model configuration
        if let Some(model) = provider.embacle_model_from_env() {
            return Some(model);
        }

        // API-based providers use PIERRE_LLM_MODEL
        match env::var(Self::MODEL_ENV_VAR) {
            Ok(model) if !model.is_empty() => Some(model),
            _ => {
                tracing::error!(
                    "{} environment variable not set. Configure it in .envrc to specify the LLM model.",
                    Self::MODEL_ENV_VAR
                );
                None
            }
        }
    }

    /// Get model name for embacle-based providers
    ///
    /// `PIERRE_LLM_MODEL` is the unified override for ALL providers (API and embacle).
    /// When set, it takes priority over provider-specific env vars (e.g. `COPILOT_SDK_MODEL`,
    /// `CLI_LLM_MODEL`). When not set, falls back to each runner's own default.
    /// Returns None for non-embacle providers.
    #[must_use]
    fn embacle_model_from_env(self) -> Option<String> {
        match self {
            Self::Gemini | Self::Groq | Self::Local => return None,
            _ => {}
        }

        // PIERRE_LLM_MODEL is the unified model override for all providers
        if let Ok(model) = env::var(Self::MODEL_ENV_VAR) {
            if !model.is_empty() {
                return Some(model);
            }
        }

        // Provider-specific defaults when PIERRE_LLM_MODEL is not set
        match self {
            // CopilotHeadlessConfig reads COPILOT_HEADLESS_MODEL with its built-in default
            Self::CopilotHeadless => Some(CopilotHeadlessConfig::from_env().model),
            // CLI runners pick their default model internally in new().
            // These fallbacks are only used for the conversation DB label when
            // RunnerConfig.model is None (no env override).
            Self::ClaudeCode => Some("opus".to_owned()),
            Self::Copilot => Some("claude-opus-4.7".to_owned()),
            Self::CursorAgent => Some("sonnet-4".to_owned()),
            Self::OpenCode => Some("anthropic/claude-sonnet-4".to_owned()),
            Self::GeminiCli => Some("gemini-2.5-pro".to_owned()),
            Self::CodexCli => Some("codex-mini".to_owned()),
            Self::GooseCli
            | Self::ClineCli
            | Self::ContinueCli
            | Self::WarpCli
            | Self::KiroCli
            | Self::KiloCli => Some("claude-sonnet-4".to_owned()),
            Self::OpenAiApi => Some(
                env::var("OPENAI_API_MODEL")
                    .ok()
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "gpt-5.4".to_owned()),
            ),
            Self::Gemini | Self::Groq | Self::Local => unreachable!(),
        }
    }

    /// Check if fallback is enabled from environment
    ///
    /// Reads `PIERRE_LLM_FALLBACK_ENABLED` - returns true only if set to "true" or "1".
    /// Disabled by default.
    #[must_use]
    pub fn is_fallback_enabled() -> bool {
        env::var(Self::FALLBACK_ENABLED_ENV_VAR)
            .is_ok_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
    }

    /// Get fallback provider from environment
    ///
    /// Reads `PIERRE_LLM_FALLBACK_PROVIDER` - returns None if not set.
    /// Must be explicitly configured.
    #[must_use]
    pub fn fallback_provider_from_env() -> Option<Self> {
        env::var(Self::FALLBACK_PROVIDER_ENV_VAR)
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| Self::from_str_or_default(&s))
    }

    /// Get fallback model from environment
    ///
    /// Reads `PIERRE_LLM_FALLBACK_MODEL` - returns None if not set.
    /// When fallback is triggered, uses this model or the default.
    #[must_use]
    pub fn fallback_model_from_env() -> Option<String> {
        env::var(Self::FALLBACK_MODEL_ENV_VAR)
            .ok()
            .filter(|s| !s.is_empty())
    }

    /// Get fallback wait time in seconds from environment
    ///
    /// Reads `PIERRE_LLM_FALLBACK_WAIT_SECS` - defaults to 10 seconds.
    #[must_use]
    pub fn fallback_wait_secs() -> u64 {
        env::var(Self::FALLBACK_WAIT_SECS_ENV_VAR)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(Self::DEFAULT_FALLBACK_WAIT_SECS)
    }

    /// Check if per-request runtime fallback is enabled.
    ///
    /// Reads [`Self::RUNTIME_FALLBACK_ENV_VAR`]. When `true`, [`ChatProvider::from_env`]
    /// builds both the primary and the fallback provider eagerly and keeps them
    /// available so individual chat requests can retry against the fallback on
    /// auth/transient errors.
    #[must_use]
    pub fn is_runtime_fallback_enabled() -> bool {
        env::var(Self::RUNTIME_FALLBACK_ENV_VAR)
            .is_ok_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
    }
}

impl Display for LlmProviderType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Groq => write!(f, "groq"),
            Self::Gemini => write!(f, "gemini"),
            Self::Local => write!(f, "local"),
            Self::ClaudeCode => write!(f, "claude_code"),
            Self::CursorAgent => write!(f, "cursor_agent"),
            Self::OpenCode => write!(f, "opencode"),
            Self::Copilot => write!(f, "copilot"),
            Self::CopilotHeadless => write!(f, "copilot_headless"),
            Self::WarpCli => write!(f, "warp_cli"),
            Self::GeminiCli => write!(f, "gemini_cli"),
            Self::CodexCli => write!(f, "codex_cli"),
            Self::GooseCli => write!(f, "goose_cli"),
            Self::ClineCli => write!(f, "cline_cli"),
            Self::ContinueCli => write!(f, "continue_cli"),
            Self::KiroCli => write!(f, "kiro_cli"),
            Self::KiloCli => write!(f, "kilo_cli"),
            Self::OpenAiApi => write!(f, "openai_api"),
        }
    }
}

/// LLM model configuration for all providers (Gemini, Groq, Local)
///
/// Reads model names from environment variables:
/// - `PIERRE_LLM_DEFAULT_MODEL`: Primary model to use
/// - `PIERRE_LLM_FALLBACK_MODEL`: Fallback model when primary fails (rate limits, errors)
#[derive(Debug, Clone)]
pub struct LlmModelConfig {
    /// Default model to use for all requests
    pub default_model: String,
    /// Fallback model when default model fails (rate limits, errors, unavailable)
    pub fallback_model: String,
}

impl LlmModelConfig {
    /// Environment variable for default model
    pub const DEFAULT_MODEL_ENV_VAR: &'static str = "PIERRE_LLM_DEFAULT_MODEL";

    /// Environment variable for fallback model
    pub const FALLBACK_MODEL_ENV_VAR: &'static str = "PIERRE_LLM_FALLBACK_MODEL";

    /// Load model configuration from environment variables
    ///
    /// # Errors
    ///
    /// Returns an error if `PIERRE_LLM_DEFAULT_MODEL` is not set.
    pub fn from_env() -> Result<Self, String> {
        let default_model = env::var(Self::DEFAULT_MODEL_ENV_VAR).map_err(|_| {
            format!(
                "{} environment variable not set",
                Self::DEFAULT_MODEL_ENV_VAR
            )
        })?;

        let fallback_model = env::var(Self::FALLBACK_MODEL_ENV_VAR).map_err(|_| {
            format!(
                "{} environment variable not set",
                Self::FALLBACK_MODEL_ENV_VAR
            )
        })?;

        Ok(Self {
            default_model,
            fallback_model,
        })
    }

    /// Get the default model name
    #[must_use]
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Get the fallback model name
    #[must_use]
    pub fn fallback_model(&self) -> &str {
        &self.fallback_model
    }
}
