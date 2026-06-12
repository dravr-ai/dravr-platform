// ABOUTME: ServerContext struct definition + post-construction setters/accessors/prompt-delegation + route-context view builders
// ABOUTME: Canonical dependency-injection container composed of 8 slices; every handler reaches its dependencies through one of the slice fields or accessor methods
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Server Resources Module
//!
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - Arc sharing of expensive resources (database, auth managers) across threads
// - Resource ownership transfers for dependency injection
//!
//! Centralized resource container for dependency injection.
//! Internally composed of 8 slice structs (see `slices.rs`) that partition the
//! ~50 shared Arc handles into semantic groups: `common`, `auth`, `coach`,
//! `fitness`, `sse`, `a2a`, `billing`, `mcp`.

#[cfg(feature = "client-chat")]
use std::env;

#[cfg(feature = "client-chat")]
use super::acp_mcp_bridge::AcpMcpBridge;
use super::slices::{
    A2ASlice, AuthSlice, BillingSlice, CoachSlice, CommonSlice, FitnessSlice, McpSlice, SseSlice,
};
use super::ServerContextBuilder;
#[cfg(feature = "client-messaging")]
use crate::services::user_approval_notifier::ApprovalNotifier;
#[cfg(feature = "provider-sciotte")]
use dravr_contremaitre::schemas::STRUCTURED_WORKOUT_SCHEMA;
use dravr_contremaitre::system::STRUCTURED_OUTPUT as STRUCTURED_OUTPUT_DIRECTIVE;
use dravr_sciotte::config::{LoginMode, ScraperConfig};
#[cfg(feature = "provider-sciotte")]
use embacle::types::LlmProvider as EmbacleLlmProvider;
#[cfg(feature = "provider-sciotte")]
use embacle::{CopilotHeadlessConfig, CopilotHeadlessRunner};
#[cfg(feature = "client-chat")]
use pierre_chat_pipeline::McpBridgeProvider;
use pierre_core::errors::{AppError, AppResult};
#[cfg(feature = "client-messaging")]
use pierre_core::models::TenantId;
use pierre_database::backends::StoreListingsRepository;
use pierre_database::database::repositories::{
    CoachesRepository, MobilityRepository, RecipeRepository, SocialRepository,
};
use pierre_llm::ChatProvider;
use pierre_mcp_schema::{OAuthCompletedNotification, ProgressNotification};
use pierre_mcp_transport::sampling_peer::SamplingPeer;
use pierre_messaging::ChannelRegistry;
use pierre_services::tenant_chat_provider::resolve_tenant_chat_provider;
use pierre_tool_runtime::protocol::types::CancellationToken;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};
use uuid::Uuid;

/// Build the shared Copilot-headless LLM provider for sciotte vision login.
///
/// Returns `None` under the default `Selector` `DRAVR_SCIOTTE_LOGIN_MODE` (the
/// LLM is never consulted), or `Some` shared runner for `Hybrid`/`Vision`. The
/// model comes from `COPILOT_HEADLESS_MODEL` (see
/// [`CopilotHeadlessConfig::from_env`]); the `copilot --acp` subprocess spawns
/// lazily on the first vision call, so the runner is cheap to hold and harmless
/// when vision never fires.
#[cfg(feature = "provider-sciotte")]
fn build_sciotte_vision_llm() -> Option<Arc<dyn EmbacleLlmProvider>> {
    if matches!(
        ScraperConfig::default().login_mode,
        LoginMode::Hybrid | LoginMode::Vision
    ) {
        let config = CopilotHeadlessConfig::from_env();
        info!(model = %config.model, "Sciotte vision login enabled (Copilot headless)");
        Some(Arc::new(CopilotHeadlessRunner::with_config(config)))
    } else {
        None
    }
}

