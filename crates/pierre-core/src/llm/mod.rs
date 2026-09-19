// ABOUTME: LLM provider trait and shared types for pluggable AI model integration
// ABOUTME: Re-exports data types from embacle; defines platform LlmProvider trait with AppError, bridges RunnerError, marks vendor-keyed HTTP tiers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Provider Types
//!
//! Shared types and trait for LLM provider integration. Data types (messages,
//! requests, responses, capabilities) come from the [`embacle`] standalone
//! library. The platform-specific [`LlmProvider`] trait and [`ChatStream`] use
//! [`AppError`](crate::errors::AppError) for error handling.
//!
//! ## Key Types
//!
//! - [`LlmCapabilities`]: Bitflags describing provider features (streaming, function calling, etc.)
//! - [`LlmProvider`]: Async trait for chat completion with streaming support
//! - [`ChatMessage`]: Role-based message structure for conversations
//! - [`ChatRequest`]: Request configuration including model, temperature, etc.
//! - [`ChatResponse`]: Completion result with content and usage stats
//! - [`ChatStream`]: Streaming response as a pinned trait object

use std::pin::Pin;

use async_trait::async_trait;
use embacle::types::{
    ChatStream as RunnerChatStream, ErrorKind, LlmProvider as Runner, RunnerError,
};
use tokio_stream::{Stream, StreamExt};

use crate::errors::{AppError, ErrorCode};

// ============================================================================
// Re-exported Data Types from embacle
// ============================================================================
// These types are the single source of truth defined in the embacle crate.
// Re-exporting here preserves the `pierre_core::llm::*` import paths.

pub use embacle::types::{
    ChatMessage, ChatRequest, ChatResponse, LlmCapabilities, McpHeader, McpServerConfig,
    McpTransport, MessageRole, StreamChunk, TokenUsage,
};

/// Text-based tool simulation for CLI LLM runners (re-exported from embacle)
pub use embacle::tool_simulation;

// ============================================================================
// Platform-Specific Stream Type
// ============================================================================

/// Stream type for chat completion responses
///
/// Uses [`AppError`] for error handling (platform-specific).
/// Embacle defines its own stream type with [`RunnerError`] for standalone use.
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<StreamChunk, AppError>> + Send>>;

// ============================================================================
// Error Conversion: embacle → AppError
// ============================================================================

impl From<RunnerError> for AppError {
    fn from(err: RunnerError) -> Self {
        match err.kind {
            ErrorKind::Internal | ErrorKind::BinaryNotFound => Self::internal(err.message),
            ErrorKind::ExternalService => Self::external_service("LLM provider", err.message),
            ErrorKind::Timeout => Self::new(
                ErrorCode::ResourceUnavailable,
                format!("LLM provider timed out: {}", err.message),
            ),
            ErrorKind::AuthFailure => Self::auth_invalid(err.message),
            ErrorKind::Config => Self::config(err.message),
            ErrorKind::Guardrail => Self::new(
                ErrorCode::InvalidInput,
                format!("Content blocked by guardrail: {}", err.message),
            ),
            ErrorKind::ContextLength => Self::new(
                ErrorCode::InvalidInput,
                format!("Prompt exceeds model context window: {}", err.message),
            ),
            // Caller-side and permanent: another provider would reject the
            // same request the same way, so it never falls through a chain.
            ErrorKind::InvalidRequest => Self::new(
                ErrorCode::InvalidInput,
                format!("Provider rejected the request: {}", err.message),
            ),
            ErrorKind::ModelUnavailable => Self::new(
                ErrorCode::ResourceUnavailable,
                format!("Model unavailable: {}", err.message),
            ),
            // A vendor throttling the platform's own key — marked by
            // `HttpApiTier` on its way out of the tier — is an upstream
            // outage to the athlete: `ExternalRateLimited`, the 503 the
            // messaging ingress reports as a failure and apologizes for.
            ErrorKind::RateLimit => match err.message.strip_prefix(VENDOR_THROTTLE_MARK) {
                Some(vendor_message) => Self::new(
                    ErrorCode::ExternalRateLimited,
                    format!("LLM provider rate limited: {vendor_message}"),
                ),
                // The account's quota (a CLI or Copilot runner): `RateLimitExceeded`,
                // HTTP 429, which the ingress classifies as a quota denial.
                //
                // Deliberately not `rate_limit_exceeded`: that constructor wants a
                // current count, a limit and a retry-after, and a runner's quota
                // refusal carries none of the three. Inventing them would put three
                // fabricated numbers in front of an operator reading a 429.
                None => Self::new(
                    ErrorCode::RateLimitExceeded,
                    format!("LLM provider quota exhausted: {}", err.message),
                ),
            },
        }
    }
}

// ============================================================================
// Vendor-keyed HTTP tiers
// ============================================================================

