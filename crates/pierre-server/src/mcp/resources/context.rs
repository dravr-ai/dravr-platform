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
//! ~50 shared Arc handles into semantic groups: `common`, `auth`, `agent`,
//! `fitness`, `sse`, `a2a`, `billing`, `mcp`.

#[cfg(feature = "client-chat")]
use std::env;

use super::slices::{
    A2ASlice, AgentSlice, AuthSlice, BillingSlice, CommonSlice, FitnessSlice, McpSlice, SseSlice,
};
// Gated on `client-chat`, the feature that owns the only consumer
// (`chat_pipeline_context`). It previously sat behind `provider-sciotte`, which
// did not match its use — that combination is not a supported build today, so
// nothing was broken, but the gate named the wrong thing.
#[cfg(feature = "client-chat")]
use dravr_contremaitre::schemas::DRAVR_VIZ_SCHEMA;
#[cfg(feature = "client-chat")]
use pierre_chat_pipeline::stages::viz_blocks;
#[cfg(feature = "client-chat")]
use pierre_chat_pipeline::stages::viz_schema::{self, SchemaTexts};
#[cfg(feature = "client-chat")]
use pierre_chat_pipeline::McpBridgeProvider;

#[cfg(feature = "client-chat")]
use super::tool_surface::HostedToolBridge;
#[cfg(feature = "client-messaging")]
use dravr_canot::ChannelRegistry;
use pierre_database::backends::StoreListingsRepository;
use pierre_database::database::repositories::AgentsRepository;
#[cfg(feature = "health-sync")]
use pierre_services::personal_bests::PersonalBests;
#[cfg(feature = "health-sync")]
use pierre_services::sync_failure_notice::SyncFailureNotices;
#[cfg(feature = "protocol-rest")]
use std::sync::Arc;

#[cfg(feature = "protocol-rest")]
use pierre_auth::google_oidc::{google_callback_url, GoogleOidcClient};
#[cfg(feature = "protocol-rest")]
use pierre_routes_identity::GoogleSignIn;
#[cfg(feature = "protocol-rest")]
use pierre_services::auth::AuthService;
#[cfg(feature = "protocol-rest")]
use tracing::info;

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
    /// Agent generation context.
    pub agent: AgentSlice,
    /// Fitness data and intelligence subsystem.
    pub fitness: FitnessSlice,
    /// Push-notification transports (SSE, AG-UI).
    pub sse: SseSlice,
    /// Agent-to-agent protocol subsystem.
    pub a2a: A2ASlice,
    /// Billing subsystem.
    pub billing: BillingSlice,
    /// MCP tool dispatch internals (registries, prompts, contremaitre config).
    pub mcp: McpSlice,
}

impl ServerContext {
    /// Get the group coaching service
    #[cfg(feature = "tools-groups")]
    #[must_use]
    pub fn group_service(&self) -> &pierre_groups::GroupService {
        &self.common.group_service
    }

    /// Get the agents repository
    #[must_use]
    pub fn agents_manager(&self) -> &dyn AgentsRepository {
        self.common.repos.agents.as_ref()
    }

