// ABOUTME: LLM provider trait and shared types for pluggable AI model integration
// ABOUTME: Re-exports data types from embacle; defines the platform LlmProvider trait over AppError and bridges RunnerError into it
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

use std::fmt;
use std::pin::Pin;

use async_trait::async_trait;
use embacle::types::{ErrorKind, RunnerError};
use tokio_stream::Stream;

use crate::errors::{AppError, ErrorCode};

// ============================================================================
// Re-exported Data Types from embacle
// ============================================================================
// These types are the single source of truth defined in the embacle crate.
// Re-exporting here preserves the `pierre_core::llm::*` import paths.

pub use embacle::types::{
    ChatMessage, ChatRequest, ChatResponse, ImagePart, LlmCapabilities, McpHeader, McpServerConfig,
    McpTransport, MessageRole, StreamChunk, TokenUsage, ToolStreamEvent,
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

/// Stream type for a completion that may call tools, streaming its text
///
/// Zero or more [`ToolStreamEvent::TextDelta`]s, then exactly one
/// [`ToolStreamEvent::Done`], or a single `Err` item and no `Done` — embacle's
/// `ToolStream` contract, with [`AppError`] for the error.
pub type ToolTurnStream = Pin<Box<dyn Stream<Item = Result<ToolStreamEvent, AppError>> + Send>>;

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
            // A provider's rate limit or spent quota is that provider's account
            // failing to serve this request: the platform holds one account per
            // provider for every athlete, so no runner ever reports an athlete's
            // own budget — the ingress quota gate refuses that before dispatch.
            // `ExternalRateLimited` is the upstream fault the messaging ingress
            // apologizes for, and the code the headless loop reroutes to the
            // next tier on.
            ErrorKind::RateLimit => Self::new(
                ErrorCode::ExternalRateLimited,
                format!("LLM provider rate limited or out of quota: {}", err.message),
            ),
        }
    }
}

/// Whether `error` is an LLM provider's fault.
///
/// The `AppError` image of embacle's [`ErrorKind::is_provider_fault`], the
/// rule embacle's `FallbackProvider` falls through on under
/// `ResponsePolicy::strict()`.
///
/// A call that runs through the chain is classified by embacle on the
/// `RunnerError` itself. A tool loop converses with the chain's head directly
/// and only ever holds the [`AppError`] the conversion above made, so it asks
/// this instead. Living beside that conversion, the two cannot drift apart
/// unnoticed: the unit test below walks every [`ErrorKind`].
///
/// The codes are the ones every provider-fault kind maps to
/// (`ResourceUnavailable` for a timeout or an unavailable model,
/// `ExternalServiceError` for a vendor fault, `AuthInvalid` for an auth
/// failure, `InternalError` for a runner bug or a missing binary,
/// `ExternalRateLimited` for a rate limit, a spent quota or an HTTP 402 credit
/// exhaustion — the next tier holds its own account) plus the platform's own
/// auth and availability codes (`ExternalAuthFailed`, `AuthExpired`,
/// `ExternalServiceUnavailable`). Deterministic failures — a rejected request
/// (`InvalidInput`), a config error — are not faults: a second tier would
/// answer them the same way. The athlete's own budget (`RateLimitExceeded`,
/// `QuotaExceeded`) is refused by the ingress gate before dispatch and is
/// never a provider's fault.
#[must_use]
pub const fn is_llm_provider_fault(error: &AppError) -> bool {
    matches!(
        error.code,
        ErrorCode::ExternalAuthFailed
            | ErrorCode::ExternalServiceUnavailable
            | ErrorCode::ExternalServiceError
            | ErrorCode::ExternalRateLimited
            | ErrorCode::ResourceUnavailable
            | ErrorCode::AuthInvalid
            | ErrorCode::AuthExpired
            | ErrorCode::InternalError
    )
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

    /// Perform a completion that may call tools, streaming its text
    ///
    /// The tools travel on `request.tools`, as for [`Self::complete`]. The
    /// default answers through `complete` as a single
    /// [`ToolStreamEvent::Done`] with no deltas, so every provider answers it;
    /// a provider that streams a tool-calling turn natively overrides it.
    ///
    /// # Errors
    ///
    /// An `Err` here means no stream was opened. A failure once it is open is
    /// the stream's own `Err` item.
    async fn complete_stream_with_tools(
        &self,
        request: &ChatRequest,
    ) -> Result<ToolTurnStream, AppError> {
        let response = self.complete(request).await?;
        Ok(Box::pin(tokio_stream::once(Ok(ToolStreamEvent::Done(
            response,
        )))))
    }

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

impl fmt::Debug for LlmProviderRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmProviderRegistry")
            .field(
                "providers",
                &self.providers.iter().map(|p| p.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every embacle error kind. The `match` has no wildcard, so a kind embacle
    /// adds fails to compile here until it is listed — and so classified by
    /// the test below.
    fn every_error_kind() -> [ErrorKind; 11] {
        let all = [
            ErrorKind::Internal,
            ErrorKind::ExternalService,
            ErrorKind::Timeout,
            ErrorKind::BinaryNotFound,
            ErrorKind::AuthFailure,
            ErrorKind::Config,
            ErrorKind::Guardrail,
            ErrorKind::ContextLength,
            ErrorKind::ModelUnavailable,
            ErrorKind::RateLimit,
            ErrorKind::InvalidRequest,
        ];
        for kind in all {
            match kind {
                ErrorKind::Internal
                | ErrorKind::ExternalService
                | ErrorKind::Timeout
                | ErrorKind::BinaryNotFound
                | ErrorKind::AuthFailure
                | ErrorKind::Config
                | ErrorKind::Guardrail
                | ErrorKind::ContextLength
                | ErrorKind::ModelUnavailable
                | ErrorKind::RateLimit
                | ErrorKind::InvalidRequest => {}
            }
        }
        all
    }

    #[test]
    fn a_provider_fault_is_exactly_what_embacle_calls_one() {
        for kind in every_error_kind() {
            let error = AppError::from(RunnerError {
                kind,
                message: "scripted".to_owned(),
            });
            assert_eq!(
                is_llm_provider_fault(&error),
                kind.is_provider_fault(),
                "{kind:?} -> {:?} must be a provider fault exactly when embacle says so",
                error.code
            );
        }
    }

    #[test]
    fn the_athletes_own_budget_and_a_rejected_request_are_not_provider_faults() {
        assert!(!is_llm_provider_fault(&AppError::new(
            ErrorCode::RateLimitExceeded,
            "quota"
        )));
        assert!(!is_llm_provider_fault(&AppError::invalid_input(
            "malformed request"
        )));
        assert!(is_llm_provider_fault(&AppError::new(
            ErrorCode::ExternalAuthFailed,
            "token revoked"
        )));
    }
}
