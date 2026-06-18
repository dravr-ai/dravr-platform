// ABOUTME: Re-exports the canonical MCP wire types from dravr-tronc for this server
// ABOUTME: Retains the fitness-domain json_schemas parameter/response types specific to Pierre
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre MCP Schema
//!
//! The generic MCP wire types (initialize, tools, capabilities, content,
//! sampling, completion, notifications, roots, …) now live in the shared engine
//! [`dravr_tronc::mcp`]. This crate re-exports them so existing
//! `pierre_mcp_schema::X` consumers resolve to the canonical tronc types, and
//! retains the fitness-domain [`json_schemas`] parameter/response types that are
//! specific to this server.

/// Type-safe JSON schema definitions for API request/response parameters.
/// Replaces dynamic `serde_json::Value` usage with compile-time validated structs.
pub mod json_schemas;

/// Canonical MCP wire types — `McpRequest`/`McpResponse`/`McpError`, `Tool`,
/// `ToolSchema`, `Content`, `ServerCapabilities`, `InitializeRequest`/`Response`,
/// sampling, completion, notifications, `Root`, … — re-exported from the shared
/// `dravr-tronc` engine so this crate carries no wire-type definitions of its own.
pub use dravr_tronc::mcp::schema::*;
