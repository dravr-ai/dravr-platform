// ABOUTME: LLM provider abstraction layer for pluggable AI model integration
// ABOUTME: Defines the contract for LLM providers (Gemini, OpenAI, etc.) with streaming support
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Provider Service Provider Interface
//!
//! This crate provides the contract that LLM providers must implement to integrate
//! with the Pierre chat system. The design mirrors the fitness provider SPI pattern
//! for consistency and extensibility.
//!
//! ## Key Concepts
//!
//! - **`LlmCapabilities`**: Bitflags describing provider features (streaming, function calling, etc.)
//! - **`LlmProvider`**: Async trait for chat completion with streaming support
//! - **`ChatMessage`**: Role-based message structure for conversations
//! - **`ChatRequest`**: Request configuration including model, temperature, etc.

#![deny(unsafe_code)]

// Re-export pierre-core modules so moved files can keep `use crate::errors::*` etc.
pub use pierre_core::errors;
pub use pierre_core::models;

// Re-export LLM types from pierre-core (canonical definitions live there)
pub use pierre_core::llm::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
    LlmProviderRegistry, MessageRole, StreamChunk, TokenUsage,
};

/// Embacle-based LLM provider facade wrapping CLI subprocess and SDK runners
mod cli_llm_provider;
/// LLM configuration types (provider selection, model settings)
pub mod config;
/// Google Gemini LLM provider implementation
mod gemini;
/// Groq LLM provider implementation
mod groq;
/// Generic OpenAI-compatible LLM provider
mod openai_compatible;
/// Model pricing registry for cost tracking
pub mod pricing;
/// System prompts for LLM interactions
pub mod prompts;
/// Unified LLM provider selector
mod provider;
/// Shared SSE parser for streaming responses
pub mod sse_parser;

pub use cli_llm_provider::{CliLlmProvider, ProviderReadiness};
pub use embacle::{AgentExecutor, AgentResult, FallbackProvider, MetricsProvider};
pub use embacle::{
    ClaudeCodeRunner, CliRunnerType, ClineCliRunner, CodexCliRunner, ContinueCliRunner,
    CopilotRunner, CursorAgentRunner, GeminiCliRunner, GooseCliRunner, OpenCodeRunner,
    WarpCliRunner,
};
pub use embacle::{CopilotHeadlessConfig, CopilotHeadlessRunner, HeadlessToolResponse};
pub use embacle::{McpToolDefinition, McpToolExecutor, QualityGateProvider};
pub use gemini::{
    ChatResponseWithTools, FunctionCall, FunctionDeclaration, FunctionResponse, GeminiProvider,
    Tool,
};
pub use groq::GroqProvider;
pub use openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleProvider};
pub use prompts::{
    get_coach_generation_prompt, get_insight_generation_prompt, get_insight_validation_prompt,
    get_messaging_context_prompt, get_pierre_system_prompt,
};
pub use provider::ChatProvider;

/// Shared HTTP client configuration for all cloud LLM providers
mod http_client {
    use std::env;
    use std::time::Duration;

    /// Default connect timeout for LLM API calls (30 seconds)
    const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 30;

    /// Default request timeout for LLM API calls (5 minutes, since completions can be slow)
    const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 300;

    /// Build a `reqwest::Client` with consistent timeout configuration
    ///
    /// Reads from environment variables with sensible defaults:
    /// - `LLM_CONNECT_TIMEOUT_SECS`: TCP connect timeout (default: 30s)
    /// - `LLM_REQUEST_TIMEOUT_SECS`: Total request timeout (default: 300s)
    #[must_use]
    pub fn build_llm_http_client() -> reqwest::Client {
        let connect_secs = env::var("LLM_CONNECT_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS);
        let request_secs = env::var("LLM_REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS);

        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(connect_secs))
            .timeout(Duration::from_secs(request_secs))
            .build()
            .unwrap_or_default()
    }
}

pub(crate) use http_client::build_llm_http_client;
