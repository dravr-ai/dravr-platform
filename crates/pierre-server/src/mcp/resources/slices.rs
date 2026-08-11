// ABOUTME: 8 composed slice structs that partition ServerContext's 52 Arc fields into semantic groups
// ABOUTME: Each slice owns the Arcs for one subsystem; ServerContext = composition of all slices
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `ServerContext` slices
//!
//! `ServerContext` (in `context.rs`) is the canonical DI container. Internally it
//! is composed of the 8 slice structs defined in this file. Each slice groups
//! the Arc handles that belong to one logical subsystem so the container's
//! field surface is semantically navigable (`ctx.auth.auth_manager`,
//! `ctx.fitness.provider_registry`) instead of one flat 50-field struct.
//!
//! ## Partition map (52 fields → 8 slices)
//!
//! - `CommonSlice` — the master repository registry (satisfies the `*Ctx` trait
//!   `repos() -> &Arc<RepositoryRegistry>` surface) plus shared/cross-cutting
//!   handles: cache, config, redaction config, email service, LLM provider
//!   pair, LLM health, messaging registry, notification service, group service,
//!   scheduler abort handles, embedding provider, slash command registries.
//! - `AuthSlice` — authentication + authorization + per-tenant OAuth: auth
//!   manager, JWKS manager, auth middleware, CSRF manager, Firebase auth,
//!   `OAuth2` rate limiter, admin JWT secret, tenant OAuth client, OAuth
//!   notification sender, sciotte nonce store + mint rate limiter. Also
//!   carries an `AuthRepos` view-struct projection.
//! - `CoachSlice` — coach generation context: database handle (still needed
//!   for migrations / pool access by `NotificationService` / `AdminConfigService`),
//!   admin config service. Also carries a `CoachRepos` view-struct projection.
//! - `FitnessSlice` — fitness data and intelligence: provider registry,
//!   activity intelligence, sync orchestrator and its abort handle, cageux /
//!   harness / persona contract config registries. Also carries a
//!   `FitnessRepos` view-struct projection.
//! - `SseSlice` — push-notification transports: SSE manager, AG-UI run
//!   registry, sampling peer, progress notification sender, cancellation
//!   registry.
//! - `A2ASlice` — agent-to-agent client manager and A2A system user service.
//! - `BillingSlice` — billing provider. Also carries a `UsageRepos` view-struct
//!   projection.
//! - `McpSlice` — MCP tool dispatch internals: tool registry, tool description
//!   registry, evidence registry, tool selection service, prompt registry,
//!   messaging-strings registry, contremaitre config.
//!
//! ## View-struct projections
//!
//! Slices that semantically partition repository access (`AuthSlice`,
//! `CoachSlice`, `FitnessSlice`, `BillingSlice`) carry a typed view-struct
//! projection (`AuthRepos`, `CoachRepos`, `FitnessRepos`, `UsageRepos`) built
//! at construction time via [`RepositoryRegistry::auth_repos`] etc. These are
//! cheap Arc clones of the same underlying repository state — narrower typed
//! surface, identical data. They are positioned for a future per-view trait
//! migration; today's trait surface still goes through `CommonSlice.repos`.

