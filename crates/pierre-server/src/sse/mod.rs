// ABOUTME: Server-Sent Events (SSE) protocol stream that drives tools/call dispatch
// ABOUTME: Sibling modules (manager, routes, notifications, a2a_task_stream) live in pierre-sse
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # SSE Module (pierre-server local pieces)
//!
//! Most of the SSE implementation lives in the [`pierre_sse`] crate. This
//! module houses the one piece that did not extract: [`protocol`], the
//! [`protocol::McpProtocolStream`] implementation that drives the
//! `tools/call` dispatcher (it depends on
//! `crate::mcp::tool_handlers::ToolHandlers`, which is still entangled
//! with `ServerContext` and not yet extracted).

/// MCP protocol streaming for bidirectional client-server communication.
///
/// Stays in `pierre-server` because [`crate::mcp::tool_handlers::ToolHandlers`]
/// has not been extracted yet; the surrounding broadcast manager lives in
/// `pierre-sse` and dispatches into this implementation via the
/// [`pierre_sse::manager::ProtocolStream`] trait.
#[cfg(feature = "protocol-mcp")]
pub mod protocol;
