// ABOUTME: Narrow runtime façade exposing only the bits tool implementations need
// ABOUTME: Lets tools/ depend on a focused surface instead of the full ServerContext god-object
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `ToolRuntime` — narrow service façade for tool execution.
//!
//! `ServerContext` (the canonical DI container in pierre-server, at
//! `crate::mcp::resources::ServerContext`) exposes 50+ services. Tools only
//! need a small subset (database, repos, cache, providers, tool registry, a
//! few prompts). `ToolRuntime` declares exactly that subset as a trait so:
//!
//! * tools cannot reach into unrelated services by accident,
//! * tool implementations sit in a leaf-ward crate that does not need to
//!   drag in everything `ServerContext` touches, and
//! * tests can substitute a hand-rolled fake instead of building a full
//!   `ServerContext`.
//!
//! `ServerContext` implements `ToolRuntime` over in pierre-server; the
//! `Arc<dyn ToolRuntime>` handle stored on `ToolExecutionContext::resources`
//! is just a narrow view of the same shared container.

use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::registry::ToolRegistry;
use crate::tool_selection::ToolSelectionService;
use pierre_auth::tenant::TenantOAuthClient;
use pierre_cache::Cache;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_database::backends::factory::Database;
use pierre_database::database::repositories::CoachesRepository;
use pierre_database::RepositoryRegistry;
use pierre_intelligence::{ActivityIntelligence, IntelligenceConfig};
use pierre_llm::LlmProvider;
use pierre_mcp_transport::sampling_peer::SamplingPeer;
use pierre_providers::ProviderRegistry;
use pierre_runtime_context::DataContext;

#[cfg(feature = "transport-sse")]
use pierre_services::provider_refresh::SyncNotifier;

#[cfg(feature = "health-sync")]
use pierre_enforme::SyncOrchestrator;
#[cfg(feature = "client-notifications")]
use pierre_notifications::NotificationService;

/// Best-effort sink that pushes a "your historical backfill finished" notice
/// back to the messaging channel that triggered it.
///
/// A deep-history `get_activities` ask on a scrape-backed mirror provider warms
/// the durable cache off the request path (see [`crate::activity_backfill`]) and
/// returns silently. When the originating turn came from a messaging channel,
/// the backfill job can later resolve that channel from the Pierre conversation
/// id and ping the user that their older history is now ready.
///
/// The push is best-effort: the implementation owns its own error handling and
/// returns unit, so a notification failure never affects the backfill itself.
/// The concrete implementation and the call from `run_activity_backfill` land
/// in a later phase; this trait and the [`ToolRuntime::backfill_notifier`]
/// getter only declare the seam.
#[async_trait]
pub trait BackfillNotifier: Send + Sync {
    /// Notify the channel behind `pierre_conversation_id` that a background
    /// historical backfill of `activity_count` activities from `provider`
    /// finished for `(user_id, tenant_id)`. Best-effort — implementations
    /// swallow and log their own failures.
    ///
    /// `after_ts` is the historical-window `after` lower bound in unix seconds
    /// (`0` when the request had no `after`). It keys the durable cross-replica
    /// dedup claim so the notice is sent exactly once per `(user, provider,
    /// window)` even when several replicas finish the same backfill.
    async fn push_backfill_complete(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
        provider: &str,
        after_ts: i64,
        activity_count: usize,
    );

    /// Notify the channel behind `pierre_conversation_id` that `provider`'s
    /// session/token expired and must be reconnected — a localized message plus
    /// a one-time hosted-login link, routed to the originating channel (any
    /// channel) via the same rail as [`Self::push_backfill_complete`].
    ///
    /// Fired from the DETACHED backfill path, where an auth failure would
    /// otherwise be silent (the foreground tool loop already nudges inline).
    /// Best-effort and deduped per `(user, provider)` window via
    /// `claim_reauth_notification`, so a flapping connection nudges once, not
    /// every failed turn. Implementations swallow and log their own failures.
    async fn push_provider_reauth(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
        provider: &str,
    );
}