#[cfg(feature = "protocol-a2a")]
use crate::a2a::client::A2AClientManager;
#[cfg(feature = "protocol-a2a")]
use crate::a2a::system_user::A2ASystemUserService;
use crate::agui::RunRegistry as AgUiRunRegistry;
use crate::config::admin::AdminConfigService;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_auth::firebase::FirebaseAuth;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_auth::tenant::TenantOAuthClient;
use pierre_cache::Cache;
#[cfg(feature = "client-messaging")]
use pierre_commands::CommandHandlerRegistry;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    ContremaitreConfig, EvidenceRegistry, MessagingStringsRegistry, PromptRegistry,
    ToolDescriptionRegistry,
};
use pierre_core::billing::BillingProvider;
use pierre_database::backends::factory::Database;
use pierre_database::views::{AuthRepos, CoachRepos, FitnessRepos, UsageRepos};
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
use pierre_intelligence::ActivityIntelligence;
use pierre_llm::embeddings::InstrumentedEmbeddingProvider;
use pierre_llm::health::LlmHealthState;
use pierre_llm::ChatProvider;
use pierre_llm::LlmProvider;
use pierre_mcp_schema::{OAuthCompletedNotification, ProgressNotification};
use pierre_mcp_transport::sampling_peer::SamplingPeer;
#[cfg(feature = "client-messaging")]
use pierre_messaging::commands::CommandRegistry;
#[cfg(feature = "client-messaging")]
use pierre_messaging::ChannelRegistry;
#[cfg(feature = "provider-sciotte")]
use pierre_middleware::provider_link_token::{MintRateLimiter, NonceStore};
use pierre_middleware::redaction::RedactionConfig;
use pierre_middleware::McpAuthMiddleware;
#[cfg(feature = "client-notifications")]
use pierre_notifications::NotificationService;
use pierre_providers::registry::ProviderRegistry;
#[cfg(feature = "health-sync")]
use pierre_services::health_sync::PierreSyncStorage;
use pierre_services::tenant_chat_provider::TenantChatProviderCache;
#[cfg(feature = "transport-sse")]
use pierre_sse::SseManager;
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use pierre_tool_runtime::protocol::types::CancellationToken;
use pierre_tool_runtime::registry::ToolRegistry;
#[cfg(feature = "client-messaging")]
use pierre_tool_runtime::runtime::BackfillNotifier;
use pierre_tool_runtime::tool_selection::ToolSelectionService;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "client-messaging")]
use std::sync::OnceLock;

#[cfg(feature = "client-messaging")]
use crate::services::backfill_notifier::ChatReentry;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::AbortHandle;

/// Cross-cutting handles + the master repository registry.
///
/// `repos` here is the single source of truth that satisfies every
/// `pierre_runtime_context::*Ctx` impl's `fn repos() -> &Arc<RepositoryRegistry>`
/// accessor. The per-subsystem slices also carry typed view-struct projections
/// (`AuthRepos`, `CoachRepos`, …) built from the same registry, but only this
/// one is the canonical Arc.
#[derive(Clone)]
pub struct CommonSlice {
    /// Trait-object repository registry — the primary data access layer.
    pub repos: Arc<RepositoryRegistry>,
    /// Cache layer for performance optimization.
    pub cache: Arc<Cache>,
    /// Server configuration loaded from environment.
    pub config: Arc<ServerConfig>,
    /// Configuration for PII redaction in logs and responses.
    pub redaction_config: Arc<RedactionConfig>,
    /// Optional email service for transactional emails (password reset codes, etc.).
    pub email_service: Option<Arc<ResendEmailService>>,
    /// Optional LLM provider for insight validation and generation.
    pub llm_provider: Option<Arc<dyn LlmProvider>>,
    /// Pre-built [`ChatProvider`] singleton shared across chat/coach/health-probe.
    pub chat_provider: Option<Arc<ChatProvider>>,
    /// TTL cache of per-tenant chat providers built from stored BYO LLM keys.
    /// Empty for tenants on the system default; resolved lazily per turn.
    pub tenant_chat_providers: TenantChatProviderCache,
    /// Shared LLM startup-probe state — read by `/ready` and `/health/llm`.
    pub llm_health: Arc<LlmHealthState>,
    /// Embedding provider wrapped for usage instrumentation. `None` when no
    /// embedding provider key is configured.
    pub embedding_provider: Option<Arc<InstrumentedEmbeddingProvider>>,
    /// Multi-channel messaging registry for webhook routing.
    #[cfg(feature = "client-messaging")]
    pub messaging_registry: Arc<ChannelRegistry>,
    /// Notification service facade for dispatch, scheduling, and persistence.
    #[cfg(feature = "client-notifications")]
    pub notification_service: Option<Arc<NotificationService>>,
    /// Abort handle for the background notification scheduler task.
    #[cfg(feature = "client-notifications")]
    pub scheduler_abort_handle: Option<AbortHandle>,
    /// Abort handle for the background usage counter pruning task.
    pub pruning_abort_handle: Option<AbortHandle>,
    /// Group coaching service for context injection and group management.
    #[cfg(feature = "tools-groups")]
    pub group_service: Arc<pierre_groups::GroupService>,
    /// Messaging slash command registry (loaded from `commands/*.md`).
    #[cfg(feature = "client-messaging")]
    pub command_registry: Option<Arc<CommandRegistry>>,
    /// Messaging command handler registry.
    #[cfg(feature = "client-messaging")]
    pub command_handler_registry: Option<Arc<CommandHandlerRegistry>>,
}