/// Centralized resource container for dependency injection.
///
/// Composed of 8 slice structs partitioning the ~50 shared Arc handles into
/// semantic groups. Handlers reach their dependencies through one of the slice
/// fields (e.g. `ctx.auth.auth_manager`, `ctx.fitness.provider_registry`) or
/// through the accessor / view-builder methods on this `impl` block.
#[derive(Clone)]
pub struct ServerContext {
    /// Cross-cutting handles + the master repository registry.
    pub common: CommonSlice,
    /// Authentication and authorization subsystem.
    pub auth: AuthSlice,
    /// Coach generation context.
    pub coach: CoachSlice,
    /// Fitness data and intelligence subsystem.
    pub fitness: FitnessSlice,
    /// Push-notification transports (SSE, AG-UI, sampling, progress).
    pub sse: SseSlice,
    /// Agent-to-agent protocol subsystem.
    pub a2a: A2ASlice,
    /// Billing subsystem.
    pub billing: BillingSlice,
    /// MCP tool dispatch internals (registries, prompts, contremaitre config).
    pub mcp: McpSlice,
}

impl ServerContext {
    /// Set the OAuth notification sender for push notifications
    pub fn set_oauth_notification_sender(
        &mut self,
        sender: broadcast::Sender<OAuthCompletedNotification>,
    ) {
        self.auth.oauth_notification_sender = Some(sender);
    }

    /// Set the sampling peer for server-initiated LLM requests (stdio transport only)
    pub fn set_sampling_peer(&mut self, peer: Arc<SamplingPeer>) {
        self.sse.sampling_peer = Some(peer);
    }

    /// Set the progress notification sender (stdio transport only)
    pub fn set_progress_notification_sender(
        &mut self,
        sender: mpsc::UnboundedSender<ProgressNotification>,
    ) {
        self.sse.progress_notification_sender = Some(sender);
    }

    /// Register a cancellation token for a progress token
    pub async fn register_cancellation_token(
        &self,
        progress_token: String,
        cancellation_token: CancellationToken,
    ) {
        let mut registry = self.sse.cancellation_registry.write().await;
        registry.insert(progress_token, cancellation_token);
    }

    /// Cancel an operation by progress token (called from MCP notifications/cancelled)
    pub async fn cancel_by_progress_token(&self, progress_token: &str) {
        let registry = self.sse.cancellation_registry.read().await;
        if let Some(token) = registry.get(progress_token) {
            info!(
                "Cancelling operation with progress token: {}",
                progress_token
            );
            token.cancel().await;
        } else {
            warn!(
                "Received cancellation for unknown progress token: {}",
                progress_token
            );
        }
    }

    /// Cleanup a cancellation token after operation completes
    pub async fn cleanup_cancellation_token(&self, progress_token: &str) {
        let mut registry = self.sse.cancellation_registry.write().await;
        registry.remove(progress_token);
    }

    /// Create a new builder for `ServerContext`
    #[must_use]
    pub const fn builder() -> ServerContextBuilder {
        ServerContextBuilder::new()
    }

    /// Get the group coaching service
    #[cfg(feature = "tools-groups")]
    #[must_use]
    pub fn group_service(&self) -> &pierre_groups::GroupService {
        &self.common.group_service
    }

    /// Get the coaches repository
    #[must_use]
    pub fn coaches_manager(&self) -> &dyn CoachesRepository {
        self.common.repos.coaches.as_ref()
    }

    /// Get the store listings repository
    #[must_use]
    pub fn store_listings_repository(&self) -> &dyn StoreListingsRepository {
        self.common.repos.store_listings.as_ref()
    }

    /// Get the recipe repository
    #[must_use]
    pub fn recipe_repository(&self) -> &dyn RecipeRepository {
        self.common.repos.recipes.as_ref()
    }

    /// Get the coaches repository (alias for compatibility)
    ///
    /// # Errors
    ///
    /// This method is infallible but returns `AppResult` for API compatibility.
    pub fn coaches_repository(&self) -> AppResult<&dyn CoachesRepository> {
        Ok(self.common.repos.coaches.as_ref())
    }

