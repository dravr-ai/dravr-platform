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
    LlmProviderRegistry, McpHeader, McpServerConfig, McpTransport, MessageRole, StreamChunk,
    TokenUsage,
};

/// Process-wide guard state for ChatProvider::Chain preemptive fallback
/// (GitHub rate-limit headroom + circuit breaker on primary).
pub mod chain_guard;
/// Embacle-based LLM provider facade wrapping CLI subprocess and SDK runners
mod cli_llm_provider;
/// Cohere LLM provider implementation (Command A and Command R family)
mod cohere;
mod cohere_errors;
/// LLM configuration types (provider selection, model settings)
pub mod config;
mod fallback_policy;
/// Google Gemini LLM provider implementation
mod gemini;
/// Groq LLM provider implementation
mod groq;
/// LLM startup probe state shared with pierre-server's /ready and /health/llm routes
pub mod health;
/// Generic LLM-as-judge helpers for structured JSON verdicts
pub mod judge;
/// Generic OpenAI-compatible LLM provider
mod openai_compatible;
/// `OpenRouter` LLM provider — unified gateway to 200+ models
mod openrouter;
/// Model pricing registry for cost tracking
pub mod pricing;
/// System prompts for LLM interactions
pub mod prompts;
mod provider;
/// Shared SSE parser for streaming responses
pub mod sse_parser;
/// Unified LLM provider selector
mod tool_bridge;

pub use cli_llm_provider::{CliLlmProvider, ProviderReadiness};
pub use cohere::CohereProvider;
pub use embacle::{AgentExecutor, AgentResult, FallbackProvider, MetricsProvider};
pub use embacle::{
    ClaudeCodeRunner, CliRunnerType, ClineCliRunner, CodexCliRunner, ContinueCliRunner,
    CopilotRunner, CursorAgentRunner, GeminiCliRunner, GooseCliRunner, OpenCodeRunner,
    WarpCliRunner,
};
pub use embacle::{
    CopilotHeadlessConfig, CopilotHeadlessRunner, HeadlessEventStream, HeadlessStreamEvent,
    HeadlessToolResponse, ObservedToolCall,
};
pub use embacle::{
    McpToolDefinition, McpToolExecutor, OpenAiApiConfig, OpenAiApiRunner, QualityGateProvider,
};
pub use fallback_policy::{is_empty_completion, is_retryable_for_fallback};
pub use gemini::{
    ChatResponseWithTools, FunctionCall, FunctionDeclaration, FunctionResponse, GeminiProvider,
    Tool,
};
pub use groq::GroqProvider;
pub use openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleProvider};
pub use openrouter::OpenRouterProvider;
pub use prompts::{
    get_activity_analysis_prompt, get_activity_analysis_system_prompt, get_coach_generation_prompt,
    get_messaging_context_prompt, get_pierre_system_prompt, get_recommendation_analysis_prompt,
    get_recommendation_system_prompt,
};
pub use provider::ChatProvider;

use pierre_core::http_client::{llm_client, SharedHttpClient};

/// Returns the shared LLM HTTP client from pierre-core (300s timeout, connection pooled)
pub(crate) fn build_llm_http_client() -> &'static SharedHttpClient {
    llm_client()
}