/// Authentication and authorization subsystem.
///
/// Holds auth manager, JWKS, auth middleware, CSRF, Firebase, the `OAuth2` rate
/// limiter, the admin JWT secret, the per-tenant OAuth client, the OAuth
/// completion broadcast sender, and (under `provider-sciotte`) the hosted-login
/// nonce store + mint rate limiter. Also carries an [`AuthRepos`] projection
/// for future per-view trait migration.
#[derive(Clone)]
pub struct AuthSlice {
    /// Authentication manager for user identity verification.
    pub auth_manager: Arc<AuthManager>,
    /// JSON Web Key Set manager for RS256 JWT signing and verification.
    pub jwks_manager: Arc<JwksManager>,
    /// Authentication middleware for MCP request validation.
    pub auth_middleware: Arc<McpAuthMiddleware>,
    /// CSRF token manager for request forgery protection.
    pub csrf_manager: Arc<CsrfTokenManager>,
    /// Firebase Authentication handler for social login (Google, Apple, etc.).
    pub firebase_auth: Option<Arc<FirebaseAuth>>,
    /// Rate limiter for `OAuth2` endpoints.
    pub oauth2_rate_limiter: Arc<OAuth2RateLimiter>,
    /// Secret key for admin JWT token generation.
    pub admin_jwt_secret: Arc<str>,
    /// OAuth client for multi-tenant authentication flows.
    pub tenant_oauth_client: Arc<TenantOAuthClient>,
    /// Broadcast channel for OAuth completion notifications.
    pub oauth_notification_sender: Option<broadcast::Sender<OAuthCompletedNotification>>,
    /// Cache-backed one-time nonce store for link-token page loads.
    #[cfg(feature = "provider-sciotte")]
    pub nonce_store: Arc<NonceStore<Cache>>,
    /// Cache-backed rate limiter for link-token minting.
    #[cfg(feature = "provider-sciotte")]
    pub mint_rate_limiter: Arc<MintRateLimiter<Cache>>,
    /// Auth-domain repository view (projection of [`CommonSlice::repos`]).
    pub repos: AuthRepos,
}

/// Coach generation context.
///
/// `database` is retained for lifecycle (migrations, encryption key updates),
/// system settings, and pool access (`NotificationService`,
/// `AdminConfigService`); data access should go through repos instead.
#[derive(Clone)]
pub struct CoachSlice {
    /// Database connection pool for persistent storage operations.
    pub database: Arc<Database>,
    /// Admin configuration service for runtime parameter management.
    pub admin_config: Option<Arc<AdminConfigService>>,
    /// Coach-domain repository view (projection of [`CommonSlice::repos`]).
    pub repos: CoachRepos,
}

/// Fitness data and intelligence subsystem.
#[derive(Clone)]
pub struct FitnessSlice {
    /// Registry of fitness data providers (Strava, Fitbit, Garmin, WHOOP, Terra).
    pub provider_registry: Arc<ProviderRegistry>,
    /// AI-powered fitness activity analysis engine.
    pub activity_intelligence: Arc<ActivityIntelligence>,
    /// Health data sync orchestrator for wearable provider synchronization.
    #[cfg(feature = "health-sync")]
    pub sync_orchestrator: Option<Arc<pierre_enforme::SyncOrchestrator>>,
    /// Storage adapter behind the sync orchestrator, kept so the
    /// AuthService-backed credential refresher can be injected post-Arc
    /// (see `services::health_sync_refresher`).
    #[cfg(feature = "health-sync")]
    pub sync_storage: Option<Arc<PierreSyncStorage>>,
    /// Abort handle for the background health data sync scheduler task.
    #[cfg(feature = "health-sync")]
    pub sync_scheduler_abort_handle: Option<AbortHandle>,
    /// Hot-swappable cageux intelligence config snapshot.
    pub cageux_config_registry: Arc<CageuxConfigRegistry>,
    /// Hot-swappable coaching harness config snapshot.
    pub harness_config_registry: Arc<HarnessConfigRegistry>,
    /// Hot-swappable Guardian policy snapshot (defaults ← DB doc ← env).
    pub guardian_config_registry: Arc<GuardianConfigRegistry>,
    /// Hot-swappable per-persona output-format conformance contracts.
    pub persona_contract_registry: Arc<PersonaContractRegistry>,
    /// Fitness-domain repository view (projection of [`CommonSlice::repos`]).
    pub repos: FitnessRepos,
}

