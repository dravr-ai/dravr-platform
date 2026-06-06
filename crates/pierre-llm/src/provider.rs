// ABOUTME: Unified LLM provider selector for runtime provider switching
// ABOUTME: Abstracts over Gemini, Groq, Local, OpenRouter, and embacle providers based on environment configuration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Provider Selector
//!
//! This module provides a unified interface for LLM providers that can be
//! configured at runtime via environment variables.
//!
//! ## Configuration
//!
//! Set `PIERRE_LLM_PROVIDER` environment variable:
//! - `gemini` (default): Use Google Gemini for full-featured capabilities
//! - `groq`: Use Groq for cost-effective open-source models
//! - `openrouter`: Use `OpenRouter` as a unified gateway to 200+ models
//! - `local`/`ollama`/`vllm`/`localai`: Use a local `OpenAI`-compatible endpoint
//! - `claude_code`/`copilot`/`cursor_agent`/`opencode`/`copilot_headless`/`warp_cli`: Use an embacle runner
//! - `openai_api`/`openai`: Use an `OpenAI`-compatible HTTP API via embacle

use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use super::{
    ChatMessage, ChatRequest, ChatResponse, ChatResponseWithTools, ChatStream, CliLlmProvider,
    CohereProvider, FunctionResponse, GeminiProvider, GroqProvider, LlmCapabilities, LlmProvider,
    OpenAiCompatibleProvider, OpenRouterProvider, Tool,
};
use crate::chain_guard::{CircuitTransition, CHAIN_GUARD};
use crate::config::LlmProviderType;
use crate::errors::AppError;
use embacle::CliRunnerType;

/// Unified chat provider that wraps Gemini, Groq, Local, `OpenRouter`, or embacle-based LLM
///
/// This enum provides a consistent interface regardless of which
/// underlying provider is configured.
pub enum ChatProvider {
    /// Google Gemini provider with full tool calling support
    Gemini(GeminiProvider),
    /// Groq provider for fast, cost-effective inference
    Groq(GroqProvider),
    /// Local LLM provider via `OpenAI`-compatible API (Ollama, vLLM, `LocalAI`)
    Local(OpenAiCompatibleProvider),
    /// `OpenRouter` — unified gateway to 200+ frontier and open-source models
    OpenRouter(OpenRouterProvider),
    /// Cohere — Command A and Command R family via Cohere v2 chat API
    Cohere(CohereProvider),
    /// Embacle-based LLM provider (CLI runners and SDK runners)
    Cli(CliLlmProvider),
    /// Custom provider supplied by the caller (used by tests to inject a
    /// deterministic mock). Never constructed in production code paths —
    /// production providers are resolved via [`ChatProvider::from_env`] or
    /// the per-tenant credential factory.
    Custom(Arc<dyn LlmProvider>),
    /// Runtime fallback chain. Built when `PIERRE_LLM_RUNTIME_FALLBACK=true`
    /// at boot. Every public method tries `primary` first and, on a
    /// retryable error (see [`is_retryable_for_fallback`]), reissues the
    /// call against `secondary`. Init-time fallback ([`from_env`]'s existing
    /// behavior) is preserved separately by `PIERRE_LLM_FALLBACK_ENABLED`.
    Chain {
        /// Provider tried first for every request.
        primary: Box<Self>,
        /// Provider tried when `primary` returns a retryable error.
        secondary: Box<Self>,
    },
}

impl ChatProvider {
    /// Create a provider from environment configuration
    ///
    /// Reads `PIERRE_LLM_PROVIDER` to determine which provider to use:
    /// - `gemini` (default): Creates `GeminiProvider` (requires `GEMINI_API_KEY`)
    /// - `groq`: Creates `GroqProvider` (requires `GROQ_API_KEY`)
    /// - `openrouter`: Creates `OpenRouterProvider` (requires `OPENROUTER_API_KEY`)
    /// - `local`/`ollama`/`vllm`/`localai`: Creates `OpenAiCompatibleProvider`
    /// - `claude_code`/`copilot`/`cursor_agent`/`opencode`/`copilot_headless`/`warp_cli`: Embacle runners
    ///
    /// When `PIERRE_LLM_FALLBACK_ENABLED=true`, if the primary provider fails,
    /// attempts to use the fallback provider specified by `PIERRE_LLM_PROVIDER_FALLBACK`.
    ///
    /// # Errors
    ///
    /// Returns an error if the required API key environment variable is missing
    /// (for cloud providers) or if the local server cannot be reached, and
    /// fallback is disabled or also fails.
    pub async fn from_env() -> Result<Self, AppError> {
        let provider_type = LlmProviderType::from_env();

        info!(
            "Initializing LLM provider: {} (set {} to change)",
            provider_type,
            LlmProviderType::ENV_VAR
        );

        let primary_result = Self::create_provider(provider_type).await;

        if LlmProviderType::is_runtime_fallback_enabled() {
            return Self::build_runtime_chain(primary_result, provider_type).await;
        }

        Self::finalize_or_fallback(primary_result, provider_type).await
    }