    /// Get the mobility repository
    #[must_use]
    pub fn mobility_repository(&self) -> &dyn MobilityRepository {
        self.common.repos.mobility.as_ref()
    }

    /// Get the social repository.
    ///
    /// # Errors
    ///
    /// Returns `AppError` if the backend is `PostgreSQL` (SQLite-only feature).
    pub fn social_repository(&self) -> AppResult<&dyn SocialRepository> {
        self.common
            .repos
            .social
            .as_deref()
            .ok_or_else(|| AppError::internal("SocialRepository is not available on PostgreSQL"))
    }

    /// Get the messaging channel registry
    #[cfg(feature = "client-messaging")]
    #[must_use]
    pub fn messaging_registry(&self) -> &ChannelRegistry {
        &self.common.messaging_registry
    }

    // ── Prompt registry delegation ─────────────────────────────────────
    // These methods provide access to system prompts from the contremaitre
    // registry, which hot-reloads from the GitHub-backed prompt repo and
    // seeds itself from the compiled-in `pierre-llm` constants as a fallback.

    /// Get the main Pierre fitness assistant system prompt.
    #[must_use]
    pub fn pierre_system_prompt(&self) -> String {
        self.mcp.prompt_registry.pierre_system_prompt()
    }

    /// Get the coach generation prompt.
    #[must_use]
    pub fn coach_generation_prompt(&self) -> String {
        self.mcp.prompt_registry.coach_generation_prompt()
    }

    /// Get the insight validation prompt.
    #[must_use]
    pub fn insight_validation_prompt(&self) -> String {
        self.mcp.prompt_registry.insight_validation_prompt()
    }

    /// Get the insight generation prompt.
    #[must_use]
    pub fn insight_generation_prompt(&self) -> String {
        self.mcp.prompt_registry.insight_generation_prompt()
    }

    /// Get the messaging context prompt.
    #[must_use]
    pub fn messaging_context_prompt(&self) -> String {
        self.mcp.prompt_registry.messaging_context_prompt()
    }

    /// Get the recommendation analysis prompt template.
    #[must_use]
    pub fn recommendation_analysis_prompt(&self) -> String {
        self.mcp.prompt_registry.recommendation_analysis_prompt()
    }

    /// Get the recommendation system prompt.
    #[must_use]
    pub fn recommendation_system_prompt(&self) -> String {
        self.mcp.prompt_registry.recommendation_system_prompt()
    }

    /// Get the activity analysis prompt template.
    #[must_use]
    pub fn activity_analysis_prompt(&self) -> String {
        self.mcp.prompt_registry.activity_analysis_prompt()
    }

    /// Get the activity analysis system prompt.
    #[must_use]
    pub fn activity_analysis_system_prompt(&self) -> String {
        self.mcp.prompt_registry.activity_analysis_system_prompt()
    }

    /// Get the mandatory tool-discipline prompt for non-messaging channels.
    #[must_use]
    pub fn tool_discipline_prompt(&self) -> String {
        self.mcp.prompt_registry.tool_discipline_prompt()
    }

    /// Get the mandatory tool-discipline prompt for messaging channels.
    #[must_use]
    pub fn tool_discipline_messaging_prompt(&self) -> String {
        self.mcp.prompt_registry.tool_discipline_messaging_prompt()
    }

    /// Get the memory extraction system prompt.
    #[must_use]
    pub fn memory_extraction_prompt(&self) -> String {
        self.mcp.prompt_registry.memory_extraction_prompt()
    }
}

