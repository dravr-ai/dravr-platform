// ABOUTME: LLM provider abstraction layer for pluggable AI model integration
// ABOUTME: Selects and chains embacle runners behind the platform LlmProvider trait, with tool-calling shapes
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

/// Process-wide guard state for the runtime fallback chain
/// (GitHub rate-limit headroom + circuit breaker on the primary).
pub mod chain_guard;
/// The FallbackObserver that drives the chain guard and emits the notify events
mod chain_observer;
/// LLM configuration types (provider selection, model settings)
pub mod config;
/// The one facade over every embacle runner, chain included
mod embacle_provider;
/// LLM startup probe state shared with pierre-server's /ready and /health/llm routes
pub mod health;
/// Construction of embacle's HTTP providers from env or tenant credentials
pub mod http_env;
/// Generic LLM-as-judge helpers for structured JSON verdicts
pub mod judge;
/// Boot-time check that the active model is one its provider publishes
mod model_check;
/// System prompts for LLM interactions
pub mod prompts;
/// Unified LLM provider selector
mod provider;
/// Which tier of a fallback chain answered the current call
pub mod served_tier;
mod tool_bridge;
/// The platform's tool-calling shapes
mod tool_types;

pub use embacle::OpenAiCompatibleConfig;
pub use embacle::{AgentExecutor, AgentResult, FallbackProvider, MetricsProvider};
pub use embacle::{
    ClaudeCodeRunner, CliRunnerType, ClineCliRunner, CodexCliRunner, ContinueCliRunner,
    CopilotRunner, CursorAgentRunner, GeminiCliRunner, GooseCliRunner, OpenCodeRunner,
    WarpCliRunner,
};
pub use embacle::{
    CopilotHeadlessConfig, CopilotHeadlessRunner, CopilotSdkConfig, CopilotSdkRunner,
    HeadlessEventStream, HeadlessStreamEvent, HeadlessToolResponse, HeadlessTurnProvider,
    ObservedToolCall,
};
pub use embacle::{
    McpToolDefinition, McpToolExecutor, OpenAiApiConfig, OpenAiApiRunner, QualityGateProvider,
};
pub use embacle_provider::{cli_credential_env_keys, cli_runner_config, EmbacleProvider};
pub use prompts::{
    get_activity_analysis_prompt, get_activity_analysis_system_prompt, get_agent_generation_prompt,
    get_messaging_context_prompt, get_pierre_system_prompt, get_recommendation_analysis_prompt,
    get_recommendation_system_prompt,
};
pub use provider::{ChainTiers, ChatProvider};
pub use tool_types::{
    ChatResponseWithTools, FunctionCall, FunctionDeclaration, FunctionResponse, Tool,
};
