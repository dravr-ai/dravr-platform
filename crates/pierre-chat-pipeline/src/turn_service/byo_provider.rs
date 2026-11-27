// ABOUTME: The chat provider one turn runs on: the tenant's own stored key, or the process-wide provider
// ABOUTME: A short TTL cache keeps the common no-key case off the database on every turn

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::models::TenantId;
use pierre_llm::ChatProvider;
use pierre_services::tenant_chat_provider::resolve_tenant_chat_provider;
use uuid::Uuid;

use crate::ChatPipelineContext;

/// The chat provider a `(tenant, athlete)` pair should run this turn on.
///
/// `Some` is the tenant's own stored key ("bring your own"); `None` leaves
/// the turn on the process-wide provider singleton. A short TTL cache keeps
/// the common no-key case off the database on every turn.
pub(super) async fn resolve_byo_chat_provider(
    ctx: &ChatPipelineContext,
    tenant_id: TenantId,
    user_id: Uuid,
) -> Option<Arc<ChatProvider>> {
    resolve_tenant_chat_provider(
        &ctx.tenant_chat_providers,
        &ctx.config.llm_chain,
        ctx.repos.llm_credentials.as_ref(),
        ctx.repos.security.as_ref(),
        tenant_id,
        user_id,
    )
    .await
}
