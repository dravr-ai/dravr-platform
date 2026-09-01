// ABOUTME: Cohere LLM provider implementation backed by the v2/chat API
// ABOUTME: Supports Command A and the Command R family with streaming and tool calling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Cohere Provider
//!
//! Implementation of the `LlmProvider` trait for Cohere's v2 chat API.
//!
//! ## Features
//!
//! - **Tool/Function Calling**: OpenAI-shaped `tools` + `tool_calls` payloads
//! - **Streaming**: Server-sent events with Cohere's typed event envelope
//!   (`message-start`, `content-delta`, `message-end`, ...). The shared SSE
//!   line parser handles framing; this module decodes the typed payloads.
//! - **Native multi-turn**: Cohere's `messages` array uses `system`/`user`/
//!   `assistant`/`tool` roles — identical to the canonical [`MessageRole`]
//!   variants so no role translation is needed.
//!
//! ## Configuration
//!
//! Set the `COHERE_API_KEY` environment variable with your API key from
//! the Cohere dashboard: <https://dashboard.cohere.com/api-keys>
//!
//! ## Supported Models
//!
//! - `command-a-03-2025` (default): Command A — 256K context, agentic
//! - `command-a-reasoning-08-2025`: Reasoning-tuned Command A
//! - `command-a-vision-07-2025`: Vision-enabled Command A
//! - `command-r-plus-08-2024`: Command R+
//! - `command-r-08-2024`: Command R
//! - `command-r7b-12-2024`: Command R7B
//!
//! ## Example
//!
//! ```rust,no_run
//! use pierre_llm::{CohereProvider, LlmProvider, ChatRequest, ChatMessage};
//! use pierre_core::errors::AppError;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), AppError> {
//!     let provider = CohereProvider::from_env()?;
//!     let request = ChatRequest::new(vec![
//!         ChatMessage::user("What is reinforcement learning?"),
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
use crate::cohere_errors::parse_error_response;
use crate::errors::AppError;

/// Environment variable for Cohere API key
const COHERE_API_KEY_ENV: &str = "COHERE_API_KEY";

/// Environment variable for default model
const COHERE_DEFAULT_MODEL_ENV: &str = "COHERE_DEFAULT_MODEL";

/// Environment variable for fallback model
const COHERE_FALLBACK_MODEL_ENV: &str = "COHERE_FALLBACK_MODEL";

/// Hardcoded fallback when no env vars are set — Command A is Cohere's
/// flagship general-purpose model and the one referenced by the docs page
/// this provider was added for.
const HARDCODED_DEFAULT_MODEL: &str = "command-a-03-2025";

/// Available Cohere models, longest-prefix-first so the pricing registry's
/// prefix matcher resolves the most specific entry.
const AVAILABLE_MODELS: &[&str] = &[
    "command-a-reasoning-08-2025",
    "command-a-vision-07-2025",
    "command-a-03-2025",
    "command-r-plus-08-2024",
    "command-r-08-2024",
    "command-r7b-12-2024",
];

/// Base URL for the Cohere v2 chat API
const API_BASE_URL: &str = "https://api.cohere.com/v2";

/// Environment variable for maximum retries
const COHERE_MAX_RETRIES_ENV: &str = "COHERE_MAX_RETRIES";

/// Environment variable for initial retry delay in milliseconds
const COHERE_INITIAL_RETRY_DELAY_MS_ENV: &str = "COHERE_INITIAL_RETRY_DELAY_MS";

/// Environment variable for maximum retry delay in milliseconds
const COHERE_MAX_RETRY_DELAY_MS_ENV: &str = "COHERE_MAX_RETRY_DELAY_MS";

// ============================================================================
// API Request/Response Types (Cohere v2 /chat)
// ============================================================================

/// Cohere v2 chat request
#[derive(Debug, Serialize)]
struct CohereRequest {
    model: String,
    messages: Vec<CohereMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<CohereTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

/// Tool definition (OpenAI-compatible shape; Cohere v2 accepts it verbatim)
#[derive(Debug, Clone, Serialize)]
struct CohereTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: CohereFunction,
}

/// Function definition within a tool
#[derive(Debug, Clone, Serialize)]
struct CohereFunction {
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Value>,
}