    /// Build a [`ChatProvider::Chain`] when `PIERRE_LLM_RUNTIME_FALLBACK=true`.
    ///
    /// Requires `PIERRE_LLM_FALLBACK_PROVIDER` to be set. If the primary
    /// fails to initialize, the chain collapses to a fallback-only provider
    /// (same effective result as the init-time fallback path). If the
    /// fallback is missing or fails to initialize, the chain collapses to
    /// primary-only so callers always get a usable provider when at least
    /// one side worked.
    async fn build_runtime_chain(
        primary_result: Result<Self, AppError>,
        primary_type: LlmProviderType,
    ) -> Result<Self, AppError> {
        let fallback_type = LlmProviderType::fallback_provider_from_env();

        let Some(fallback_type) = fallback_type else {
            warn!(
                "{} is true but {} is unset — runtime fallback disabled",
                LlmProviderType::RUNTIME_FALLBACK_ENV_VAR,
                LlmProviderType::FALLBACK_PROVIDER_ENV_VAR,
            );
            return Self::finalize_or_fallback(primary_result, primary_type).await;
        };

        if fallback_type == primary_type {
            warn!(
                "{} matches {} ({primary_type}) — runtime fallback disabled",
                LlmProviderType::FALLBACK_PROVIDER_ENV_VAR,
                LlmProviderType::ENV_VAR,
            );
            return Self::finalize_or_fallback(primary_result, primary_type).await;
        }

        let secondary_result = Self::create_fallback_provider(fallback_type).await;

        // Optional tertiary: when PIERRE_LLM_TERTIARY_PROVIDER is set, wrap
        // the secondary into a nested Chain so the chain becomes
        // primary -> {secondary, tertiary}. The secondary path keeps the
        // same retry-classification rules — a retryable error inside the
        // nested chain cascades to the tertiary before bubbling back up.
        let tertiary_pair = Self::maybe_build_tertiary(primary_type, fallback_type).await;
        let secondary_result =
            Self::compose_secondary_with_tertiary(secondary_result, tertiary_pair, fallback_type);

        Self::resolve_runtime_chain(
            primary_result,
            secondary_result,
            primary_type,
            fallback_type,
        )
    }

    /// Resolve the tertiary provider configuration (if any) and build it.
    ///
    /// Returns `Some((type, result))` when [`LlmProviderType::TERTIARY_PROVIDER_ENV_VAR`]
    /// is set and resolves to a different type than both primary and
    /// secondary (so the chain doesn't degenerate). Returns `None`
    /// otherwise — the chain stays two-tier with original behavior.
    async fn maybe_build_tertiary(
        primary_type: LlmProviderType,
        fallback_type: LlmProviderType,
    ) -> Option<(LlmProviderType, Result<Self, AppError>)> {
        let tertiary_type = LlmProviderType::tertiary_provider_from_env()?;

        if tertiary_type == primary_type || tertiary_type == fallback_type {
            warn!(
                "{} ({tertiary_type}) matches an existing tier — tertiary fallback disabled",
                LlmProviderType::TERTIARY_PROVIDER_ENV_VAR,
            );
            return None;
        }

        Some((
            tertiary_type,
            Self::create_tertiary_provider(tertiary_type).await,
        ))
    }

    /// Build the tertiary provider, applying the dedicated tertiary model
    /// override env var. Same dispatch logic as
    /// [`Self::create_fallback_provider`] but reading the tertiary's env
    /// vars so each tier can target a different model namespace
    /// (e.g. Copilot's `claude-opus-4.7`, Gemini's
    /// `gemini-flash-lite-latest`, Cohere's `command-a-03-2025`).
    async fn create_tertiary_provider(tertiary_type: LlmProviderType) -> Result<Self, AppError> {
        let model_override = LlmProviderType::tertiary_provider_model_from_env();

        if let Some(runner_type) = cli_runner_type_for(tertiary_type) {
            return Ok(Self::Cli(CliLlmProvider::from_runner_type_with_model(
                runner_type,
                model_override.as_deref(),
            )?));
        }

        let provider = Self::create_provider(tertiary_type).await?;
        Ok(match model_override {
            Some(m) => provider.with_model(&m),
            None => provider,
        })
    }

    /// Compose the secondary with an optional tertiary into a nested chain.
    ///
    /// Possible outcomes:
    /// - No tertiary configured -> pass `secondary_result` through unchanged.
    /// - Both secondary and tertiary built -> wrap them in
    ///   `Chain { primary: secondary, secondary: tertiary }`.
    /// - Secondary failed, tertiary built -> use the tertiary in place of
    ///   the secondary (graceful degradation; `resolve_runtime_chain` still
    ///   sees an `Ok(provider)` and builds a two-tier chain
    ///   primary -> tertiary).
    /// - Secondary built, tertiary failed -> log the tertiary failure and
    ///   fall back to the original two-tier chain.
    /// - Both failed -> bubble the secondary's error up so
    ///   `resolve_runtime_chain` can degrade to primary-only.
    ///
    /// Each branch delegates to a dedicated helper (`compose_*`) so the
    /// match itself stays below the workspace cognitive-complexity budget.
    fn compose_secondary_with_tertiary(
        secondary_result: Result<Self, AppError>,
        tertiary_pair: Option<(LlmProviderType, Result<Self, AppError>)>,
        fallback_type: LlmProviderType,
    ) -> Result<Self, AppError> {
        let Some((tertiary_type, tertiary_result)) = tertiary_pair else {
            return secondary_result;
        };

        match (secondary_result, tertiary_result) {
            (Ok(secondary), Ok(tertiary)) => Ok(Self::compose_both_built(
                secondary,
                tertiary,
                fallback_type,
                tertiary_type,
            )),
            (Err(secondary_err), Ok(tertiary)) => Ok(Self::compose_promote_tertiary(
                tertiary,
                fallback_type,
                tertiary_type,
                &secondary_err,
            )),
            (Ok(secondary), Err(tertiary_err)) => Ok(Self::compose_keep_secondary(
                secondary,
                fallback_type,
                tertiary_type,
                &tertiary_err,
            )),
            (Err(secondary_err), Err(tertiary_err)) => Err(Self::compose_both_failed(
                fallback_type,
                tertiary_type,
                &tertiary_err,
                secondary_err,
            )),
        }
    }

    /// Build the nested `Chain { secondary, tertiary }` when both providers
    /// initialized cleanly.
    fn compose_both_built(
        secondary: Self,
        tertiary: Self,
        fallback_type: LlmProviderType,
        tertiary_type: LlmProviderType,
    ) -> Self {
        info!(
            secondary = %fallback_type,
            tertiary = %tertiary_type,
            "Tertiary LLM fallback initialized; chain is primary -> {{secondary, tertiary}}"
        );
        validate_model_for_provider(&tertiary);
        Self::Chain {
            primary: Box::new(secondary),
            secondary: Box::new(tertiary),
        }
    }

