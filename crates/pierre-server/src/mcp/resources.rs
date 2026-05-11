// ABOUTME: ServerContext — the canonical dependency-injection container for the server
// ABOUTME: Holds every shared service (database, auth, providers, cache, …) plus narrow extractors
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
//! Eliminates anti-patterns of recreating expensive objects and excessive Arc cloning.

#[cfg(feature = "protocol-a2a")]
use crate::a2a::client::A2AClientManager;
#[cfg(feature = "protocol-a2a")]
use crate::a2a::system_user::A2ASystemUserService;
use crate::admin::FirebaseAuth;
use crate::agui::RunRegistry as AgUiRunRegistry;
use crate::cache::factory::Cache;
use crate::cageux_config::CageuxConfigRegistry;
use crate::commands;
use crate::config::admin::AdminConfigService;
use crate::config::environment::ServerConfig;
#[cfg(feature = "contremaitre")]
use crate::contremaitre::sync::full_sync;
#[cfg(feature = "contremaitre")]
use crate::contremaitre::{
    ContremaitreConfig, EvidenceRegistry, MessagingStringsRegistry, PromptRegistry,
    ToolDescriptionRegistry,
};
use crate::email::ResendEmailService;
use crate::errors::{AppError, AppResult};
use crate::harness_config_registry::HarnessConfigRegistry;
use crate::intelligence::{
    ActivityIntelligence, ContextualFactors, PerformanceMetrics, TimeOfDay, TrendDirection,
    TrendIndicators,
};
use crate::llm::LlmProvider;
use crate::mcp::sampling_peer::SamplingPeer;
use crate::mcp::schema::{OAuthCompletedNotification, ProgressNotification};
use crate::mcp::tool_selection::ToolSelectionService;
#[cfg(feature = "provider-sciotte")]
use crate::middleware::provider_link_token::{
    MintRateLimiter, NonceStore, MINT_RATE_LIMIT_PER_WINDOW, MINT_RATE_LIMIT_WINDOW_SECS,
};
use crate::middleware::redaction::RedactionConfig;
use crate::middleware::{CsrfMiddleware, McpAuthMiddleware};
use crate::persona_contracts::PersonaContractRegistry;
use crate::protocols::universal::types::CancellationToken;
use crate::providers::ProviderRegistry;
use crate::services::commands::{
    account::LogoutHandler,
    coach::{CoachAssignHandler, CoachListHandler, CoachSelectHandler},
    group::{
        GroupConsentHandler, GroupInviteHandler, GroupLeaveHandler, GroupListHandler,
        GroupMembersHandler, GroupStatusHandler,
    },
    help::HelpHandler,
    privacy::{PrivacyOffHandler, PrivacyOnHandler, PrivacyStatusHandler},
    status::StatusHandler,
    CommandHandlerRegistry,
};
use crate::services::embedding_sink::RepositoryEmbeddingSink;
use crate::services::pricing_loader;
use crate::services::usage_pruning::start_usage_pruning_task;
#[cfg(feature = "transport-sse")]
use crate::sse::SseManager;
use crate::tools::registry::ToolRegistry;
use crate::tools::traits::McpTool;
#[cfg(feature = "transport-websocket")]
use crate::websocket::WebSocketManager;
use chrono::Utc;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_auth::tenant::{oauth_manager::TenantOAuthManager, TenantOAuthClient};
use pierre_core::billing::{dummy::DummyProvider, BillingProvider};
use pierre_database::backends::factory::Database;
use pierre_database::backends::StoreListingsRepository;
use pierre_database::database::repositories::{
    CoachesRepository, MobilityRepository, RecipeRepository, SocialRepository,
};
use pierre_database::RepositoryRegistry;
#[cfg(feature = "tools-groups")]
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_llm::embeddings::{
    EmbeddingProvider, EmbeddingUsageSink, GeminiEmbeddingProvider, InstrumentedEmbeddingProvider,
};
use pierre_messaging::commands::CommandRegistry;
#[cfg(feature = "client-messaging")]
use pierre_messaging::ChannelRegistry;
#[cfg(feature = "client-notifications")]
use pierre_notifications::NotificationService;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::AbortHandle;
use tracing::{error, info, warn};

/// Optional initialization parameters for `ServerContext`
///
/// Used to pass optional configuration during server initialization without
/// exceeding function argument limits. All fields have sensible defaults.
#[derive(Default)]
pub struct ServerContextOptions {
    /// Size of RSA keys for JWT signing (2048 for tests, 4096 for production)
    pub rsa_key_size_bits: Option<usize>,
    /// Pre-existing JWKS manager (for test performance - reuses RSA keys)
    pub jwks_manager: Option<Arc<JwksManager>>,
    /// LLM provider for insight validation (injected for testing with mock providers)
    pub llm_provider: Option<Arc<dyn LlmProvider>>,
    /// Extra MCP tools to register in the default tool registry.
    ///
    /// Populated by messaging-eval integration tests that need a
    /// no-auth stub tool to exercise the tool-execution loop
    /// end-to-end without requiring a real provider connection.
    /// Production callers leave this empty — the default registry
    /// already holds every user-facing tool.
    pub extra_tools: Vec<Arc<dyn McpTool>>,
}

impl ServerContextOptions {
    /// Create options with production defaults (4096-bit RSA keys)
    #[must_use]
    pub fn production() -> Self {
        Self {
            rsa_key_size_bits: Some(4096),
            jwks_manager: None,
            llm_provider: None,
            extra_tools: Vec::new(),
        }
    }

    /// Create options for testing (2048-bit RSA keys for speed)
    #[must_use]
    pub fn testing() -> Self {
        Self {
            rsa_key_size_bits: Some(2048),
            jwks_manager: None,
            llm_provider: None,
            extra_tools: Vec::new(),
        }
    }

    /// Set the RSA key size
    #[must_use]
    pub const fn with_rsa_key_size(mut self, size: usize) -> Self {
        self.rsa_key_size_bits = Some(size);
        self
    }

    /// Set the JWKS manager
    #[must_use]
    pub fn with_jwks_manager(mut self, jwks: Arc<JwksManager>) -> Self {
        self.jwks_manager = Some(jwks);
        self
    }

    /// Set the LLM provider
    #[must_use]
    pub fn with_llm_provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(provider);
        self
    }
}

