// ABOUTME: Model Context Protocol (MCP) implementation for AI assistant integration
// ABOUTME: Multi-tenant MCP server functionality for MCP clients and AI assistants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// MCP request processing and routing
pub mod mcp_request_processor;
/// Multi-tenant MCP server implementation
pub mod multitenant;
/// MCP protocol types and message handling
pub mod protocol;
/// Resource management for MCP
pub mod resources;
/// Server lifecycle management
pub mod server_lifecycle;
/// MCP tool handler implementations
pub mod tool_handlers;
/// Server-startup catalog sync — bridges the live `ToolRegistry` to the `tool_catalog` table.
///
/// The per-tenant filtering service (`ToolSelectionService`) lives in
/// `pierre_tool_runtime::tool_selection`; only the registry-coupled startup
/// helpers remain here.
pub mod tool_selection;
/// Transport layer abstraction
pub mod transport_manager;

// Transport primitives moved to the `pierre-mcp-transport` leaf crate.
// Callers now use the canonical path `pierre_mcp_transport::{progress,
// sampling_peer, tenant_isolation, oauth_flow_manager}` directly per the
// facade-shim deletion plan (gist item #12).