    /// Promote the tertiary into the secondary slot when the secondary
    /// failed to initialize. The outer chain stays two-tier
    /// (primary -> tertiary) so the caller still gets a usable chain.
    fn compose_promote_tertiary(
        tertiary: Self,
        fallback_type: LlmProviderType,
        tertiary_type: LlmProviderType,
        secondary_err: &AppError,
    ) -> Self {
        warn!(
            secondary = %fallback_type,
            tertiary = %tertiary_type,
            error = %secondary_err,
            "Secondary provider failed to initialize; promoting tertiary"
        );
        validate_model_for_provider(&tertiary);
        tertiary
    }

    /// Keep the secondary as-is when only the tertiary failed to
    /// initialize. The chain degrades to the original two-tier shape.
    fn compose_keep_secondary(
        secondary: Self,
        fallback_type: LlmProviderType,
        tertiary_type: LlmProviderType,
        tertiary_err: &AppError,
    ) -> Self {
        warn!(
            secondary = %fallback_type,
            tertiary = %tertiary_type,
            error = %tertiary_err,
            "Tertiary provider failed to initialize; running two-tier chain"
        );
        secondary
    }

    /// Bubble the secondary error when both providers failed —
    /// `resolve_runtime_chain` will degrade to primary-only.
    fn compose_both_failed(
        fallback_type: LlmProviderType,
        tertiary_type: LlmProviderType,
        tertiary_err: &AppError,
        secondary_err: AppError,
    ) -> AppError {
        warn!(
            secondary = %fallback_type,
            tertiary = %tertiary_type,
            secondary_error = %secondary_err,
            tertiary_error = %tertiary_err,
            "Both secondary and tertiary failed; falling back to primary-only"
        );
        secondary_err
    }

    /// Build the runtime chain's secondary provider.
    ///
    /// Differs from [`Self::create_provider`] in two ways:
    ///
    /// 1. **Explicit runner dispatch for CLI types.** `Self::create_provider`
    ///    routes every CLI variant through `Self::cli()`, which re-reads
    ///    `PIERRE_LLM_PROVIDER` and would build the primary's runner type a
    ///    second time. The secondary needs the FALLBACK runner, not the
    ///    primary, so we map `LlmProviderType` → `CliRunnerType` here and
    ///    bypass that env-var dispatch.
    /// 2. **Honors `PIERRE_LLM_FALLBACK_PROVIDER_MODEL`.** When set, the
    ///    secondary uses this model instead of inheriting `PIERRE_LLM_MODEL`
    ///    from the primary — required when the two providers use different
    ///    naming conventions for the same upstream SKU (Copilot's
    ///    `claude-opus-4.7` vs Anthropic's `claude-opus-4-7`).
    ///
    /// Non-CLI fallback types (Groq, Gemini, Local, `OpenRouter`) start from
    /// [`Self::create_provider`], then have `PIERRE_LLM_FALLBACK_PROVIDER_MODEL`
    /// applied on top. Their stock `from_env()` paths read
    /// `PIERRE_LLM_DEFAULT_MODEL`, which the primary owns — so without this
    /// override the Gemini fallback would send Copilot's
    /// `claude-opus-4.7` model name to `generativelanguage.googleapis.com`
    /// and 404 the entire fallback chain.
    async fn create_fallback_provider(fallback_type: LlmProviderType) -> Result<Self, AppError> {
        let model_override = LlmProviderType::fallback_provider_model_from_env();

        if let Some(runner_type) = cli_runner_type_for(fallback_type) {
            return Ok(Self::Cli(CliLlmProvider::from_runner_type_with_model(
                runner_type,
                model_override.as_deref(),
            )?));
        }

        let provider = Self::create_provider(fallback_type).await?;
        Ok(match model_override {
            Some(m) => provider.with_model(&m),
            None => provider,
        })
    }

    /// Combine the primary and secondary init results into a single provider
    /// (the chain, or whichever side worked, or a combined error).
    ///
    /// Split from [`Self::build_runtime_chain`] purely to keep its cognitive
    /// complexity under the workspace clippy budget — same match block,
    /// no behavioral change.
    fn resolve_runtime_chain(
        primary_result: Result<Self, AppError>,
        secondary_result: Result<Self, AppError>,
        primary_type: LlmProviderType,
        fallback_type: LlmProviderType,
    ) -> Result<Self, AppError> {
        match (primary_result, secondary_result) {
            (Ok(primary), Ok(secondary)) => {
                info!(
                    primary = %primary_type,
                    secondary = %fallback_type,
                    "Runtime LLM fallback chain initialized"
                );
                validate_model_for_provider(&primary);
                validate_model_for_provider(&secondary);
                Ok(Self::Chain {
                    primary: Box::new(primary),
                    secondary: Box::new(secondary),
                })
            }
            (Ok(primary), Err(secondary_err)) => {
                warn!(
                    primary = %primary_type,
                    secondary = %fallback_type,
                    error = %secondary_err,
                    "Fallback provider failed to initialize; running primary-only"
                );
                validate_model_for_provider(&primary);
                Ok(primary)
            }
            (Err(primary_err), Ok(secondary)) => {
                warn!(
                    primary = %primary_type,
                    secondary = %fallback_type,
                    error = %primary_err,
                    "Primary provider failed to initialize; running fallback-only"
                );
                validate_model_for_provider(&secondary);
                Ok(secondary)
            }
            (Err(primary_err), Err(secondary_err)) => Err(AppError::config(format!(
                "Both runtime-chain providers failed. Primary ({primary_type}): {primary_err}. \
                 Secondary ({fallback_type}): {secondary_err}"
            ))),
        }
    }

    /// Finalize provider initialization or attempt fallback on failure
    async fn finalize_or_fallback(
        result: Result<Self, AppError>,
        provider_type: LlmProviderType,
    ) -> Result<Self, AppError> {
        match result {
            Ok(provider) => {
                debug!(
                    "Provider {} initialized with model: {}",
                    provider.display_name(),
                    provider.default_model()
                );
                validate_model_for_provider(&provider);
                Ok(provider)
            }
            Err(primary_error) => Self::try_fallback(provider_type, primary_error).await,
        }
    }

