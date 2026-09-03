// ABOUTME: Model Context Protocol (MCP) implementation for AI assistant integration
// ABOUTME: Multi-tenant MCP server functionality for MCP clients and AI assistants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Platform host seams (auth/dispatch/method) wiring onto the tronc MCP engine
pub mod audit;
pub mod host_seams;
/// Tenant-aware tool helpers + Axum server orchestration
pub mod multitenant;
/// Curated, user-invokable analysis prompt templates (prompts/list + prompts/get)
pub mod prompt_templates;
/// Coach marketplace catalog backing MCP resources/list + resources/read
pub mod resource_catalog;
/// Resource management for MCP
pub mod resources;
/// Durable owner-scoped store for MCP Tasks extension handles
pub mod task_store;
/// MCP tool handler implementations
pub mod tool_handlers;
/// Server-startup catalog sync — bridges the live `ToolRegistry` to the `tool_catalog` table.
///
/// The per-tenant filtering service (`ToolSelectionService`) lives in
/// `pierre_tool_runtime::tool_selection`; only the registry-coupled startup
/// helpers remain here.
pub mod tool_selection;

// Transport primitives moved to the `pierre-mcp-transport` leaf crate.
// Callers now use the canonical path `pierre_mcp_transport::{progress,
// sampling_peer, tenant_isolation, oauth_flow_manager}` directly per the
// facade-shim deletion plan (gist item #12).
