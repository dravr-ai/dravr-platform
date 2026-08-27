// ABOUTME: OpenRouter LLM provider implementation with streaming support
// ABOUTME: Uses OpenAI-compatible API for unified access to 200+ models via openrouter.ai
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `OpenRouter` Provider
//!
//! Implementation of the `LlmProvider` trait for `OpenRouter`, a unified
//! gateway exposing 200+ frontier and open-source models behind a single
//! OpenAI-compatible endpoint.
//!
//! ## Features
//!
//! - **Tool/Function Calling**: Full support via the OpenAI tool calling API
//! - **Streaming**: Real-time response streaming via SSE
//! - **Model Diversity**: One API key unlocks Anthropic, OpenAI, Google,
//!   Meta, Mistral, and others — model selected per-request via slug.
//!
//! ## Configuration
//!
//! Required:
//! - `OPENROUTER_API_KEY`: API key from <https://openrouter.ai/keys>
//!
//! Optional:
//! - `OPENROUTER_DEFAULT_MODEL`: Model slug (e.g. `anthropic/claude-3.5-sonnet`)
//! - `OPENROUTER_FALLBACK_MODEL`: Fallback model slug
//! - `OPENROUTER_SITE_URL`: Sent as `HTTP-Referer` so `OpenRouter` can rank traffic
//! - `OPENROUTER_APP_TITLE`: Sent as `X-Title` so `OpenRouter` can rank traffic
//! - `OPENROUTER_MAX_RETRIES`, `OPENROUTER_INITIAL_RETRY_DELAY_MS`,
//!   `OPENROUTER_MAX_RETRY_DELAY_MS`: Retry tuning for transient failures
//!
//! ## Models
//!
//! Model slugs follow the `provider/model` convention. The default
//! (`meta-llama/llama-3.3-70b-instruct`) is one of the cheapest capable
//! tool-calling models on the network. See <https://openrouter.ai/models>
//! for the full catalog.
//!
//! ## Example
//!
//! ```rust,no_run
//! use pierre_llm::{OpenRouterProvider, LlmProvider, ChatRequest, ChatMessage};
//! use pierre_core::errors::AppError;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), AppError> {
//!     let provider = OpenRouterProvider::from_env()?;
//!     let request = ChatRequest::new(vec![
//!         ChatMessage::user("What is machine learning?"),
//!     ]);
//!     let response = provider.complete(&request).await?;
//!     println!("{}", response.content);
//!     Ok(())
//! }
//! ```

