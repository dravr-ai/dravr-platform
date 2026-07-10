// ABOUTME: AuthService-backed credential refresher for the health-sync scheduler
// ABOUTME: Bridges enforme's CredentialStore refresh path to the platform OAuth flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use pierre_enforme::error::{EnformeError, EnformeResult};
use pierre_enforme::models::connection::ProviderCredentials;
use pierre_services::health_sync::SyncCredentialRefresher;
use pierre_tool_runtime::protocol::auth::TokenData;
use pierre_tool_runtime::protocol::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::info;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// `AuthService`-backed implementation of enforme's credential-refresh bridge.
///
/// Routes scheduled-sync credential reads through the same token path live
/// tool calls use: `get_valid_token` refreshes near-expiry tokens, persists
/// the result, and maintains provider-connection status. Holds a
/// `Weak<ServerContext>` — the storage this installs into lives inside the
/// context's sync orchestrator, so a strong handle would create a DI-graph
/// cycle (mirrors the backfill re-entry handle).
pub struct AuthServiceCredentialRefresher {
    /// Composition-root handle, upgraded per call.
    runtime: Weak<ServerContext>,
}

impl AuthServiceCredentialRefresher {
    /// Build an `AuthService` from the still-alive server context.
    fn auth_service(&self) -> EnformeResult<AuthService> {
        let ctx = self.runtime.upgrade().ok_or_else(|| {
            EnformeError::store("server context dropped; cannot refresh credentials")
        })?;
        Ok(AuthService::new(ctx as Arc<dyn ToolRuntime>))
    }
}

/// Map the tool-runtime token shape onto enforme's credential model.
fn token_to_credentials(user_id: Uuid, token: TokenData) -> ProviderCredentials {
    let scopes = if token.scopes.is_empty() {
        Vec::new()
    } else {
        token.scopes.split(' ').map(String::from).collect()
    };
    let refresh_token = if token.refresh_token.is_empty() {
        None
    } else {
        Some(token.refresh_token)
    };
    ProviderCredentials {
        access_token: token.access_token,
        refresh_token,
        expires_at: Some(token.expires_at),
        scopes,
        user_id: user_id.to_string(),
        provider: token.provider,
    }
}

#[async_trait]
impl SyncCredentialRefresher for AuthServiceCredentialRefresher {
    async fn valid_credentials(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>> {
        let auth = self.auth_service()?;
        let token = auth
            .get_valid_token(user_id, provider, Some(tenant_id))
            .await
            .map_err(|e| EnformeError::store(format!("OAuth token lookup failed: {e}")))?;
        Ok(token.map(|t| token_to_credentials(user_id, t)))
    }

    async fn force_refresh(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>> {
        let auth = self.auth_service()?;
        let token = auth
            .force_refresh_token(user_id, tenant_id, provider)
            .await
            .map_err(|e| EnformeError::store(format!("OAuth token refresh failed: {e}")))?;
        Ok(token.map(|t| token_to_credentials(user_id, t)))
    }
}

/// Install the `AuthService`-backed refresher on the health-sync storage now
/// that the composition-root `Arc<ServerContext>` exists.
///
/// The storage is built pre-Arc inside `ServerContext::new` (the refresh path
/// needs the runtime, which does not exist yet at that point), so the bridge
/// is plumbed in here — mirroring the SSE protocol-factory install. Runs well
/// before the first scheduled sync tick (the scheduler sleeps a full jittered
/// poll interval before its first cycle).
pub fn install_health_sync_refresher(resources: &Arc<ServerContext>) {
    let Some(storage) = resources.fitness.sync_storage.as_ref() else {
        return;
    };
    storage.set_credential_refresher(Arc::new(AuthServiceCredentialRefresher {
        runtime: Arc::downgrade(resources),
    }));
    info!("Health-sync credential refresher installed (AuthService-backed)");
}