/// Centralized resource container for dependency injection
///
/// This struct holds all shared server resources to eliminate the anti-pattern
/// of recreating expensive objects like `AuthManager` and excessive Arc cloning.
#[derive(Clone)]
pub struct ServerContext {
    /// Database connection pool for persistent storage operations.
    ///
    /// Retained for lifecycle (migrations, encryption key updates), system settings,
    /// and pool access (`NotificationService`, `AdminConfigService`).
    /// Data access should go through `repos` instead.
    pub database: Arc<Database>,
    /// Trait-object repository registry — the primary data access layer.
    ///
    /// Each field holds `Arc<dyn XRepository>`, constructed once at startup
    /// from whichever backend the database enum wraps.
    pub repos: Arc<RepositoryRegistry>,
    /// Embedding provider wrapped in [`InstrumentedEmbeddingProvider`] so
    /// every `embed_for(tenant, user, text)` call writes a row into
    /// `embedding_usage` via the shared sink. `None` when no embedding
    /// provider key is configured (e.g. `GEMINI_API_KEY` unset).
    /// Memory and harness consumers must take this Arc instead of
    /// constructing `GeminiEmbeddingProvider` directly so embedding
    /// billing stays accurate.
    pub embedding_provider: Option<Arc<InstrumentedEmbeddingProvider>>,
    /// Authentication manager for user identity verification
    pub auth_manager: Arc<AuthManager>,
    /// JSON Web Key Set manager for RS256 JWT signing and verification
    pub jwks_manager: Arc<JwksManager>,
    /// Authentication middleware for MCP request validation
    pub auth_middleware: Arc<McpAuthMiddleware>,
    /// WebSocket connection manager for real-time updates
    #[cfg(feature = "transport-websocket")]
    pub websocket_manager: Arc<WebSocketManager>,
    /// Server-Sent Events manager for streaming notifications and MCP protocol
    #[cfg(feature = "transport-sse")]
    pub sse_manager: Arc<SseManager>,
    /// AG-UI (Agent-User Interaction) run registry. Chat pipeline
    /// publishes per-run broadcast channels here; SSE subscribers
    /// consume them via `/api/agui/runs/{run_id}/stream`.
    pub agui_registry: Arc<AgUiRunRegistry>,
    /// OAuth client for multi-tenant authentication flows
    pub tenant_oauth_client: Arc<TenantOAuthClient>,
    /// Registry of fitness data providers (Strava, Fitbit, Garmin, WHOOP, Terra)
    pub provider_registry: Arc<ProviderRegistry>,
    /// Secret key for admin JWT token generation
    pub admin_jwt_secret: Arc<str>,
    /// Server configuration loaded from environment
    pub config: Arc<ServerConfig>,
    /// Hot-swappable cageux intelligence config snapshot.
    ///
    /// Compiled-in defaults overlaid with env vars at startup,
    /// replaced by the contremaitre sync's `config/cageux.yaml`
    /// overlay when the feature is enabled. Every handler that needs
    /// an `IntelligenceConfig` reads it through this registry.
    pub cageux_config_registry: Arc<CageuxConfigRegistry>,
    /// Hot-swappable per-persona output-format conformance contracts.
    ///
    /// Hydrated from `config/persona_contracts.yaml` in dravr-contremaitre
    /// at startup and on every webhook push. The chat-pipeline
    /// `persona_conformance` stage reads it post-LLM-dispatch and
    /// surfaces violations as `warn!` events (or hard-blocks the reply
    /// when a persona's `strict_mode` is set).
    pub persona_contract_registry: Arc<PersonaContractRegistry>,
    /// Hot-swappable coaching harness config snapshot (compaction + Tier 6 guardrails).
    ///
    /// Loaded from `system_settings.harness_config` at startup. The chat
    /// pipeline reads `CompactionConfig` and `TextGuardrails` projections
    /// through this registry on every turn; `PUT /admin/settings/harness`
    /// calls `install` to swap the snapshot live without a restart.
    pub harness_config_registry: Arc<HarnessConfigRegistry>,
    /// AI-powered fitness activity analysis engine
    pub activity_intelligence: Arc<ActivityIntelligence>,
    /// A2A protocol client manager for agent-to-agent communication
    #[cfg(feature = "protocol-a2a")]
    pub a2a_client_manager: Arc<A2AClientManager>,
    /// Service for managing A2A system user accounts
    #[cfg(feature = "protocol-a2a")]
    pub a2a_system_user_service: Arc<A2ASystemUserService>,
    /// Broadcast channel for OAuth completion notifications
    pub oauth_notification_sender: Option<broadcast::Sender<OAuthCompletedNotification>>,
    /// Cache layer for performance optimization
    pub cache: Arc<Cache>,
    /// Configuration for PII redaction in logs and responses
    pub redaction_config: Arc<RedactionConfig>,
    /// Rate limiter for `OAuth2` endpoints
    pub oauth2_rate_limiter: Arc<OAuth2RateLimiter>,
    /// CSRF token manager for request forgery protection
    pub csrf_manager: Arc<CsrfTokenManager>,
    /// CSRF validation middleware
    pub csrf_middleware: Arc<CsrfMiddleware>,
    /// Optional sampling peer for server-initiated LLM requests (stdio transport only)
    pub sampling_peer: Option<Arc<SamplingPeer>>,
    /// Optional progress notification sender (stdio transport only)
    pub progress_notification_sender: Option<mpsc::UnboundedSender<ProgressNotification>>,
    /// Cancellation token registry for progress token -> cancellation token mapping
    pub cancellation_registry: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Firebase Authentication handler for social login (Google, Apple, etc.)
    pub firebase_auth: Option<Arc<FirebaseAuth>>,
    /// Admin configuration service for runtime parameter management
    pub admin_config: Option<Arc<AdminConfigService>>,
    /// Tool selection service for per-tenant MCP tool filtering
    pub tool_selection: Arc<ToolSelectionService>,
    /// Central registry for MCP tool discovery and execution
    pub tool_registry: Arc<ToolRegistry>,
    /// Optional LLM provider for insight validation and generation (injected for testing)
    pub llm_provider: Option<Arc<dyn LlmProvider>>,
    /// Abort handle for the background usage counter pruning task
    pub pruning_abort_handle: Option<AbortHandle>,
    /// Optional email service for transactional emails (password reset codes, etc.)
    pub email_service: Option<Arc<ResendEmailService>>,
    /// Multi-channel messaging registry for webhook routing
    #[cfg(feature = "client-messaging")]
    pub messaging_registry: Arc<ChannelRegistry>,
    /// Notification service facade for dispatch, scheduling, and persistence
    #[cfg(feature = "client-notifications")]
    pub notification_service: Option<Arc<NotificationService>>,
    /// Health data sync orchestrator for wearable provider synchronization
    #[cfg(feature = "health-sync")]
    pub sync_orchestrator: Option<Arc<dravr_enforme::SyncOrchestrator>>,
    /// Abort handle for the background health data sync scheduler task
    #[cfg(feature = "health-sync")]
    pub sync_scheduler_abort_handle: Option<AbortHandle>,
    /// Abort handle for the background notification scheduler task
    #[cfg(feature = "client-notifications")]
    pub scheduler_abort_handle: Option<AbortHandle>,
    /// Group coaching service for context injection and group management
    #[cfg(feature = "tools-groups")]
    pub group_service: Arc<pierre_groups::GroupService>,
    /// Messaging slash command registry (loaded from commands/*.md)
    #[cfg(feature = "client-messaging")]
    pub command_registry: Option<Arc<CommandRegistry>>,
    /// Messaging command handler registry
    #[cfg(feature = "client-messaging")]
    pub command_handler_registry: Option<Arc<CommandHandlerRegistry>>,
    /// Prompt registry for hot-reloadable system prompts and coach personas
    #[cfg(feature = "contremaitre")]
    pub prompt_registry: Arc<PromptRegistry>,
    /// Tool description registry for hot-reloadable MCP tool schema overlays
    #[cfg(feature = "contremaitre")]
    pub tool_description_registry: Arc<ToolDescriptionRegistry>,
    /// Evidence registry for hot-reloadable Tier 5.5 claim verification corpus
    #[cfg(feature = "contremaitre")]
    pub evidence_registry: Arc<EvidenceRegistry>,
    /// Messaging strings registry for hot-reloadable user-facing canned replies
    #[cfg(feature = "contremaitre")]
    pub messaging_strings_registry: Arc<MessagingStringsRegistry>,
    /// Contremaitre configuration for GitHub sync and webhook verification
    #[cfg(feature = "contremaitre")]
    pub contremaitre_config: Option<ContremaitreConfig>,
    /// Cache-backed one-time nonce store for link-token page loads
    #[cfg(feature = "provider-sciotte")]
    pub nonce_store: Arc<NonceStore>,
    /// Cache-backed rate limiter for link-token minting
    #[cfg(feature = "provider-sciotte")]
    pub mint_rate_limiter: Arc<MintRateLimiter>,
    /// Pluggable billing provider (Stripe / `RevenueCat` / Dummy / …).
    ///
    /// The platform binary picks one impl at startup and routes every
    /// `/api/billing/*` + `/webhooks/{provider}` call through it.
    /// Defaults to the in-tree [`DummyProvider`] when no real provider
    /// is wired — production binaries override this from a `dravr-*`
    /// vendor crate.
    pub billing_provider: Arc<dyn BillingProvider>,
    /// Shared LLM startup-probe state — populated by a background task
    /// spawned shortly after the context is built, and read by the
    /// `/ready` and `/health/llm` Axum handlers. Boot defaults to
    /// `LlmHealthStatus::Unknown` so the readiness gate stays open
    /// during the probe's first round-trip.
    pub llm_health: Arc<crate::health::LlmHealthState>,
}