use async_trait::async_trait;
use pierre_core::http_client::{SharedHttpClient, SharedRequestBuilder as RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use tokio::time::sleep;
use tracing::{debug, error, info, instrument, warn};

use super::sse_parser::{self, RetryConfig};
use super::{
    ChatMessage, ChatRequest, ChatResponse, ChatResponseWithTools, ChatStream, FunctionCall,
    LlmCapabilities, LlmProvider, StreamChunk, TokenUsage, Tool,
};
use crate::build_llm_http_client;
use crate::errors::{AppError, ErrorCode};

/// Environment variable for `OpenRouter` API key
const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";

/// Environment variable for default model
const OPENROUTER_DEFAULT_MODEL_ENV: &str = "OPENROUTER_DEFAULT_MODEL";

/// Environment variable for fallback model
const OPENROUTER_FALLBACK_MODEL_ENV: &str = "OPENROUTER_FALLBACK_MODEL";

/// Environment variable for the `HTTP-Referer` ranking header
const OPENROUTER_SITE_URL_ENV: &str = "OPENROUTER_SITE_URL";

/// Environment variable for the `X-Title` ranking header
const OPENROUTER_APP_TITLE_ENV: &str = "OPENROUTER_APP_TITLE";

/// Hardcoded fallback if env vars not set — one of the cheapest tool-calling
/// capable models on `OpenRouter`, mirrors the existing Groq Llama default.
const HARDCODED_DEFAULT_MODEL: &str = "meta-llama/llama-3.3-70b-instruct";

/// A curated subset of `OpenRouter` model slugs surfaced for validation.
///
/// `OpenRouter` exposes 200+ models. This list is intentionally short — it
/// drives the soft model-validation warning in [`ChatProvider`] startup
/// and is **not** authoritative. Operators can use any slug from
/// <https://openrouter.ai/models> via `OPENROUTER_DEFAULT_MODEL` or
/// `PIERRE_LLM_MODEL`; a slug not in this list only triggers a warning,
/// not a startup failure.
const AVAILABLE_MODELS: &[&str] = &[
    "meta-llama/llama-3.3-70b-instruct",
    "meta-llama/llama-3.1-8b-instruct",
    "anthropic/claude-3.5-sonnet",
    "anthropic/claude-3.5-haiku",
    "openai/gpt-4o",
    "openai/gpt-4o-mini",
    "google/gemini-2.0-flash-001",
    "google/gemini-pro-1.5",
    "mistralai/mistral-large",
    "mistralai/mistral-nemo",
    "qwen/qwen-2.5-72b-instruct",
];

/// Base URL for the `OpenRouter` API (OpenAI-compatible)
const API_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Environment variable for maximum retries
const OPENROUTER_MAX_RETRIES_ENV: &str = "OPENROUTER_MAX_RETRIES";

/// Environment variable for initial retry delay in milliseconds
const OPENROUTER_INITIAL_RETRY_DELAY_MS_ENV: &str = "OPENROUTER_INITIAL_RETRY_DELAY_MS";

/// Environment variable for maximum retry delay in milliseconds
const OPENROUTER_MAX_RETRY_DELAY_MS_ENV: &str = "OPENROUTER_MAX_RETRY_DELAY_MS";

// ============================================================================
// API Request/Response Types (OpenAI-compatible format)
// ============================================================================

/// `OpenRouter` API request structure (OpenAI-compatible)
#[derive(Debug, Serialize)]
struct OpenRouterRequest {
    model: String,
    messages: Vec<OpenRouterMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenRouterTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

/// Tool definition for `OpenRouter` API (OpenAI-compatible format)
#[derive(Debug, Clone, Serialize)]
struct OpenRouterTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenRouterFunction,
}

/// Function definition within a tool
#[derive(Debug, Clone, Serialize)]
struct OpenRouterFunction {
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Value>,
}

/// Message structure for `OpenRouter` API (OpenAI-compatible)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenRouterMessage {
    role: String,
    content: String,
}

impl From<&ChatMessage> for OpenRouterMessage {
    fn from(msg: &ChatMessage) -> Self {
        Self {
            role: msg.role.as_str().to_owned(),
            content: msg.content.clone(),
        }
    }
}

/// `OpenRouter` API response structure (OpenAI-compatible)
#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    choices: Vec<OpenRouterChoice>,
    #[serde(default)]
    usage: Option<OpenRouterUsage>,
    model: String,
}

/// Choice in `OpenRouter` response
#[derive(Debug, Deserialize)]
struct OpenRouterChoice {
    message: OpenRouterResponseMessage,
    finish_reason: Option<String>,
}

/// Message in `OpenRouter` response
#[derive(Debug, Deserialize)]
struct OpenRouterResponseMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenRouterToolCall>>,
}

/// Tool call in `OpenRouter` response (OpenAI-compatible)
#[derive(Debug, Clone, Deserialize)]
struct OpenRouterToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenRouterFunctionCall,
}

/// Function call details in `OpenRouter` response
#[derive(Debug, Clone, Deserialize)]
struct OpenRouterFunctionCall {
    name: String,
    arguments: String,
}

/// Usage statistics in `OpenRouter` response
#[derive(Debug, Deserialize)]
struct OpenRouterUsage {
    #[serde(rename = "prompt_tokens")]
    prompt: u32,
    #[serde(rename = "completion_tokens")]
    completion: u32,
    #[serde(rename = "total_tokens")]
    total: u32,
    /// Breakdown of `prompt_tokens`. Carries the cache-read share, which this
    /// struct never declared — so it deserialized away on every response.
    #[serde(default)]
    prompt_tokens_details: Option<OpenRouterUsageDetails>,
}