// ─── pierre-routes-auth context builder ─────────────────────────────────
// Helper that materializes a `pierre_routes_auth::AuthRoutesContext` from
// the canonical `ServerContext`. Used by the composition root in
// `mcp::multitenant` and by every integration test that mounts
// `pierre_routes_auth::AuthRoutes::routes(...)` directly.
#[cfg(feature = "protocol-rest")]
impl ServerContext {
    /// Build an [`pierre_routes_auth::AuthRoutesContext`] view over this
    /// `ServerContext` — collects every Arc handle the auth route group
    /// needs (auth manager, JWKS, CSRF, repos, config, data, optional
    /// Firebase / Resend, OAuth notification sender, tenant OAuth client,
    /// provider registry, sync notifier, optional sync orchestrator,
    /// cache, admin JWT secret, and — under `provider-sciotte` — the
    /// hosted-login rate limiter and nonce store).
    #[must_use]
    pub fn auth_routes_context(&self) -> pierre_routes_auth::AuthRoutesContext {
        pierre_routes_auth::AuthRoutesContext {
            auth_manager: self.auth.auth_manager.clone(),
            jwks_manager: self.auth.jwks_manager.clone(),
            csrf_manager: self.auth.csrf_manager.clone(),
            auth_middleware: self.auth.auth_middleware.clone(),
            repos: self.common.repos.clone(),
            config: self.common.config.clone(),
            data: self.data(),
            firebase_auth: self.auth.firebase_auth.clone(),
            email_service: self.common.email_service.clone(),
            oauth_notification_sender: self.auth.oauth_notification_sender.clone(),
            tenant_oauth_client: self.auth.tenant_oauth_client.clone(),
            provider_registry: self.fitness.provider_registry.clone(),
            sync_notifier: self.sse.sse_manager.clone(),
            #[cfg(feature = "health-sync")]
            sync_orchestrator: self.fitness.sync_orchestrator.clone(),
            cache: self.common.cache.clone(),
            admin_jwt_secret: self.auth.admin_jwt_secret.clone(),
            #[cfg(feature = "provider-sciotte")]
            mint_rate_limiter: self.auth.mint_rate_limiter.clone(),
            #[cfg(feature = "provider-sciotte")]
            nonce_store: self.auth.nonce_store.clone(),
            #[cfg(feature = "provider-sciotte")]
            sciotte_vision_llm: build_sciotte_vision_llm(),
        }
    }

