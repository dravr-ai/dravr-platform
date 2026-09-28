// ABOUTME: The OAuth 2.0 discovery documents — authorization server metadata (RFC 8414) and protected resource metadata (RFC 9728)
// ABOUTME: Names the issuer's endpoints, and the MCP resource identifier of the origin the client dialed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::{extract::State, http::HeaderMap, Json};
use pierre_auth::config::oauth::DIALED_HOST_HEADERS;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use tokio::task::spawn_blocking;

use super::{OAuth2Context, OAuth2Routes};

impl OAuth2Routes {
    /// Handle OAuth 2.0 discovery (RFC 8414)
    pub(super) async fn handle_discovery(
        State(context): State<OAuth2Context>,
    ) -> Json<serde_json::Value> {
        let issuer_url = context.config.issuer_url.clone();

        // Use spawn_blocking for JSON serialization (CPU-bound operation)
        let discovery_json = spawn_blocking(move || {
            serde_json::json!({
                "issuer": issuer_url,
                "authorization_endpoint": format!("{issuer_url}/oauth2/authorize"),
                "token_endpoint": format!("{issuer_url}/oauth2/token"),
                "registration_endpoint": format!("{issuer_url}/oauth2/register"),
                "jwks_uri": format!("{issuer_url}/.well-known/jwks.json"),
                "grant_types_supported": ["authorization_code", "client_credentials", "refresh_token"],
                "response_types_supported": ["code"],
                "token_endpoint_auth_methods_supported": ["client_secret_post"],
                "scopes_supported": OAuth2AuthorizationServer::supported_scopes(),
                "response_modes_supported": ["query"],
                "code_challenge_methods_supported": ["S256"]
            })
        })
        .await
        .unwrap_or_else(|_| {
            serde_json::json!({
                "error": "internal_error",
                "error_description": "Failed to generate discovery document"
            })
        });

        Json(discovery_json)
    }

    /// Handle OAuth 2.0 Protected Resource Metadata discovery (RFC 9728).
    ///
    /// The MCP authorization spec requires a protected MCP server to act as an OAuth 2.1 resource
    /// server publishing this document, so a client can find the authorization server after a 401;
    /// `/mcp` 401s point here. `resource` is the MCP resource identifier of the origin the client
    /// dialed (`MCP_RESOURCE_URL` or an alias, see `OAuth2ServerConfig::mcp_resource_for_host`),
    /// which RFC 9728 §3.3 has it compare against the URL it fetched this document from — and
    /// `authorization_servers` names the issuer, which may be another host. The same identifier
    /// is a `resource` parameter the authorize and token endpoints accept (RFC 8707), and the
    /// audience of the tokens they bind to it.
    pub(super) async fn handle_protected_resource_metadata(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
    ) -> Json<serde_json::Value> {
        let issuer_url = context.config.issuer_url.clone();
        let dialed = DIALED_HOST_HEADERS
            .iter()
            .find_map(|name| headers.get(*name).and_then(|value| value.to_str().ok()));
        let resource_url = context.config.mcp_resource_for_host(dialed).to_owned();

        // Use spawn_blocking for JSON serialization (CPU-bound operation)
        let metadata_json = spawn_blocking(move || {
            serde_json::json!({
                "resource": resource_url,
                "authorization_servers": [issuer_url],
                "jwks_uri": format!("{issuer_url}/.well-known/jwks.json"),
                "scopes_supported": OAuth2AuthorizationServer::supported_scopes(),
                "bearer_methods_supported": ["header"]
            })
        })
        .await
        .unwrap_or_else(|_| {
            serde_json::json!({
                "error": "internal_error",
                "error_description": "Failed to generate protected resource metadata"
            })
        });

        Json(metadata_json)
    }
}