    /// Get the store listings repository
    #[must_use]
    pub fn store_listings_repository(&self) -> &dyn StoreListingsRepository {
        self.common.repos.store_listings.as_ref()
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

    /// Get the agent generation prompt.
    #[must_use]
    pub fn agent_generation_prompt(&self) -> String {
        self.mcp.prompt_registry.agent_generation_prompt()
    }

    /// Get the messaging context prompt.
    #[must_use]
    pub fn messaging_context_prompt(&self) -> String {
        self.mcp.prompt_registry.messaging_context_prompt()
    }

    /// Get the plan-then-verify planner's workflow grammar.
    #[must_use]
    pub fn guardian_planner_prompt(&self) -> String {
        self.mcp.prompt_registry.guardian_planner_prompt()
    }

    /// Get the mandatory tool-discipline prompt for non-messaging channels.
    #[must_use]
    pub fn tool_discipline_prompt(&self) -> String {
        let registry = &self.mcp.prompt_registry;
        registry.tool_discipline_with_shared_rules(&registry.tool_discipline_prompt())
    }

    /// Get the mandatory tool-discipline prompt for messaging channels.
    #[must_use]
    pub fn tool_discipline_messaging_prompt(&self) -> String {
        let registry = &self.mcp.prompt_registry;
        registry.tool_discipline_with_shared_rules(&registry.tool_discipline_messaging_prompt())
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
    /// The sync-failure notices over this server's provider connections and
    /// notification service: the one notice a failing provider owes an
    /// athlete, re-armed when one of its syncs lands.
    #[cfg(feature = "health-sync")]
    #[must_use]
    pub fn sync_failure_notices(&self) -> SyncFailureNotices {
        #[cfg(feature = "client-notifications")]
        let service = self.common.notification_service.clone();
        #[cfg(not(feature = "client-notifications"))]
        let service = None;
        SyncFailureNotices::new(Arc::clone(&self.common.repos.provider_connections), service)
    }

    /// The athlete's personal bests over this server's repositories and
    /// notification service, paced by its provider rate limiter, with each
    /// athlete's measuring leased in the worker ledger: each run measured
    /// once, and a beaten all-time best told once the walk of the athlete's
    /// history is complete.
    #[cfg(feature = "health-sync")]
    #[must_use]
    pub fn personal_bests(&self) -> PersonalBests {
        #[cfg(feature = "client-notifications")]
        let service = self.common.notification_service.clone();
        #[cfg(not(feature = "client-notifications"))]
        let service = None;
        PersonalBests::new(
            Arc::clone(&self.common.repos.personal_bests),
            service,
            Arc::clone(&self.common.repos.worker_runs),
        )
    }

    /// Build an [`pierre_routes_auth::AuthRoutesContext`] view over this
    /// `ServerContext` — collects every Arc handle the auth route group
    /// needs (auth manager, JWKS, CSRF, repos, config, data, optional
    /// Firebase / Resend, OAuth notification sender, tenant OAuth client,
    /// provider registry, sync notifier, optional sync orchestrator,
    /// cache, string catalogue, admin JWT secret, and — under `provider-sciotte` — the
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
            provider_registry: self.fitness.provider_registry.clone(),
            sync_notifier: self.sse.sse_manager.clone(),
            #[cfg(feature = "health-sync")]
            sync_orchestrator: self.fitness.sync_orchestrator.clone(),
            #[cfg(feature = "health-sync")]
            sync_failure_notices: self.sync_failure_notices(),
            cache: self.common.cache.clone(),
            messaging_strings: self.mcp.messaging_strings_registry.clone(),
            admin_jwt_secret: self.auth.admin_jwt_secret.clone(),
            #[cfg(feature = "provider-sciotte")]
            nonce_store: self.auth.nonce_store.clone(),
        }
    }

    /// The account rules the hosted OAuth 2.0 login page signs athletes in
    /// through: the same [`AuthService`] the web app's sign-in routes build.
    #[must_use]
    pub fn oauth2_accounts(&self) -> AuthService {
        AuthService::new(
            self.auth.auth_manager.clone(),
            self.auth.jwks_manager.clone(),
            self.common.config.clone(),
            self.data(),
        )
    }

    /// "Continue with Google" on the hosted OAuth 2.0 login page, when
    /// `GOOGLE_OAUTH_CLIENT_ID`/`GOOGLE_OAUTH_CLIENT_SECRET` configure it.
    ///
    /// Logs, once per router it is built for, whether it is on, and the
    /// redirect URI to register on the Google client; the secret appears only
    /// as its length and fingerprint.
    #[must_use]
    pub fn oauth2_google_sign_in(&self) -> Option<GoogleSignIn> {
        let oauth2_server = &self.common.config.oauth2_server;
        let Some(google) = oauth2_server.google_sign_in.clone() else {
            info!("Google sign-in on the OAuth login page: disabled (GOOGLE_OAUTH_CLIENT_ID/SECRET unset)");
            return None;
        };
        info!(
            client_id = %google.client_id,
            secret_length = google.client_secret.len(),
            secret_fingerprint = %google.secret_fingerprint(),
            redirect_uri = %google_callback_url(&oauth2_server.issuer_url),
            "Google sign-in on the OAuth login page: enabled"
        );
        Some(GoogleSignIn {
            oidc: Arc::new(GoogleOidcClient::new(*google)),
            security: self.common.repos.security.clone(),
        })
    }