    /// Attempt to initialize a fallback provider after primary fails
    async fn try_fallback(
        primary_type: LlmProviderType,
        primary_error: AppError,
    ) -> Result<Self, AppError> {
        let fallback_enabled = LlmProviderType::is_fallback_enabled();
        let fallback_provider = LlmProviderType::fallback_provider_from_env();

        let Some(fallback) = fallback_provider else {
            return Err(primary_error);
        };

        if !fallback_enabled || fallback == primary_type {
            return Err(primary_error);
        }

        let wait_secs = LlmProviderType::fallback_wait_secs();
        info!(
            "Primary provider {} failed, waiting {}s before fallback to {}",
            primary_type, wait_secs, fallback
        );

        sleep(Duration::from_secs(wait_secs)).await;

        match Self::create_provider(fallback).await {
            Ok(provider) => {
                info!(
                    "Fallback provider {} initialized with model: {}",
                    provider.display_name(),
                    provider.default_model()
                );
                Ok(provider)
            }
            Err(fallback_error) => Err(AppError::config(format!(
                "Both primary ({primary_type}) and fallback ({fallback}) providers failed. \
                Primary: {primary_error}. Fallback: {fallback_error}"
            ))),
        }
    }

    /// Create a provider for a specific type
    async fn create_provider(provider_type: LlmProviderType) -> Result<Self, AppError> {
        match provider_type {
            LlmProviderType::Groq => Self::groq(),
            LlmProviderType::Gemini => Self::gemini(),
            LlmProviderType::Local => Self::local(),
            LlmProviderType::OpenRouter => Self::openrouter(),
            LlmProviderType::Cohere => Self::cohere(),
            LlmProviderType::ClaudeCode
            | LlmProviderType::Copilot
            | LlmProviderType::CursorAgent
            | LlmProviderType::OpenCode
            | LlmProviderType::CopilotHeadless
            | LlmProviderType::GeminiCli
            | LlmProviderType::CodexCli
            | LlmProviderType::GooseCli
            | LlmProviderType::ClineCli
            | LlmProviderType::ContinueCli
            | LlmProviderType::WarpCli
            | LlmProviderType::KiroCli
            | LlmProviderType::KiloCli
            | LlmProviderType::OpenAiApi => Self::cli().await,
        }
    }

    /// Create a Gemini provider explicitly
    ///
    /// # Errors
    ///
    /// Returns an error if `GEMINI_API_KEY` is not set.
    pub fn gemini() -> Result<Self, AppError> {
        Ok(Self::Gemini(GeminiProvider::from_env()?))
    }

    /// Create a Groq provider explicitly
    ///
    /// # Errors
    ///
    /// Returns an error if `GROQ_API_KEY` is not set.
    pub fn groq() -> Result<Self, AppError> {
        Ok(Self::Groq(GroqProvider::from_env()?))
    }

    /// Create a local LLM provider explicitly
    ///
    /// Uses environment variables for configuration:
    /// - `LOCAL_LLM_BASE_URL`: API endpoint (default: Ollama at localhost:11434)
    /// - `LOCAL_LLM_MODEL`: Model name (default: qwen2.5:14b-instruct)
    /// - `LOCAL_LLM_API_KEY`: API key (optional)
    ///
    /// # Errors
    ///
    /// Returns an error if the provider cannot be initialized.
    pub fn local() -> Result<Self, AppError> {
        Ok(Self::Local(OpenAiCompatibleProvider::from_env()?))
    }

    /// Create an `OpenRouter` provider explicitly
    ///
    /// # Errors
    ///
    /// Returns an error if `OPENROUTER_API_KEY` is not set.
    pub fn openrouter() -> Result<Self, AppError> {
        Ok(Self::OpenRouter(OpenRouterProvider::from_env()?))
    }

    /// Create a Cohere provider explicitly
    ///
    /// # Errors
    ///
    /// Returns an error if `COHERE_API_KEY` is not set.
    pub fn cohere() -> Result<Self, AppError> {
        Ok(Self::Cohere(CohereProvider::from_env()?))
    }

    /// Create an embacle-based LLM provider explicitly
    ///
    /// Auto-detects or reads `PIERRE_LLM_PROVIDER` to select the runner
    /// (Claude Code, Copilot CLI, Cursor Agent, `OpenCode`, or Copilot SDK).
    ///
    /// # Errors
    ///
    /// Returns an error if no runner can be detected or initialized.
    pub async fn cli() -> Result<Self, AppError> {
        Ok(Self::Cli(CliLlmProvider::from_env().await?))
    }

    /// Create a Gemini provider with a specific API key
    ///
    /// Use this when you have already resolved the API key from tenant/user credentials.
    ///
    /// # Errors
    ///
    /// Returns an error if LLM model config is not set in environment.
    pub fn gemini_with_key(api_key: &str) -> Result<Self, AppError> {
        Ok(Self::Gemini(GeminiProvider::new(api_key)?))
    }

    /// Create a Groq provider with a specific API key
    ///
    /// Use this when you have already resolved the API key from tenant/user credentials.
    #[must_use]
    pub fn groq_with_key(api_key: String) -> Self {
        Self::Groq(GroqProvider::new(api_key))
    }

    /// Create an `OpenRouter` provider with a specific API key
    ///
    /// Use this when you have already resolved the API key from tenant/user credentials.
    #[must_use]
    pub fn openrouter_with_key(api_key: String) -> Self {
        Self::OpenRouter(OpenRouterProvider::new(api_key))
    }

    /// Create a Cohere provider with a specific API key
    ///
    /// Use this when you have already resolved the API key from tenant/user credentials.
    #[must_use]
    pub fn cohere_with_key(api_key: String) -> Self {
        Self::Cohere(CohereProvider::new(api_key))
    }

    /// Get the provider type
    #[must_use]
    pub fn provider_type(&self) -> LlmProviderType {
        match self {
            // Test-injected custom providers report as Gemini for metrics/analytics
            // — the variant is test-only and not part of the production taxonomy.
            Self::Gemini(_) | Self::Custom(_) => LlmProviderType::Gemini,
            Self::Groq(_) => LlmProviderType::Groq,
            Self::Local(_) => LlmProviderType::Local,
            Self::OpenRouter(_) => LlmProviderType::OpenRouter,
            Self::Cohere(_) => LlmProviderType::Cohere,
            Self::Cli(p) => p.provider_type(),
            // The chain reports as the primary — metrics and analytics
            // should attribute requests to the active first-line provider;
            // when fallback fires, the warn! log line carries both names.
            Self::Chain { primary, .. } => primary.provider_type(),
        }
    }

