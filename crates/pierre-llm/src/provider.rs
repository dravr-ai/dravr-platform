// ABOUTME: Unified LLM provider selector for runtime provider switching
// ABOUTME: Abstracts over Gemini, Groq, Local, and embache providers based on environment configuration
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
//! - `local`/`ollama`/`vllm`/`localai`: Use a local `OpenAI`-compatible endpoint
//! - `claude_code`/`copilot`/`cursor_agent`/`opencode`/`copilot_sdk`: Use an embache runner

use std::fmt;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info};

use super::{
    ChatMessage, ChatRequest, ChatResponse, ChatResponseWithTools, ChatStream, CliLlmProvider,
    FunctionResponse, GeminiProvider, GroqProvider, LlmCapabilities, LlmProvider,
    OpenAiCompatibleProvider, Tool,
};
use crate::config::LlmProviderType;
use crate::errors::AppError;

/// Unified chat provider that wraps Gemini, Groq, Local, or embache-based LLM
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
    /// Embache-based LLM provider (CLI runners and SDK runners)
    Cli(CliLlmProvider),
}

impl ChatProvider {
    /// Create a provider from environment configuration
    ///
    /// Reads `PIERRE_LLM_PROVIDER` to determine which provider to use:
    /// - `gemini` (default): Creates `GeminiProvider` (requires `GEMINI_API_KEY`)
    /// - `groq`: Creates `GroqProvider` (requires `GROQ_API_KEY`)
    /// - `local`/`ollama`/`vllm`/`localai`: Creates `OpenAiCompatibleProvider`
    /// - `claude_code`/`copilot`/`cursor_agent`/`opencode`/`copilot_sdk`: Embache runners
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

        let result = Self::create_provider(provider_type);
        Self::finalize_or_fallback(result, provider_type).await
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

        match Self::create_provider(fallback) {
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
    fn create_provider(provider_type: LlmProviderType) -> Result<Self, AppError> {
        match provider_type {
            LlmProviderType::Groq => Self::groq(),
            LlmProviderType::Gemini => Self::gemini(),
            LlmProviderType::Local => Self::local(),
            LlmProviderType::ClaudeCode
            | LlmProviderType::Copilot
            | LlmProviderType::CursorAgent
            | LlmProviderType::OpenCode
            | LlmProviderType::CopilotSdk => Self::cli(),
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

    /// Create an embache-based LLM provider explicitly
    ///
    /// Auto-detects or reads `PIERRE_LLM_PROVIDER` to select the runner
    /// (Claude Code, Copilot CLI, Cursor Agent, `OpenCode`, or Copilot SDK).
    ///
    /// # Errors
    ///
    /// Returns an error if no runner can be detected or initialized.
    pub fn cli() -> Result<Self, AppError> {
        Ok(Self::Cli(CliLlmProvider::from_env()?))
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

    /// Get the provider type
    #[must_use]
    pub fn provider_type(&self) -> LlmProviderType {
        match self {
            Self::Gemini(_) => LlmProviderType::Gemini,
            Self::Groq(_) => LlmProviderType::Groq,
            Self::Local(_) => LlmProviderType::Local,
            Self::Cli(p) => p.provider_type(),
        }
    }

    /// Check if this provider supports tool calling
    #[must_use]
    pub fn supports_tool_calling(&self) -> bool {
        self.capabilities().supports_function_calling()
    }

    /// Get the inner `CliLlmProvider` if this is an embache-based provider
    #[must_use]
    pub fn as_cli_provider(&self) -> Option<&CliLlmProvider> {
        match self {
            Self::Cli(p) => Some(p),
            _ => None,
        }
    }

    /// Perform a chat completion with tool/function calling support
    ///
    /// Gemini, Groq, and Local providers all support native function/tool calling
    /// via their respective APIs (Gemini native, Groq/Local OpenAI-compatible).
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
            Self::Cli(_) => Err(AppError::invalid_input(
                "Embache-based providers do not support structured tool calling via this path",
            )),
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

impl fmt::Debug for ChatProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gemini(_) => f.debug_tuple("ChatProvider::Gemini").finish(),
            Self::Groq(_) => f.debug_tuple("ChatProvider::Groq").finish(),
            Self::Local(_) => f.debug_tuple("ChatProvider::Local").finish(),
            Self::Cli(_) => f.debug_tuple("ChatProvider::Cli").finish(),
        }
    }
}

// Implement LlmProvider trait for ChatProvider to enable trait object usage
#[async_trait::async_trait]
impl LlmProvider for ChatProvider {
    fn name(&self) -> &'static str {
        match self {
            Self::Gemini(p) => p.name(),
            Self::Groq(p) => p.name(),
            Self::Local(p) => p.name(),
            Self::Cli(p) => p.name(),
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Self::Gemini(p) => p.display_name(),
            Self::Groq(p) => p.display_name(),
            Self::Local(p) => p.display_name(),
            Self::Cli(p) => p.display_name(),
        }
    }

    fn capabilities(&self) -> LlmCapabilities {
        match self {
            Self::Gemini(p) => p.capabilities(),
            Self::Groq(p) => p.capabilities(),
            Self::Local(p) => p.capabilities(),
            Self::Cli(p) => p.capabilities(),
        }
    }

    fn default_model(&self) -> &str {
        match self {
            Self::Gemini(p) => p.default_model(),
            Self::Groq(p) => p.default_model(),
            Self::Local(p) => p.default_model(),
            Self::Cli(p) => p.default_model(),
        }
    }

    fn available_models(&self) -> &[String] {
        match self {
            Self::Gemini(p) => p.available_models(),
            Self::Groq(p) => p.available_models(),
            Self::Local(p) => p.available_models(),
            Self::Cli(p) => p.available_models(),
        }
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        match self {
            Self::Gemini(p) => p.complete(request).await,
            Self::Groq(p) => p.complete(request).await,
            Self::Local(p) => p.complete(request).await,
            Self::Cli(p) => p.complete(request).await,
        }
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        match self {
            Self::Gemini(p) => p.complete_stream(request).await,
            Self::Groq(p) => p.complete_stream(request).await,
            Self::Local(p) => p.complete_stream(request).await,
            Self::Cli(p) => p.complete_stream(request).await,
        }
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        match self {
            Self::Gemini(p) => p.health_check().await,
            Self::Groq(p) => p.health_check().await,
            Self::Local(p) => p.health_check().await,
            Self::Cli(p) => p.health_check().await,
        }
    }
}