/// The `prompt_tokens_details` sub-object of an OpenAI-compatible usage block.
#[derive(Debug, Deserialize)]
struct OpenRouterUsageDetails {
    /// Prompt tokens served from the provider's cache.
    #[serde(default)]
    cached_tokens: Option<u32>,
}

/// Streaming chunk structure (OpenAI-compatible)
#[derive(Debug, Deserialize)]
struct OpenRouterStreamChunk {
    choices: Vec<OpenRouterStreamChoice>,
}

/// Choice in streaming chunk
#[derive(Debug, Deserialize)]
struct OpenRouterStreamChoice {
    delta: OpenRouterDelta,
    finish_reason: Option<String>,
}

/// Delta content in streaming chunk
#[derive(Debug, Deserialize)]
struct OpenRouterDelta {
    #[serde(default)]
    content: Option<String>,
}

/// `OpenRouter` API error response
#[derive(Debug, Deserialize)]
struct OpenRouterErrorResponse {
    error: OpenRouterErrorDetail,
}

/// Error detail structure
#[derive(Debug, Deserialize)]
struct OpenRouterErrorDetail {
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
}

// ============================================================================
// Provider Implementation
// ============================================================================

/// `OpenRouter` LLM provider — unified access to 200+ models
///
/// Routes requests through <https://openrouter.ai/api/v1>, an
/// OpenAI-compatible gateway. Model selection happens per-request via
/// the `model` slug (e.g. `anthropic/claude-3.5-sonnet`).
pub struct OpenRouterProvider {
    client: &'static SharedHttpClient,
    api_key: String,
    default_model: String,
    fallback_model: String,
    available_models: Vec<String>,
    /// Optional value for the `HTTP-Referer` header (`OpenRouter` traffic ranking)
    site_url: Option<String>,
    /// Optional value for the `X-Title` header (`OpenRouter` traffic ranking)
    app_title: Option<String>,
    /// Retry configuration for transient failures (429, 503, network errors)
    retry_config: RetryConfig,
}

impl OpenRouterProvider {
    /// Create a new `OpenRouter` provider with the given API key
    ///
    /// Uses environment variables for model configuration:
    /// - `OPENROUTER_DEFAULT_MODEL`: Primary model slug
    /// - `OPENROUTER_FALLBACK_MODEL`: Fallback model slug
    /// - `OPENROUTER_SITE_URL` / `OPENROUTER_APP_TITLE`: Ranking headers
    #[must_use]
    pub fn new(api_key: String) -> Self {
        let default_model = env::var(OPENROUTER_DEFAULT_MODEL_ENV)
            .unwrap_or_else(|_| HARDCODED_DEFAULT_MODEL.to_owned());
        let fallback_model = env::var(OPENROUTER_FALLBACK_MODEL_ENV)
            .unwrap_or_else(|_| HARDCODED_DEFAULT_MODEL.to_owned());

        let site_url = env::var(OPENROUTER_SITE_URL_ENV)
            .ok()
            .filter(|s| !s.is_empty());
        let app_title = env::var(OPENROUTER_APP_TITLE_ENV)
            .ok()
            .filter(|s| !s.is_empty());

        let max_retries = env::var(OPENROUTER_MAX_RETRIES_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RetryConfig::default_config().max_retries);
        let initial_delay_ms = env::var(OPENROUTER_INITIAL_RETRY_DELAY_MS_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RetryConfig::default_config().initial_delay_ms);
        let max_delay_ms = env::var(OPENROUTER_MAX_RETRY_DELAY_MS_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RetryConfig::default_config().max_delay_ms);

        Self {
            client: build_llm_http_client(),
            api_key,
            default_model,
            fallback_model,
            available_models: AVAILABLE_MODELS.iter().map(|s| (*s).to_owned()).collect(),
            site_url,
            app_title,
            retry_config: RetryConfig {
                max_retries,
                initial_delay_ms,
                max_delay_ms,
            },
        }
    }