    /// Check if this provider supports tool calling
    #[must_use]
    pub fn supports_tool_calling(&self) -> bool {
        self.capabilities().supports_function_calling()
    }

    /// Get the inner `CliLlmProvider` if this is an embacle-based provider
    #[must_use]
    pub fn as_cli_provider(&self) -> Option<&CliLlmProvider> {
        match self {
            Self::Cli(p) => Some(p),
            _ => None,
        }
    }

    /// Perform a chat completion with tool/function calling support
    ///
    /// Gemini, Groq, Local, and `OpenRouter` providers all support native function/tool calling
    /// via their respective APIs (Gemini native, the others via OpenAI-compatible).
    ///
    /// Tool-calling always uses non-streaming mode. Streaming tool-call accumulation
    /// is complex and provides negligible UX benefit for short tool-call payloads.
    /// If `request.stream` is true, it is ignored for tool-calling requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    pub async fn complete_with_tools(
        &self,
        request: &ChatRequest,
        tools: Option<Vec<Tool>>,
    ) -> Result<ChatResponseWithTools, AppError> {
        match self {
            Self::Gemini(provider) => provider.complete_with_tools(request, tools).await,
            Self::Groq(provider) => provider.complete_with_tools(request, tools).await,
            Self::Local(provider) => provider.complete_with_tools(request, tools).await,
            Self::OpenRouter(provider) => provider.complete_with_tools(request, tools).await,
            Self::Cohere(provider) => provider.complete_with_tools(request, tools).await,
            Self::Cli(_) => Err(AppError::invalid_input(
                "Embacle-based providers do not support structured tool calling via this path",
            )),
            // Custom providers run through the plain `complete()` path. Tools
            // are advertised via the system prompt; the mock in tests replies
            // with plain text so there is no tool-call payload to decode.
            Self::Custom(inner) => {
                let response = inner.complete(request).await?;
                Ok(ChatResponseWithTools {
                    content: Some(response.content),
                    model: response.model,
                    usage: response.usage,
                    function_calls: None,
                    finish_reason: response.finish_reason,
                })
            }
            Self::Chain { primary, secondary } => {
                // Recursive async over `Self` requires Box::pin to satisfy
                // the compiler's "recursive async fn must introduce
                // indirection" rule.
                let primary_call = Box::pin(primary.complete_with_tools(request, tools.clone()));
                match primary_call.await {
                    Ok(response) => Ok(response),
                    Err(primary_err) if is_retryable_for_fallback(&primary_err) => {
                        warn!(
                            primary = primary.name(),
                            secondary = secondary.name(),
                            error = %primary_err,
                            "Primary LLM complete_with_tools() failed with retryable error; falling back"
                        );
                        let forwarded = request_for_secondary(request);
                        let secondary_call =
                            Box::pin(secondary.complete_with_tools(&forwarded, tools));
                        secondary_call.await
                    }
                    Err(primary_err) => Err(primary_err),
                }
            }
        }
    }

    /// Add function responses to messages for multi-turn tool execution
    ///
    /// This helper adds function response content back to the conversation
    /// for the next LLM iteration.
    pub fn add_function_responses_to_messages(
        messages: &mut Vec<ChatMessage>,
        function_responses: &[FunctionResponse],
    ) {
        for func_response in function_responses {
            let response_text =
                serde_json::to_string(&func_response.response).unwrap_or_else(|_| "{}".to_owned());
            messages.push(ChatMessage::user(format!(
                "[Tool Result for {}]: {}",
                func_response.name, response_text
            )));
        }
    }
}

// Delegate LlmProvider trait methods to the underlying provider.
// The canonical match-arm delegation lives in the LlmProvider trait impl below.
// These inherent methods delegate to it so callers don't need to import the trait.
impl ChatProvider {
    /// Get provider name
    #[must_use]
    pub fn name(&self) -> &'static str {
        LlmProvider::name(self)
    }

    /// Apply a model override to a provider after construction.
    ///
    /// Used by the runtime fallback chain to honor
    /// `PIERRE_LLM_FALLBACK_PROVIDER_MODEL` for the secondary provider —
    /// the secondary's `from_env()` would otherwise read
    /// `PIERRE_LLM_DEFAULT_MODEL`, which belongs to the primary and may be
    /// in a model namespace the secondary's API cannot resolve (e.g. a
    /// Copilot Claude SKU sent to `generativelanguage.googleapis.com`
    /// returns 404 and collapses the fallback).
    ///
    /// Only affects the variants whose URL/path embeds a model name
    /// (Gemini, Groq, Local, `OpenRouter`). `Cli` variants take their model
    /// from the embacle runner config and are handled via
    /// `CliLlmProvider::from_runner_type_with_model` instead. `Chain`
    /// and `Custom` are pass-through — chain composition applies the
    /// override to its primary/secondary before they're nested, and
    /// `Custom` is test-only.
    #[must_use]
    fn with_model(self, model: &str) -> Self {
        match self {
            Self::Gemini(p) => Self::Gemini(p.with_default_model(model)),
            Self::Groq(p) => Self::Groq(p.with_default_model(model)),
            Self::Local(p) => Self::Local(p.with_default_model(model)),
            Self::OpenRouter(p) => Self::OpenRouter(p.with_default_model(model)),
            Self::Cohere(p) => Self::Cohere(p.with_default_model(model)),
            other => other,
        }
    }

    /// Get provider display name
    #[must_use]
    pub fn display_name(&self) -> &'static str {
        LlmProvider::display_name(self)
    }

    /// Get provider capabilities
    #[must_use]
    pub fn capabilities(&self) -> LlmCapabilities {
        LlmProvider::capabilities(self)
    }

    /// Get default model
    #[must_use]
    pub fn default_model(&self) -> &str {
        LlmProvider::default_model(self)
    }

    /// Get available models
    #[must_use]
    pub fn available_models(&self) -> &[String] {
        LlmProvider::available_models(self)
    }

    /// Perform a chat completion
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    pub async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        LlmProvider::complete(self, request).await
    }

    /// Perform a streaming chat completion
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    pub async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        LlmProvider::complete_stream(self, request).await
    }

    /// Check provider health
    ///
    /// # Errors
    ///
    /// Returns an error if the health check fails.
    pub async fn health_check(&self) -> Result<bool, AppError> {
        LlmProvider::health_check(self).await
    }
}

