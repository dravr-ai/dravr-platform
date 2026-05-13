// ABOUTME: Provider data freshness stage — triggers background refresh and injects coach hint
// ABOUTME: Extracted from services/chat_orchestration.rs::inject_refresh_context (2026-04-16)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider data freshness hint.
//!
//! Checks whether connected fitness providers have recent data, triggers a
//! non-blocking background refresh for stale ones, and optionally appends a
//! "some data may be stale" hint to the system prompt so the coach can
//! acknowledge the limitation to the user rather than silently analyzing
//! outdated data.
//!
//! Controlled by [`pierre_core::models::RefreshConfig`] — the stage is a
//! no-op when `on_chat_enabled` is false.

use std::sync::Arc;

use pierre_core::models::RefreshConfig;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::RepositoryRegistry;

use crate::models::TenantId;
use crate::services::provider_refresh::RefreshService;
use crate::sse::SseManager;

/// Inputs to [`inject_refresh_context`].
///
/// Bundles the per-call dependencies so the function signature stays
/// under clippy's `too_many_arguments` ceiling and the caller does not
/// need to pass the full [`crate::mcp::resources::ServerContext`]. All
/// fields are borrowed references, so the struct is `Copy` and cheap to
/// pass by value.
#[derive(Clone, Copy)]
pub struct RefreshDeps<'a> {
    /// Repository registry used to look up provider freshness rows.
    pub repos: &'a Arc<RepositoryRegistry>,
    /// Optional sync orchestrator used to schedule background refreshes.
    #[cfg(feature = "health-sync")]
    pub sync_orchestrator: &'a Option<Arc<pierre_enforme::SyncOrchestrator>>,
    /// SSE manager used to push refresh-status events to the client.
    pub sse_manager: &'a Arc<SseManager>,
}

/// Trigger a background provider refresh and append a freshness hint.
///
/// The refresh runs non-blocking; the appended hint makes the coach
/// aware that some data may be stale so it can acknowledge the
/// limitation to the user rather than silently analyzing outdated data.
///
/// When [`RefreshConfig::on_chat_enabled`] is false the stage is a no-op
/// and returns `base_prompt` unchanged. When the coach hint is disabled
/// the refresh still fires but nothing is appended to the prompt.
pub async fn inject_refresh_context(
    deps: RefreshDeps<'_>,
    user_id: &str,
    tenant_id: TenantId,
    base_prompt: String,
) -> String {
    let RefreshDeps {
        repos,
        #[cfg(feature = "health-sync")]
        sync_orchestrator,
        sse_manager,
    } = deps;
    let config = RefreshConfig::default();
    if !config.on_chat_enabled {
        return base_prompt;
    }

    let Ok(user_uuid) = parse_uuid(user_id) else {
        return base_prompt;
    };

    let refresh_service = RefreshService::new(
        repos.clone(),
        #[cfg(feature = "health-sync")]
        sync_orchestrator.clone(),
        sse_manager.clone(),
    );

    let status = refresh_service
        .check_and_refresh(user_uuid, tenant_id, &config)
        .await;

    if !config.inject_coach_hint {
        return base_prompt;
    }

    match RefreshService::build_coach_hint(&status.details) {
        Some(hint) => format!("{base_prompt}\n\n{hint}"),
        None => base_prompt,
    }
}