    /// Override the default model after construction.
    ///
    /// Used by the runtime fallback chain when
    /// `PIERRE_LLM_FALLBACK_PROVIDER_MODEL` redirects the secondary to an
    /// `OpenRouter`-specific slug that differs from the primary's
    /// `PIERRE_LLM_DEFAULT_MODEL`.
    #[must_use]
    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    /// Create an `OpenRouter` provider from environment variable
    ///
    /// # Errors
    ///
    /// Returns an error if `OPENROUTER_API_KEY` is not set
    pub fn from_env() -> Result<Self, AppError> {
        let api_key = env::var(OPENROUTER_API_KEY_ENV).map_err(|_| {
            AppError::config(format!(
                "Missing {OPENROUTER_API_KEY_ENV} environment variable. Get your API key from https://openrouter.ai/keys"
            ))
        })?;

        let provider = Self::new(api_key);
        info!(
            default_model = %provider.default_model,
            fallback_model = %provider.fallback_model,
            site_url_set = provider.site_url.is_some(),
            app_title_set = provider.app_title.is_some(),
            max_retries = provider.retry_config.max_retries,
            initial_delay_ms = provider.retry_config.initial_delay_ms,
            "OpenRouter provider initialized"
        );

        Ok(provider)
    }

    /// Get the default model
    #[must_use]
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Get the fallback model
    #[must_use]
    pub fn fallback_model(&self) -> &str {
        &self.fallback_model
    }

    /// Build the API URL for a given endpoint
    fn api_url(endpoint: &str) -> String {
        format!("{API_BASE_URL}/{endpoint}")
    }

    /// Convert internal messages to `OpenRouter` format
    fn convert_messages(messages: &[ChatMessage]) -> Vec<OpenRouterMessage> {
        messages.iter().map(OpenRouterMessage::from).collect()
    }

    /// Parse error response from `OpenRouter` API
    fn parse_error_response(status: reqwest::StatusCode, body: &str) -> AppError {
        if let Ok(error_response) = serde_json::from_str::<OpenRouterErrorResponse>(body) {
            let error_type = error_response
                .error
                .error_type
                .unwrap_or_else(|| "unknown".to_owned());

            match status.as_u16() {
                401 => AppError::auth_invalid(format!(
                    "OpenRouter API authentication failed: {}",
                    error_response.error.message
                )),
                402 => AppError::new(
                    ErrorCode::ExternalServiceError,
                    format!(
                        "OpenRouter credit balance exhausted: {}",
                        error_response.error.message
                    ),
                ),
                429 => {
                    let user_message =
                        Self::extract_rate_limit_message(&error_response.error.message);
                    AppError::new(ErrorCode::ExternalRateLimited, user_message)
                }
                400 => AppError::invalid_input(format!(
                    "OpenRouter API validation error: {}",
                    error_response.error.message
                )),
                _ => AppError::external_service(
                    "OpenRouter",
                    format!("{} - {}", error_type, error_response.error.message),
                ),
            }
        } else {
            debug!(
                status = %status,
                body_preview = %body.chars().take(200).collect::<String>(),
                "OpenRouter API returned non-JSON error response"
            );
            AppError::external_service(
                "OpenRouter",
                format!("unexpected error response (HTTP {status})"),
            )
        }
    }

    /// Extract a user-friendly rate limit message from an `OpenRouter` error.
    ///
    /// `OpenRouter`'s rate-limit messages vary by upstream provider; we keep
    /// a generic surface and let the raw message through when no obvious
    /// pattern matches.
    fn extract_rate_limit_message(message: &str) -> String {
        if message.contains("per minute") || message.contains("tokens per") {
            "OpenRouter rate limit reached (per-minute quota). Please wait a moment and try again."
                .to_owned()
        } else {
            format!("OpenRouter rate limit reached: {message}")
        }
    }