/// Map an [`LlmProviderType`] to its corresponding embacle [`CliRunnerType`]
/// for the subprocess-CLI runners.
///
/// Returns `None` for provider types that are not driven by an embacle
/// `CliRunnerType` dispatch — Gemini, Groq, Local, `OpenRouter` (each
/// construct directly from their own env vars), and `CopilotHeadless` /
/// `OpenAiApi` (which follow their own bespoke construction paths in
/// [`CliLlmProvider`]).
///
/// Used by [`ChatProvider::create_fallback_provider`] to build the runtime
/// chain's secondary against a specific runner type instead of re-reading
/// `PIERRE_LLM_PROVIDER`.
fn cli_runner_type_for(provider_type: LlmProviderType) -> Option<CliRunnerType> {
    match provider_type {
        LlmProviderType::ClaudeCode => Some(CliRunnerType::ClaudeCode),
        LlmProviderType::Copilot => Some(CliRunnerType::Copilot),
        LlmProviderType::CursorAgent => Some(CliRunnerType::CursorAgent),
        LlmProviderType::OpenCode => Some(CliRunnerType::OpenCode),
        LlmProviderType::GeminiCli => Some(CliRunnerType::GeminiCli),
        LlmProviderType::CodexCli => Some(CliRunnerType::CodexCli),
        LlmProviderType::GooseCli => Some(CliRunnerType::GooseCli),
        LlmProviderType::ClineCli => Some(CliRunnerType::ClineCli),
        LlmProviderType::ContinueCli => Some(CliRunnerType::ContinueCli),
        LlmProviderType::WarpCli => Some(CliRunnerType::WarpCli),
        LlmProviderType::KiroCli => Some(CliRunnerType::KiroCli),
        LlmProviderType::KiloCli => Some(CliRunnerType::KiloCli),
        LlmProviderType::Gemini
        | LlmProviderType::Groq
        | LlmProviderType::Local
        | LlmProviderType::OpenRouter
        | LlmProviderType::Cohere
        | LlmProviderType::CopilotHeadless
        | LlmProviderType::OpenAiApi => None,
    }
}

/// Check if the active model is recognized by the provider and log a warning on mismatch.
///
/// This is a soft check — it does NOT block startup. Warnings help operators
/// detect misconfigurations like sending a Gemini model name to the Claude Code CLI.
fn validate_model_for_provider(provider: &ChatProvider) {
    let model = provider.default_model();
    let available = provider.available_models();

    if available.is_empty() {
        // Provider doesn't publish a model list — skip validation
        return;
    }

    if available.iter().any(|m| m == model) {
        info!(
            provider = provider.name(),
            model,
            available_count = available.len(),
            "Model validated against provider's available models"
        );
    } else {
        warn!(
            provider = provider.name(),
            model,
            available = ?available,
            "Active model is not in this provider's known model list — \
             this may cause runtime errors. Check PIERRE_LLM_MODEL and PIERRE_LLM_PROVIDER"
        );
    }
}

impl fmt::Debug for ChatProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gemini(_) => f.debug_tuple("ChatProvider::Gemini").finish(),
            Self::Groq(_) => f.debug_tuple("ChatProvider::Groq").finish(),
            Self::Local(_) => f.debug_tuple("ChatProvider::Local").finish(),
            Self::OpenRouter(_) => f.debug_tuple("ChatProvider::OpenRouter").finish(),
            Self::Cohere(_) => f.debug_tuple("ChatProvider::Cohere").finish(),
            Self::Cli(_) => f.debug_tuple("ChatProvider::Cli").finish(),
            Self::Custom(_) => f.debug_tuple("ChatProvider::Custom").finish(),
            Self::Chain { primary, secondary } => f
                .debug_struct("ChatProvider::Chain")
                .field("primary", primary)
                .field("secondary", secondary)
                .finish(),
        }
    }
}

/// Strip the per-request model override before handing a request to a chain's
/// secondary provider.
///
/// The chat pipeline stamps the *primary's* model onto every `ChatRequest`
/// (e.g. Copilot's `claude-opus-4.8`). Each provider resolves its model as
/// `request.model.as_deref().unwrap_or(&self.default_model)`, so a non-`None`
/// `request.model` overrides the secondary's own `with_model`-configured
/// `default_model`. Without clearing it, the fallback chain ships the primary's
/// model name to a different provider's API — Gemini 404s and Cohere returns
/// "model 'claude-opus-4.8' not found", collapsing the entire chain. Clearing
/// `model` lets each tier fall back to the model it was configured with via
/// `PIERRE_LLM_FALLBACK_PROVIDER_MODEL` / `PIERRE_LLM_TERTIARY_PROVIDER_MODEL`.
fn request_for_secondary(request: &ChatRequest) -> ChatRequest {
    let mut forwarded = request.clone();
    forwarded.model = None;
    forwarded
}

/// Decide whether a runtime error should trigger a fallback retry.
///
/// We only fall back on errors that hint at primary-provider unavailability
/// (auth failures, upstream 5xx, transient network). Bad input or quota
/// errors stay on the primary so the caller sees the right diagnostic
/// instead of getting silently rerouted to a different provider.
fn is_retryable_for_fallback(error: &AppError) -> bool {
    use pierre_core::errors::ErrorCode;
    matches!(
        error.code,
        ErrorCode::ExternalAuthFailed
            | ErrorCode::ExternalServiceUnavailable
            | ErrorCode::ExternalServiceError
            | ErrorCode::ResourceUnavailable
            | ErrorCode::AuthInvalid
            | ErrorCode::AuthExpired
            | ErrorCode::InternalError
    )
}