/// Run a contremaitre full-sync against the freshly-built registries,
/// logging the active backend (gcs vs github) and the result/error.
///
/// Extracted from [`init_contremaitre_registries`] to keep that function's
/// cognitive-complexity budget under the workspace's clippy threshold;
/// the block contains an `if-let` plus `match`, which clippy counts as
/// two arms each.
#[cfg(feature = "contremaitre")]
#[allow(clippy::too_many_arguments)]
async fn run_contremaitre_full_sync(
    config: &ContremaitreConfig,
    prompt_registry: &Arc<PromptRegistry>,
    tool_desc_registry: &Arc<ToolDescriptionRegistry>,
    evidence_registry: &Arc<EvidenceRegistry>,
    cageux_config_registry: &Arc<CageuxConfigRegistry>,
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    persona_contract_registry: &Arc<PersonaContractRegistry>,
) {
    let store = config.store();
    info!(
        backend = store.backend_label(),
        "Contremaitre sync starting"
    );
    let outcome = full_sync(
        prompt_registry,
        tool_desc_registry,
        evidence_registry,
        cageux_config_registry,
        messaging_strings_registry,
        persona_contract_registry,
        store.as_ref(),
    )
    .await;
    match outcome {
        Ok(result) => info!(
            %result,
            backend = store.backend_label(),
            "Contremaitre sync complete"
        ),
        Err(e) => warn!(
            error = %e,
            backend = store.backend_label(),
            "Contremaitre sync failed, using compiled-in defaults"
        ),
    }
}

/// Initialize prompt, tool description, and evidence registries and sync
/// from contremaitre when configured.
///
/// The cageux config registry is passed in separately so that the cageux
/// snapshot exists whether or not the `contremaitre` feature is enabled.
/// When contremaitre IS enabled, its sync also populates the cageux
/// registry via the manifest's `config.cageux` entry.
#[cfg(feature = "contremaitre")]
async fn init_contremaitre_registries(
    cageux_config_registry: &Arc<CageuxConfigRegistry>,
    persona_contract_registry: &Arc<PersonaContractRegistry>,
) -> (
    Arc<PromptRegistry>,
    Arc<ToolDescriptionRegistry>,
    Arc<EvidenceRegistry>,
    Arc<MessagingStringsRegistry>,
) {
    let prompt_registry = Arc::new(PromptRegistry::new());
    let tool_desc_registry = Arc::new(ToolDescriptionRegistry::new());
    let evidence_registry = Arc::new(EvidenceRegistry::new());
    let messaging_strings_registry = Arc::new(MessagingStringsRegistry::new());

    if let Some(config) = ContremaitreConfig::from_env() {
        run_contremaitre_full_sync(
            &config,
            &prompt_registry,
            &tool_desc_registry,
            &evidence_registry,
            cageux_config_registry,
            &messaging_strings_registry,
            persona_contract_registry,
        )
        .await;
    } else {
        info!("Contremaitre not configured, using compiled-in defaults");
    }

    (
        prompt_registry,
        tool_desc_registry,
        evidence_registry,
        messaging_strings_registry,
    )
}