/// The prefix a vendor's `RateLimit` carries from an [`HttpApiTier`] to the
/// bridge above.
///
/// A `RunnerError` is a kind and a message, and a fallback chain hands back
/// whichever tier's error propagated without saying which tier — so the only
/// thing that reaches `From<RunnerError>` is the error itself. The tier
/// prefixes the message on the way out; the bridge strips it on the way in.
/// Nothing else reads it, and a message that escapes the bridge unstripped
/// still reads as what it is.
const VENDOR_THROTTLE_MARK: &str = "upstream rate limit: ";

/// An embacle runner that reaches a vendor over HTTP with the platform's own
/// key: Gemini, Cohere, Groq, `OpenRouter`, an `OpenAI`-compatible endpoint,
/// the `OpenAI` API.
///
/// Such a runner's [`ErrorKind::RateLimit`] is the vendor throttling the
/// platform, not the athlete spending a quota, and the platform bridges it to
/// [`ErrorCode::ExternalRateLimited`] rather than
/// [`ErrorCode::RateLimitExceeded`] — the code the messaging ingress treats as
/// the athlete's own budget refusing the turn. A CLI or Copilot runner is
/// never wrapped: its `RateLimit` is the account's quota and keeps its code.
///
/// Fall-through is untouched: the kind is preserved, and neither code moves a
/// request to the next tier of a chain.
pub struct HttpApiTier {
    inner: Box<dyn Runner>,
}

impl HttpApiTier {
    /// Wrap `inner`, a runner whose rate limit is the vendor's.
    #[must_use]
    pub fn new(inner: Box<dyn Runner>) -> Self {
        Self { inner }
    }

    /// Mark a vendor `RateLimit` so the bridge can tell it from a quota;
    /// every other error passes unchanged.
    fn mark_vendor_throttle(mut err: RunnerError) -> RunnerError {
        if err.kind == ErrorKind::RateLimit {
            err.message.insert_str(0, VENDOR_THROTTLE_MARK);
        }
        err
    }
}

#[async_trait]
impl Runner for HttpApiTier {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn display_name(&self) -> &str {
        self.inner.display_name()
    }

    fn capabilities(&self) -> LlmCapabilities {
        self.inner.capabilities()
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    fn available_models(&self) -> &[String] {
        self.inner.available_models()
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.inner
            .complete(request)
            .await
            .map_err(Self::mark_vendor_throttle)
    }

    async fn complete_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<RunnerChatStream, RunnerError> {
        let stream = self
            .inner
            .complete_stream(request)
            .await
            .map_err(Self::mark_vendor_throttle)?;
        Ok(Box::pin(
            stream.map(|chunk| chunk.map_err(Self::mark_vendor_throttle)),
        ))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        self.inner
            .health_check()
            .await
            .map_err(Self::mark_vendor_throttle)
    }
}

// ============================================================================
// Provider Trait
// ============================================================================

/// LLM provider trait for chat completion
///
/// Implement this trait to add a new LLM provider to Pierre.
/// The design follows the async trait pattern for compatibility
/// with tokio-based async runtime.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Unique provider identifier (e.g., "gemini", "openai", "ollama")
    fn name(&self) -> &'static str;

    /// Human-readable display name for the provider
    fn display_name(&self) -> &'static str;

    /// Provider capabilities (streaming, function calling, etc.)
    fn capabilities(&self) -> LlmCapabilities;

    /// Default model to use if not specified in request
    fn default_model(&self) -> &str;

    /// Available models for this provider
    fn available_models(&self) -> &[String];

    /// Perform a chat completion (non-streaming)
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError>;

    /// Perform a streaming chat completion
    ///
    /// Returns a stream of chunks that can be consumed incrementally.
    /// Falls back to non-streaming if not supported.
    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError>;

    /// Check if the provider is healthy and API key is valid
    async fn health_check(&self) -> Result<bool, AppError>;
}

// ============================================================================
// Provider Registry
// ============================================================================

/// Registry for LLM providers
///
/// Manages available providers and provides lookup by name.
pub struct LlmProviderRegistry {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl LlmProviderRegistry {
    /// Create a new empty registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a provider
    pub fn register(&mut self, provider: Box<dyn LlmProvider>) {
        self.providers.push(provider);
    }

    /// Get a provider by name
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn LlmProvider> {
        self.providers
            .iter()
            .find(|p| p.name() == name)
            .map(AsRef::as_ref)
    }

    /// List all registered providers
    #[must_use]
    pub fn list(&self) -> Vec<&dyn LlmProvider> {
        self.providers.iter().map(AsRef::as_ref).collect()
    }

    /// Get the default provider (first registered)
    #[must_use]
    pub fn default_provider(&self) -> Option<&dyn LlmProvider> {
        self.providers.first().map(AsRef::as_ref)
    }
}

impl Default for LlmProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