/// Message structure for Cohere v2 chat — roles match the canonical
/// `system`/`user`/`assistant`/`tool` values produced by `MessageRole`.
///
/// Assistant tool-call turns arrive with empty `content` and the payload in
/// `tool_calls`; tool-result turns carry `tool_call_id`. Cohere v2 rejects any
/// message that has neither non-empty content nor tool calls, so empty content
/// is omitted from the wire and the tool fields are carried through verbatim.
#[derive(Debug, Clone, Serialize)]
struct CohereMessage {
    role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<CohereOutboundToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

/// Assistant tool-call request serialized into a Cohere v2 chat message.
/// Cohere accepts the OpenAI-compatible shape verbatim, mirroring [`CohereTool`].
#[derive(Debug, Clone, Serialize)]
struct CohereOutboundToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: CohereOutboundFunction,
}

/// Function name plus JSON-encoded arguments inside an outbound tool call.
#[derive(Debug, Clone, Serialize)]
struct CohereOutboundFunction {
    name: String,
    /// Cohere v2 expects arguments as a JSON-encoded string, not an object.
    arguments: String,
}

impl From<&ChatMessage> for CohereMessage {
    fn from(msg: &ChatMessage) -> Self {
        let tool_calls = msg.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .map(|call| CohereOutboundToolCall {
                    id: call.id.clone(),
                    call_type: "function".to_owned(),
                    function: CohereOutboundFunction {
                        name: call.function_name.clone(),
                        arguments: call.arguments.to_string(),
                    },
                })
                .collect()
        });

        Self {
            role: msg.role.as_str().to_owned(),
            content: msg.content.clone(),
            tool_calls,
            tool_call_id: msg.tool_call_id.clone(),
        }
    }
}

/// Cohere v2 chat response envelope
#[derive(Debug, Deserialize)]
struct CohereResponse {
    #[serde(default)]
    id: Option<String>,
    message: CohereResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    usage: Option<CohereUsage>,
}

/// Assistant message inside a Cohere v2 chat response
#[derive(Debug, Deserialize)]
struct CohereResponseMessage {
    /// Content is returned as an array of typed blocks; we surface the
    /// concatenated `text` payloads to match the trait's flat `content`
    /// string. Tool-only responses arrive with an empty/missing content
    /// array and the model output in `tool_calls`.
    #[serde(default)]
    content: Vec<CohereContentBlock>,
    #[serde(default)]
    tool_calls: Option<Vec<CohereToolCall>>,
}

/// Single content block in a Cohere v2 message — only `text` blocks are
/// surfaced to callers today. Unknown variants are skipped via the
/// `#[serde(other)]` catch-all so future block kinds don't break parsing.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum CohereContentBlock {
    Text {
        text: String,
    },
    #[serde(other)]
    Unknown,
}

/// Tool call payload returned by Cohere v2 (OpenAI-compatible shape)
#[derive(Debug, Clone, Deserialize)]
struct CohereToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: CohereFunctionCall,
}

/// Function call details inside a Cohere tool call
#[derive(Debug, Clone, Deserialize)]
struct CohereFunctionCall {
    name: String,
    /// Arguments are returned as a JSON-encoded string by Cohere v2.
    arguments: String,
}

/// Usage statistics. Cohere v2 reports both `billed_units` and `tokens`;
/// `billed_units` is what gets invoiced so that's what we surface upstream
/// for cost tracking.
#[derive(Debug, Deserialize)]
struct CohereUsage {
    #[serde(default)]
    billed_units: Option<CohereTokenCounts>,
    #[serde(default)]
    tokens: Option<CohereTokenCounts>,
}

