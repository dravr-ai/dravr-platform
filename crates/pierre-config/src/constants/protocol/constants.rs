// ABOUTME: Server-side runtime-configurable MCP protocol values (server name, MCP version)
// ABOUTME: Pure compile-time protocol constants live in pierre_core::constants::protocol
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Runtime-configurable protocol values for MCP.
//!
//! Pure compile-time constants (`JSONRPC_VERSION`, etc.) live in
//! `pierre_core::constants::protocol::constants` and are re-exported
//! via the parent module's `pub use pierre_core::constants::protocol::*`.

use crate::constants::get_server_config;

/// Get MCP Protocol version from environment or default
#[must_use]
pub fn mcp_protocol_version() -> String {
    get_server_config().map_or_else(
        || "2024-11-05".to_owned(),
        |c| c.mcp.protocol_version.clone(),
    )
}

/// Get server name from environment or default
#[must_use]
pub fn server_name() -> String {
    get_server_config().map_or_else(
        || "pierre-fitness-api".to_owned(),
        |c| c.mcp.server_name.clone(),
    )
}

/// Get server name variant with specific suffix
#[must_use]
pub fn server_name_multitenant() -> String {
    get_server_config().map_or_else(
        || "pierre-fitness-api".to_owned(),
        |c| c.mcp.server_name.clone(),
    )
}
