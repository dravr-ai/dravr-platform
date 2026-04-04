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
use bitflags::bitflags;
use serde_json::Value;

use crate::errors::AppResult;
use crate::mcp::schema::{JsonSchema, ToolAnnotations};

use super::context::ToolExecutionContext;
use super::result::ToolResult;

bitflags! {
    /// Capabilities that tools can declare for filtering and discovery.
    ///
    /// These flags enable:
    /// - Role-based access control (admin vs user tools)
    /// - Provider dependency checking (tools that need connected providers)
    /// - Feature categorization for plan-based filtering
    /// - Caching decisions based on read/write behavior
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ToolCapabilities: u16 {
        /// Tool requires an authenticated user
        const REQUIRES_AUTH = 0b0000_0000_0001;
        /// Tool requires tenant context
        const REQUIRES_TENANT = 0b0000_0000_0010;
        /// Tool requires a connected fitness provider
        const REQUIRES_PROVIDER = 0b0000_0000_0100;
        /// Tool reads data (activities, stats, etc.)
        const READS_DATA = 0b0000_0000_1000;
        /// Tool writes/modifies data
        const WRITES_DATA = 0b0000_0001_0000;
        /// Tool performs analytics/calculations
        const ANALYTICS = 0b0000_0010_0000;
        /// Tool manages goals
        const GOALS = 0b0000_0100_0000;
        /// Tool manages configuration
        const CONFIGURATION = 0b0000_1000_0000;
        /// Tool manages recipes
        const RECIPES = 0b0001_0000_0000;
        /// Tool manages coaches
        const COACHES = 0b0010_0000_0000;
        /// Tool requires admin privileges
        const ADMIN_ONLY = 0b0100_0000_0000;
        /// Tool handles sleep/recovery data
        const SLEEP_RECOVERY = 0b1000_0000_0000;
    }
}

impl ToolCapabilities {
    /// Check if tool requires any form of authentication
    #[must_use]
    pub const fn requires_auth(self) -> bool {
        self.contains(Self::REQUIRES_AUTH)
    }

    /// Check if tool requires tenant context
    #[must_use]
    pub const fn requires_tenant(self) -> bool {
        self.contains(Self::REQUIRES_TENANT)
    }

    /// Check if tool requires a connected provider
    #[must_use]
    pub const fn requires_provider(self) -> bool {
        self.contains(Self::REQUIRES_PROVIDER)
    }

    /// Check if tool is admin-only
    #[must_use]
    pub const fn is_admin_only(self) -> bool {
        self.contains(Self::ADMIN_ONLY)
    }

    /// Check if tool reads data (useful for caching decisions)
    #[must_use]
    pub const fn reads_data(self) -> bool {
        self.contains(Self::READS_DATA)
    }

    /// Check if tool writes data (useful for cache invalidation)
    #[must_use]
    pub const fn writes_data(self) -> bool {
        self.contains(Self::WRITES_DATA)
    }

    /// Check if tool performs analytics
    #[must_use]
    pub const fn is_analytics(self) -> bool {
        self.contains(Self::ANALYTICS)
    }

    /// Get a description of all enabled capabilities for logging
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();

        if self.contains(Self::REQUIRES_AUTH) {
            parts.push("requires_auth");
        }
        if self.contains(Self::REQUIRES_TENANT) {
            parts.push("requires_tenant");
        }
        if self.contains(Self::REQUIRES_PROVIDER) {
            parts.push("requires_provider");
        }
        if self.contains(Self::READS_DATA) {
            parts.push("reads_data");
        }
        if self.contains(Self::WRITES_DATA) {
            parts.push("writes_data");
        }
        if self.contains(Self::ANALYTICS) {
            parts.push("analytics");
        }
        if self.contains(Self::GOALS) {
            parts.push("goals");
        }
        if self.contains(Self::CONFIGURATION) {
            parts.push("configuration");
        }
        if self.contains(Self::RECIPES) {
            parts.push("recipes");
        }
        if self.contains(Self::COACHES) {
            parts.push("coaches");
        }
        if self.contains(Self::ADMIN_ONLY) {
            parts.push("admin_only");
        }
        if self.contains(Self::SLEEP_RECOVERY) {
            parts.push("sleep_recovery");
        }

        if parts.is_empty() {
            "none".to_owned()
        } else {
            parts.join(", ")
        }
    }
}

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
/// use pierre_mcp_server::tools::{McpTool, ToolCapabilities, ToolResult, ToolExecutionContext};
/// use pierre_mcp_server::mcp::schema::JsonSchema;
/// use pierre_mcp_server::errors::AppResult;
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