    /// Check if an error from the `OpenRouter` API is retryable (transient)
    fn is_retryable_error(status: u16) -> bool {
        sse_parser::is_retryable_status(status)
    }

    /// Build an authenticated HTTP request to the `OpenRouter` API,
    /// applying ranking headers when configured.
    fn build_request(&self, body: &OpenRouterRequest) -> RequestBuilder {
        let mut builder = self
            .client
            .post(Self::api_url("chat/completions"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");

        if let Some(ref site) = self.site_url {
            builder = builder.header("HTTP-Referer", site);
        }
        if let Some(ref title) = self.app_title {
            builder = builder.header("X-Title", title);
        }

        builder.json(body)
    }

    /// Parse an `OpenRouter` SSE data payload into a `StreamChunk`
    fn parse_stream_data(json_str: &str) -> Option<Result<StreamChunk, AppError>> {
        match serde_json::from_str::<OpenRouterStreamChunk>(json_str) {
            Ok(chunk) => {
                let choice = chunk.choices.into_iter().next()?;
                let delta = choice.delta.content.unwrap_or_default();
                let is_final = choice.finish_reason.is_some();
                Some(Ok(StreamChunk {
                    delta,
                    is_final,
                    finish_reason: choice.finish_reason,
                }))
            }
            Err(e) => {
                warn!("Failed to parse OpenRouter stream chunk: {e}");
                None
            }
        }
    }

    /// Convert internal Tool format to `OpenRouter`'s OpenAI-compatible format
    fn convert_tools(tools: &[Tool]) -> Vec<OpenRouterTool> {
        tools
            .iter()
            .flat_map(|tool| {
                tool.function_declarations
                    .iter()
                    .map(|func| OpenRouterTool {
                        tool_type: "function".to_owned(),
                        function: OpenRouterFunction {
                            name: func.name.clone(),
                            description: func.description.clone(),
                            parameters: func.parameters.clone(),
                        },
                    })
            })
            .collect()
    }

    /// Convert `OpenRouter` tool calls to internal `FunctionCall` format
    fn convert_tool_calls(tool_calls: &[OpenRouterToolCall]) -> Vec<FunctionCall> {
        tool_calls
            .iter()
            .map(|call| {
                debug!(
                    tool_call_id = %call.id,
                    tool_call_type = %call.call_type,
                    function_name = %call.function.name,
                    "Converting OpenRouter tool call to FunctionCall"
                );
                let args: Value =
                    serde_json::from_str(&call.function.arguments).unwrap_or_default();
                FunctionCall {
                    name: call.function.name.clone(),
                    args,
                }
            })
            .collect()
    }

    /// Perform a chat completion with tool/function calling support
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails or response parsing fails.
    #[instrument(skip(self, request, tools), fields(model = %request.model.as_deref().unwrap_or(&self.default_model)))]
    pub async fn complete_with_tools(
        &self,
        request: &ChatRequest,
        tools: Option<Vec<Tool>>,
    ) -> Result<ChatResponseWithTools, AppError> {
        let model = request.model.as_deref().unwrap_or(&self.default_model);

        debug!("Sending chat completion request to OpenRouter with tools");

        let or_tools = tools.as_ref().map(|t| Self::convert_tools(t));

        let or_request = OpenRouterRequest {
            model: model.to_owned(),
            messages: Self::convert_messages(&request.messages),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: Some(false),
            tools: or_tools,
            tool_choice: tools.as_ref().map(|_| "auto".to_owned()),
        };

        let response = self.build_request(&or_request).send().await.map_err(|e| {
            error!("Failed to send request to OpenRouter API: {}", e);
            AppError::external_service("OpenRouter", format!("Failed to connect: {e}"))
        })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| {
            error!("Failed to read OpenRouter API response: {}", e);
            AppError::external_service("OpenRouter", format!("Failed to read response: {e}"))
        })?;

        if !status.is_success() {
            return Err(Self::parse_error_response(status, &body));
        }

        let or_response: OpenRouterResponse = serde_json::from_str(&body).map_err(|e| {
            error!("Failed to parse OpenRouter API response: {}", e);
            AppError::external_service("OpenRouter", format!("Failed to parse response: {e}"))
        })?;

        let choice =
            or_response.choices.into_iter().next().ok_or_else(|| {
                AppError::external_service("OpenRouter", "API returned no choices")
            })?;

        let content = choice.message.content;
        let function_calls = choice.message.tool_calls.map(|calls| {
            info!("OpenRouter returned {} tool calls", calls.len());
            Self::convert_tool_calls(&calls)
        });

        debug!(
            "Received response from OpenRouter: content={:?}, tool_calls={:?}, finish_reason: {:?}",
            content.as_ref().map(String::len),
            function_calls.as_ref().map(Vec::len),
            choice.finish_reason
        );

        Ok(ChatResponseWithTools {
            content,
            function_calls,
            model: or_response.model,
            usage: or_response.usage.map(|u| {
                // No cache-WRITE count in this API shape: writes are
                // implicit and unbilled, so None is accurate, not unknown.
                TokenUsage::new(u.prompt, u.completion, u.total)
                    .with_cache(u.prompt_tokens_details.and_then(|d| d.cached_tokens), None)
            }),
            finish_reason: choice.finish_reason,
        })
    }
}

#[async_trait]
impl LlmProvider for OpenRouterProvider {
    fn name(&self) -> &'static str {
        "openrouter"
    }

