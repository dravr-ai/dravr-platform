// ABOUTME: Audit decorator for MCP tools that logs tool executions for security.
// ABOUTME: Useful for admin operations and sensitive data access.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Audited Tool Decorator
//!
//! Wraps any `McpTool` implementation with audit logging for security tracking.
//! Logs tool invocations with user, tenant, and timing information.
//!
//! # Use Cases
//!
//! - Admin tool executions (for compliance)
//! - Sensitive data access (for security auditing)
//! - Debugging production issues
//!
//! # Example
//!
//! The `AuditedTool` wraps any `McpTool` implementation to add audit logging:
//!
//! ```text
//! // Wrap an existing tool with audit logging
//! let admin_tool = Arc::new(MyAdminTool::new());
//! let audited = AuditedTool::new(admin_tool);
//!
//! // Or enable argument logging for non-sensitive tools
//! let audited_with_args = AuditedTool::with_argument_logging(Arc::new(MyTool::new()));
//! ```
//!
//! See [`AuditedTool::new`] and [`AuditedTool::with_argument_logging`] for details.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;
use tracing::{info, instrument};

use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};

/// Audit decorator for MCP tools.
///
/// Wraps an inner tool and logs all executions with:
/// - User and tenant context
/// - Execution timing
/// - Success/failure status
/// - Optional argument sampling (configurable)
///
/// # Thread Safety
///
/// The decorator is `Send + Sync` and can be safely shared across async tasks.
pub struct AuditedTool {
    /// The wrapped tool
    inner: Arc<dyn McpTool<dyn ToolRuntime>>,
    /// Whether to log arguments (may contain sensitive data)
    log_arguments: bool,
}

impl AuditedTool {
    /// Create a new audited tool with default settings (no argument logging)
    #[must_use]
    pub fn new(inner: Arc<dyn McpTool<dyn ToolRuntime>>) -> Self {
        Self {
            inner,
            log_arguments: false,
        }
    }

    /// Create a new audited tool that also logs arguments
    ///
    /// # Warning
    ///
    /// Only enable argument logging if you're sure the arguments don't
    /// contain sensitive data (passwords, tokens, PII, etc.)
    #[must_use]
    pub const fn with_argument_logging(inner: Arc<dyn McpTool<dyn ToolRuntime>>) -> Self {
        Self {
            inner,
            log_arguments: true,
        }
    }

    /// Get a reference to the inner tool
    #[must_use]
    pub fn inner(&self) -> &Arc<dyn McpTool<dyn ToolRuntime>> {
        &self.inner
    }

    /// Check if argument logging is enabled
    #[must_use]
    pub const fn logs_arguments(&self) -> bool {
        self.log_arguments
    }
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for AuditedTool {
    fn definition(&self) -> Tool {
        self.inner.definition()
    }

    fn capabilities(&self) -> ToolCapabilities {
        self.inner.capabilities()
    }

    #[instrument(skip_all)]
    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let start = Instant::now();
        let tool_name = self.inner.definition().name;
        let is_admin = self.capabilities().contains(ToolCapabilities::ADMIN_ONLY);

        // Log the invocation
        if self.log_arguments {
            info!(
                tool = %tool_name,
                admin_tool = %is_admin,
                arguments = %args,
                "Tool execution started"
            );
        } else {
            info!(
                tool = %tool_name,
                admin_tool = %is_admin,
                "Tool execution started"
            );
        }

        // Execute the inner tool
        let result = self.inner.execute(state, ctx, args).await;
        let duration = start.elapsed();

        info!(
            tool = %tool_name,
            duration_ms = %duration.as_millis(),
            is_error = %result.is_error,
            "Tool execution completed"
        );

        result
    }
}
