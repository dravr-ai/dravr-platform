// ABOUTME: MCP (Model Context Protocol) route handlers for AI assistant integration
// ABOUTME: Mounts the shared dravr-tronc MCP engine at POST /mcp and serves GET /mcp/tools discovery
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! MCP protocol routes for AI assistant integration.
//!
//! The `/mcp` JSON-RPC endpoint is served by the generic
//! [`dravr_tronc::mcp`] engine: [`build_mcp_server`] wires the platform's three
//! host seams (auth, tool dispatch, non-tool methods) and
//! [`dravr_tronc::mcp::transport::http::mcp_router`] handles request parsing,
//! the RFC 9728 auth posture (401/403), and JSON/SSE rendering. The standalone
//! `GET /mcp/tools` discovery endpoint stays here for unauthenticated registry
//! enumeration.

use axum::{extract::State, routing::get, Json, Router};
use dravr_tronc::mcp::transport::http::mcp_router;
use serde_json::Value;
use std::sync::Arc;
use tokio::task::yield_now;

use crate::mcp::{host_seams::build_mcp_server, resources::ServerContext};

/// MCP routes state for the discovery endpoint.
#[derive(Clone)]
pub struct McpRoutesState {
    resources: Arc<ServerContext>,
}

/// MCP routes implementation
pub struct McpRoutes;

impl McpRoutes {
    /// Create all MCP routes with server resources.
    ///
    /// Mounts `GET /mcp/tools` (registry discovery) plus the tronc-backed
    /// `POST /mcp` JSON-RPC engine.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        let server = build_mcp_server(resources.clone());
        let state = McpRoutesState { resources };

        Router::new()
            .route("/mcp/tools", get(Self::handle_tools))
            .with_state(state)
            .merge(mcp_router(server))
    }

    /// Handle MCP tools discovery.
    ///
    /// Returns all available MCP tools for client discovery via `ToolRegistry`.
    /// This endpoint allows MCP clients to enumerate available tools before
    /// making tool call requests.
    async fn handle_tools(State(state): State<McpRoutesState>) -> Json<Value> {
        // Yield to scheduler for cooperative multitasking
        yield_now().await;

        let tools = state.resources.mcp.tool_registry.all_schemas();
        Json(serde_json::json!({ "tools": tools }))
    }
}
