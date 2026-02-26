// ABOUTME: GitHub Copilot SDK-based LLM provider for the Pierre platform.
// ABOUTME: Wraps copilot-sdk crate to communicate with copilot --headless via JSON-RPC.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! GitHub Copilot SDK-based LLM provider for the Pierre platform.
//!
//! Wraps the community `copilot-sdk` crate to communicate with `copilot --headless`
//! via JSON-RPC (stdio or TCP). Provides native tool calling support through
//! the SDK's session and tool handler infrastructure.

/// Configuration for the Copilot SDK provider.
pub mod config;
/// LLM provider implementation wrapping the Copilot SDK client.
pub mod provider;
/// Tool definition conversion between Pierre and Copilot SDK formats.
pub mod tool_bridge;

pub use config::CopilotSdkConfig;
pub use provider::CopilotSdkProvider;