/// Per-side token counts inside a Cohere usage payload
#[derive(Debug, Default, Deserialize)]
struct CohereTokenCounts {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

/// Cohere v2 streaming event envelope. Cohere uses typed events instead of
/// the OpenAI-style `delta` field, so the variant tag dictates how we map
/// each frame onto a [`StreamChunk`].
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CohereStreamEvent {
    /// First event in a stream — carries the response id; nothing to emit.
    #[serde(rename = "message-start")]
    MessageStart {},
    /// Marks the start of a content block; nothing user-visible yet.
    #[serde(rename = "content-start")]
    ContentStart {},
    /// Incremental text payload — the only event we forward as delta text.
    #[serde(rename = "content-delta")]
    ContentDelta {
        #[serde(default)]
        delta: Option<CohereContentDeltaPayload>,
    },
    /// Marks the end of a content block; nothing to emit.
    #[serde(rename = "content-end")]
    ContentEnd {},
    /// Terminal event — carries the finish reason and final usage payload.
    #[serde(rename = "message-end")]
    MessageEnd {
        #[serde(default)]
        delta: Option<CohereMessageEndDelta>,
    },
    /// Tool-call frames are accepted for streaming completeness; they are
    /// emitted as empty deltas because tool calls go through the
    /// non-streaming `complete_with_tools()` path in this codebase.
    #[serde(other)]
    Other,
}

/// Inner payload of a `content-delta` event
#[derive(Debug, Deserialize)]
struct CohereContentDeltaPayload {
    #[serde(default)]
    message: Option<CohereContentDeltaMessage>,
}

/// `message` wrapper inside a `content-delta` event
#[derive(Debug, Deserialize)]
struct CohereContentDeltaMessage {
    #[serde(default)]
    content: Option<CohereContentDeltaContent>,
}

/// Final text carrier inside a `content-delta` event
#[derive(Debug, Deserialize)]
struct CohereContentDeltaContent {
    #[serde(default)]
    text: Option<String>,
}

/// `delta` field of a `message-end` event
#[derive(Debug, Deserialize)]
struct CohereMessageEndDelta {
    #[serde(default)]
    finish_reason: Option<String>,
}

// ============================================================================
// Provider Implementation
// ============================================================================

/// Cohere LLM provider using the v2 chat API
pub struct CohereProvider {
    client: &'static SharedHttpClient,
    api_key: String,
    default_model: String,
    fallback_model: String,
    available_models: Vec<String>,
    /// Retry configuration for transient failures (429, 503, network errors)
    retry_config: RetryConfig,
}

impl CohereProvider {
    /// Create a new Cohere provider with the given API key
    ///
    /// Uses environment variables for model and retry configuration:
    /// - `COHERE_DEFAULT_MODEL`: Primary model (default: command-a-03-2025)
    /// - `COHERE_FALLBACK_MODEL`: Fallback model (default: command-a-03-2025)
    #[must_use]
    pub fn new(api_key: String) -> Self {
        let default_model = env::var(COHERE_DEFAULT_MODEL_ENV)
            .unwrap_or_else(|_| HARDCODED_DEFAULT_MODEL.to_owned());
        let fallback_model = env::var(COHERE_FALLBACK_MODEL_ENV)
            .unwrap_or_else(|_| HARDCODED_DEFAULT_MODEL.to_owned());

        let max_retries = env::var(COHERE_MAX_RETRIES_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RetryConfig::default_config().max_retries);
        let initial_delay_ms = env::var(COHERE_INITIAL_RETRY_DELAY_MS_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RetryConfig::default_config().initial_delay_ms);
        let max_delay_ms = env::var(COHERE_MAX_RETRY_DELAY_MS_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RetryConfig::default_config().max_delay_ms);

        Self {
            client: build_llm_http_client(),
            api_key,
            default_model,
            fallback_model,
            available_models: AVAILABLE_MODELS.iter().map(|s| (*s).to_owned()).collect(),
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
    /// `PIERRE_LLM_FALLBACK_PROVIDER_MODEL` redirects the secondary to a
    /// Cohere-specific model name that differs from the primary's
    /// `PIERRE_LLM_DEFAULT_MODEL`.
    #[must_use]
    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    /// Create a Cohere provider from environment variable
    ///
    /// # Errors
    ///
    /// Returns an error if `COHERE_API_KEY` is not set
    pub fn from_env() -> Result<Self, AppError> {
        let api_key = env::var(COHERE_API_KEY_ENV).map_err(|_| {
            AppError::config(format!(
                "Missing {COHERE_API_KEY_ENV} environment variable. Get your API key from https://dashboard.cohere.com/api-keys"
            ))
        })?;

        let provider = Self::new(api_key);
        info!(
            default_model = %provider.default_model,
            fallback_model = %provider.fallback_model,
            max_retries = provider.retry_config.max_retries,
            initial_delay_ms = provider.retry_config.initial_delay_ms,
            "Cohere provider initialized"
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

    /// Body substituted for a tool-result message whose tool produced no output,
    /// so it satisfies Cohere v2's "non-empty content or tool calls" rule without
    /// being dropped (dropping it would orphan its matching tool call).
    const EMPTY_TOOL_RESULT_PLACEHOLDER: &str = "(no result)";

    /// Convert internal messages to Cohere v2 format.
    ///
    /// Cohere v2 rejects any message with neither non-empty content nor
    /// `tool_calls` (empty content is skipped on the wire, leaving a bare
    /// `{"role": ...}` that 400s the WHOLE request — "invalid message provided
    /// at index N: must have non-empty content or tool calls"). A content-less,
    /// tool-call-less turn — e.g. an empty or failed assistant turn left in a
    /// long history — conveys nothing to the fallback and is dropped; an empty
    /// tool-result (carries `tool_call_id`) is kept with a placeholder body so
    /// the call/result pairing Cohere requires stays intact.
    fn convert_messages(messages: &[ChatMessage]) -> Vec<CohereMessage> {
        messages
            .iter()
            .map(CohereMessage::from)
            .filter_map(|mut m| {
                if !m.content.is_empty() || m.tool_calls.is_some() {
                    Some(m)
                } else if m.tool_call_id.is_some() {
                    Self::EMPTY_TOOL_RESULT_PLACEHOLDER.clone_into(&mut m.content);
                    Some(m)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Concatenate all text content blocks in a response message
    fn collect_message_text(content: &[CohereContentBlock]) -> String {
        let mut out = String::new();
        for block in content {
            if let CohereContentBlock::Text { text } = block {
                out.push_str(text);
            }
        }
        out
    }

    /// Resolve token usage from a Cohere usage payload, preferring billed
    /// units (what gets invoiced) over the raw token count.
    fn resolve_usage(usage: Option<CohereUsage>) -> Option<TokenUsage> {
        let usage = usage?;
        let counts = usage.billed_units.or(usage.tokens)?;
        Some(TokenUsage::new(
            counts.input_tokens,
            counts.output_tokens,
            counts.input_tokens.saturating_add(counts.output_tokens),
        ))
    }

    /// Check if an error from the Cohere API is retryable (transient)
    fn is_retryable_error(status: u16) -> bool {
        sse_parser::is_retryable_status(status)
    }

    /// Build an authenticated HTTP request to the Cohere chat endpoint
    fn build_request(&self, cohere_request: &CohereRequest) -> RequestBuilder {
        self.client
            .post(Self::api_url("chat"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(cohere_request)
    }

    /// Parse a Cohere v2 SSE data payload into a `StreamChunk`.
    ///
    /// Returns `None` for events that produce no delta (envelope frames
    /// like `message-start`, `content-start`, `content-end`, unknown
    /// variants). `message-end` produces a final empty chunk carrying the
    /// finish reason so consumers know the response has terminated.
    fn parse_stream_data(json_str: &str) -> Option<Result<StreamChunk, AppError>> {
        match serde_json::from_str::<CohereStreamEvent>(json_str) {
            Ok(CohereStreamEvent::ContentDelta { delta }) => {
                let text = delta
                    .and_then(|d| d.message)
                    .and_then(|m| m.content)
                    .and_then(|c| c.text)
                    .unwrap_or_default();
                if text.is_empty() {
                    None
                } else {
                    Some(Ok(StreamChunk {
                        delta: text,
                        is_final: false,
                        finish_reason: None,
                    }))
                }
            }
            Ok(CohereStreamEvent::MessageEnd { delta }) => {
                let finish_reason = delta
                    .and_then(|d| d.finish_reason)
                    .or_else(|| Some("stop".to_owned()));
                Some(Ok(StreamChunk {
                    delta: String::new(),
                    is_final: true,
                    finish_reason,
                }))
            }
            Ok(_) => None,
            Err(e) => {
                warn!("Failed to parse Cohere stream chunk: {e}");
                None
            }
        }
    }

    /// Convert internal `Tool` format to Cohere v2's OpenAI-compatible format
    fn convert_tools(tools: &[Tool]) -> Vec<CohereTool> {
        tools
            .iter()
            .flat_map(|tool| {
                tool.function_declarations.iter().map(|func| CohereTool {
                    tool_type: "function".to_owned(),
                    function: CohereFunction {
                        name: func.name.clone(),
                        description: func.description.clone(),
                        parameters: func.parameters.clone(),
                    },
                })
            })
            .collect()
    }

    /// Convert Cohere tool calls to internal `FunctionCall` format
    fn convert_tool_calls(tool_calls: &[CohereToolCall]) -> Vec<FunctionCall> {
        tool_calls
            .iter()
            .map(|call| {
                debug!(
                    tool_call_id = %call.id,
                    tool_call_type = %call.call_type,
                    function_name = %call.function.name,
                    "Converting Cohere tool call to FunctionCall"
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

    /// Perform a chat completion with tool/function calling support.
    ///
    /// Cohere v2 advertises an OpenAI-compatible `tools` payload, so we
    /// reuse the same conversion path as Groq and the response message
    /// carries `tool_calls` alongside any text blocks.
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

        debug!("Sending chat completion request to Cohere with tools");

        let cohere_tools = tools.as_ref().map(|t| Self::convert_tools(t));

        let cohere_request = CohereRequest {
            model: model.to_owned(),
            messages: Self::convert_messages(&request.messages),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: Some(false),
            tools: cohere_tools,
            tool_choice: tools.as_ref().map(|_| "auto".to_owned()),
        };

        let response = self
            .build_request(&cohere_request)
            .send()
            .await
            .map_err(|e| {
                error!("Failed to send request to Cohere API: {}", e);
                AppError::external_service("Cohere", format!("Failed to connect: {e}"))
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| {
            error!("Failed to read Cohere API response: {}", e);
            AppError::external_service("Cohere", format!("Failed to read response: {e}"))
        })?;

        if !status.is_success() {
            return Err(parse_error_response(status, &body));
        }

        let cohere_response: CohereResponse = serde_json::from_str(&body).map_err(|e| {
            error!("Failed to parse Cohere API response: {}", e);
            AppError::external_service("Cohere", format!("Failed to parse response: {e}"))
        })?;

        let text = Self::collect_message_text(&cohere_response.message.content);
        let content = if text.is_empty() { None } else { Some(text) };
        let function_calls = cohere_response.message.tool_calls.map(|calls| {
            info!("Cohere returned {} tool calls", calls.len());
            Self::convert_tool_calls(&calls)
        });

        debug!(
            "Received response from Cohere: id={:?}, content_chars={:?}, tool_calls={:?}, finish_reason: {:?}",
            cohere_response.id,
            content.as_ref().map(String::len),
            function_calls.as_ref().map(Vec::len),
            cohere_response.finish_reason
        );

        Ok(ChatResponseWithTools {
            content,
            function_calls,
            model: model.to_owned(),
            usage: Self::resolve_usage(cohere_response.usage),
            finish_reason: cohere_response.finish_reason,
        })
    }
}

#[async_trait]
impl LlmProvider for CohereProvider {
    fn name(&self) -> &'static str {
        "cohere"
    }

    fn display_name(&self) -> &'static str {
        "Cohere (Command)"
    }

    fn capabilities(&self) -> LlmCapabilities {
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

        let cohere_request = CohereRequest {
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
                    "Retrying Cohere request after transient failure"
                );
                sleep(delay).await;
            }

            debug!(
                attempt = attempt,
                "Sending chat completion request to Cohere"
            );

            let response = match self.build_request(&cohere_request).send().await {
                Ok(r) => r,
                Err(e) => {
                    let error =
                        AppError::external_service("Cohere", format!("Failed to connect: {e}"));
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
                        "Cohere",
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
                let error = parse_error_response(status, &body);
                if Self::is_retryable_error(status.as_u16())
                    && attempt < self.retry_config.max_retries
                {
                    warn!(status = %status, attempt = attempt, "Cohere API returned retryable error");
                    last_error = Some(error);
                    continue;
                }
                return Err(error);
            }

            let cohere_response: CohereResponse = serde_json::from_str(&body).map_err(|e| {
                error!("Failed to parse Cohere API response: {e}");
                AppError::external_service("Cohere", format!("Failed to parse response: {e}"))
            })?;

            let content = Self::collect_message_text(&cohere_response.message.content);

            debug!(
                attempt = attempt,
                "Received response from Cohere: {} chars, finish_reason: {:?}",
                content.len(),
                cohere_response.finish_reason
            );

            return Ok(ChatResponse {
                content,
                model: model.to_owned(),
                usage: Self::resolve_usage(cohere_response.usage),
                finish_reason: cohere_response.finish_reason,
                warnings: None,
                tool_calls: None,
            });
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::external_service("Cohere", "Request failed after retries")
        }))
    }

    #[instrument(skip(self, request), fields(model = %request.model.as_deref().unwrap_or(&self.default_model)))]
    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let model = request.model.as_deref().unwrap_or(&self.default_model);

        let cohere_request = CohereRequest {
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
                    "Retrying Cohere streaming request after transient failure"
                );
                sleep(delay).await;
            }

            debug!(
                attempt = attempt,
                "Sending streaming chat completion request to Cohere"
            );

            let response = match self.build_request(&cohere_request).send().await {
                Ok(r) => r,
                Err(e) => {
                    let error =
                        AppError::external_service("Cohere", format!("Failed to connect: {e}"));
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
                let error = parse_error_response(status, &body);
                if Self::is_retryable_error(status.as_u16())
                    && attempt < self.retry_config.max_retries
                {
                    warn!(status = %status, attempt = attempt, "Cohere streaming API returned retryable error");
                    last_error = Some(error);
                    continue;
                }
                return Err(error);
            }

            let stream = sse_parser::create_sse_stream(
                response.bytes_stream(),
                Self::parse_stream_data,
                "Cohere",
            );
            return Ok(stream);
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::external_service("Cohere", "Streaming request failed after retries")
        }))
    }

    #[instrument(skip(self))]
    async fn health_check(&self) -> Result<bool, AppError> {
        debug!("Performing Cohere API health check");

        // The /v2/models endpoint is the lightest authenticated probe Cohere
        // exposes — same idea as Groq's models endpoint.
        let response = self
            .client
            .get(Self::api_url("models"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| {
                error!("Cohere health check failed: {}", e);
                AppError::external_service("Cohere", format!("Health check failed: {e}"))
            })?;

        let healthy = response.status().is_success();

        if healthy {
            debug!("Cohere API health check passed");
        } else {
            warn!(
                "Cohere API health check failed with status: {}",
                response.status()
            );
        }

        Ok(healthy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::is_retryable_for_fallback;
    use embacle::types::ToolCallRequest;
    use pierre_core::errors::AppResult;
    use pierre_core::errors::ErrorCode;

    #[test]
    fn empty_completion_422_is_retryable_for_fallback() {
        // Cohere 422 "No tool calls or response was generated" is a provider-side
        // empty completion, not a malformed request — it must classify as a
        // retryable error so the runtime fallback chain cascades to the next
        // provider. Regression for the 2026-06-17 cold-start incident: Copilot
        // auth failed, Cohere returned this, and the turn errored out (generic
        // "Dravr indisponible") instead of trying Gemini.
        let body = r#"{"message":"No tool calls or response was generated. Try updating messages or tool definitions"}"#;
        let err = parse_error_response(reqwest::StatusCode::UNPROCESSABLE_ENTITY, body);
        assert!(
            is_retryable_for_fallback(&err),
            "empty-completion 422 must cascade; got code {:?}",
            err.code
        );
    }

    #[test]
    fn the_other_empty_completion_phrasing_also_cascades() {
        // Cohere states the same inability two ways, and only one was matched.
        // Verbatim from live-incident-eval run 33492743565 (2026-09-01), where
        // it ended 2 of 16 empty-turn handoffs: the athlete got the canned
        // "Dravr est temporairement indisponible" while the Gemini tertiary was
        // never tried, because a provider-side non-answer had been classified
        // as a malformed request.
        let body = r#"{"message":"No valid response generated. Try updating messages"}"#;
        let err = parse_error_response(reqwest::StatusCode::BAD_REQUEST, body);
        assert!(
            is_retryable_for_fallback(&err),
            "this phrasing must cascade to the tertiary too; got code {:?}",
            err.code
        );
    }

    #[test]
    fn genuine_validation_422_stays_non_retryable() {
        // A real malformed-request 422 would fail on every provider, so it must
        // NOT cascade — it stays a non-retryable InvalidInput.
        let body = r#"{"message":"invalid request: messages must not be empty"}"#;
        let err = parse_error_response(reqwest::StatusCode::UNPROCESSABLE_ENTITY, body);
        assert!(
            !is_retryable_for_fallback(&err),
            "a genuine validation 422 must not cascade; got code {:?}",
            err.code
        );
    }

    #[test]
    fn role_mapping_matches_cohere_v2_schema() {
        let messages = vec![
            ChatMessage::system("be terse"),
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello"),
        ];
        let converted = CohereProvider::convert_messages(&messages);
        let roles: Vec<&str> = converted.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["system", "user", "assistant"]);
    }

    #[test]
    fn assistant_tool_call_message_omits_empty_content_and_keeps_tool_calls() -> AppResult<()> {
        // A prior assistant turn that was a pure tool call: empty text, payload
        // in `tool_calls`. Cohere v2 rejects a message with neither content nor
        // tool calls, so the wire form must omit `content` and carry the call.
        let mut msg = ChatMessage::assistant("");
        msg.tool_calls = Some(vec![ToolCallRequest {
            id: "call_1".to_owned(),
            function_name: "get_activities".to_owned(),
            arguments: serde_json::json!({ "limit": 5 }),
        }]);

        let converted = CohereProvider::convert_messages(&[msg]);
        let json =
            serde_json::to_value(&converted[0]).map_err(|e| AppError::internal(e.to_string()))?;

        assert!(
            json.get("content").is_none(),
            "empty content must be omitted so Cohere does not reject the message"
        );
        let calls = json
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AppError::internal("tool_calls should be present"))?;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_1");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[0]["function"]["name"], "get_activities");
        assert_eq!(calls[0]["function"]["arguments"], "{\"limit\":5}");
        Ok(())
    }

    #[test]
    fn empty_content_no_tool_calls_message_is_dropped() {
        // The "invalid message at index N" 400: a content-less assistant turn
        // with no tool_calls serializes to a bare {"role": ...}, which Cohere
        // rejects and sinks the WHOLE request. It must be dropped so a fallback
        // turn still goes through.
        let messages = vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant(""),
            ChatMessage::assistant("real answer"),
        ];
        let converted = CohereProvider::convert_messages(&messages);
        let roles: Vec<&str> = converted.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant"]);
        assert_eq!(converted[1].content, "real answer");
    }

    #[test]
    fn empty_tool_result_kept_with_placeholder_not_dropped() {
        // A tool that returned nothing yields an empty tool-result. Dropping it
        // would orphan its matching tool call (Cohere requires call/result
        // pairing), so it is kept with a placeholder body to satisfy the
        // non-empty rule.
        let converted =
            CohereProvider::convert_messages(&[ChatMessage::tool("get_activities", "call_1", "")]);
        assert_eq!(
            converted.len(),
            1,
            "empty tool-result must be kept, not dropped"
        );
        assert!(
            !converted[0].content.is_empty(),
            "empty tool-result gets a placeholder body so Cohere accepts it"
        );
        assert_eq!(converted[0].tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn tool_result_message_carries_tool_call_id_and_content() -> AppResult<()> {
        let msg = ChatMessage::tool("get_activities", "call_1", "5 runs this week");
        let converted = CohereProvider::convert_messages(&[msg]);
        let json =
            serde_json::to_value(&converted[0]).map_err(|e| AppError::internal(e.to_string()))?;

        assert_eq!(json["role"], "tool");
        assert_eq!(json["tool_call_id"], "call_1");
        assert_eq!(json["content"], "5 runs this week");
        Ok(())
    }

    #[test]
    fn plain_text_messages_serialize_without_tool_fields() -> AppResult<()> {
        let msg = ChatMessage::user("hi");
        let converted = CohereProvider::convert_messages(&[msg]);
        let json =
            serde_json::to_value(&converted[0]).map_err(|e| AppError::internal(e.to_string()))?;

        assert_eq!(json["content"], "hi");
        assert!(json.get("tool_calls").is_none());
        assert!(json.get("tool_call_id").is_none());
        Ok(())
    }

    #[test]
    fn collect_message_text_concatenates_text_blocks_and_skips_unknown() {
        let blocks = vec![
            CohereContentBlock::Text {
                text: "Hello, ".to_owned(),
            },
            CohereContentBlock::Unknown,
            CohereContentBlock::Text {
                text: "world".to_owned(),
            },
        ];
        assert_eq!(
            CohereProvider::collect_message_text(&blocks),
            "Hello, world"
        );
    }

    #[test]
    fn resolve_usage_prefers_billed_units_over_raw_tokens() -> AppResult<()> {
        let usage = CohereUsage {
            billed_units: Some(CohereTokenCounts {
                input_tokens: 10,
                output_tokens: 20,
            }),
            tokens: Some(CohereTokenCounts {
                input_tokens: 99,
                output_tokens: 99,
            }),
        };
        let resolved = CohereProvider::resolve_usage(Some(usage))
            .ok_or_else(|| AppError::internal("usage should resolve"))?;
        assert_eq!(resolved.prompt_tokens, 10);
        assert_eq!(resolved.completion_tokens, 20);
        assert_eq!(resolved.total_tokens, 30);
        Ok(())
    }

    #[test]
    fn resolve_usage_falls_back_to_raw_tokens() -> AppResult<()> {
        let usage = CohereUsage {
            billed_units: None,
            tokens: Some(CohereTokenCounts {
                input_tokens: 5,
                output_tokens: 7,
            }),
        };
        let resolved = CohereProvider::resolve_usage(Some(usage))
            .ok_or_else(|| AppError::internal("usage should resolve"))?;
        assert_eq!(resolved.prompt_tokens, 5);
        assert_eq!(resolved.completion_tokens, 7);
        assert_eq!(resolved.total_tokens, 12);
        Ok(())
    }

    #[test]
    fn parse_stream_data_extracts_content_delta_text() -> AppResult<()> {
        let frame =
            r#"{"type":"content-delta","index":0,"delta":{"message":{"content":{"text":"Hi"}}}}"#;
        let chunk = CohereProvider::parse_stream_data(frame)
            .ok_or_else(|| AppError::internal("delta should produce an event"))??;
        assert_eq!(chunk.delta, "Hi");
        assert!(!chunk.is_final);
        Ok(())
    }

    #[test]
    fn parse_stream_data_emits_final_chunk_on_message_end() -> AppResult<()> {
        let frame = r#"{"type":"message-end","delta":{"finish_reason":"COMPLETE"}}"#;
        let chunk = CohereProvider::parse_stream_data(frame)
            .ok_or_else(|| AppError::internal("message-end should produce an event"))??;
        assert!(chunk.is_final);
        assert_eq!(chunk.finish_reason.as_deref(), Some("COMPLETE"));
        Ok(())
    }

    #[test]
    fn parse_stream_data_skips_envelope_frames() {
        let frames = [
            r#"{"type":"message-start","id":"123"}"#,
            r#"{"type":"content-start","index":0}"#,
            r#"{"type":"content-end","index":0}"#,
        ];
        for frame in frames {
            assert!(
                CohereProvider::parse_stream_data(frame).is_none(),
                "envelope frame `{frame}` should not produce a delta"
            );
        }
    }

    #[test]
    fn parse_error_response_maps_401_to_auth_invalid() {
        let body = r#"{"message":"invalid api token"}"#;
        let err = parse_error_response(reqwest::StatusCode::UNAUTHORIZED, body);
        assert_eq!(err.code, ErrorCode::AuthInvalid);
        assert!(err.message.contains("invalid api token"));
    }

    #[test]
    fn parse_error_response_maps_429_to_rate_limited() {
        let body = r#"{"message":"too many requests"}"#;
        let err = parse_error_response(reqwest::StatusCode::TOO_MANY_REQUESTS, body);
        assert_eq!(err.code, ErrorCode::ExternalRateLimited);
    }

    #[test]
    fn parse_error_response_falls_back_to_raw_body_on_unknown_shape() {
        let body = "<html>nginx 502</html>";
        let err = parse_error_response(reqwest::StatusCode::BAD_GATEWAY, body);
        assert_eq!(err.code, ErrorCode::ExternalServiceError);
        assert!(err.message.contains("502"));
    }
}
