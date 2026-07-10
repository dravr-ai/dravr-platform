// ABOUTME: ServerContext::new constructor + private bootstrap helpers (JWKS keys, notification service, tool catalog, health sync)
// ABOUTME: Assembles the ~50 shared Arc handles the runtime injects into every handler from raw startup ingredients
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::contremaitre::init_contremaitre_registries;
use super::ServerContext;
use super::ServerContextOptions;
#[cfg(feature = "protocol-a2a")]
use crate::a2a::client::A2AClientManager;
#[cfg(feature = "protocol-a2a")]
use crate::a2a::system_user::A2ASystemUserService;
use crate::agui::RunRegistry as AgUiRunRegistry;
use crate::config::admin::AdminConfigService;
#[cfg(feature = "client-messaging")]
use crate::services::backfill_notifier::{ChatReentry, ServerBackfillNotifier};
use chrono::Utc;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_auth::firebase::FirebaseAuth;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_auth::tenant::{oauth_manager::TenantOAuthManager, TenantOAuthClient};
use pierre_cache::Cache;
#[cfg(feature = "client-messaging")]
use pierre_commands as commands;
#[cfg(feature = "client-messaging")]
use pierre_commands::{
    account::LogoutHandler,
    coach::{CoachAssignHandler, CoachListHandler, CoachSelectHandler},
    group::{
        GroupCoachHandler, GroupConsentHandler, GroupInviteHandler, GroupLeaveHandler,
        GroupListHandler, GroupMembersHandler, GroupStatusHandler,
    },
    help::HelpHandler,
    onboarding::ContextHandler,
    privacy::{PrivacyOffHandler, PrivacyOnHandler, PrivacyStatusHandler},
    status::StatusHandler,
    timezone::TimezoneHandler,
    CommandHandlerRegistry,
};
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::ContremaitreConfig;
use pierre_core::billing::{dummy::DummyProvider, BillingProvider};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
#[cfg(feature = "tools-groups")]
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_intelligence::{
    ActivityIntelligence, ContextualFactors, PerformanceMetrics, TimeOfDay, TrendDirection,
    TrendIndicators,
};
use pierre_llm::embeddings::{
    EmbeddingProvider, EmbeddingUsageSink, GeminiEmbeddingProvider, InstrumentedEmbeddingProvider,
};
use pierre_llm::health::LlmHealthState;
use pierre_llm::ChatProvider;
#[cfg(feature = "client-messaging")]
use pierre_messaging::commands::CommandRegistry;
#[cfg(feature = "client-messaging")]
use pierre_messaging::ChannelRegistry;
#[cfg(feature = "provider-sciotte")]
use pierre_middleware::provider_link_token::{
    MintRateLimiter, NonceStore, MINT_RATE_LIMIT_PER_WINDOW, MINT_RATE_LIMIT_WINDOW_SECS,
};
use pierre_middleware::redaction::RedactionConfig;
use pierre_middleware::McpAuthMiddleware;
#[cfg(feature = "client-notifications")]
use pierre_notifications::NotificationService;
use pierre_providers::registry::ProviderRegistry;
use pierre_services::embedding_sink::RepositoryEmbeddingSink;
#[cfg(feature = "health-sync")]
use pierre_services::health_sync::PierreSyncStorage;
use pierre_services::pricing_loader;
use pierre_services::tenant_chat_provider::TenantChatProviderCache;
use pierre_services::usage_pruning::start_usage_pruning_task;
#[cfg(feature = "transport-sse")]
use pierre_sse::SseManager;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_selection::ToolSelectionService;
use std::collections::HashMap;
use std::env;
#[cfg(feature = "client-messaging")]
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "client-messaging")]
use std::sync::OnceLock;
#[cfg(feature = "provider-sciotte")]
use std::time::Duration;
use tokio::sync::RwLock;
#[cfg(feature = "health-sync")]
use tokio::task::AbortHandle;
use tracing::{error, info, warn};