impl ServerContext {
    /// Create new server resources with proper Arc sharing
    ///
    /// # Parameters
    /// - `options`: Optional initialization parameters (RSA key size, JWKS manager, LLM provider)
    // Function exceeds line limit because it assembles 20+ interdependent resources
    // Splitting would reduce clarity without improving maintainability
    #[allow(clippy::too_many_lines)]
    pub async fn new(
        database: Database,
        auth_manager: AuthManager,
        admin_jwt_secret: &str,
        config: Arc<ServerConfig>,
        cache: Cache,
        options: ServerContextOptions,
    ) -> Self {
        let rsa_key_size_bits = options.rsa_key_size_bits.unwrap_or(4096);
        let jwks_manager = options.jwks_manager;
        let llm_provider = options.llm_provider;

        let database_arc = Arc::new(database);
        let repos = Arc::new(database_arc.repositories());

        let auth_manager_arc = Arc::new(auth_manager);

        // Create tenant OAuth client and provider registry once
        let tenant_oauth_client = Arc::new(TenantOAuthClient::new(TenantOAuthManager::new(
            Arc::new(config.oauth.clone()),
        )));
        let provider_registry = Arc::new(ProviderRegistry::new());

        // Seed the cageux config registry with the layered stack of
        // compiled-in defaults + INTELLIGENCE_* env vars. The contremaitre
        // sync (when enabled) will replace this snapshot once it fetches
        // the YAML overlay from `config/cageux.yaml`. The registry falls
        // back to compiled-in defaults if env parsing fails; startup-time
        // env validation is handled upstream by `init_all_configs()`.
        let cageux_config_registry = Arc::new(CageuxConfigRegistry::from_env());

        // Empty persona-contract registry; the contremaitre sync engine
        // hydrates it from `config/persona_contracts.yaml` either at
        // boot (full sync) or on the next webhook push that touches the
        // file. While empty, the chat-pipeline `persona_conformance`
        // stage no-ops cleanly — least-restrictive default per the
        // vault doc.
        let persona_contract_registry = Arc::new(PersonaContractRegistry::new());

        // Load the harness config snapshot from `system_settings.harness_config`.
        // Falls back to compile-time defaults if the row is absent or invalid;
        // the chat pipeline reads compaction + Tier 6 guardrails through this
        // registry, and `PUT /admin/settings/harness` calls `install` on it
        // after persisting a new document.
        let harness_config_registry =
            Arc::new(HarnessConfigRegistry::from_database(&database_arc).await);

        // Create activity intelligence once for shared use
        let activity_intelligence = Self::create_default_intelligence();

        // Create A2A services for agent-to-agent communication
        #[cfg(feature = "protocol-a2a")]
        let a2a_system_user_service = Arc::new(A2ASystemUserService::new(repos.users.clone()));
        #[cfg(feature = "protocol-a2a")]
        let a2a_client_manager = Arc::new(A2AClientManager::new(
            repos.clone(),
            a2a_system_user_service.clone(),
        ));

        // Wrap cache in Arc for shared access across handlers
        let cache_arc = Arc::new(cache);

        // Initialize PII redaction config from environment
        let redaction_config = Arc::new(RedactionConfig::from_env());
        info!(
            "Redaction middleware initialized: enabled={}",
            redaction_config.enabled
        );

        // Use provided JWKS manager or load/create new one for RS256 JWT signing
        let jwks_manager_arc =
            Self::resolve_jwks_manager(jwks_manager, &database_arc, rsa_key_size_bits).await;

        // Create websocket manager after jwks_manager is initialized
        #[cfg(feature = "transport-websocket")]
        let websocket_manager = Arc::new(WebSocketManager::new(
            repos.clone(),
            &auth_manager_arc,
            &jwks_manager_arc,
            config.rate_limiting.clone(),
        ));

        // Create SSE manager with configured buffer size
        #[cfg(feature = "transport-sse")]
        let sse_manager = Arc::new(SseManager::new(config.sse.max_buffer_size));

        // Initialize health data sync with Pierre-aware scheduler (needs sse_manager)
        #[cfg(feature = "health-sync")]
        let (sync_orchestrator, sync_scheduler_abort_handle) =
            Self::init_health_sync(&repos, &sse_manager);

        // Create auth middleware after jwks_manager is initialized
        let auth_middleware = Arc::new(McpAuthMiddleware::new(
            (*auth_manager_arc).clone(),
            repos.clone(),
            jwks_manager_arc.clone(),
            config.rate_limiting.clone(),
        ));

        // Create OAuth2 rate limiter once for shared use
        let oauth2_rate_limiter = Arc::new(OAuth2RateLimiter::from_rate_limit_config(
            config.rate_limiting.clone(),
        ));

        // Create stateless CSRF token manager (HMAC-signed, derived from admin JWT secret)
        let csrf_manager = Arc::new(CsrfTokenManager::from_jwt_secret(admin_jwt_secret));

        // Create CSRF validation middleware
        let csrf_middleware = Arc::new(CsrfMiddleware::new(csrf_manager.clone()));

        // Create Firebase auth handler if configured
        let firebase_auth = if config.firebase.is_configured() {
            Some(Arc::new(FirebaseAuth::new(config.firebase.clone())))
        } else {
            None
        };

        // Create admin config service if SQLite is available
        // This provides runtime-configurable parameters via admin API
        let admin_config = Self::init_admin_config_service(&database_arc).await;

        // Start background usage counter pruning task (hourly, removes records older than 90 days)
        let pruning_abort_handle = admin_config.as_ref().map(|config| {
            start_usage_pruning_task(Arc::clone(&repos.usage_counters), Arc::clone(config))
        });

        // Create tool selection service for per-tenant tool filtering
        let tool_selection = Arc::new(ToolSelectionService::new(repos.clone()));

        // Create email service if Resend credentials are configured
        let email_service = config
            .resend_api_key
            .as_ref()
            .zip(config.resend_from_email.as_ref())
            .map(|(api_key, from_email)| {
                info!("Resend email service configured");
                Arc::new(ResendEmailService::new(api_key.clone(), from_email.clone()))
            });
        if email_service.is_none() {
            warn!("Resend email service not configured — password reset emails will be skipped");
        }

        // Create notification service and start scheduler if notifications feature is enabled
        #[cfg(feature = "client-notifications")]
        let notification_service = Some(Self::create_notification_service(&database_arc));

        // Start the background notification scheduler if service is available
        #[cfg(feature = "client-notifications")]
        let scheduler_abort_handle = notification_service.as_ref().map(|s| s.start_scheduler());

        // Create group coaching service (before struct construction to avoid borrow-after-move)
        #[cfg(feature = "tools-groups")]
        let group_service = Arc::new(pierre_groups::GroupService::new(
            repos.groups.clone(),
            tier_strategy_for("professional"),
        ));

        // Load messaging slash commands from commands/ directory.
        //
        // `PIERRE_COMMANDS_DIR` overrides the default CWD-relative lookup so
        // tests and non-default deployments can point at an absolute path.
        #[cfg(feature = "client-messaging")]
        let (command_registry, command_handler_registry) = {
            let commands_dir_override = env::var("PIERRE_COMMANDS_DIR").ok();
            let commands_dir = commands_dir_override
                .as_deref()
                .map_or_else(|| Path::new("commands").to_path_buf(), PathBuf::from);
            let defs = commands::load_command_definitions(&commands_dir);
            let mut registry = CommandRegistry::new();
            for def in defs {
                registry.register(def);
            }
            let registry = Arc::new(registry);

            let mut handler_reg = CommandHandlerRegistry::new();
            handler_reg.register("help", Arc::new(HelpHandler::new(Arc::clone(&registry))));
            handler_reg.register("status", Arc::new(StatusHandler));
            handler_reg.register("logout", Arc::new(LogoutHandler));
            handler_reg.register("group", Arc::new(GroupListHandler));
            handler_reg.register("group-status", Arc::new(GroupStatusHandler));
            handler_reg.register("group-members", Arc::new(GroupMembersHandler));
            handler_reg.register("group-invite", Arc::new(GroupInviteHandler));
            handler_reg.register("group-leave", Arc::new(GroupLeaveHandler));
            handler_reg.register("group-consent", Arc::new(GroupConsentHandler));
            handler_reg.register("coach", Arc::new(CoachListHandler));
            handler_reg.register("coach-select", Arc::new(CoachSelectHandler));
            handler_reg.register("coach-assign", Arc::new(CoachAssignHandler));
            handler_reg.register("privacy", Arc::new(PrivacyStatusHandler));
            handler_reg.register("privacy-on", Arc::new(PrivacyOnHandler));
            handler_reg.register("privacy-off", Arc::new(PrivacyOffHandler));
            (Some(registry), Some(Arc::new(handler_reg)))
        };

        // Create and populate tool registry with all built-in tools. Any
        // `extra_tools` supplied via `ServerContextOptions` (used by
        // messaging-eval integration tests that need a no-auth stub
        // tool) land in the same registry as the built-ins, so the
        // pipeline's tool dispatcher can route to them with zero
        // special-casing.
        let tool_registry = {
            let mut registry = Self::create_tool_registry();
            for tool in options.extra_tools {
                registry.register(tool);
            }
            Arc::new(registry)
        };

        // Sync tool_catalog table with registry so tenant filtering always has complete data
        Self::run_tool_catalog_sync(&tool_registry, &repos).await;

        // Initialize contremaitre registries (prompts + tool descriptions +
        // evidence). The cageux config registry is passed in so the
        // contremaitre sync can also overlay its snapshot.
        #[cfg(feature = "contremaitre")]
        let (
            contremaitre_prompt_registry,
            contremaitre_tool_desc_registry,
            contremaitre_evidence_registry,
            contremaitre_messaging_strings_registry,
        ) = init_contremaitre_registries(&cageux_config_registry, &persona_contract_registry).await;

        // Cache-backed nonce store + rate limiter for channel-initiated provider links
        #[cfg(feature = "provider-sciotte")]
        let nonce_store = Arc::new(NonceStore::new(cache_arc.clone()));
        #[cfg(feature = "provider-sciotte")]
        let mint_rate_limiter = Arc::new(MintRateLimiter::new(
            MINT_RATE_LIMIT_PER_WINDOW,
            Duration::from_secs(MINT_RATE_LIMIT_WINDOW_SECS),
            cache_arc.clone(),
        ));

        // Phase 1 → 4: load admin pricing overrides into the process-wide
        // PricingRegistry before the first chat request can land. Failures
        // are logged inside the loader; the compile-time PRICING_TABLE
        // remains the safe fallback when no overrides exist.
        pricing_loader::load_pricing_overrides(repos.llm_credentials.as_ref()).await;

        // Phase 1 closer: build the instrumented embedding provider so any
        // future memory/harness consumer takes the wrapped form (which
        // writes an embedding_usage row per call) instead of constructing
        // a raw GeminiEmbeddingProvider. Skipped when GEMINI_API_KEY is
        // unset — embedding-driven features are best-effort.
        let embedding_provider = env::var("GEMINI_API_KEY").ok().map(|key| {
            let inner: Box<dyn EmbeddingProvider> = Box::new(GeminiEmbeddingProvider::new(key));
            let sink: Arc<dyn EmbeddingUsageSink> =
                Arc::new(RepositoryEmbeddingSink::new(Arc::clone(&repos.llm_usage)));
            Arc::new(InstrumentedEmbeddingProvider::new(inner, sink))
        });

        Self {
            database: database_arc,
            repos,
            embedding_provider,
            auth_manager: auth_manager_arc,
            jwks_manager: jwks_manager_arc,
            auth_middleware,
            #[cfg(feature = "transport-websocket")]
            websocket_manager,
            #[cfg(feature = "transport-sse")]
            sse_manager,
            agui_registry: Arc::new(AgUiRunRegistry::new()),
            tenant_oauth_client,
            provider_registry,
            admin_jwt_secret: admin_jwt_secret.into(),
            config,
            cageux_config_registry,
            persona_contract_registry,
            harness_config_registry,
            activity_intelligence,
            #[cfg(feature = "protocol-a2a")]
            a2a_client_manager,
            #[cfg(feature = "protocol-a2a")]
            a2a_system_user_service,
            oauth_notification_sender: None,
            cache: cache_arc,
            redaction_config,
            oauth2_rate_limiter,
            csrf_manager,
            csrf_middleware,
            sampling_peer: None,
            progress_notification_sender: None,
            cancellation_registry: Arc::new(RwLock::new(HashMap::new())),
            firebase_auth,
            admin_config,
            tool_selection,
            tool_registry,
            llm_provider,
            pruning_abort_handle,
            email_service,
            #[cfg(feature = "client-messaging")]
            messaging_registry: Arc::new(ChannelRegistry::new()),
            #[cfg(feature = "client-notifications")]
            notification_service,
            #[cfg(feature = "client-notifications")]
            scheduler_abort_handle,
            #[cfg(feature = "tools-groups")]
            group_service,
            #[cfg(feature = "client-messaging")]
            command_registry,
            #[cfg(feature = "client-messaging")]
            command_handler_registry,
            #[cfg(feature = "health-sync")]
            sync_orchestrator: Some(sync_orchestrator),
            #[cfg(feature = "health-sync")]
            sync_scheduler_abort_handle: Some(sync_scheduler_abort_handle),
            #[cfg(feature = "contremaitre")]
            prompt_registry: contremaitre_prompt_registry,
            #[cfg(feature = "contremaitre")]
            tool_description_registry: contremaitre_tool_desc_registry,
            #[cfg(feature = "contremaitre")]
            evidence_registry: contremaitre_evidence_registry,
            #[cfg(feature = "contremaitre")]
            messaging_strings_registry: contremaitre_messaging_strings_registry,
            #[cfg(feature = "contremaitre")]
            contremaitre_config: ContremaitreConfig::from_env(),
            #[cfg(feature = "provider-sciotte")]
            nonce_store,
            #[cfg(feature = "provider-sciotte")]
            mint_rate_limiter,
            // Default to the in-tree DummyProvider so platform binaries
            // compile and run without a vendor crate. Production binaries
            // override via ServerContextBuilder::with_billing_provider.
            billing_provider: Arc::new(DummyProvider::new()) as Arc<dyn BillingProvider>,
            llm_health: Arc::new(crate::health::LlmHealthState::new()),
        }
    }

