// ABOUTME: CLI-based LLM runner implementations for the Pierre platform
// ABOUTME: Re-exports runners for Claude Code, Cursor Agent, and OpenCode CLIs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # CLI LLM Runners
//!
//! Pluggable [`LlmProvider`] implementations that delegate to CLI tools
//! (Claude Code, Cursor Agent, `OpenCode`) via subprocess execution.
//!
//! Each runner wraps a CLI binary, builds prompts from [`ChatMessage`] sequences,
//! parses JSON output, and manages session continuity.
//!
//! ## Modules
//!
//! - [`config`] — Runner types, execution modes, and configuration
//! - [`compat`] — Version compatibility and capability detection
//! - [`container`] — Container-based execution backend
//! - [`discovery`] — Automatic binary detection on the host
//! - [`auth`] — Readiness and authentication checking
//! - [`process`] — Subprocess spawning with timeout and output limits
//! - [`sandbox`] — Environment variable whitelisting and working directory control
//! - [`prompt`] — Prompt building from `ChatMessage` slices
//! - [`claude_code`] — Claude Code CLI runner
//! - [`cursor_agent`] — Cursor Agent CLI runner
//! - [`opencode`] — `OpenCode` CLI runner

/// Auth readiness checking for CLI runners
pub mod auth;
/// Claude Code CLI runner
pub mod claude_code;
/// Version compatibility and capability detection
pub mod compat;
/// Shared configuration types for CLI runners
pub mod config;
/// Container-based execution backend
pub mod container;
/// GitHub Copilot CLI runner
pub mod copilot;
/// Cursor Agent CLI runner
pub mod cursor_agent;
/// Binary auto-detection and discovery
pub mod discovery;
/// `OpenCode` CLI runner
pub mod opencode;
/// Subprocess spawning with safety limits
pub mod process;
/// Prompt construction from `ChatMessage` sequences
pub mod prompt;
/// Environment sandboxing and tool policy
pub mod sandbox;

// Re-export the runner structs for ergonomic access
pub use auth::ProviderReadiness;
pub use claude_code::ClaudeCodeRunner;
pub use compat::CliCapabilities;
pub use config::{CliRunnerType, ExecutionMode, RunnerConfig};
pub use container::{ContainerConfig, ContainerExecutor, NetworkMode};
pub use copilot::CopilotRunner;
pub use cursor_agent::CursorAgentRunner;
pub use discovery::{discover_runner, resolve_binary};
pub use opencode::OpenCodeRunner;