/// Push-notification transports.
#[derive(Clone)]
pub struct SseSlice {
    /// Server-Sent Events manager for streaming notifications and MCP protocol.
    #[cfg(feature = "transport-sse")]
    pub sse_manager: Arc<SseManager>,
    /// AG-UI run registry — chat pipeline publishes per-run broadcast channels here.
    pub agui_registry: Arc<AgUiRunRegistry>,
    /// Optional sampling peer for server-initiated LLM requests (stdio transport only).
    pub sampling_peer: Option<Arc<SamplingPeer>>,
    /// Optional progress notification sender (stdio transport only).
    pub progress_notification_sender: Option<mpsc::UnboundedSender<ProgressNotification>>,
    /// Cancellation token registry for progress token → cancellation token mapping.
    pub cancellation_registry: Arc<RwLock<HashMap<String, CancellationToken>>>,
}

/// Agent-to-agent protocol subsystem.
#[derive(Clone)]
pub struct A2ASlice {
    /// A2A protocol client manager for agent-to-agent communication.
    #[cfg(feature = "protocol-a2a")]
    pub a2a_client_manager: Arc<A2AClientManager>,
    /// Service for managing A2A system user accounts.
    #[cfg(feature = "protocol-a2a")]
    pub a2a_system_user_service: Arc<A2ASystemUserService>,
}

/// Billing subsystem.
#[derive(Clone)]
pub struct BillingSlice {
    /// Pluggable billing provider (Stripe / `RevenueCat` / Dummy / …).
    pub billing_provider: Arc<dyn BillingProvider>,
    /// Usage-domain repository view (projection of [`CommonSlice::repos`]).
    pub repos: UsageRepos,
}

/// MCP tool dispatch internals.
#[derive(Clone)]
pub struct McpSlice {
    /// Central registry for MCP tool discovery and execution.
    pub tool_registry: Arc<ToolRegistry>,
    /// Tool selection service for per-tenant MCP tool filtering.
    pub tool_selection: Arc<ToolSelectionService>,
    /// Prompt registry for hot-reloadable system prompts and coach personas.
    pub prompt_registry: Arc<PromptRegistry>,
    /// Tool description registry for hot-reloadable MCP tool schema overlays.
    pub tool_description_registry: Arc<ToolDescriptionRegistry>,
    /// Evidence registry for hot-reloadable claim verification corpus.
    pub evidence_registry: Arc<EvidenceRegistry>,
    /// Messaging strings registry for hot-reloadable user-facing canned replies.
    pub messaging_strings_registry: Arc<MessagingStringsRegistry>,
    /// Best-effort notifier that pushes a "your historical backfill finished"
    /// notice back to the channel that triggered a background activity backfill.
    /// `None` when messaging is compiled out. Exposed to the detached backfill
    /// task through [`pierre_tool_runtime::runtime::ToolRuntime::backfill_notifier`].
    #[cfg(feature = "client-messaging")]
    pub backfill_notifier: Option<Arc<dyn BackfillNotifier>>,
    /// Chat-pipeline re-entry handle for the backfill-completion push, shared
    /// (same `Arc`) with [`ServerBackfillNotifier`] so it can synthesize a real
    /// coach answer instead of a templated list. Empty until the composition
    /// root installs it post-`Arc` in `spawn_background_workers` (the handle
    /// needs the `Arc<ServerContext>`, absent when the notifier is built).
    /// Mirrors the SSE protocol-factory install.
    #[cfg(feature = "client-messaging")]
    pub backfill_reentry: Arc<OnceLock<Arc<dyn ChatReentry>>>,
    /// Contremaitre configuration for GitHub sync and webhook verification.
    pub contremaitre_config: Option<ContremaitreConfig>,
}
