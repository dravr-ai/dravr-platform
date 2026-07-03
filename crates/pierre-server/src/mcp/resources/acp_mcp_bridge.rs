// ABOUTME: McpBridgeProvider impl — mints per-turn /mcp tokens so Copilot ACP calls Dravr tools natively.
// ABOUTME: Lives in pierre-server where auth signing keys and the user repo are available.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Duration;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_chat_pipeline::McpBridgeProvider;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_llm::{McpHeader, McpServerConfig, McpTransport};
use tracing::warn;
use uuid::Uuid;

/// Bridges Dravr's MCP tools into a Copilot ACP session.
///
/// For each turn it mints a JWT scoped to `(user, tenant)` and hands Copilot an
/// HTTP MCP server config pointing at Dravr's own `/mcp` endpoint, so the model
/// calls Dravr tools natively over the Agent Client Protocol rather than via
/// text-based `<tool_call>` simulation.
pub struct AcpMcpBridge {
    auth_manager: Arc<AuthManager>,
    jwks_manager: Arc<JwksManager>,
    repos: Arc<RepositoryRegistry>,
    /// Fully-qualified URL of Dravr's MCP HTTP endpoint (e.g.
    /// `http://localhost:8081/mcp`). The Copilot subprocess runs in the same
    /// container, so localhost resolves to this server.
    mcp_url: String,
    /// Whether the bridge is active. Gated by the same env flag that makes the
    /// Copilot Headless provider advertise SDK tool calling, so one setting
    /// controls both routing and token minting.
    enabled: bool,
}

impl AcpMcpBridge {
    /// Build a bridge. `self_origin` is this server's own loopback origin
    /// (e.g. `http://localhost:8081`) — the Copilot subprocess reaches `/mcp`
    /// over loopback, not via the public `base_url`. `enabled` should mirror
    /// the Copilot Headless `COPILOT_HEADLESS_MCP_TOOL_CALLING` toggle.
    #[must_use]
    pub fn new(
        auth_manager: Arc<AuthManager>,
        jwks_manager: Arc<JwksManager>,
        repos: Arc<RepositoryRegistry>,
        self_origin: &str,
        enabled: bool,
    ) -> Self {
        Self {
            auth_manager,
            jwks_manager,
            repos,
            mcp_url: format!("{}/mcp", self_origin.trim_end_matches('/')),
            enabled,
        }
    }
}

#[async_trait]
impl McpBridgeProvider for AcpMcpBridge {
    async fn mcp_servers_for(&self, user_id: &str, tenant_id: TenantId) -> Vec<McpServerConfig> {
        if !self.enabled {
            return Vec::new();
        }

        let Ok(uuid) = Uuid::parse_str(user_id) else {
            warn!(
                user_id,
                "ACP MCP bridge: invalid user id; skipping native tools"
            );
            return Vec::new();
        };

        let user = match self.repos.users.get_global(uuid).await {
            Ok(Some(user)) => user,
            Ok(None) => {
                warn!(%uuid, "ACP MCP bridge: user not found; skipping native tools");
                return Vec::new();
            }
            Err(e) => {
                warn!(%uuid, error = %e, "ACP MCP bridge: user lookup failed; skipping native tools");
                return Vec::new();
            }
        };

        // Mint a SHORT-LIVED token scoped to the active tenant. The /mcp
        // endpoint validates this via `validate_jwt_token_for_mcp`, isolating
        // tool execution to (user, tenant). The TTL only needs to outlast a
        // single chat turn (tool calls + ACP prompt timeout), so 15 minutes
        // keeps the blast radius small if the token leaks from the subprocess
        // env/headers. The token is never logged.
        let token = match self.auth_manager.generate_token_with_tenant_and_ttl(
            &user,
            &self.jwks_manager,
            Some(tenant_id.to_string()),
            Duration::minutes(15),
            // Minted fresh per chat turn → the Guardian keys taint/budget on its
            // jti for this turn's native tool calls (#2).
            true,
        ) {
            Ok(token) => token,
            Err(e) => {
                warn!(%uuid, error = %e, "ACP MCP bridge: token mint failed; skipping native tools");
                return Vec::new();
            }
        };

        vec![McpServerConfig {
            name: "dravr".to_owned(),
            transport: McpTransport::Http {
                url: self.mcp_url.clone(),
                headers: vec![McpHeader {
                    name: "Authorization".to_owned(),
                    value: format!("Bearer {token}"),
                }],
            },
        }]
    }
}
