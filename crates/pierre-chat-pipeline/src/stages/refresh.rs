// ABOUTME: Provider data freshness stage — triggers the chat refresh and builds the agent hint
// ABOUTME: Provides refresh_and_hint — the hint rides the turn clock, outside the cached prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider data freshness hint.
//!
//! Checks whether connected fitness providers have recent data, triggers a
//! non-blocking background refresh for stale ones, and optionally builds a
//! "some data may be stale" hint so the agent can acknowledge the limitation
//! to the user rather than silently analyzing outdated data.
//!
//! The hint names each stale provider's age ("3h ago"), so it changes as time
//! passes. It rides the turn clock message, after the cache breakpoints, for
//! the clock's own reason (carnet#895): inside the system prompt it sat ahead
//! of the memory, playbook and plan blocks and re-wrote all of them in the
//! cache every time an age ticked over.
//!
//! Controlled by [`pierre_core::models::RefreshConfig`] — the stage is a
//! no-op when `on_chat_enabled` is false.

use std::fmt;
use std::sync::Arc;

use pierre_core::models::RefreshConfig;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::repositories::ActivityCacheRepository;
#[cfg(feature = "health-sync")]
use pierre_database::repositories::{CoachingGroupRepository, ProviderConnectionRepository};
use pierre_database::AuthRepos;

use pierre_core::models::TenantId;
use pierre_services::provider_refresh::RefreshService;
#[cfg(feature = "health-sync")]
use pierre_services::provider_refresh::SyncNotifier;
#[cfg(feature = "health-sync")]
use pierre_sse::SseManager;

/// Inputs to [`refresh_and_hint`].
///
/// Bundles the per-call dependencies so the function signature stays
/// under clippy's `too_many_arguments` ceiling and the caller does not
/// need to pass the full [`crate::mcp::resources::ServerContext`]. All
/// fields are borrowed references, so the struct is cheap to pass by
/// value.
#[derive(Clone)]
pub struct RefreshDeps<'a> {
    /// Narrow auth view used to read provider freshness rows
    /// (`oauth_tokens.last_sync`) — internally cloned into the
    /// `RefreshService`.
    pub auth_repos: &'a AuthRepos,
    /// Activity cache, source of truth for on-demand providers' freshness
    /// (sciotte's `synced_at`). Cloned into the `RefreshService`.
    pub activity_cache: Arc<dyn ActivityCacheRepository>,
    /// The user's provider connections, which the refresh reads to tell a
    /// wellness source of their own. Cloned into the `RefreshService`.
    #[cfg(feature = "health-sync")]
    pub provider_connections: Arc<dyn ProviderConnectionRepository>,
    /// The user's group memberships, which the refresh reads for the coach
    /// consents a link's sync goes by. Cloned into the `RefreshService`.
    #[cfg(feature = "health-sync")]
    pub groups: Arc<dyn CoachingGroupRepository>,
    /// Optional sync orchestrator used to schedule background refreshes.
    #[cfg(feature = "health-sync")]
    pub sync_orchestrator: &'a Option<Arc<dravr_enforme::SyncOrchestrator>>,
    /// SSE manager used to push refresh-status events to the client. Present
    /// only under `health-sync`, the feature that owns the background sync the
    /// events describe.
    #[cfg(feature = "health-sync")]
    pub sse_manager: &'a Arc<SseManager>,
}

impl fmt::Debug for RefreshDeps<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("RefreshDeps");
        debug.field("auth_repos", &self.auth_repos);
        #[cfg(feature = "health-sync")]
        debug.field("sync_orchestrator", &self.sync_orchestrator);
        #[cfg(feature = "health-sync")]
        debug.field("sse_manager", &self.sse_manager);
        debug.finish_non_exhaustive()
    }
}

/// Trigger the chat refresh and build the freshness hint for this turn.
///
/// The hint makes the agent aware that some data may be stale so it can
/// acknowledge the limitation to the user rather than silently analyzing
/// outdated data. The caller sends it with the turn clock.
///
/// When [`RefreshConfig::on_chat_enabled`] is false the stage is a no-op
/// and returns `None`. When the agent hint is disabled the refresh still
/// fires but no hint is returned.
pub async fn refresh_and_hint(
    deps: RefreshDeps<'_>,
    user_id: &str,
    tenant_id: TenantId,
) -> Option<String> {
    let RefreshDeps {
        auth_repos,
        activity_cache,
        #[cfg(feature = "health-sync")]
        provider_connections,
        #[cfg(feature = "health-sync")]
        groups,
        #[cfg(feature = "health-sync")]
        sync_orchestrator,
        #[cfg(feature = "health-sync")]
        sse_manager,
    } = deps;
    let config = RefreshConfig::default();
    if !config.on_chat_enabled {
        return None;
    }

    let Ok(user_uuid) = parse_uuid(user_id) else {
        return None;
    };

    #[cfg(feature = "health-sync")]
    let notifier: Arc<dyn SyncNotifier> = sse_manager.clone();
    let refresh_service = RefreshService::new(
        auth_repos,
        activity_cache,
        #[cfg(feature = "health-sync")]
        provider_connections,
        #[cfg(feature = "health-sync")]
        groups,
        #[cfg(feature = "health-sync")]
        sync_orchestrator.clone(),
        #[cfg(feature = "health-sync")]
        notifier,
    );

    let status = refresh_service
        .check_and_refresh(user_uuid, tenant_id, &config)
        .await;

    if !config.inject_agent_hint {
        return None;
    }

    RefreshService::build_agent_hint(&status.details)
}
