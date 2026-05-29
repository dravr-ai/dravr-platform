// ABOUTME: Tool execution scaffolding — McpTool trait, ToolExecutionContext, ToolRuntime, ToolRegistry
// ABOUTME: Batch B extraction; tool implementations + protocol envelopes stay in pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Tool Runtime
//!
//! Scaffolding crate for tool execution infrastructure shared across the
//! Pierre platform.
//!
//! ## What lives here
//!
//! - [`ToolCapabilities`] — bitflags describing what a tool requires
//!   (`REQUIRES_AUTH`, `REQUIRES_PROVIDER`, `ADMIN_ONLY`, …) and helpers
//!   for capability-based filtering.
//! - [`ToolSelectionService`] — per-tenant tool filtering that combines
//!   global env-driven disabling, plan restrictions, and admin-configured
//!   per-tenant overrides into a single effective tool list with LRU caching.
//! - [`McpTool`] trait — the canonical interface every tool implements.
//! - [`ToolExecutionContext`] — request-scoped context handed to each tool
//!   (user, tenant, narrow runtime façade, auth method).
//! - [`runtime::ToolRuntime`] — narrow service façade the context exposes;
//!   pierre-server's `ServerContext` implements it.
//! - [`registry::ToolRegistry`] — central registry with capability-based
//!   filtering, schema generation, and execution dispatch.
//! - [`decorators::AuditedTool`] — wrapping decorator that logs every tool
//!   invocation for audit/security tracking.
//!
//! ## What still lives in pierre-server
//!
//! - `tools::implementations::*` — most concrete tools (~17k LOC). The
//!   clean-leaf categories `admin`, `coaches`, `mobility` have moved into
//!   [`implementations`] here; pierre-server re-exports them via a shim so
//!   `crate::tools::implementations::{admin,coaches,mobility}` paths stay
//!   stable for the registry-builtin wiring and tests.
//! - `tools::protocol::*` — JSON-RPC envelope, executor, auth, format helpers.
//! - `tools::engine.rs` / `tools::providers.rs` — pierre-server-internal
//!   plumbing tied to the HTTP transport.
//! - `tools::registry_builtin::register_builtin_tools` — wires the
//!   `super::implementations::*` modules into a [`registry::ToolRegistry`]
//!   instance at startup.
//! - `mcp::resources::ServerContext` — the canonical DI container that
//!   implements [`runtime::ToolRuntime`].
//!
//! ## What's *not* here
//!
//! Tool *I/O value types* (`ToolResult`, `ToolNotification`, `NotificationType`)
//! live in [`pierre_tools_core`]; the two crates compose to form the complete
//! pluggable surface.

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Shared provider activity fetching (used by group snapshots + coach recs).
pub mod activity_fetch;
pub mod capabilities;
pub mod context;
pub mod decorators;
/// Unified tool execution engine
pub mod engine;
/// Batch fitness snapshot fetcher for group coaching context
#[cfg(feature = "tools-groups")]
pub mod group_fitness;
pub mod implementations;
/// Universal protocol envelope (types, executor, auth, format, provider helpers, handlers)
pub mod protocol;
/// Protocol conversion + universal request/response re-exports
pub mod protocols;
pub mod registry;
pub mod runtime;
/// Tool execution strategies for multi-turn LLM chat (API, headless, CLI modes).
///
/// Three strategies share the same MCP executor infrastructure:
/// - `run_api_tool_loop`: native function calling (Gemini, Groq)
/// - `run_headless_tool_loop`: ACP-managed tool calling (Copilot Headless)
/// - `run_cli_tool_loop`: text-based `<tool_call>` blocks (CLI providers)
#[cfg(feature = "client-chat")]
pub mod tool_execution;
pub mod tool_selection;
pub mod traits;

pub use capabilities::ToolCapabilities;
pub use context::{AuthMethod, ToolExecutionContext};
pub use decorators::AuditedTool;
pub use registry::ToolRegistry;
pub use runtime::ToolRuntime;
pub use tool_selection::ToolSelectionService;
pub use traits::McpTool;
