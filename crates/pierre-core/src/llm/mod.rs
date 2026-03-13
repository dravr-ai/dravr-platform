// ABOUTME: LLM provider trait and shared types for pluggable AI model integration
// ABOUTME: Re-exports data types from embacle; defines platform LlmProvider trait with AppError
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
use embacle::types::{ErrorKind, RunnerError};
use tokio_stream::Stream;

use crate::errors::{AppError, ErrorCode};

// ============================================================================
// Re-exported Data Types from embacle
// ============================================================================
// These types are the single source of truth defined in the embacle crate.
// Re-exporting here preserves the `pierre_core::llm::*` import paths.

pub use embacle::types::{
    ChatMessage, ChatRequest, ChatResponse, LlmCapabilities, MessageRole, StreamChunk, TokenUsage,
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
            ErrorKind::ExternalService => Self::external_service("CLI runner", err.message),
            ErrorKind::Timeout => Self::new(
                ErrorCode::ResourceUnavailable,
                format!("CLI runner timed out: {}", err.message),
            ),
            ErrorKind::AuthFailure => Self::auth_invalid(err.message),
            ErrorKind::Config => Self::config(err.message),
            ErrorKind::Guardrail => Self::new(
                ErrorCode::InvalidInput,
                format!("Content blocked by guardrail: {}", err.message),
            ),
        }
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