impl ServerContext {
    /// Create new server resources with proper Arc sharing
    ///
    /// # Parameters
    /// - `options`: Optional initialization parameters (RSA key size, JWKS manager, LLM provider)
    // Function exceeds line limit because it assembles 20+ interdependent resources
    // Splitting would reduce clarity without improving maintainability
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
        // Honor explicit chat_provider passthrough first; otherwise wrap
        // a test-injected llm_provider so test paths keep working without
        // pre-building a ChatProvider. Production callers always pass
        // a pre-built ChatProvider (the binary does this in `main`).
        let chat_provider: Option<Arc<ChatProvider>> = options.chat_provider.or_else(|| {
            llm_provider
                .as_ref()
                .map(|p| Arc::new(ChatProvider::Custom(Arc::clone(p))))
        });

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
        let a2a_auth_repos = repos.auth_repos();
        #[cfg(feature = "protocol-a2a")]
        let a2a_client_manager = Arc::new(A2AClientManager::new(
            &a2a_auth_repos,
            a2a_system_user_service.clone(),
        ));

        // Wrap cache in Arc for shared access across handlers
        let cache_arc = Arc::new(cache);

        // Initialize PII redaction config from runtime ServerConfig
        let redaction_config = Arc::new(RedactionConfig::new(
            config.logging.redact_pii,
            config.logging.redaction_placeholder.clone(),
        ));
        info!(
            "Redaction middleware initialized: enabled={}",
            redaction_config.enabled
        );

        // Use provided JWKS manager or load/create new one for RS256 JWT signing
        let jwks_manager_arc =
            Self::resolve_jwks_manager(jwks_manager, &database_arc, rsa_key_size_bits).await;

        // Create SSE manager with configured buffer size
        #[cfg(feature = "transport-sse")]
        let sse_manager = Arc::new(SseManager::new(config.sse.max_buffer_size));

        // Initialize health data sync with Pierre-aware scheduler (needs sse_manager)
        #[cfg(feature = "health-sync")]
        let (sync_storage, sync_orchestrator, sync_scheduler_abort_handle) =
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

        // Create Firebase auth handler if configured
        let firebase_auth = config
            .firebase
            .is_configured()
            .then(|| Arc::new(FirebaseAuth::new(config.firebase.clone())));

        // Create admin config service if SQLite is available
        // This provides runtime-configurable parameters via admin API
        let admin_config = Self::init_admin_config_service(&database_arc).await;

        // Start background usage counter pruning task (hourly, removes records older than 90 days)
        let pruning_abort_handle = admin_config.as_ref().map(|config| {
            let lookup: Arc<dyn pierre_runtime_context::AdminConfigLookup> =
                Arc::clone(config) as Arc<dyn pierre_runtime_context::AdminConfigLookup>;
            start_usage_pruning_task(Arc::clone(&repos.usage_counters), lookup)
        });

        // Create tool selection service for per-tenant tool filtering
        let tool_selection = Arc::new(ToolSelectionService::new(&repos));

        // Build the outbound email service (gated on ServerConfig: non-empty
        // Resend creds + not in CI mode). See `build_email_service`.
        let email_service = Self::build_email_service(&config);

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

        // Load messaging slash commands + handlers (see `build_command_registries`).
        #[cfg(feature = "client-messaging")]
        let (command_registry, command_handler_registry) = Self::build_command_registries();

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
        // contremaitre sync can also overlay its snapshot. The GitHub/GCS
        // overlay runs in the background (off this bind path), so a slow sync
        // never stalls startup; registries serve compiled-in defaults until
        // the first background tick converges.
        let (
            contremaitre_prompt_registry,
            contremaitre_tool_desc_registry,
            contremaitre_evidence_registry,
            contremaitre_messaging_strings_registry,
        ) = init_contremaitre_registries(&cageux_config_registry, &persona_contract_registry);

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

        // Precompute view-struct projections from the master registry once;
        // each is an Arc-clone-only operation. The full registry continues
        // to live in `CommonSlice.repos` as the single source of truth.
        let auth_repos_view = repos.auth_repos();
        let coach_repos_view = repos.coach_repos();
        let fitness_repos_view = repos.fitness_repos();
        let usage_repos_view = repos.usage_repos();

        // Capture the public base URL before `config` moves into CommonSlice —
        // the backfill notifier needs it (with the admin JWT secret) to mint the
        // hosted-login link for a provider-reauth nudge.
        let base_url = config.base_url.clone();
        let common = super::slices::CommonSlice {
            repos,
            cache: cache_arc,
            config,
            redaction_config,
            email_service,
            llm_provider,
            chat_provider,
            tenant_chat_providers: TenantChatProviderCache::new(),
            llm_health: Arc::new(LlmHealthState::new()),
            embedding_provider,
            #[cfg(feature = "client-messaging")]
            messaging_registry: Arc::new(ChannelRegistry::new()),
            #[cfg(feature = "client-notifications")]
            notification_service,
            #[cfg(feature = "client-notifications")]
            scheduler_abort_handle,
            pruning_abort_handle,
            #[cfg(feature = "tools-groups")]
            group_service,
            #[cfg(feature = "client-messaging")]
            command_registry,
            #[cfg(feature = "client-messaging")]
            command_handler_registry,
        };