    /// Create the notification service, dispatching to the appropriate backend
    #[cfg(feature = "client-notifications")]
    fn create_notification_service(database: &Arc<Database>) -> Arc<NotificationService> {
        let service = match database.as_ref() {
            Database::SQLite(db) => NotificationService::from_sqlite(db.pool().clone()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => NotificationService::from_postgres(db.pool().clone()),
        };
        info!("Notification service initialized");
        Arc::new(service)
    }

    /// Initialize the health data sync orchestrator and start the Pierre-aware scheduler.
    ///
    /// Uses Pierre's `start_scheduled_sync` instead of enforme's built-in scheduler
    /// to add post-sync behaviors: `last_sync` updates and SSE notifications.
    ///
    /// Returns the orchestrator and the abort handle for the scheduler task.
    #[cfg(feature = "health-sync")]
    fn init_health_sync(
        repos: &Arc<RepositoryRegistry>,
        sse_manager: &Arc<SseManager>,
    ) -> (Arc<dravr_enforme::SyncOrchestrator>, AbortHandle) {
        use crate::services::health_sync::PierreSyncStorage;
        use crate::services::provider_refresh::start_scheduled_sync;

        use crate::services::provider_rate_limiter::ProviderRateLimiter;

        let adapter = PierreSyncStorage::new(Arc::clone(repos));
        let orchestrator = adapter.into_orchestrator();
        let rate_limiter = Arc::new(ProviderRateLimiter::new());
        let abort_handle = start_scheduled_sync(
            Arc::clone(&orchestrator),
            Arc::clone(repos),
            Arc::clone(sse_manager),
            Some(rate_limiter),
        );
        info!("Health data sync scheduler started (Pierre-aware)");
        (orchestrator, abort_handle)
    }

    /// Create and initialize the tool registry with all built-in tools
    fn create_tool_registry() -> ToolRegistry {
        use tracing::info;

        let mut registry = ToolRegistry::new();
        registry.register_builtin_tools();

        // Log total schema token cost at startup for capacity planning
        let estimate = registry.total_schema_token_estimate();
        info!(
            event_type = "tool_schema_size",
            total_bytes = estimate.total_bytes,
            estimated_tokens = estimate.estimated_tokens,
            tool_count = estimate.tool_count,
            "Tool registry schema size at startup"
        );

        registry
    }

    /// Sync tool catalog table with the live tool registry at startup.
    async fn run_tool_catalog_sync(
        tool_registry: &Arc<ToolRegistry>,
        repos: &Arc<RepositoryRegistry>,
    ) {
        if let Err(e) =
            super::tool_selection::sync_tool_catalog(tool_registry, repos.tool_selection.as_ref())
                .await
        {
            tracing::warn!(error = %e, "Tool catalog sync failed, catalog may be incomplete");
        }
    }

    /// Create default activity intelligence for MCP server
    fn create_default_intelligence() -> Arc<ActivityIntelligence> {
        Arc::new(ActivityIntelligence::new(
            "MCP Intelligence".into(),
            vec![],
            PerformanceMetrics {
                relative_effort: Some(7.5),
                zone_distribution: None,
                personal_records: vec![],
                efficiency_score: Some(85.0),
                trend_indicators: TrendIndicators {
                    pace_trend: TrendDirection::Improving,
                    effort_trend: TrendDirection::Stable,
                    distance_trend: TrendDirection::Improving,
                    consistency_score: 8.2,
                },
            },
            ContextualFactors {
                weather: None,
                location: None,
                time_of_day: TimeOfDay::Morning,
                days_since_last_activity: Some(1),
                weekly_load: None,
                seasonal_context: None,
            },
        ))
    }

    /// Generate a unique key ID based on current timestamp
    fn generate_key_id() -> String {
        format!("key_{}", Utc::now().format("%Y%m%d_%H%M%S"))
    }

    /// Generate and persist a new RSA keypair
    async fn generate_and_persist_keypair(
        database: &Arc<Database>,
        jwks_manager: &mut JwksManager,
        rsa_key_size_bits: usize,
    ) -> AppResult<()> {
        let kid = Self::generate_key_id();
        jwks_manager.generate_rsa_key_pair_with_size(&kid, rsa_key_size_bits)?;

        let key = jwks_manager
            .get_active_key()
            .map_err(|e| AppError::internal(format!("Failed to get active key: {e}")))?;

        let private_pem = key.export_private_key_pem()?;
        let public_pem = key.export_public_key_pem()?;

        database
            .as_security_repository()
            .save_rsa_keypair(
                &kid,
                &private_pem,
                &public_pem,
                key.created_at,
                true,
                i32::try_from(rsa_key_size_bits).map_err(|e| {
                    AppError::internal(format!("RSA key size exceeds i32 maximum: {e}"))
                })?,
            )
            .await?;

        info!("Generated and persisted new RSA keypair: {}", kid);
        Ok(())
    }

    /// Initialize admin config service from the active database backend
    ///
    /// Returns None if initialization fails.
    async fn init_admin_config_service(
        database: &Arc<Database>,
    ) -> Option<Arc<AdminConfigService>> {
        let result = match database.as_ref() {
            Database::SQLite(db) => AdminConfigService::new(db.pool().clone()).await,
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => AdminConfigService::from_postgres(db.pool().clone()).await,
        };

        match result {
            Ok(service) => {
                info!("Admin configuration service initialized successfully");
                Some(Arc::new(service))
            }
            Err(e) => {
                warn!(
                    "Failed to initialize admin config service: {}. Runtime config will not be available.",
                    e
                );
                None
            }
        }
    }

    /// Resolve JWKS manager from provided instance or create new one
    ///
    /// Uses provided manager if available, otherwise loads from database or creates new keys.
    async fn resolve_jwks_manager(
        provided: Option<Arc<JwksManager>>,
        database: &Arc<Database>,
        rsa_key_size_bits: usize,
    ) -> Arc<JwksManager> {
        if let Some(mgr) = provided {
            return mgr;
        }

        match Self::load_or_create_jwks_manager(database, rsa_key_size_bits).await {
            Ok(jwks) => Arc::new(jwks),
            Err(e) => {
                error!(
                    "Failed to initialize JWKS manager: {}. Creating new keys without persistence.",
                    e
                );
                let mut new_jwks = JwksManager::new();
                if let Err(e) =
                    new_jwks.generate_rsa_key_pair_with_size("initial_key", rsa_key_size_bits)
                {
                    warn!(
                        "Failed to generate initial JWKS key pair: {}. RS256 tokens will not be available.",
                        e
                    );
                }
                Arc::new(new_jwks)
            }
        }
    }

    /// Load persisted RSA keys from database or create new ones
    ///
    /// # Errors
    /// Returns error if database operations fail
    async fn load_or_create_jwks_manager(
        database: &Arc<Database>,
        rsa_key_size_bits: usize,
    ) -> AppResult<JwksManager> {
        let mut jwks_manager = JwksManager::new();

        match database.as_security_repository().load_rsa_keypairs().await {
            Ok(keypairs) if !keypairs.is_empty() => {
                Self::load_existing_keys(&mut jwks_manager, keypairs)?;
            }
            Ok(_) => {
                Self::generate_new_keys(database, &mut jwks_manager, rsa_key_size_bits).await?;
            }
            Err(e) => {
                Self::fallback_generate_keys(&mut jwks_manager, rsa_key_size_bits, &e)?;
            }
        }

        Ok(jwks_manager)
    }

    fn load_existing_keys(
        jwks_manager: &mut JwksManager,
        keypairs: Vec<(String, String, String, chrono::DateTime<Utc>, bool)>,
    ) -> AppResult<()> {
        info!(
            "Loading {} persisted RSA keypairs from database",
            keypairs.len()
        );
        jwks_manager.load_keys_from_database(keypairs)?;
        info!("Successfully loaded RSA keys from database");
        Ok(())
    }

    async fn generate_new_keys(
        database: &Arc<Database>,
        jwks_manager: &mut JwksManager,
        rsa_key_size_bits: usize,
    ) -> AppResult<()> {
        info!("No persisted RSA keys found, generating new keypair");
        Self::generate_and_persist_keypair(database, jwks_manager, rsa_key_size_bits).await
    }

    fn fallback_generate_keys(
        jwks_manager: &mut JwksManager,
        rsa_key_size_bits: usize,
        error: &AppError,
    ) -> AppResult<()> {
        warn!(
            "Failed to load RSA keys from database: {}. Generating new keys without persistence.",
            error
        );
        let kid = Self::generate_key_id();
        jwks_manager.generate_rsa_key_pair_with_size(&kid, rsa_key_size_bits)?;
        Ok(())
    }

    /// Set the OAuth notification sender for push notifications
    pub fn set_oauth_notification_sender(
        &mut self,
        sender: broadcast::Sender<OAuthCompletedNotification>,
    ) {
        self.oauth_notification_sender = Some(sender);
    }

    /// Set the sampling peer for server-initiated LLM requests (stdio transport only)
    pub fn set_sampling_peer(&mut self, peer: Arc<SamplingPeer>) {
        self.sampling_peer = Some(peer);
    }

    /// Set the progress notification sender (stdio transport only)
    pub fn set_progress_notification_sender(
        &mut self,
        sender: mpsc::UnboundedSender<ProgressNotification>,
    ) {
        self.progress_notification_sender = Some(sender);
    }

    /// Register a cancellation token for a progress token
    pub async fn register_cancellation_token(
        &self,
        progress_token: String,
        cancellation_token: CancellationToken,
    ) {
        let mut registry = self.cancellation_registry.write().await;
        registry.insert(progress_token, cancellation_token);
    }

    /// Cancel an operation by progress token (called from MCP notifications/cancelled)
    pub async fn cancel_by_progress_token(&self, progress_token: &str) {
        let registry = self.cancellation_registry.read().await;
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
        let mut registry = self.cancellation_registry.write().await;
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
        &self.group_service
    }

    /// Get the coaches repository
    #[must_use]
    pub fn coaches_manager(&self) -> &dyn CoachesRepository {
        self.repos.coaches.as_ref()
    }

    /// Get the store listings repository
    #[must_use]
    pub fn store_listings_repository(&self) -> &dyn StoreListingsRepository {
        self.repos.store_listings.as_ref()
    }

    /// Get the recipe repository
    #[must_use]
    pub fn recipe_repository(&self) -> &dyn RecipeRepository {
        self.repos.recipes.as_ref()
    }

    /// Get the coaches repository (alias for compatibility)
    ///
    /// # Errors
    ///
    /// This method is infallible but returns `AppResult` for API compatibility.
    pub fn coaches_repository(&self) -> AppResult<&dyn CoachesRepository> {
        Ok(self.repos.coaches.as_ref())
    }

    /// Get the mobility repository
    #[must_use]
    pub fn mobility_repository(&self) -> &dyn MobilityRepository {
        self.repos.mobility.as_ref()
    }

    /// Get the social repository.
    ///
    /// # Errors
    ///
    /// Returns `AppError` if the backend is `PostgreSQL` (SQLite-only feature).
    pub fn social_repository(&self) -> AppResult<&dyn SocialRepository> {
        self.repos
            .social
            .as_deref()
            .ok_or_else(|| AppError::internal("SocialRepository is not available on PostgreSQL"))
    }

    /// Get the messaging channel registry
    #[cfg(feature = "client-messaging")]
    #[must_use]
    pub fn messaging_registry(&self) -> &ChannelRegistry {
        &self.messaging_registry
    }

    // ── Prompt registry delegation ─────────────────────────────────────
    // These methods provide access to system prompts from the contremaitre
    // registry (hot-reloadable) when the feature is enabled, or fall back
    // to compiled-in constants from `pierre-llm` when it is not.

    /// Get the main Pierre fitness assistant system prompt.
    #[must_use]
    pub fn pierre_system_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.pierre_system_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::PIERRE_SYSTEM_PROMPT.to_owned()
        }
    }

    /// Get the coach generation prompt.
    #[must_use]
    pub fn coach_generation_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.coach_generation_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::COACH_GENERATION_PROMPT.to_owned()
        }
    }

    /// Get the insight validation prompt.
    #[must_use]
    pub fn insight_validation_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.insight_validation_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::INSIGHT_VALIDATION_PROMPT.to_owned()
        }
    }

    /// Get the insight generation prompt.
    #[must_use]
    pub fn insight_generation_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.insight_generation_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::INSIGHT_GENERATION_PROMPT.to_owned()
        }
    }

    /// Get the messaging context prompt.
    #[must_use]
    pub fn messaging_context_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.messaging_context_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::MESSAGING_CONTEXT_PROMPT.to_owned()
        }
    }

    /// Get the recommendation analysis prompt template.
    #[must_use]
    pub fn recommendation_analysis_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.recommendation_analysis_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::RECOMMENDATION_ANALYSIS_PROMPT.to_owned()
        }
    }

    /// Get the recommendation system prompt.
    #[must_use]
    pub fn recommendation_system_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.recommendation_system_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::RECOMMENDATION_SYSTEM_PROMPT.to_owned()
        }
    }

    /// Get the activity analysis prompt template.
    #[must_use]
    pub fn activity_analysis_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.activity_analysis_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::ACTIVITY_ANALYSIS_PROMPT.to_owned()
        }
    }

    /// Get the activity analysis system prompt.
    #[must_use]
    pub fn activity_analysis_system_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.activity_analysis_system_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::ACTIVITY_ANALYSIS_SYSTEM_PROMPT.to_owned()
        }
    }

    /// Get the mandatory tool-discipline prompt for non-messaging channels.
    #[must_use]
    pub fn tool_discipline_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.tool_discipline_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::TOOL_DISCIPLINE_PROMPT.to_owned()
        }
    }

    /// Get the mandatory tool-discipline prompt for messaging channels.
    #[must_use]
    pub fn tool_discipline_messaging_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.tool_discipline_messaging_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::TOOL_DISCIPLINE_MESSAGING_PROMPT.to_owned()
        }
    }

    /// Get the memory extraction system prompt.
    #[must_use]
    pub fn memory_extraction_prompt(&self) -> String {
        #[cfg(feature = "contremaitre")]
        {
            self.prompt_registry.memory_extraction_prompt()
        }
        #[cfg(not(feature = "contremaitre"))]
        {
            pierre_llm::prompts::MEMORY_EXTRACTION_PROMPT.to_owned()
        }
    }
}

