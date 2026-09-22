// ABOUTME: Write-back of a token pair a provider refreshed on its own, over the row its credentials came from
// ABOUTME: Only while that row still stands, so a reconnect that replaced it meanwhile is never overwritten
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_providers::OAuth2Credentials;
use tracing::{info, warn};
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Persist a provider-refreshed token to the database.
/// Called from the token refresh callback wired into providers.
///
/// Errors are logged but not propagated — the in-memory token is still valid
/// for the current request.
pub(super) async fn persist_refreshed_token(
    resources: &dyn ToolRuntime,
    user_id: Uuid,
    tenant_id: Option<&str>,
    provider: &str,
    row_id: &str,
    creds: &OAuth2Credentials,
) {
    let Some(tenant_id_str) = tenant_id else {
        return;
    };
    let Ok(tid) = TenantId::parse_str(tenant_id_str) else {
        return;
    };

    match write_back_refreshed(resources, user_id, tid, provider, row_id, creds).await {
        Ok(true) => info!(
            "Persisted provider-refreshed {} token for user {}",
            provider, user_id
        ),
        Ok(false) => info!(
            "Provider-refreshed {} token for user {} not persisted: the token it refreshed was replaced since",
            provider, user_id
        ),
        Err(e) => warn!(
            "Failed to persist provider-refreshed {} token for user {}: {}",
            provider, user_id, e
        ),
    }
}

/// Write a provider-refreshed pair over the row the provider's credentials
/// were read from (`row_id`), only while that row is still the one stored: a
/// reconnect that replaced it meanwhile stands. `false` when nothing was
/// written.
async fn write_back_refreshed(
    resources: &dyn ToolRuntime,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    row_id: &str,
    creds: &OAuth2Credentials,
) -> Result<bool, AppError> {
    let tokens = &resources.repos().oauth_tokens;
    let Some(stored) = tokens
        .get_token(user_id, tenant_id, provider)
        .await?
        .filter(|stored| stored.id == row_id)
    else {
        return Ok(false);
    };
    tokens
        .refresh_token(
            &stored,
            creds.access_token.as_deref().unwrap_or_default(),
            creds.refresh_token.as_deref(),
            creds.expires_at,
        )
        .await
}