    /// Build a [`pierre_chat_pipeline::ChatPipelineContext`] view over this
    /// `ServerContext` — collects every Arc handle the chat pipeline stages
    /// need (repos, data, tool registry, tool runtime, config, admin JWT
    /// secret, optional admin config, chat / LLM providers, contremaitre
    /// registries, SSE manager, optional sync orchestrator, group service,
    /// LLM health, and the prompt strings resolved through the
    /// hot-reloadable prompt registry).
    #[cfg(feature = "client-chat")]
    #[must_use]
    pub fn chat_pipeline_context(self: &Arc<Self>) -> pierre_chat_pipeline::ChatPipelineContext {
        use pierre_tool_runtime::runtime::ToolRuntime;
        let tool_runtime: Arc<dyn ToolRuntime> = Arc::clone(self) as _;
        let admin_config = self
            .agent
            .admin_config
            .as_ref()
            .map(|c| Arc::clone(c) as Arc<dyn pierre_runtime_context::AdminConfigLookup>);
        // The bridge follows the Copilot providers' own tool-calling toggles:
        // either one makes its provider route turns to the headless loop with
        // no text catalogue in the prompt, so a provider armed without the
        // bridge would answer with no tools at all.
        let mcp_bridge_enabled = [
            "COPILOT_HEADLESS_MCP_TOOL_CALLING",
            "COPILOT_SDK_MCP_TOOL_CALLING",
        ]
        .iter()
        .any(|flag| {
            env::var(flag).is_ok_and(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
        });
        // The tool surface is published on embacle's own loopback listener, so
        // the server no longer needs to know its own reachable address — the
        // subprocess never dials back into `/mcp`.
        let mcp_bridge: Option<Arc<dyn McpBridgeProvider>> = Some(Arc::new(HostedToolBridge::new(
            mcp_bridge_enabled,
            self.mcp.tool_registry.clone(),
            self.common.repos.clone(),
            tool_runtime.clone(),
        )));
        let command_ctx: Arc<dyn pierre_runtime_context::CommandCtx> = Arc::clone(self) as _;
        pierre_chat_pipeline::ChatPipelineContext {
            repos: self.common.repos.clone(),
            data: self.data(),
            tool_registry: self.mcp.tool_registry.clone(),
            tool_runtime,
            command_ctx,
            command_registry: self.common.command_registry.clone(),
            command_handler_registry: self.common.command_handler_registry.clone(),
            tenant_chat_providers: self.common.tenant_chat_providers.clone(),
            config: self.common.config.clone(),
            admin_jwt_secret: self.auth.admin_jwt_secret.clone(),
            admin_config,
            chat_provider: self.common.chat_provider.clone(),
            llm_provider: self.common.llm_provider.clone(),
            prompt_registry: self.mcp.prompt_registry.clone(),
            tool_description_registry: self.mcp.tool_description_registry.clone(),
            evidence_registry: self.mcp.evidence_registry.clone(),
            training_catalogue_registry: self.mcp.training_catalogue_registry.clone(),
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
            visual_blocks_prompt: self.visual_blocks_prompt(),
            viz_schemas: Self::viz_schemas(),
            memory_extraction_prompt: self.memory_extraction_prompt(),
            mcp_bridge,
        }
    }

    /// The inline-visual directive: contremaitre's prose plus the bounds read
    /// off the schema that actually validates the blocks.
    ///
    /// Split by what is derivable. The prose carries judgement — when a visual
    /// earns its place, that the interpretation goes in the sentence — which no
    /// schema encodes. The limits are generated, because transcribing them by
    /// hand is what failed: the prose states the maxima and omits that a chart
    /// series needs at least two points, so an agent writing a two-athlete
    /// comparison as one series per athlete had its block refused on every
    /// attempt while the athlete saw prose and no chart (2026-08-31).
    ///
    /// The prose comes from the hot-reload prompt registry, like every other
    /// system prompt on this path, so an edit to `visual_blocks.md` reaches the
    /// next turn through the webhook rather than through a rev bump and a
    /// redeploy.
    #[cfg(feature = "client-chat")]
    fn visual_blocks_prompt(&self) -> String {
        let directive = self.mcp.prompt_registry.visual_blocks_prompt();
        let generated = viz_blocks::schema_contract(&Self::viz_schemas());
        if generated.is_empty() {
            return directive;
        }
        format!("{directive}\n\n{generated}")
    }

    /// Every block schema the pipeline can validate against, keyed by the id a
    /// block names in its fence.
    ///
    /// Lives here because this is where contremaitre data enters the pipeline
    /// context — the same path the system prompts take. Adding a schema is one
    /// line plus a contremaitre constant; the registry compiles what it is given.
    #[cfg(feature = "client-chat")]
    fn viz_schemas() -> SchemaTexts {
        SchemaTexts::from([(
            viz_schema::DRAVR_VIZ.to_owned(),
            DRAVR_VIZ_SCHEMA.to_owned(),
        )])
    }

    /// Build a [`pierre_routes_web_admin::WebAdminContext`] view over this
    /// `ServerContext` — collects every Arc handle the cookie-auth
    /// `/api/admin/*` route group needs (auth manager, JWKS, CSRF, auth
    /// middleware, repos, data context, and tool-selection service).
    #[cfg(feature = "client-admin-ui")]
    #[must_use]
    pub fn web_admin_context(&self) -> pierre_routes_web_admin::WebAdminContext {
        pierre_routes_web_admin::WebAdminContext {
            auth_manager: self.auth.auth_manager.clone(),
            jwks_manager: self.auth.jwks_manager.clone(),
            csrf_manager: self.auth.csrf_manager.clone(),
            auth_middleware: self.auth.auth_middleware.clone(),
            repos: self.common.repos.clone(),
            data: self.data(),
            tool_selection: self.mcp.tool_selection.clone(),
        }
    }
}