/// Builder pattern for `ServerContext` to avoid manual resource assembly anti-patterns
pub struct ServerContextBuilder {
    database: Option<Database>,
    auth_manager: Option<AuthManager>,
    admin_jwt_secret: Option<String>,
    config: Option<Arc<ServerConfig>>,
    cache: Option<Cache>,
    rsa_key_size_bits: usize,
    jwks_manager: Option<Arc<JwksManager>>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
}

impl ServerContextBuilder {
    /// Create a new builder with production defaults (4096-bit RSA keys)
    #[must_use]
    pub const fn new() -> Self {
        Self {
            database: None,
            auth_manager: None,
            admin_jwt_secret: None,
            config: None,
            cache: None,
            rsa_key_size_bits: 4096, // Production default
            jwks_manager: None,
            llm_provider: None,
        }
    }

    /// Set the database
    #[must_use]
    pub fn with_database(mut self, database: Database) -> Self {
        self.database = Some(database);
        self
    }

    /// Set the auth manager
    #[must_use]
    pub const fn with_auth_manager(mut self, auth_manager: AuthManager) -> Self {
        self.auth_manager = Some(auth_manager);
        self
    }

    /// Set the admin JWT secret
    #[must_use]
    pub fn with_admin_jwt_secret(mut self, admin_jwt_secret: impl Into<String>) -> Self {
        self.admin_jwt_secret = Some(admin_jwt_secret.into());
        self
    }