// Implement LlmProvider trait for ChatProvider to enable trait object usage
#[async_trait::async_trait]
impl LlmProvider for ChatProvider {
    fn name(&self) -> &'static str {
        match self {
            Self::Gemini(p) => p.name(),
            Self::Groq(p) => p.name(),
            Self::Local(p) => p.name(),
            Self::OpenRouter(p) => p.name(),
            Self::Cohere(p) => p.name(),
            Self::Cli(p) => p.name(),
            Self::Custom(p) => p.name(),
            Self::Chain { primary, .. } => primary.name(),
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Self::Gemini(p) => p.display_name(),
            Self::Groq(p) => p.display_name(),
            Self::Local(p) => p.display_name(),
            Self::OpenRouter(p) => p.display_name(),
            Self::Cohere(p) => p.display_name(),
            Self::Cli(p) => p.display_name(),
            Self::Custom(p) => p.display_name(),
            Self::Chain { primary, .. } => primary.display_name(),
        }
    }

    fn capabilities(&self) -> LlmCapabilities {
        match self {
            Self::Gemini(p) => p.capabilities(),
            Self::Groq(p) => p.capabilities(),
            Self::Local(p) => p.capabilities(),
            Self::OpenRouter(p) => p.capabilities(),
            Self::Cohere(p) => p.capabilities(),
            Self::Cli(p) => p.capabilities(),
            Self::Custom(p) => p.capabilities(),
            Self::Chain { primary, .. } => primary.capabilities(),
        }
    }

    fn default_model(&self) -> &str {
        match self {
            Self::Gemini(p) => p.default_model(),
            Self::Groq(p) => p.default_model(),
            Self::Local(p) => p.default_model(),
            Self::OpenRouter(p) => p.default_model(),
            Self::Cohere(p) => p.default_model(),
            Self::Cli(p) => p.default_model(),
            Self::Custom(p) => p.default_model(),
            Self::Chain { primary, .. } => primary.default_model(),
        }
    }

    fn available_models(&self) -> &[String] {
        match self {
            Self::Gemini(p) => p.available_models(),
            Self::Groq(p) => p.available_models(),
            Self::Local(p) => p.available_models(),
            Self::OpenRouter(p) => p.available_models(),
            Self::Cohere(p) => p.available_models(),
            Self::Cli(p) => p.available_models(),
            Self::Custom(p) => p.available_models(),
            Self::Chain { primary, .. } => primary.available_models(),
        }
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        match self {
            Self::Gemini(p) => p.complete(request).await,
            Self::Groq(p) => p.complete(request).await,
            Self::Local(p) => p.complete(request).await,
            Self::OpenRouter(p) => p.complete(request).await,
            Self::Cohere(p) => p.complete(request).await,
            Self::Cli(p) => p.complete(request).await,
            Self::Custom(p) => p.complete(request).await,
            Self::Chain { primary, secondary } => {
                // Preemptive skip: CHAIN_GUARD short-circuits to the
                // secondary when GitHub rate-limit headroom is below
                // threshold (Strategy A) or when the circuit breaker
                // is open after consecutive primary auth failures
                // (Strategy B). Half-open / closed states fall through
                // to the normal "try primary, fall back on retryable
                // error" path so a single half-open probe gets to
                // exercise the primary.
                if CHAIN_GUARD.should_skip_primary() {
                    warn!(
                        primary = primary.name(),
                        secondary = secondary.name(),
                        budget_low = CHAIN_GUARD.is_github_budget_low(),
                        circuit_open = CHAIN_GUARD.is_circuit_open(),
                        "Chain skipping primary preemptively; using secondary directly"
                    );
                    info!(
                        target: "notify",
                        event = "embacle.fallback_triggered",
                        from_provider = primary.name(),
                        to_provider = secondary.name(),
                        reason = "preemptive_guard",
                        "Runtime LLM fallback engaged preemptively (guard)"
                    );
                    return secondary.complete(&request_for_secondary(request)).await;
                }
                match primary.complete(request).await {
                    Ok(response) => {
                        if matches!(
                            CHAIN_GUARD.record_primary_success(),
                            CircuitTransition::Closed
                        ) {
                            info!(
                                target: "notify",
                                event = "llm.circuit_closed",
                                provider = primary.name(),
                                "Chain circuit closed on primary recovery"
                            );
                        }
                        Ok(response)
                    }
                    Err(primary_err) if is_retryable_for_fallback(&primary_err) => {
                        if matches!(
                            CHAIN_GUARD.record_primary_failure(),
                            CircuitTransition::Opened
                        ) {
                            info!(
                                target: "notify",
                                event = "llm.circuit_opened",
                                provider = primary.name(),
                                reason = ?primary_err.code,
                                "Chain circuit opened after consecutive primary failures"
                            );
                        }
                        warn!(
                            primary = primary.name(),
                            secondary = secondary.name(),
                            error = %primary_err,
                            "Primary LLM complete() failed with retryable error; falling back"
                        );
                        info!(
                            target: "notify",
                            event = "embacle.fallback_triggered",
                            from_provider = primary.name(),
                            to_provider = secondary.name(),
                            reason = ?primary_err.code,
                            "Runtime LLM fallback engaged on complete()"
                        );
                        secondary.complete(&request_for_secondary(request)).await
                    }
                    Err(primary_err) => Err(primary_err),
                }
            }
        }
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        match self {
            Self::Gemini(p) => p.complete_stream(request).await,
            Self::Groq(p) => p.complete_stream(request).await,
            Self::Local(p) => p.complete_stream(request).await,
            Self::OpenRouter(p) => p.complete_stream(request).await,
            Self::Cohere(p) => p.complete_stream(request).await,
            Self::Cli(p) => p.complete_stream(request).await,
            Self::Custom(p) => p.complete_stream(request).await,
            Self::Chain { primary, secondary } => {
                // Same preemptive-skip logic as `complete()` — see
                // there for the rationale. We record success/failure on
                // the same shared CHAIN_GUARD so the breaker is unified
                // across streaming and non-streaming calls.
                if CHAIN_GUARD.should_skip_primary() {
                    warn!(
                        primary = primary.name(),
                        secondary = secondary.name(),
                        budget_low = CHAIN_GUARD.is_github_budget_low(),
                        circuit_open = CHAIN_GUARD.is_circuit_open(),
                        "Chain skipping primary preemptively on stream; using secondary directly"
                    );
                    info!(
                        target: "notify",
                        event = "embacle.fallback_triggered",
                        from_provider = primary.name(),
                        to_provider = secondary.name(),
                        reason = "preemptive_guard",
                        "Runtime LLM fallback engaged preemptively (guard, stream)"
                    );
                    return secondary
                        .complete_stream(&request_for_secondary(request))
                        .await;
                }
                match primary.complete_stream(request).await {
                    Ok(stream) => {
                        // Note: success here only means the stream was
                        // OPENED — content may still error mid-stream.
                        // The breaker tracks transport-level establishment,
                        // which is what auth/rate-limit failures hit first.
                        if matches!(
                            CHAIN_GUARD.record_primary_success(),
                            CircuitTransition::Closed
                        ) {
                            info!(
                                target: "notify",
                                event = "llm.circuit_closed",
                                provider = primary.name(),
                                "Chain circuit closed on primary stream recovery"
                            );
                        }
                        Ok(stream)
                    }
                    Err(primary_err) if is_retryable_for_fallback(&primary_err) => {
                        if matches!(
                            CHAIN_GUARD.record_primary_failure(),
                            CircuitTransition::Opened
                        ) {
                            info!(
                                target: "notify",
                                event = "llm.circuit_opened",
                                provider = primary.name(),
                                reason = ?primary_err.code,
                                "Chain circuit opened on primary stream failure"
                            );
                        }
                        warn!(
                            primary = primary.name(),
                            secondary = secondary.name(),
                            error = %primary_err,
                            "Primary LLM complete_stream() failed with retryable error; falling back"
                        );
                        info!(
                            target: "notify",
                            event = "embacle.fallback_triggered",
                            from_provider = primary.name(),
                            to_provider = secondary.name(),
                            reason = ?primary_err.code,
                            "Runtime LLM fallback engaged on complete_stream()"
                        );
                        secondary
                            .complete_stream(&request_for_secondary(request))
                            .await
                    }
                    Err(primary_err) => Err(primary_err),
                }
            }
        }
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        match self {
            Self::Gemini(p) => p.health_check().await,
            Self::Groq(p) => p.health_check().await,
            Self::Local(p) => p.health_check().await,
            Self::OpenRouter(p) => p.health_check().await,
            Self::Cohere(p) => p.health_check().await,
            Self::Cli(p) => p.health_check().await,
            Self::Custom(p) => p.health_check().await,
            Self::Chain { primary, secondary } => {
                // Healthy if EITHER side is healthy — the chain is usable as
                // long as at least one provider can serve requests.
                let primary_ok = primary.health_check().await.unwrap_or(false);
                if primary_ok {
                    return Ok(true);
                }
                secondary.health_check().await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GroqProvider;

    /// `compose_secondary_with_tertiary` is the pure-sync core of the
    /// 3-tier chain assembly. Direct testing of `build_runtime_chain`
    /// requires real env vars + real API keys + real network — these
    /// tests cover the composition logic in isolation so the chain
    /// shape is exercised without external dependencies.
    fn mk_groq() -> ChatProvider {
        ChatProvider::Groq(GroqProvider::new("test-key".to_owned()))
    }

    fn mk_cohere() -> ChatProvider {
        ChatProvider::Cohere(CohereProvider::new("test-key".to_owned()))
    }

    #[test]
    fn compose_returns_secondary_unchanged_when_tertiary_unset() {
        let secondary = mk_groq();
        let composed = ChatProvider::compose_secondary_with_tertiary(
            Ok(secondary),
            None,
            LlmProviderType::Groq,
        );
        assert!(matches!(composed, Ok(ChatProvider::Groq(_))));
    }

    #[test]
    fn compose_builds_nested_chain_when_both_succeed() -> Result<(), AppError> {
        let secondary = mk_groq();
        let tertiary = mk_cohere();
        let composed = ChatProvider::compose_secondary_with_tertiary(
            Ok(secondary),
            Some((LlmProviderType::Cohere, Ok(tertiary))),
            LlmProviderType::Groq,
        )?;
        let ChatProvider::Chain {
            primary,
            secondary: inner,
        } = composed
        else {
            return Err(AppError::internal(format!(
                "expected nested Chain, got {composed:?}"
            )));
        };
        assert!(matches!(*primary, ChatProvider::Groq(_)));
        assert!(matches!(*inner, ChatProvider::Cohere(_)));
        Ok(())
    }

    #[test]
    fn compose_promotes_tertiary_when_secondary_init_fails() {
        let composed = ChatProvider::compose_secondary_with_tertiary(
            Err(AppError::config("secondary boom")),
            Some((LlmProviderType::Cohere, Ok(mk_cohere()))),
            LlmProviderType::Groq,
        );
        assert!(matches!(composed, Ok(ChatProvider::Cohere(_))));
    }

    #[test]
    fn compose_keeps_secondary_when_tertiary_init_fails() {
        let composed = ChatProvider::compose_secondary_with_tertiary(
            Ok(mk_groq()),
            Some((
                LlmProviderType::Cohere,
                Err(AppError::config("tertiary boom")),
            )),
            LlmProviderType::Groq,
        );
        assert!(matches!(composed, Ok(ChatProvider::Groq(_))));
    }

    #[test]
    fn compose_returns_secondary_error_when_both_fail() {
        let composed = ChatProvider::compose_secondary_with_tertiary(
            Err(AppError::config("secondary boom")),
            Some((
                LlmProviderType::Cohere,
                Err(AppError::config("tertiary boom")),
            )),
            LlmProviderType::Groq,
        );
        assert!(composed.is_err());
    }
}
