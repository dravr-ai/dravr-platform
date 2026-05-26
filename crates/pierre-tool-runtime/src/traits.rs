// ABOUTME: Defines the McpTool trait and ToolCapabilities for the pluggable tools architecture.
// ABOUTME: Tools implement this trait to be registered and executed via the ToolRegistry.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # MCP Tool Trait and Capabilities
//!
//! This module defines the core abstraction for MCP tools. All tools implement
//! the `McpTool` trait which provides:
//! - Tool metadata (name, description, input schema)
//! - Capability flags for filtering and validation
//! - Async execution with context
//!
//! The design mirrors the `FitnessProvider` trait pattern from `src/providers/core.rs`
//! to maintain consistency across the codebase.

use async_trait::async_trait;
use serde_json::Value;

use pierre_core::errors::AppResult;
use pierre_mcp_schema::{JsonSchema, ToolAnnotations};

pub use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use pierre_tools_core::ToolResult;

/// The main trait that all MCP tools must implement.
///
/// This trait provides a consistent interface for tool discovery, validation,
/// and execution. Tools are registered with the `ToolRegistry` and can be
/// discovered via capability filtering.
///
/// # Design Notes
///
/// - Tools are `Send + Sync` for safe sharing across async tasks
/// - `name()` returns `&'static str` for zero-allocation tool lookup
/// - `capabilities()` enables efficient bitflag-based filtering
/// - `execute()` is async for I/O-bound operations
///
/// # Example
///
/// ```text
/// use async_trait::async_trait;
/// use pierre_tool_runtime::{McpTool, ToolCapabilities, ToolExecutionContext};
/// use pierre_tools_core::ToolResult;
/// use pierre_mcp_schema::JsonSchema;
/// use pierre_core::errors::AppResult;
/// use serde_json::Value;
///
/// struct GetActivitiesTool;
///
/// #[async_trait]
/// impl McpTool for GetActivitiesTool {
///     fn name(&self) -> &'static str { "get_activities" }
///     fn description(&self) -> &'static str { "Retrieve activities" }
///     fn input_schema(&self) -> JsonSchema { /* ... */ }
///     fn capabilities(&self) -> ToolCapabilities {
///         ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
///     }
///     async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
///         Ok(ToolResult::ok(serde_json::json!({"activities": []})))
///     }
/// }
/// ```
///
/// For complete working examples, see the tools in `src/tools/implementations/`.
#[async_trait]
pub trait McpTool: Send + Sync {
    /// Unique identifier for the tool (e.g., `get_activities`)
    ///
    /// This name is used for:
    /// - Tool lookup in the registry
    /// - MCP protocol tool calls
    /// - Logging and debugging
    fn name(&self) -> &'static str;

    /// Human-readable description for LLM consumption
    ///
    /// This should describe what the tool does in a way that helps
    /// LLMs understand when to use it.
    fn description(&self) -> &'static str;

    /// JSON Schema for input parameters
    ///
    /// This schema is returned in tools/list responses and used
    /// by clients to validate tool arguments.
    fn input_schema(&self) -> JsonSchema;

    /// Capability flags for filtering and validation
    ///
    /// These flags are used for:
    /// - Admin vs user tool filtering
    /// - Provider availability checks
    /// - Caching decisions
    fn capabilities(&self) -> ToolCapabilities;

    /// Behavioral annotations for MCP tool discovery (MCP 2025-11-25)
    ///
    /// Returns hints about tool behavior such as read-only, destructive,
    /// idempotent, or open-world characteristics. Used in `tools/list` responses
    /// to help clients make better UX decisions.
    ///
    /// Default implementation returns `None` (no annotations).
    fn annotations(&self) -> Option<ToolAnnotations> {
        None
    }

    /// Execute the tool with given arguments and context
    ///
    /// # Arguments
    ///
    /// * `args` - Tool arguments as JSON value
    /// * `context` - Execution context with user/tenant info and resources
    ///
    /// # Returns
    ///
    /// `ToolResult` containing the response content and optional notifications
    ///
    /// # Errors
    ///
    /// Returns `AppError` for validation failures, auth issues, or execution errors
    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult>;
}