        let auth = super::slices::AuthSlice {
            auth_manager: auth_manager_arc,
            jwks_manager: jwks_manager_arc,
            auth_middleware,
            csrf_manager,
            firebase_auth,
            oauth2_rate_limiter,
            admin_jwt_secret: admin_jwt_secret.into(),
            tenant_oauth_client,
            oauth_notification_sender: None,
            #[cfg(feature = "provider-sciotte")]
            nonce_store,
            #[cfg(feature = "provider-sciotte")]
            mint_rate_limiter,
            repos: auth_repos_view,
        };

        let coach = super::slices::CoachSlice {
            database: database_arc,
            admin_config,
            repos: coach_repos_view,
        };

        let fitness = super::slices::FitnessSlice {
            provider_registry,
            activity_intelligence,
            #[cfg(feature = "health-sync")]
            sync_orchestrator: Some(sync_orchestrator),
            #[cfg(feature = "health-sync")]
            sync_storage: Some(sync_storage),
            #[cfg(feature = "health-sync")]
            sync_scheduler_abort_handle: Some(sync_scheduler_abort_handle),
            cageux_config_registry,
            harness_config_registry,
            persona_contract_registry,
            repos: fitness_repos_view,
        };

        let sse = super::slices::SseSlice {
            #[cfg(feature = "transport-sse")]
            sse_manager,
            agui_registry: Arc::new(AgUiRunRegistry::new()),
            sampling_peer: None,
            progress_notification_sender: None,
            cancellation_registry: Arc::new(RwLock::new(HashMap::new())),
        };

        let a2a = super::slices::A2ASlice {
            #[cfg(feature = "protocol-a2a")]
            a2a_client_manager,
            #[cfg(feature = "protocol-a2a")]
            a2a_system_user_service,
        };

        // Use the injected billing provider when present (production binaries
        // pass StripeProvider when Stripe env is configured). Otherwise fall
        // back to the in-tree DummyProvider so the platform compiles, tests
        // run, and local dev works without a vendor crate configured.
        let billing_provider = options
            .billing_provider
            .unwrap_or_else(|| Arc::new(DummyProvider::new()) as Arc<dyn BillingProvider>);
        let billing = super::slices::BillingSlice {
            billing_provider,
            repos: usage_repos_view,
        };

        // Best-effort backfill-completion notifier, built from the same shared
        // repos + strings registry the approval notifier uses. Stored on the
        // context so the detached historical-backfill task can push a "your
        // history is ready" notice back to the originating channel.
        //
        // The chat-pipeline re-entry handle (which lets that push synthesize a
        // real coach answer) needs the composition-root `Arc<ServerContext>`,
        // which doesn't exist yet — so the notifier and the context share this
        // empty `OnceLock` slot, filled post-`Arc` by `install_backfill_reentry`.
        #[cfg(feature = "client-messaging")]
        let backfill_reentry: Arc<OnceLock<Arc<dyn ChatReentry>>> = Arc::new(OnceLock::new());
        #[cfg(feature = "client-messaging")]
        let backfill_notifier = Some(ServerBackfillNotifier::from_handles(
            common.repos.clone(),
            contremaitre_messaging_strings_registry.clone(),
            backfill_reentry.clone(),
            admin_jwt_secret.into(),
            base_url,
        ));

        let mcp = super::slices::McpSlice {
            tool_registry,
            tool_selection,
            prompt_registry: contremaitre_prompt_registry,
            tool_description_registry: contremaitre_tool_desc_registry,
            evidence_registry: contremaitre_evidence_registry,
            messaging_strings_registry: contremaitre_messaging_strings_registry,
            #[cfg(feature = "client-messaging")]
            backfill_notifier,
            #[cfg(feature = "client-messaging")]
            backfill_reentry,
            contremaitre_config: ContremaitreConfig::from_env(),
        };

