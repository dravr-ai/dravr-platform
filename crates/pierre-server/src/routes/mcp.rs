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
//! the RFC 9728 auth posture (401/403), and JSON/SSE rendering.
//!
//! `GET /mcp/tools` is the REST-shaped twin of JSON-RPC `tools/list`: it runs
//! the *same* [`PierreAuthHook`] and [`PierreToolDispatcher`] seams the engine
//! uses, so a caller sees exactly the tools `tools/list` would return for the
//! same credentials — anonymous callers see none. It exists because SDK type
//! generation wants the catalog over plain HTTP without speaking JSON-RPC.

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use dravr_tronc::mcp::auth::{AuthError, AuthHook};
use dravr_tronc::mcp::host::ToolDispatcher;
use dravr_tronc::mcp::protocol::JsonRpcRequest;
use dravr_tronc::mcp::transport::http::mcp_router;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use std::sync::Arc;
use tokio::task::yield_now;

use crate::mcp::{
    host_seams::{build_mcp_server, PierreAuthHook, PierreToolDispatcher},
    resources::ServerContext,
};

/// MCP routes state for the discovery endpoint.
#[derive(Clone)]
pub struct McpRoutesState {
    resources: Arc<ServerContext>,
}

/// Extract the bearer token from an `Authorization` header.
///
/// Mirrors the tronc HTTP transport's own extraction (which is private to the
/// engine) so `GET /mcp/tools` accepts exactly the credential forms `POST /mcp`
/// accepts — the token is then handed to the same [`PierreAuthHook`].
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(ToOwned::to_owned)
}

/// MCP routes implementation
pub struct McpRoutes;

impl McpRoutes {
    /// Create all MCP routes with server resources.
    ///
    /// Mounts `GET /mcp/tools` (authenticated registry discovery) plus the
    /// tronc-backed `POST /mcp` JSON-RPC engine.
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
    /// Authenticates the caller through [`PierreAuthHook`] and lists tools
    /// through [`PierreToolDispatcher`], so the returned set is identical to
    /// what JSON-RPC `tools/list` returns for the same bearer: global admins get
    /// the whole registry, tenant members get their tenant-filtered set minus
    /// `ADMIN_ONLY` tools, and callers with no valid bearer get a 401 carrying
    /// the RFC 9728 `WWW-Authenticate` challenge instead of the catalog.
    async fn handle_tools(State(state): State<McpRoutesState>, headers: HeaderMap) -> Response {
        // Yield to scheduler for cooperative multitasking
        yield_now().await;

        let mut request = JsonRpcRequest::new("tools/list", None);
        request.auth_token = bearer_token(&headers);

        let runtime: Arc<dyn ToolRuntime> = state.resources.clone();
        let auth_hook = PierreAuthHook {
            resources: state.resources.clone(),
        };

        let ctx = match auth_hook.authenticate(&request, &runtime).await {
            Ok(ctx) => ctx,
            Err(AuthError::Unauthorized { www_authenticate }) => {
                return (
                    StatusCode::UNAUTHORIZED,
                    [(header::WWW_AUTHENTICATE, www_authenticate)],
                    Json(json!({ "error": "unauthorized" })),
                )
                    .into_response();
            }
            Err(AuthError::Forbidden { reason }) => {
                return (StatusCode::FORBIDDEN, Json(json!({ "error": reason }))).into_response();
            }
        };

        let dispatcher = PierreToolDispatcher {
            resources: state.resources,
        };
        let tools = dispatcher.list_tools(&runtime, &ctx).await;

        Json(json!({ "tools": tools })).into_response()
    }
}
