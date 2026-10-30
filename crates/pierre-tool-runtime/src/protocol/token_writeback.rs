// ABOUTME: Write-back of a token a provider refreshed or rotated on its own, over the row its credentials came from
// ABOUTME: Only while that row still stands, so a reconnect that replaced it meanwhile is never overwritten
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::TenantId;
use pierre_providers::token_writeback::StoredTokenRow;
use pierre_providers::OAuth2Credentials;
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Persist a provider-refreshed token to the database, over the row the
/// provider's credentials were read from (`row_id`), only while that row is
/// still the one stored ([`StoredTokenRow::write_back`]).
/// Called from the token refresh callback wired into providers.
///
/// Errors are logged but not propagated — the in-memory token is still valid
/// for the current request. A read outside a tenant has no row to write.
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
    StoredTokenRow::new(user_id, tid, provider, row_id)
        .write_back(&*resources.repos().oauth_tokens, creds)
        .await;
}