    fn display_name(&self) -> &'static str {
        "OpenRouter"
    }

    fn capabilities(&self) -> LlmCapabilities {
        // Capabilities here reflect the gateway, not any single model.
        // Vision is supported by many OpenRouter models but not all; we
        // advertise the shared OpenAI-compatible surface and let model
        // selection determine effective per-request capabilities.
        LlmCapabilities::STREAMING
            | LlmCapabilities::FUNCTION_CALLING
            | LlmCapabilities::SYSTEM_MESSAGES
            | LlmCapabilities::JSON_MODE
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn available_models(&self) -> &[String] {
        &self.available_models
    }

    #[instrument(skip(self, request), fields(model = %request.model.as_deref().unwrap_or(&self.default_model)))]
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let model = request.model.as_deref().unwrap_or(&self.default_model);

        let or_request = OpenRouterRequest {
            model: model.to_owned(),
            messages: Self::convert_messages(&request.messages),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: Some(false),
            tools: None,
            tool_choice: None,
        };

        let mut last_error: Option<AppError> = None;

        for attempt in 0..=self.retry_config.max_retries {
            if attempt > 0 {
                let delay = self.retry_config.delay_for_attempt(attempt - 1);
                warn!(
                    attempt = attempt,
                    delay_ms = delay.as_millis(),
                    "Retrying OpenRouter request after transient failure"
                );
                sleep(delay).await;
            }

            debug!(
                attempt = attempt,
                "Sending chat completion request to OpenRouter"
            );

            let response = match self.build_request(&or_request).send().await {
                Ok(r) => r,
                Err(e) => {
                    let error =
                        AppError::external_service("OpenRouter", format!("Failed to connect: {e}"));
                    if sse_parser::is_retryable_request_error(&e)
                        && attempt < self.retry_config.max_retries
                    {
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            };

            let status = response.status();
            let body = match response.text().await {
                Ok(t) => t,
                Err(e) => {
                    let error = AppError::external_service(
                        "OpenRouter",
                        format!("Failed to read response: {e}"),
                    );
                    if attempt < self.retry_config.max_retries {
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            };

            if !status.is_success() {
                let error = Self::parse_error_response(status, &body);
                if Self::is_retryable_error(status.as_u16())
                    && attempt < self.retry_config.max_retries
                {
                    warn!(status = %status, attempt = attempt, "OpenRouter API returned retryable error");
                    last_error = Some(error);
                    continue;
                }
                return Err(error);
            }

            let or_response: OpenRouterResponse = serde_json::from_str(&body).map_err(|e| {
                error!("Failed to parse OpenRouter API response: {e}");
                AppError::external_service("OpenRouter", format!("Failed to parse response: {e}"))
            })?;

            let choice = or_response.choices.into_iter().next().ok_or_else(|| {
                AppError::external_service("OpenRouter", "API returned no choices")
            })?;

            let content = choice.message.content.unwrap_or_default();

            debug!(
                attempt = attempt,
                "Received response from OpenRouter: {} chars, finish_reason: {:?}",
                content.len(),
                choice.finish_reason
            );

            return Ok(ChatResponse {
                content,
                model: or_response.model,
                usage: or_response.usage.map(|u| {
                    // No cache-WRITE count in this API shape: writes are
                    // implicit and unbilled, so None is accurate, not unknown.
                    TokenUsage::new(u.prompt, u.completion, u.total)
                        .with_cache(u.prompt_tokens_details.and_then(|d| d.cached_tokens), None)
                }),
                finish_reason: choice.finish_reason,
                warnings: None,
                tool_calls: None,
            });
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::external_service("OpenRouter", "Request failed after retries")
        }))
    }

    #[instrument(skip(self, request), fields(model = %request.model.as_deref().unwrap_or(&self.default_model)))]
    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let model = request.model.as_deref().unwrap_or(&self.default_model);

        let or_request = OpenRouterRequest {
            model: model.to_owned(),
            messages: Self::convert_messages(&request.messages),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: Some(true),
            tools: None,
            tool_choice: None,
        };

        let mut last_error: Option<AppError> = None;

        for attempt in 0..=self.retry_config.max_retries {
            if attempt > 0 {
                let delay = self.retry_config.delay_for_attempt(attempt - 1);
                warn!(
                    attempt = attempt,
                    delay_ms = delay.as_millis(),
                    "Retrying OpenRouter streaming request after transient failure"
                );
                sleep(delay).await;
            }

            debug!(
                attempt = attempt,
                "Sending streaming chat completion request to OpenRouter"
            );

            let response = match self.build_request(&or_request).send().await {
                Ok(r) => r,
                Err(e) => {
                    let error =
                        AppError::external_service("OpenRouter", format!("Failed to connect: {e}"));
                    if sse_parser::is_retryable_request_error(&e)
                        && attempt < self.retry_config.max_retries
                    {
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            };

            let status = response.status();

            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                let error = Self::parse_error_response(status, &body);
                if Self::is_retryable_error(status.as_u16())
                    && attempt < self.retry_config.max_retries
                {
                    warn!(status = %status, attempt = attempt, "OpenRouter streaming API returned retryable error");
                    last_error = Some(error);
                    continue;
                }
                return Err(error);
            }

            let stream = sse_parser::create_sse_stream(
                response.bytes_stream(),
                Self::parse_stream_data,
                "OpenRouter",
            );
            return Ok(stream);
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::external_service("OpenRouter", "Streaming request failed after retries")
        }))
    }

    #[instrument(skip(self))]
    async fn health_check(&self) -> Result<bool, AppError> {
        debug!("Performing OpenRouter API health check");

        let mut builder = self
            .client
            .get(Self::api_url("models"))
            .header("Authorization", format!("Bearer {}", self.api_key));

        if let Some(ref site) = self.site_url {
            builder = builder.header("HTTP-Referer", site);
        }
        if let Some(ref title) = self.app_title {
            builder = builder.header("X-Title", title);
        }

        let response = builder.send().await.map_err(|e| {
            error!("OpenRouter health check failed: {}", e);
            AppError::external_service("OpenRouter", format!("Health check failed: {e}"))
        })?;

        let healthy = response.status().is_success();

        if healthy {
            debug!("OpenRouter API health check passed");
        } else {
            warn!(
                "OpenRouter API health check failed with status: {}",
                response.status()
            );
        }

        Ok(healthy)
    }
}
