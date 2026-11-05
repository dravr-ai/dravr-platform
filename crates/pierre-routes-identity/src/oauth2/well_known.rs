// ABOUTME: The OAuth 2.0 discovery documents — authorization server metadata (RFC 8414) and protected resource metadata (RFC 9728)
// ABOUTME: Names the issuer's endpoints, and the MCP resource identifier of the origin the client dialed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pierre_auth::config::oauth::DIALED_HOST_HEADERS;
use pierre_core::permissions::scopes::OAuthScope;
use tokio::task::spawn_blocking;

use super::{OAuth2Context, OAuth2Routes};

/// The dialed authority a request carries, per [`DIALED_HOST_HEADERS`].
fn dialed_host(headers: &HeaderMap) -> Option<&str> {
    DIALED_HOST_HEADERS
        .iter()
        .find_map(|name| headers.get(*name).and_then(|value| value.to_str().ok()))
}

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
                "revocation_endpoint": format!("{issuer_url}/oauth2/revoke"),
                "token_endpoint_auth_methods_supported": ["none", "client_secret_basic", "client_secret_post"],
                "scopes_supported": OAuthScope::advertised_as_str(),
                "response_modes_supported": ["query"],
                "code_challenge_methods_supported": ["S256"],
                // RFC 9207: every authorization response carries `iss`.
                "authorization_response_iss_parameter_supported": true,
                // OAuth Client ID Metadata Documents: an https URL `client_id`
                // is fetched and validated instead of registered.
                "client_id_metadata_document_supported": true
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

    /// Handle OAuth 2.0 Protected Resource Metadata discovery (RFC 9728) at the
    /// root well-known path.
    ///
    /// The MCP authorization spec requires a protected MCP server to act as an OAuth 2.1 resource
    /// server publishing this document, so a client can find the authorization server after a 401.
    /// This is the document a client connected before the resource carried a path fetched, so its
    /// `resource` stays the bare origin of the host the client dialed
    /// (`OAuth2ServerConfig::mcp_root_resource_for_host`), an identifier whose tokens are still
    /// accepted. `/mcp` 401s point at the path-suffixed document instead
    /// ([`Self::handle_protected_resource_metadata_at`]). `authorization_servers` names the
    /// issuer, which may be another host. The identifier is a `resource` parameter the authorize
    /// and token endpoints accept (RFC 8707), and the audience of the tokens they bind to it.
    pub(super) async fn handle_protected_resource_metadata(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
    ) -> Json<serde_json::Value> {
        let resource_url = context
            .config
            .mcp_root_resource_for_host(dialed_host(&headers))
            .to_owned();
        Self::protected_resource_document(&context, resource_url).await
    }

    /// Handle the path-suffixed Protected Resource Metadata (RFC 9728 §3.1):
    /// `/.well-known/oauth-protected-resource/mcp` describes the resource
    /// `https://<dialed host>/mcp`, the identifier a client given the URL
    /// `https://<dialed host>/mcp` compares `resource` to. A path this server
    /// publishes no resource under is a 404.
    pub(super) async fn handle_protected_resource_metadata_at(
        State(context): State<OAuth2Context>,
        Path(path): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        let Some(resource_url) = context
            .config
            .mcp_resource_at(dialed_host(&headers), &path)
            .map(str::to_owned)
        else {
            return StatusCode::NOT_FOUND.into_response();
        };
        Self::protected_resource_document(&context, resource_url)
            .await
            .into_response()
    }

    /// The RFC 9728 document of the resource `resource_url`, served by the
    /// authorization server `context` names.
    async fn protected_resource_document(
        context: &OAuth2Context,
        resource_url: String,
    ) -> Json<serde_json::Value> {
        let issuer_url = context.config.issuer_url.clone();

        // Use spawn_blocking for JSON serialization (CPU-bound operation)
        let metadata_json = spawn_blocking(move || {
            serde_json::json!({
                "resource": resource_url,
                "authorization_servers": [issuer_url],
                "jwks_uri": format!("{issuer_url}/.well-known/jwks.json"),
                "scopes_supported": OAuthScope::advertised_as_str(),
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