    /// Set the server configuration
    #[must_use]
    pub fn with_config(mut self, config: Arc<ServerConfig>) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the cache
    #[must_use]
    pub fn with_cache(mut self, cache: Cache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Set the RSA key size for JWT signing (2048 for tests, 4096 for production)
    #[must_use]
    pub const fn with_rsa_key_size_bits(mut self, rsa_key_size_bits: usize) -> Self {
        self.rsa_key_size_bits = rsa_key_size_bits;
        self
    }

    /// Set a pre-existing JWKS manager (for test performance - reuses RSA keys)
    #[must_use]
    pub fn with_jwks_manager(mut self, jwks_manager: Arc<JwksManager>) -> Self {
        self.jwks_manager = Some(jwks_manager);
        self
    }

    /// Set an LLM provider for insight validation (for testing with mock providers)
    #[must_use]
    pub fn with_llm_provider(mut self, llm_provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(llm_provider);
        self
    }

    /// Build the `ServerContext`
    ///
    /// # Errors
    ///
    /// Returns an error if any required fields are missing
    pub async fn build(self) -> Result<ServerContext, &'static str> {
        let database = self.database.ok_or("Database is required")?;
        let auth_manager = self.auth_manager.ok_or("AuthManager is required")?;
        let admin_jwt_secret = self
            .admin_jwt_secret
            .ok_or("Admin JWT secret is required")?;
        let config = self.config.ok_or("Server config is required")?;
        let cache = self.cache.ok_or("Cache is required")?;

        let options = ServerContextOptions {
            rsa_key_size_bits: Some(self.rsa_key_size_bits),
            jwks_manager: self.jwks_manager,
            llm_provider: self.llm_provider,
            extra_tools: Vec::new(),
        };

        let resources = ServerContext::new(
            database,
            auth_manager,
            &admin_jwt_secret,
            config,
            cache,
            options,
        )
        .await;
        Ok(resources)
    }

    /// Build the `ServerContext` wrapped in an `Arc`
    ///
    /// # Errors
    ///
    /// Returns an error if any required fields are missing
    pub async fn build_arc(self) -> Result<Arc<ServerContext>, &'static str> {
        Ok(Arc::new(self.build().await?))
    }
}

impl Default for ServerContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}