        Self {
            common,
            auth,
            coach,
            fitness,
            sse,
            a2a,
            billing,
            mcp,
        }
    }

    /// Build the outbound transactional email service, or `None` when Resend is
    /// unconfigured or the server is in CI mode.
    ///
    /// The decision lives entirely in [`ServerConfig::outbound_email_credentials`]
    /// (non-empty creds + not CI); this just constructs the service or logs why
    /// it was skipped, keeping `new` free of the branching.
    fn build_email_service(config: &ServerConfig) -> Option<Arc<ResendEmailService>> {
        let Some((api_key, from_email)) = config.outbound_email_credentials() else {
            if config.is_ci_mode() {
                info!("CI mode active — outbound Resend email disabled (no transactional email will be sent)");
            } else {
                warn!(
                    "Resend email service not configured — password reset emails will be skipped"
                );
            }
            return None;
        };
        info!("Resend email service configured");
        Some(Arc::new(ResendEmailService::new(
            api_key.to_owned(),
            from_email.to_owned(),
        )))
    }

    /// Load messaging slash-command definitions and register their handlers.
    ///
    /// `PIERRE_COMMANDS_DIR` overrides the default CWD-relative `commands/`
    /// lookup so tests and non-default deployments can point at an absolute path.
    #[cfg(feature = "client-messaging")]
    fn build_command_registries() -> (
        Option<Arc<CommandRegistry>>,
        Option<Arc<CommandHandlerRegistry>>,
    ) {
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
        handler_reg.register("group-coach", Arc::new(GroupCoachHandler));
        handler_reg.register("group-leave", Arc::new(GroupLeaveHandler));
        handler_reg.register("group-consent", Arc::new(GroupConsentHandler));
        handler_reg.register("coach", Arc::new(CoachListHandler));
        handler_reg.register("coach-select", Arc::new(CoachSelectHandler));
        handler_reg.register("coach-assign", Arc::new(CoachAssignHandler));
        handler_reg.register("privacy", Arc::new(PrivacyStatusHandler));
        handler_reg.register("privacy-on", Arc::new(PrivacyOnHandler));
        handler_reg.register("privacy-off", Arc::new(PrivacyOffHandler));
        handler_reg.register("timezone", Arc::new(TimezoneHandler));
        handler_reg.register("context", Arc::new(ContextHandler));
        (Some(registry), Some(Arc::new(handler_reg)))
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
    /// Returns the storage adapter (kept for post-Arc injection of the
    /// credential refresher), the orchestrator, and the abort handle for the
    /// scheduler task.
    #[cfg(feature = "health-sync")]
    fn init_health_sync(
        repos: &Arc<RepositoryRegistry>,
        sse_manager: &Arc<SseManager>,
    ) -> (
        Arc<PierreSyncStorage>,
        Arc<pierre_enforme::SyncOrchestrator>,
        AbortHandle,
    ) {
        use pierre_services::provider_refresh::start_scheduled_sync;

        use pierre_services::provider_rate_limiter::ProviderRateLimiter;

        use pierre_services::provider_refresh::SyncNotifier;
        let adapter = Arc::new(PierreSyncStorage::new(repos));
        let orchestrator = adapter.build_orchestrator();
        let rate_limiter = Arc::new(ProviderRateLimiter::new());
        let notifier: Arc<dyn SyncNotifier> = Arc::clone(sse_manager) as Arc<dyn SyncNotifier>;
        let auth_repos = repos.auth_repos();
        let abort_handle = start_scheduled_sync(
            Arc::clone(&orchestrator),
            &auth_repos,
            notifier,
            Some(rate_limiter),
        );
        info!("Health data sync scheduler started (Pierre-aware)");
        (adapter, orchestrator, abort_handle)
    }

    /// Create and initialize the tool registry with all built-in tools
    fn create_tool_registry() -> ToolRegistry {
        use crate::tools::registry_builtin::register_builtin_tools;

        let mut registry = ToolRegistry::new();
        register_builtin_tools(&mut registry);

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
        use crate::mcp::tool_selection::sync_tool_catalog;
        if let Err(e) = sync_tool_catalog(tool_registry, repos.tool_selection.as_ref()).await {
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
                error!(
                    "Failed to initialize admin config service: {}. Runtime config overrides are unavailable; quota enforcement degrades to compile-time tier defaults.",
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
}