/// Narrow façade over the server's shared services that tool implementations
/// are allowed to depend on.
///
/// Implemented by `ServerContext` for production. Tests can implement it on
/// their own fakes if they only want to wire a subset of the dependencies.
pub trait ToolRuntime: Send + Sync + 'static {
    /// Database handle (for lifecycle and system settings — most data
    /// access should go through [`Self::repos`]).
    fn database(&self) -> &Arc<Database>;

    /// Repository registry — primary data-access surface.
    fn repos(&self) -> &Arc<RepositoryRegistry>;

    /// Shared cache layer.
    fn cache(&self) -> &Arc<Cache>;

    /// Server configuration (immutable runtime settings).
    fn config(&self) -> &Arc<ServerConfig>;

    /// Registry of fitness providers (Strava, Fitbit, …).
    fn provider_registry(&self) -> &Arc<ProviderRegistry>;

    /// Tenant-scoped OAuth client manager.
    fn tenant_oauth_client(&self) -> &Arc<TenantOAuthClient>;

    /// Hot-reloadable `IntelligenceConfig` snapshot registry.
    fn cageux_config_registry(&self) -> &Arc<CageuxConfigRegistry>;

    /// Convenience: snapshot of the current `IntelligenceConfig`.
    fn cageux_config(&self) -> Arc<IntelligenceConfig<true>> {
        self.cageux_config_registry().current()
    }

    /// Activity intelligence engine.
    fn activity_intelligence(&self) -> &Arc<ActivityIntelligence>;

    /// Tool registry (for discovery / introspection from inside tools).
    fn tool_registry(&self) -> &Arc<ToolRegistry>;

    /// Tool-selection scoring service.
    fn tool_selection(&self) -> &Arc<ToolSelectionService>;

    /// Coaches repository (kept as a method because it returns `&dyn`).
    fn coaches_manager(&self) -> &dyn CoachesRepository;

    /// Recommendation system prompt (delegates to the contremaitre prompt
    /// registry when the feature is enabled, otherwise the compiled-in
    /// default).
    fn recommendation_system_prompt(&self) -> String;

    /// Recommendation analysis prompt template.
    fn recommendation_analysis_prompt(&self) -> String;

    /// Activity analysis prompt template.
    fn activity_analysis_prompt(&self) -> String;

    /// Activity analysis system prompt.
    fn activity_analysis_system_prompt(&self) -> String;

    /// Insight-generation prompt template.
    fn insight_generation_prompt(&self) -> String;

    /// Insight-validation prompt template.
    fn insight_validation_prompt(&self) -> String;

    /// Optional LLM provider (when configured).
    fn llm_provider(&self) -> Option<&Arc<dyn LlmProvider>>;

    /// Hot-reloadable evidence registry for claim verification.
    ///
    /// Gated behind the `contremaitre` feature because the registry only
    /// exists when the contremaitre GitHub-sync subsystem is wired in.
    fn evidence_registry(&self) -> &Arc<pierre_contremaitre::EvidenceRegistry>;

    /// Materialize a [`DataContext`] (the focused data-access slice).
    fn data(&self) -> DataContext;

    /// Optional best-effort notifier that pushes a backfill-completion notice
    /// back to the messaging channel that triggered a historical
    /// [`crate::activity_backfill`] job. Defaults to `None` so existing
    /// `ToolRuntime` impls (and test fakes) keep compiling unchanged; the
    /// production `ServerContext` overrides it once the push lands.
    fn backfill_notifier(&self) -> Option<&Arc<dyn BackfillNotifier>> {
        None
    }

    /// Optional MCP sampling peer (present when an MCP client supports
    /// sampling).
    fn sampling_peer(&self) -> Option<&Arc<SamplingPeer>>;

    /// Optional notification service (present when `client-notifications`
    /// is enabled and a notification backend is wired).
    #[cfg(feature = "client-notifications")]
    fn notification_service(&self) -> Option<&Arc<NotificationService>>;

    /// Narrow sync-notification handle (push interface used by the refresh
    /// service to inform connected clients). In production this is backed
    /// by `SseManager`, but the trait only exposes the `SyncNotifier`
    /// surface so the trait body stays free of pierre-server-internal SSE
    /// machinery.
    #[cfg(feature = "transport-sse")]
    fn sync_notifier(&self) -> Arc<dyn SyncNotifier>;

    /// Sync orchestrator for refreshing fitness data from providers.
    #[cfg(feature = "health-sync")]
    fn sync_orchestrator(&self) -> Option<&Arc<SyncOrchestrator>>;
}