    /// Build a [`pierre_chat_pipeline::ChatPipelineContext`] view over this
    /// `ServerContext` — collects every Arc handle the chat pipeline stages
    /// need (repos, data, tool registry, tool runtime, config, admin JWT
    /// secret, optional admin config, chat / LLM providers, contremaitre
    /// registries, SSE manager, optional sync orchestrator, group service,
    /// LLM health, and the four prompt strings resolved through the
    /// hot-reloadable prompt registry).
    #[cfg(feature = "client-chat")]
    #[must_use]
    pub fn chat_pipeline_context(self: &Arc<Self>) -> pierre_chat_pipeline::ChatPipelineContext {
        use pierre_tool_runtime::runtime::ToolRuntime;
        let tool_runtime: Arc<dyn ToolRuntime> = Arc::clone(self) as _;
        let admin_config = self
            .coach
            .admin_config
            .as_ref()
            .map(|c| Arc::clone(c) as Arc<dyn pierre_runtime_context::AdminConfigLookup>);
        // Mirror the Copilot Headless `COPILOT_HEADLESS_MCP_TOOL_CALLING` toggle:
        // when set, ACP turns expose Dravr tools natively via the MCP bridge.
        let mcp_bridge_enabled = env::var("COPILOT_HEADLESS_MCP_TOOL_CALLING")
            .is_ok_and(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"));
        // The Copilot ACP subprocess runs inside this container, so it reaches
        // our MCP endpoint over loopback on the server's own HTTP port — NOT
        // `base_url`, which is the public/front-end origin (used for OAuth
        // redirects) and does not serve `/mcp`.
        let mcp_self_url = format!("http://localhost:{}", self.common.config.http_port);
        let mcp_bridge: Option<Arc<dyn McpBridgeProvider>> = Some(Arc::new(AcpMcpBridge::new(
            self.auth.auth_manager.clone(),
            self.auth.jwks_manager.clone(),
            self.common.repos.clone(),
            &mcp_self_url,
            mcp_bridge_enabled,
        )));
        pierre_chat_pipeline::ChatPipelineContext {
            repos: self.common.repos.clone(),
            data: self.data(),
            tool_registry: self.mcp.tool_registry.clone(),
            tool_runtime,
            config: self.common.config.clone(),
            admin_jwt_secret: self.auth.admin_jwt_secret.clone(),
            admin_config,
            chat_provider: self.common.chat_provider.clone(),
            llm_provider: self.common.llm_provider.clone(),
            prompt_registry: self.mcp.prompt_registry.clone(),
            tool_description_registry: self.mcp.tool_description_registry.clone(),
            evidence_registry: self.mcp.evidence_registry.clone(),
            messaging_strings_registry: self.mcp.messaging_strings_registry.clone(),
            cageux_config_registry: self.fitness.cageux_config_registry.clone(),
            harness_config_registry: self.fitness.harness_config_registry.clone(),
            persona_contract_registry: self.fitness.persona_contract_registry.clone(),
            sse_manager: self.sse.sse_manager.clone(),
            #[cfg(feature = "health-sync")]
            sync_orchestrator: self.fitness.sync_orchestrator.clone(),
            #[cfg(feature = "tools-groups")]
            group_service: self.common.group_service.clone(),
            llm_health: self.common.llm_health.clone(),
            pierre_system_prompt: self.pierre_system_prompt(),
            tool_discipline_prompt: self.tool_discipline_prompt(),
            tool_discipline_messaging_prompt: self.tool_discipline_messaging_prompt(),
            structured_output_prompt: STRUCTURED_OUTPUT_DIRECTIVE.to_owned(),
            structured_output_schema: STRUCTURED_WORKOUT_SCHEMA.to_owned(),
            memory_extraction_prompt: self.memory_extraction_prompt(),
            mcp_bridge,
        }
    }

    /// Resolve the chat provider a `(tenant, user)` should use for a turn,
    /// honoring a stored BYO LLM key, or `None` to use the global singleton.
    pub async fn resolve_byo_chat_provider(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> Option<Arc<ChatProvider>> {
        resolve_tenant_chat_provider(
            &self.common.tenant_chat_providers,
            self.common.repos.llm_credentials.as_ref(),
            self.common.repos.security.as_ref(),
            tenant_id,
            user_id,
        )
        .await
    }

    /// Build a [`pierre_routes_web_admin::WebAdminContext`] view over this
    /// `ServerContext` — collects every Arc handle the cookie-auth
    /// `/api/admin/*` route group needs (auth manager, JWKS, CSRF, auth
    /// middleware, repos, config, data context, admin JWT secret, and
    /// tool-selection service).
    #[cfg(feature = "client-admin-ui")]
    #[must_use]
    pub fn web_admin_context(&self) -> pierre_routes_web_admin::WebAdminContext {
        pierre_routes_web_admin::WebAdminContext {
            auth_manager: self.auth.auth_manager.clone(),
            jwks_manager: self.auth.jwks_manager.clone(),
            csrf_manager: self.auth.csrf_manager.clone(),
            auth_middleware: self.auth.auth_middleware.clone(),
            repos: self.common.repos.clone(),
            config: self.common.config.clone(),
            data: self.data(),
            admin_jwt_secret: self.auth.admin_jwt_secret.clone(),
            tool_selection: self.mcp.tool_selection.clone(),
            approval_notifier: {
                #[cfg(feature = "client-messaging")]
                {
                    Some(ApprovalNotifier::from_context(self))
                }
                #[cfg(not(feature = "client-messaging"))]
                {
                    None
                }
            },
        }
    }
}
