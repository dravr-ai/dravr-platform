// ABOUTME: Unified tool execution engine providing fitness analysis and data processing tools
// ABOUTME: Central tool registry for MCP protocol tools, A2A tools, and fitness intelligence operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Unified Tool Execution Engine
//!
//! This module provides a shared tool execution engine that can be used
//! by both single-tenant and multi-tenant MCP implementations, eliminating
//! code duplication and providing a single source of truth for tool logic.
//!
//! ## Architecture
//!
//! The scaffolding (trait surface, registry, decorators, runtime façade)
//! lives in the `pierre-tool-runtime` crate and is re-exported here so the
//! `crate::tools::*` paths stay stable for callers inside pierre-server.
//!
//! - **Core Types** (`traits`, `context`, plus `pierre_tools_core` for I/O values)
//!   - `McpTool` trait — the main interface for all tools
//!   - `ToolCapabilities` — bitflags for capability-based filtering
//!   - `ToolExecutionContext` — context for tool execution
//!   - `pierre_tools_core::ToolResult` — structured result with notifications
//!
//! - **Registry** (`registry`)
//!   - Central tool registration and lookup (in `pierre-tool-runtime`)
//!   - Built-in wiring of `implementations::*` lives in
//!     [`registry_builtin`] (pierre-server-side, because it imports
//!     `super::implementations::*`).
//!
//! - **Decorators** (`decorators`)
//!   - `AuditedTool` — audit logging for admin operations and security tracking
//!
//! - **Implementations** (`implementations`)
//!   - Tool implementations organized by category
//!   - Each category behind a feature flag
//!
//! ## Feature Flags
//!
//! Tool categories can be enabled/disabled via Cargo features:
//!
//! - `tools-connection` - Provider connection management
//! - `tools-data` - Data access tools
//! - `tools-analytics` - Analytics and metrics tools
//! - `tools-goals` - Goal management tools
//! - `tools-config` - Configuration tools
//! - `tools-nutrition` - Nutrition tools
//! - `tools-sleep` - Sleep/recovery tools
//! - `tools-recipes` - Recipe management tools
//! - `tools-coaches` - AI coach tools
//! - `tools-admin` - Admin-only tools
//!
//! ## Example
//!
//! ```text
//! use pierre_mcp_server::tools::{registry::ToolRegistry, registry_builtin};
//!
//! let mut registry = ToolRegistry::new();
//! registry_builtin::register_builtin_tools(&mut registry);
//!
//! // List user-visible tools
//! let is_admin = false;
//! let schemas = registry.list_schemas_for_role(is_admin);
//! ```

// ============================================================================
// pierre-server-side modules — depend on `implementations::*` and stay local
// ============================================================================

/// Wires built-in tools (in [`implementations`]) into a
/// [`registry::ToolRegistry`]. Free functions that mirror the original
/// inherent methods on `ToolRegistry`.
pub mod registry_builtin;

/// Upcast `Arc<ServerContext>` to `Arc<dyn ToolRuntime>`.
pub mod runtime_adapter;

/// Tool implementations organized by category
pub mod implementations;
