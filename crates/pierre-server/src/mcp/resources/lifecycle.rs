// ABOUTME: ServerContext::new constructor + private bootstrap helpers (JWKS keys, notification service, tool catalog, health sync)
// ABOUTME: Assembles the ~50 shared Arc handles the runtime injects into every handler from raw startup ingredients
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::contremaitre::{init_contremaitre_registries, ContremaitreRegistrySet};
use super::ServerContext;
use super::ServerContextOptions;
#[cfg(feature = "protocol-a2a")]
use crate::a2a::client::A2AClientManager;
use crate::agui::RunRegistry as AgUiRunRegistry;
use crate::config::admin::AdminConfigService;
use crate::config::sciotte_read_caps::SciotteReadCaps;
use crate::services::archive_import::ArchiveImports;
#[cfg(feature = "client-messaging")]
use crate::services::backfill_notifier::ServerBackfillNotifier;
#[cfg(all(feature = "client-messaging", feature = "client-groups"))]
use crate::services::group_chat_poster::ServerGroupChatPoster;
use crate::services::originals::blob_store_when_unset;
use crate::services::photograveur_client::PhotograveurClient;
#[cfg(feature = "client-messaging")]
use crate::services::platform_post::PlatformPost;
#[cfg(feature = "client-notifications")]
use crate::services::session_landed::SessionDigest;
use crate::services::session_landed::SessionLandedRouter;
use crate::services::turn_lifecycle::InFlightTurns;
use crate::services::turn_runner::TurnRunner;
use pierre_auth::admin::jwks::{load_or_store_first_keypair, JwksManager};
use pierre_auth::auth::AuthManager;
use pierre_auth::firebase::FirebaseAuth;
use pierre_auth::human_check::{HumanCheck, TurnstileCheck};
use pierre_auth::oauth2_server::first_party::MOBILE_PLAY_PACKAGE_NAME;
use pierre_auth::oauth2_server::grant_standing::GrantStandings;
use pierre_auth::oauth2_server::play_integrity_decoder::GooglePlayIntegrityDecoder;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::oauth2_server::KnownDeviceKey;
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_cache::Cache;
#[cfg(feature = "slash-commands")]
use pierre_commands as commands;
#[cfg(feature = "slash-commands")]
use pierre_commands::{
    account::LogoutHandler,
    account_deletion::DeleteAccountHandler,
    agent::{
        AgentAddHandler, AgentAssignHandler, AgentListHandler, AgentRemoveHandler,
        CoachInviteHandler,
    },
    agent_create::AgentCreateHandler,
    calibration::CalibrateHandler,
    discover::{DiscoverHandler, DiscoverInstallHandler},
    fortnight::FortnightHandler,
    group::{
        GroupCoachHandler, GroupDigestHandler, GroupLeaveHandler, GroupListHandler,
        GroupMembersHandler, GroupRespondHandler, GroupStatusHandler,
    },
    group_consent::GroupConsentHandler,
    group_invite::GroupInviteHandler,
    group_membership::{GroupCreateHandler, GroupJoinHandler},
    group_reads::GroupReadsHandler,
    help::HelpHandler,
    language::LanguageHandler,
    onboarding::PillarsHandler,
    pending_confirm::{ConfirmHandler, DenyHandler},
    plan::{PlanShareHandler, PlanShowHandler},
    privacy::{PrivacyOffHandler, PrivacyOnHandler, PrivacyStatusHandler},
    reset::ResetHandler,
    season::SeasonHandler,
    status::StatusHandler,
    timezone::TimezoneHandler,
    CommandHandler, CommandHandlerRegistry,
};
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
#[cfg(feature = "client-notifications")]
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_providers::originals::OriginalsStore;
use pierre_services::provider_purge::ProviderPurgeService;
use pierre_services::{barrage_live_store::live_store, originals_bundle::OriginalsBundleService};

#[cfg(feature = "slash-commands")]
use dravr_canot::commands::CommandRegistry;
#[cfg(feature = "client-messaging")]
use dravr_canot::ChannelRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::ContremaitreConfig;
use pierre_core::billing::dummy::DummyProvider;
use pierre_core::constants::oauth_providers;
use pierre_core::errors::AppResult;
use pierre_core::http_client::api_inner_client;
use pierre_database::backends::factory::Database;
#[cfg(feature = "client-notifications")]
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::backends::shared::key_material::KeyStoreQueue;
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
use pierre_fitness_compute::weather_clients::WeatherClients;
#[cfg(feature = "tools-groups")]
use pierre_groups::delegation::DelegationStore;
#[cfg(feature = "tools-groups")]
use pierre_groups::strategies::tier::tier_strategy_for;
#[cfg(feature = "tools-groups")]
use pierre_groups::AthleteBilling;
use pierre_llm::health::LlmHealthState;
use pierre_llm::ChatProvider;
#[cfg(feature = "provider-sciotte")]
use pierre_middleware::provider_link_token::NonceStore;
use pierre_middleware::redaction::RedactionConfig;
use pierre_middleware::McpAuthMiddleware;
#[cfg(feature = "client-notifications")]
use pierre_notifications::NotificationService;
#[cfg(feature = "client-notifications")]
use pierre_providers::ai_scope;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{ProviderRateLimiter, FIFTEEN_MINUTES, ONE_DAY};
use pierre_providers::sciotte_provider::{startup_login_client, startup_reads};
use pierre_services::api_key_cleanup::start_api_key_cleanup_task;
#[cfg(feature = "client-messaging")]
use pierre_services::channel_adapters::ChannelAdapterFactory;
use pierre_services::coach_read_retention::start_coach_read_retention_task;
#[cfg(feature = "health-sync")]
use pierre_services::health_sync::{PierreSyncStorage, WellnessOrigins};
#[cfg(all(feature = "client-notifications", feature = "client-messaging"))]
use pierre_services::notification_channel_sink::MessagingChannelSink;
#[cfg(feature = "client-notifications")]
use pierre_services::notification_localizer::UserLocaleNotificationLocalizer;
#[cfg(feature = "client-notifications")]
use pierre_services::persona_notification_policy_gate::PersonaNotificationPolicyGate;
use pierre_services::pricing_loader;
#[cfg(feature = "health-sync")]
use pierre_services::sync_failure_notice::SyncFailureNotices;
use pierre_services::tenant_chat_provider::TenantChatProviderCache;
use pierre_services::usage_pruning::start_usage_pruning_task;
#[cfg(feature = "transport-sse")]
use pierre_sse::SseManager;
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_selection::ToolSelectionService;
#[cfg(feature = "health-sync")]
use secrecy::SecretString;
#[cfg(feature = "slash-commands")]
use std::collections::HashMap;
// Only `CommandRegistries` carries a HashSet, and that whole type is
// slash-commands-gated.
#[cfg(feature = "slash-commands")]
use std::collections::HashSet;
// Its one reader is the commands-directory override, which is
// slash-commands-gated like the catalogue it points at.
#[cfg(feature = "slash-commands")]
use std::env;
#[cfg(feature = "slash-commands")]
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "protocol-rest")]
use std::sync::OnceLock;
use std::time::Duration;
#[cfg(feature = "health-sync")]
use tokio::task::AbortHandle;
use tracing::{error, info, warn};

/// Everything one parse of the `commands/*.md` catalogue produces: the command
/// definitions, the handlers keyed by command name, the argument signatures
/// `/help` renders beside each, and the names of the commands that act on
/// their caller alone. `None` in a host built without the catalogue.
#[cfg(feature = "slash-commands")]
type CommandRegistries = (
    Option<Arc<CommandRegistry>>,
    Option<Arc<CommandHandlerRegistry>>,
    Option<Arc<HashMap<String, String>>>,
    HashSet<String>,
);

impl ServerContext {
    /// Create new server resources with proper Arc sharing
    ///
    /// # Parameters
    /// - `options`: Optional initialization parameters (RSA key size, JWKS manager, LLM provider)
    ///
    /// # Errors
    ///
    /// Returns an error when no JWKS manager is provided and the persisted RSA
    /// keypairs cannot be read, or the first one cannot be generated and
    /// stored: a context never signs with a key the store does not hold
    /// (carnet#696).
    // Function exceeds line limit because it assembles 20+ interdependent resources
    // Splitting would reduce clarity without improving maintainability
    pub async fn new(
        database: Database,
        auth_manager: AuthManager,
        admin_jwt_secret: &str,
        config: Arc<ServerConfig>,
        cache: Cache,
        options: ServerContextOptions,
    ) -> AppResult<Self> {
        let rsa_key_size_bits = options.rsa_key_size_bits.unwrap_or(4096);
        // Use the injected billing provider when present (production binaries
        // pass StripeProvider when Stripe env is configured). Otherwise fall
        // back to the in-tree DummyProvider so the platform compiles, tests
        // run, and local dev works without a vendor crate configured. Resolved
        // first because the group service bills athletes through it too.
        let billing_provider = options
            .billing_provider
            .unwrap_or_else(|| Arc::new(DummyProvider::new()));
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

        // The binary wires archive imports from its database and environment
        // before building the context (`ArchiveImports::from_env`); a context
        // built without them answers every import request that they are not
        // available.
        let archive_imports = options.archive_imports.unwrap_or_else(|| {
            ArchiveImports::disabled("this server was started without archive imports")
        });
        let database_arc = Arc::new(database);
        let repos = Arc::clone(database_arc.repositories());
        // The binary passes the deployment's blob store; a context built
        // without one keeps blobs in memory over an in-memory database and
        // has none over any other, where uploads are refused.
        let blobs = options
            .blobs
            .unwrap_or_else(|| blob_store_when_unset(&config.database.url));
        // One record store (imports write it, the export reads it), one live
        // store over it and the blob store, and the one purge every disconnect takes.
        let activity_records = archive_imports.record_store(options.live_records)?;
        let live_activities = live_store(Arc::clone(&activity_records), blobs.clone().ok(), &repos);
        let provider_purge = ProviderPurgeService::shared(&repos, Arc::clone(&live_activities));
        let originals_bundles = OriginalsBundleService::over(&activity_records, &blobs, &repos);
        let originals = Arc::new(OriginalsStore::new(
            blobs,
            Arc::clone(&repos.uploaded_activity_files),
        ));

        let auth_manager_arc = Arc::new(auth_manager);

        // Create the provider registry once, over the deployment's endpoint
        // overrides, admitting every provider call against the signing app's
        // budget, and holding the server's sciotte clients: the read services
        // (a session's calls on the one its id picks) and the sign-in client
        // every step of a sign-in shares (carnet#1504)
        let sciotte = &config.external_services.sciotte;
        let request_limiter = Self::create_provider_rate_limiter(&config, &repos);
        let provider_registry = Arc::new(
            ProviderRegistry::new()
                .with_endpoint_overrides(&config.oauth.endpoints)
                .with_request_limiter(Arc::clone(&request_limiter))
                .with_sciotte_reads(startup_reads(
                    &sciotte.read_urls,
                    sciotte.audience.as_deref(),
                ))
                .with_sciotte_login_client(startup_login_client(
                    sciotte.login_url.as_deref(),
                    sciotte.audience.as_deref(),
                )),
        );

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
        // registry, `PUT /admin/settings/harness` calls `install` on it after
        // persisting a new document, and `start_settings_poll` keeps every other
        // instance converged on the row.
        let harness_config_registry =
            Arc::new(HarnessConfigRegistry::from_database(&database_arc).await);

        // Load the Guardian policy snapshot from `system_settings.guardian_config`
        // (the compiled-in caps with the stored document over them).
        // Every dispatch point reads the effective Arc<Guardian> through the
        // ServerContext ToolRuntime override, `PUT /admin/settings/guardian` calls
        // `install` on it after persisting a new document, and
        // `start_settings_poll` keeps every other instance converged on the row.
        let guardian_config_registry =
            Arc::new(GuardianConfigRegistry::from_database(&database_arc).await);

        // Create A2A services for agent-to-agent communication
        #[cfg(feature = "protocol-a2a")]
        let a2a_auth_repos = repos.auth_repos();
        #[cfg(feature = "protocol-a2a")]
        let a2a_client_manager = Arc::new(A2AClientManager::new(&a2a_auth_repos));

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
            Self::resolve_jwks_manager(jwks_manager, database_arc.as_ref(), rsa_key_size_bits)
                .await?;

        // Create SSE manager with configured buffer size
        #[cfg(feature = "transport-sse")]
        let sse_manager = Arc::new(SseManager::new(config.sse.max_buffer_size));

        // Create auth middleware after jwks_manager is initialized
        let auth_middleware = Arc::new(
            McpAuthMiddleware::new(
                (*auth_manager_arc).clone(),
                repos.clone(),
                jwks_manager_arc.clone(),
            )
            .with_grant_standings(GrantStandings::new(
                repos.oauth2_server.clone(),
                cache_arc.clone(),
            )),
        );

        // On Redis every replica counts into the same OAuth2 window; an
        // in-process cache is not shared, so the limiter keeps its own store
        let oauth2_rate_limiter = Arc::new(
            OAuth2RateLimiter::new(
                cache_arc
                    .is_shared_across_processes()
                    .then(|| cache_arc.clone()),
                OAuth2RateLimiter::local_window_store(),
                &config.rate_limiting,
            )
            .with_known_device_key(KnownDeviceKey::from_jwt_secret(admin_jwt_secret)),
        );

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

        // Each athlete's sciotte session reads are held to their tier's caps,
        // as the admin config's sciotte_reads.* parameters set them, each
        // answer kept a minute (carnet#1661). Bound even when the admin
        // config could not start, so the compiled caps still apply by tier.
        request_limiter.bind_session_read_limits(
            Arc::new(SciotteReadCaps::new(
                admin_config.clone().map(|config| {
                    let lookup: Arc<dyn pierre_runtime_context::AdminConfigLookup> = config;
                    lookup
                }),
                Arc::clone(&repos.users),
            )),
            Duration::from_mins(1),
        );

        // Start background usage counter pruning task (hourly, removes records older than 90 days)
        let pruning_abort_handle = admin_config.as_ref().map(|config| {
            let lookup: Arc<dyn pierre_runtime_context::AdminConfigLookup> =
                Arc::<AdminConfigService>::clone(config);
            start_usage_pruning_task(
                Arc::clone(&repos.usage_counters),
                lookup,
                Arc::clone(&repos.worker_runs),
            )
        });

        // Hourly sweep of expired API keys, on the shared worker ledger.
        start_api_key_cleanup_task(Arc::clone(&repos.api_keys), Arc::clone(&repos.worker_runs));

        // Hourly purge of the athletes' records of their human coach's reads
        // past their 12 months, on the shared worker ledger (carnet#1445).
        start_coach_read_retention_task(
            Arc::clone(&repos.coach_data_reads),
            Arc::clone(&repos.worker_runs),
        );

        // Create tool selection service for per-tenant tool filtering
        let tool_selection = Arc::new(ToolSelectionService::new(&repos));

        // Build the outbound email service (gated on ServerConfig: non-empty
        // Resend creds + not in CI mode). See `build_email_service`.
        let email_service = Self::build_email_service(&config);

        // Create group coaching service (before struct construction to avoid borrow-after-move)
        #[cfg(feature = "tools-groups")]
        let group_service = Arc::new(pierre_groups::GroupService::new(
            repos.groups.clone(),
            Arc::new(DelegationStore::new(&repos, Arc::clone(&provider_purge))),
            tier_strategy_for("professional"),
            Arc::new(AthleteBilling::new(
                Arc::clone(&repos.groups),
                Arc::clone(&repos.tenants),
                Arc::clone(&repos.subscriptions),
                Arc::clone(&billing_provider),
            )),
        ));

        // Load messaging slash commands + handlers (see `build_command_registries`).
        #[cfg(feature = "slash-commands")]
        let (command_registry, command_handler_registry, command_arg_specs, personal_commands) =
            Self::build_command_registries();

        // Initialize contremaitre registries (prompts + tool descriptions +
        // evidence + training catalogue). The cageux config registry is passed in so the
        // contremaitre sync can also overlay its snapshot. The GitHub/GCS
        // overlay runs in the background (off this bind path), so a slow sync
        // never stalls startup; registries serve compiled-in defaults until
        // the first background tick converges.
        let ContremaitreRegistrySet {
            prompt: contremaitre_prompt_registry,
            tool_desc: contremaitre_tool_desc_registry,
            evidence: contremaitre_evidence_registry,
            messaging_strings: contremaitre_messaging_strings_registry,
            training_catalogue: contremaitre_training_catalogue_registry,
        } = init_contremaitre_registries(&cageux_config_registry, &persona_contract_registry);

        // Create and populate tool registry with all built-in tools. Any
        // `extra_tools` supplied via `ServerContextOptions` (used by
        // messaging-eval integration tests that need a no-auth stub
        // tool) land in the same registry as the built-ins, so the
        // pipeline's tool dispatcher can route to them with zero
        // special-casing.
        //
        // The contremaitre tool-description registry is attached to the same
        // registry instance so `tools/list`, `GET /mcp/tools`, the chat
        // function-calling surface and the generated SDK types all serve the
        // synced overlay. The overlay registry carries its own `RwLock`, so
        // attaching the `Arc` once here is enough: every later webhook or poll
        // sync writes through it and the next schema build reads the new text
        // without a redeploy.
        let tool_registry = {
            let mut registry = Self::create_tool_registry();
            for tool in options.extra_tools {
                registry.register(tool);
            }
            registry.set_tool_descriptions(Arc::clone(&contremaitre_tool_desc_registry));
            Self::report_tool_catalogue_size(&registry, &config.llm_chain);
            Arc::new(registry)
        };

        // Sync tool_catalog table with registry so tenant filtering always has complete data
        Self::run_tool_catalog_sync(&tool_registry, &repos).await;

        // The tracker the shutdown drain awaits. Created ahead of the
        // notification service, which spawns every trigger dispatch onto it.
        let turns = Arc::new(InFlightTurns::new());

        // The one adapter factory of this context (carnet#1600), named by
        // whoever built it and never defaulted here. Taken ahead of the
        // notification service because its messaging sink sends through it.
        #[cfg(feature = "client-messaging")]
        let channel_adapters: Arc<dyn ChannelAdapterFactory> = options.channel_adapters;

        // Create notification service and start scheduler if notifications
        // feature is enabled. Built after the contremaitre registries because
        // the messaging sink renders notification bodies through the localized
        // string registry.
        #[cfg(feature = "client-notifications")]
        let notification_service = Some(Self::create_notification_service(
            &database_arc,
            &repos,
            &contremaitre_messaging_strings_registry,
            &persona_contract_registry,
            &turns,
            #[cfg(feature = "client-messaging")]
            &channel_adapters,
        ));

        // Start the background notification scheduler if service is available.
        // Its tick also sweeps the pending push receipts, so it runs with no
        // route creating schedules.
        #[cfg(feature = "client-notifications")]
        let scheduler_abort_handle = notification_service.as_ref().map(|s| s.start_scheduler());

        // Initialize health data sync with the Pierre-aware scheduler. After
        // the notification service, which tells an athlete when a provider's
        // scheduled sync fails.
        #[cfg(all(feature = "health-sync", feature = "client-notifications"))]
        let sync_notice_service = notification_service.clone();
        #[cfg(all(feature = "health-sync", not(feature = "client-notifications")))]
        let sync_notice_service = None;
        #[cfg(feature = "health-sync")]
        let (sync_storage, sync_orchestrator, sync_scheduler_abort_handle) =
            Self::init_health_sync(
                &repos,
                &provider_registry,
                config.webhooks.whoop_secret.as_ref(),
                &request_limiter,
                &sse_manager,
                SyncFailureNotices::new(
                    Arc::clone(&repos.provider_connections),
                    sync_notice_service,
                ),
            )?;

        // Cache-backed nonce store for channel-initiated provider links
        #[cfg(feature = "provider-sciotte")]
        let nonce_store = Arc::new(NonceStore::new(cache_arc.clone()));

        // Phase 1 → 4: load admin pricing overrides into the process-wide
        // PricingRegistry before the first chat request can land. Failures
        // are logged inside the loader; the compile-time PRICING_TABLE
        // remains the safe fallback when no overrides exist.
        pricing_loader::load_pricing_overrides(repos.llm_credentials.as_ref()).await;

        // Precompute view-struct projections from the master registry once;
        // each is an Arc-clone-only operation. The full registry continues
        // to live in `CommonSlice.repos` as the single source of truth.
        let auth_repos_view = repos.auth_repos();
        let agent_repos_view = repos.agent_repos();
        let fitness_repos_view = repos.fitness_repos();
        let usage_repos_view = repos.usage_repos();

        // Capture the public base URL before `config` moves into CommonSlice —
        // the backfill notifier needs it (with the admin JWT secret) to mint the
        // hosted-login link for a provider-reauth nudge.
        #[cfg(feature = "client-messaging")]
        let base_url = config.base_url.clone();
        let common = super::slices::CommonSlice {
            repos,
            cache: cache_arc,
            turns,
            // The binary selects the runner from the environment and passes
            // it in; a context built without one runs turns in-process.
            turn_runner: options
                .turn_runner
                .unwrap_or_else(|| Arc::new(TurnRunner::InProcess)),
            archive_imports,
            live_activities,
            activity_records,
            originals_bundles,
            provider_purge,
            originals,
            // Reads PHOTOGRAVEUR_URL; absent means messaging charts stay off.
            photograveur: Arc::new(PhotograveurClient::from_env()),
            config,
            redaction_config,
            email_service,
            llm_provider,
            chat_provider,
            tenant_chat_providers: TenantChatProviderCache::new(),
            llm_health: Arc::new(LlmHealthState::new()),
            #[cfg(feature = "client-messaging")]
            messaging_registry: Arc::new(ChannelRegistry::new()),
            #[cfg(feature = "client-notifications")]
            notification_service,
            #[cfg(feature = "client-notifications")]
            scheduler_abort_handle,
            pruning_abort_handle,
            #[cfg(feature = "tools-groups")]
            group_service,
            #[cfg(feature = "slash-commands")]
            command_registry,
            #[cfg(feature = "slash-commands")]
            command_handler_registry,
            #[cfg(feature = "slash-commands")]
            command_arg_specs,
            #[cfg(feature = "slash-commands")]
            personal_commands,
        };

        let auth = super::slices::AuthSlice {
            auth_manager: auth_manager_arc,
            jwks_manager: jwks_manager_arc,
            auth_middleware,
            csrf_manager,
            firebase_auth,
            oauth2_rate_limiter,
            play_integrity: Arc::new(GooglePlayIntegrityDecoder::from_metadata_server(
                MOBILE_PLAY_PACKAGE_NAME,
            )),
            human_check: TurnstileCheck::from_env()
                .map(|check| -> Arc<dyn HumanCheck> { Arc::new(check) }),
            admin_jwt_secret: admin_jwt_secret.into(),
            #[cfg(feature = "provider-sciotte")]
            nonce_store,
            repos: auth_repos_view,
        };

        let agent = super::slices::AgentSlice {
            database: database_arc,
            admin_config,
            repos: agent_repos_view,
        };

        let fitness = super::slices::FitnessSlice {
            provider_registry,
            weather: Arc::new(WeatherClients::new(
                common.config.fitness.weather_api.as_ref(),
            )),
            #[cfg(feature = "health-sync")]
            sync_orchestrator: Some(sync_orchestrator),
            #[cfg(feature = "health-sync")]
            sync_storage: Some(sync_storage),
            #[cfg(feature = "health-sync")]
            sync_scheduler_abort_handle: Some(sync_scheduler_abort_handle),
            cageux_config_registry,
            harness_config_registry,
            guardian_config_registry,
            persona_contract_registry,
            repos: fitness_repos_view,
        };

        let sse = super::slices::SseSlice {
            #[cfg(feature = "transport-sse")]
            sse_manager,
            agui_registry: Arc::new(AgUiRunRegistry::new()),
        };

        let a2a = super::slices::A2ASlice {
            #[cfg(feature = "protocol-a2a")]
            a2a_client_manager,
        };

        let billing = super::slices::BillingSlice {
            billing_provider,
            repos: usage_repos_view,
        };

        // Best-effort backfill-completion notifier, built from the same shared
        // repos + strings registry the approval notifier uses. Stored on the
        // context so the detached historical-backfill task can push a "your
        // history is ready" notice back to the originating channel.
        //
        // The platform post (which lets that push answer in the agent's own
        // words) needs the composition-root `Arc<ServerContext>`, which
        // doesn't exist yet — so the notifier and the context share this empty
        // `OnceLock` slot, filled post-`Arc` by `install_platform_post`.
        #[cfg(feature = "client-messaging")]
        let platform_post: Arc<OnceLock<Arc<dyn PlatformPost>>> = Arc::new(OnceLock::new());
        #[cfg(feature = "client-messaging")]
        let backfill_notifier = Some(ServerBackfillNotifier::from_handles(
            common.repos.clone(),
            contremaitre_messaging_strings_registry.clone(),
            platform_post.clone(),
            admin_jwt_secret.into(),
            base_url,
            Arc::clone(&channel_adapters),
            // The in-app arm's ping. Cloned from the already-assembled common
            // slice, the same handle the commitment sweep is given.
            #[cfg(feature = "client-notifications")]
            common.notification_service.clone(),
        ));

        // Where a live write's landing goes. Built here, in every build and
        // every test resource, so a landing always reaches the per-session
        // digest; the landing post is layered on post-`Arc` by
        // `install_activity_landed`.
        #[cfg(feature = "client-notifications")]
        let session_landed = Arc::new(SessionLandedRouter::with_digest(SessionDigest {
            repos: common.repos.clone(),
            notifications: common.notification_service.clone(),
            strings: contremaitre_messaging_strings_registry.clone(),
        }));
        #[cfg(not(feature = "client-notifications"))]
        let session_landed = Arc::new(SessionLandedRouter::default());

        let mcp = super::slices::McpSlice {
            tool_registry,
            tool_selection,
            prompt_registry: contremaitre_prompt_registry,
            tool_description_registry: contremaitre_tool_desc_registry,
            evidence_registry: contremaitre_evidence_registry,
            messaging_strings_registry: contremaitre_messaging_strings_registry,
            training_catalogue_registry: contremaitre_training_catalogue_registry,
            #[cfg(feature = "client-messaging")]
            backfill_notifier,
            #[cfg(all(feature = "client-messaging", feature = "client-groups"))]
            group_chat_poster: ServerGroupChatPoster::from_repos(
                common.repos.clone(),
                Arc::clone(&channel_adapters),
            ),
            #[cfg(feature = "client-messaging")]
            channel_adapters,
            #[cfg(feature = "client-messaging")]
            platform_post,
            session_landed,
            // The coach's first post, installed post-`Arc` the same way.
            #[cfg(feature = "protocol-rest")]
            first_sync: Arc::new(OnceLock::new()),
            contremaitre_config: ContremaitreConfig::from_env(),
        };

        Ok(Self {
            common,
            auth,
            agent,
            fitness,
            sse,
            a2a,
            billing,
            mcp,
        })
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
        ResendEmailService::try_new(
            api_inner_client().clone(),
            api_key.to_owned(),
            from_email.to_owned(),
        )
            .inspect(|_| info!("Resend email service configured"))
            .inspect_err(|e| {
                warn!(error = %e, "Resend email service not built — transactional email will be skipped");
            })
            .ok()
            .map(Arc::new)
    }

    /// Load messaging slash-command definitions and register their handlers.
    ///
    /// `PIERRE_COMMANDS_DIR` overrides the default CWD-relative `commands/`
    /// lookup so tests and non-default deployments can point at an absolute path.
    #[cfg(feature = "slash-commands")]
    fn build_command_registries() -> CommandRegistries {
        let commands_dir_override = env::var("PIERRE_COMMANDS_DIR").ok();
        let commands_dir = commands_dir_override
            .as_deref()
            .map_or_else(|| Path::new("commands").to_path_buf(), PathBuf::from);
        let catalog = commands::load_command_catalog(&commands_dir);
        let mut registry = CommandRegistry::new();
        for def in catalog.definitions {
            registry.register(def);
        }
        let registry = Arc::new(registry);
        // Argument signatures ride alongside the registry: `/help` renders them
        // in each command's line so options like `yes|no` are discoverable.
        let arg_specs = Arc::new(catalog.arg_specs);
        let personal = catalog.personal;

        // Built before the registry so `/help` can hold the same handler
        // instances and ask each whether it would refuse the caller. `/help`
        // is deliberately absent from this map — it is the one command with no
        // precondition, and including it would need a self-reference.
        let entries: [(&str, Arc<dyn CommandHandler>); _] = [
            ("status", Arc::new(StatusHandler)),
            ("logout", Arc::new(LogoutHandler)),
            ("delete-account", Arc::new(DeleteAccountHandler)),
            ("group", Arc::new(GroupListHandler)),
            ("group-status", Arc::new(GroupStatusHandler)),
            ("group-members", Arc::new(GroupMembersHandler)),
            ("group-invite", Arc::new(GroupInviteHandler)),
            ("group-coach", Arc::new(GroupCoachHandler)),
            ("group-respond", Arc::new(GroupRespondHandler)),
            ("group-digest", Arc::new(GroupDigestHandler)),
            ("group-leave", Arc::new(GroupLeaveHandler)),
            ("group-consent", Arc::new(GroupConsentHandler)),
            ("group-reads", Arc::new(GroupReadsHandler)),
            ("group-create", Arc::new(GroupCreateHandler)),
            ("group-join", Arc::new(GroupJoinHandler)),
            ("discover", Arc::new(DiscoverHandler)),
            ("discover-install", Arc::new(DiscoverInstallHandler)),
            ("agent-list", Arc::new(AgentListHandler)),
            ("agent-add", Arc::new(AgentAddHandler)),
            ("agent-remove", Arc::new(AgentRemoveHandler)),
            ("agent-create", Arc::new(AgentCreateHandler)),
            ("agent-assign", Arc::new(AgentAssignHandler)),
            ("coach-invite", Arc::new(CoachInviteHandler)),
            ("privacy", Arc::new(PrivacyStatusHandler)),
            ("reset", Arc::new(ResetHandler)),
            ("privacy-on", Arc::new(PrivacyOnHandler)),
            ("privacy-off", Arc::new(PrivacyOffHandler)),
            ("timezone", Arc::new(TimezoneHandler)),
            ("language", Arc::new(LanguageHandler)),
            ("confirm", Arc::new(ConfirmHandler)),
            ("deny", Arc::new(DenyHandler)),
            ("pillars", Arc::new(PillarsHandler)),
            ("plan", Arc::new(PlanShowHandler)),
            ("plan-share", Arc::new(PlanShareHandler)),
            ("calibrate", Arc::new(CalibrateHandler)),
            ("season", Arc::new(SeasonHandler)),
            ("fortnight", Arc::new(FortnightHandler)),
        ];
        let handlers: HashMap<String, Arc<dyn CommandHandler>> = entries
            .into_iter()
            .map(|(name, handler)| (name.to_owned(), handler))
            .collect();
        let handlers = Arc::new(handlers);

        let mut handler_reg = CommandHandlerRegistry::new();
        handler_reg.register(
            "help",
            Arc::new(HelpHandler::new(
                Arc::clone(&registry),
                Arc::clone(&arg_specs),
                Arc::clone(&handlers),
                personal.clone(),
            )),
        );
        for (name, handler) in handlers.iter() {
            handler_reg.register(name, Arc::clone(handler));
        }
        (
            Some(registry),
            Some(Arc::new(handler_reg)),
            Some(arg_specs),
            personal,
        )
    }

    /// Create the notification service, dispatching to the appropriate backend
    /// and attaching the messaging delivery sink and the persona policy gate.
    ///
    /// Without the sink the dispatcher has exactly two outlets — the persisted
    /// notification row and Expo push — so an athlete who only ever talks to
    /// Dravr on Telegram, Slack or `WhatsApp` receives nothing, for any category.
    /// The sink hangs off `dispatch` rather than off any one caller, so every
    /// category that raises a notification reaches messaging.
    ///
    /// The persona policy gate hangs off dispatch for the same reason: every
    /// tiered dispatch consults the recipient's persona push policy (shadow
    /// verdicts until `FeatureKey::PersonaNotificationPolicy` arms it), so no
    /// call site can route around the contract's notification promise.
    ///
    /// Every trigger dispatch is spawned onto `turns`' tracker, so the
    /// shutdown drain awaits a notification fired just before SIGTERM the way
    /// it awaits a turn, instead of the runtime's teardown cutting it.
    #[cfg(feature = "client-notifications")]
    fn create_notification_service(
        database: &Arc<Database>,
        repos: &Arc<RepositoryRegistry>,
        messaging_strings: &Arc<MessagingStringsRegistry>,
        persona_contracts: &Arc<PersonaContractRegistry>,
        turns: &InFlightTurns,
        #[cfg(feature = "client-messaging")] channel_adapters: &Arc<dyn ChannelAdapterFactory>,
    ) -> Arc<NotificationService> {
        let service = match database.backend() {
            DatabaseBackend::SQLite(db) => {
                NotificationService::from_sqlite(db.pool().clone(), turns.tracker())
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                NotificationService::from_postgres(db.pool().clone(), turns.tracker())
            }
        };
        #[cfg(feature = "client-messaging")]
        let service = service.with_channel_sink(Arc::new(MessagingChannelSink::new(
            Arc::clone(repos),
            Arc::clone(messaging_strings),
            Arc::clone(channel_adapters),
        )));
        let service = service.with_policy_gate(Arc::new(PersonaNotificationPolicyGate::new(
            Arc::clone(repos),
            Arc::clone(persona_contracts),
        )));
        // Unconditionally, unlike the messaging sink: an Expo push is read
        // once and cannot be re-rendered, so every deployment needs the
        // recipient's own language on it the first time.
        let service = service.with_localizer(Arc::new(UserLocaleNotificationLocalizer::new(
            Arc::clone(repos),
            Arc::clone(messaging_strings),
        )));
        // A notification raised from what a turn or tool served carries its
        // stamp, so an external reader of the feed is withheld it (carnet#769).
        let service = service.with_provenance(Arc::new(ai_scope::derived_policy));
        info!("Notification service initialized");
        Arc::new(service)
    }

    /// The provider rate limiter, counting in `usage_counters` so every
    /// instance shares its windows, sizing the Intervals.icu app's pool by
    /// the grants `provider_connections` counts, with Strava's 15-minute and daily
    /// budgets as `STRAVA_RATE_LIMIT_15MIN` and `STRAVA_RATE_LIMIT_DAILY`
    /// configure them.
    ///
    /// A budget of zero would refuse every counted Strava request, and it is
    /// what a config built by `Default` rather than from the environment
    /// carries, so only a pair of positive budgets replaces the limiter's
    /// built-in Strava windows, which hold the same defaults the environment
    /// falls back to.
    fn create_provider_rate_limiter(
        config: &ServerConfig,
        repos: &Arc<RepositoryRegistry>,
    ) -> Arc<ProviderRateLimiter> {
        let strava = config.strava_api_config();
        let limiter = ProviderRateLimiter::new(
            Arc::clone(&repos.usage_counters),
            Arc::clone(&repos.provider_connections),
        );
        if strava.rate_limit_15min > 0 && strava.rate_limit_daily > 0 {
            limiter.set_budgets(
                oauth_providers::STRAVA,
                &[
                    (strava.rate_limit_15min, FIFTEEN_MINUTES),
                    (strava.rate_limit_daily, ONE_DAY),
                ],
            );
        }
        Arc::new(limiter)
    }

    /// Initialize the health data sync orchestrator and start the Pierre-aware scheduler.
    ///
    /// Uses Pierre's `start_scheduled_sync` instead of enforme's built-in scheduler
    /// to add post-sync behaviors: `last_sync` updates and SSE notifications.
    ///
    /// Returns the storage adapter (kept for post-Arc injection of the
    /// credential refresher), the orchestrator, and the abort handle for the
    /// scheduler task.
    ///
    /// # Errors
    ///
    /// When a health provider's HTTP client cannot be built; enforme never
    /// replaces one with a client that has no timeout.
    #[cfg(feature = "health-sync")]
    fn init_health_sync(
        repos: &Arc<RepositoryRegistry>,
        provider_registry: &ProviderRegistry,
        whoop_webhook_secret: Option<&SecretString>,
        request_limiter: &Arc<ProviderRateLimiter>,
        sse_manager: &Arc<SseManager>,
        notices: SyncFailureNotices,
    ) -> AppResult<(
        Arc<PierreSyncStorage>,
        Arc<dravr_enforme::SyncOrchestrator>,
        AbortHandle,
    )> {
        use pierre_core::errors::AppError;
        use pierre_services::provider_refresh::start_scheduled_sync;

        use pierre_services::provider_refresh::SyncNotifier;
        // The wellness readers ask Intervals.icu and WHOOP at the API the
        // provider registry names for each, as the activity and link calls
        // do. A build that registers no provider for one of them reads none
        // of its wellness. WHOOP's webhooks are verified against the secret
        // the configuration read at startup, the one the route checks.
        // Every daily-summary read of a scrape session spends its owner's
        // session reads (carnet#1661).
        let adapter =
            Arc::new(PierreSyncStorage::new(repos).with_read_budget(Arc::clone(request_limiter)));
        let orchestrator = adapter
            .build_orchestrator(
                provider_registry.sciotte_reads().cloned(),
                WellnessOrigins::named_by(provider_registry),
                whoop_webhook_secret,
            )
            .map_err(|e| {
                AppError::internal(format!("health sync providers could not be built: {e}"))
            })?;
        let notifier: Arc<dyn SyncNotifier> = Arc::<SseManager>::clone(sse_manager);
        let abort_handle = start_scheduled_sync(
            Arc::clone(&orchestrator),
            Arc::clone(&adapter),
            notifier,
            notices,
        );
        info!("Health data sync scheduler started (Pierre-aware)");
        Ok((adapter, orchestrator, abort_handle))
    }

    /// Create and initialize the tool registry with all built-in tools
    fn create_tool_registry() -> ToolRegistry {
        use crate::tools::registry_builtin::register_builtin_tools;

        let mut registry = ToolRegistry::new();
        register_builtin_tools(&mut registry);
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

    /// Initialize admin config service from the active database backend
    ///
    /// Returns None if initialization fails.
    async fn init_admin_config_service(
        database: &Arc<Database>,
    ) -> Option<Arc<AdminConfigService>> {
        match AdminConfigService::for_database(database).await {
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

    /// Resolve JWKS manager from provided instance or load the persisted one
    ///
    /// Without a provided manager the keypairs are loaded through
    /// [`Self::load_or_create_jwks_manager`]; a failed read or a failed first
    /// store is returned rather than replaced by a key held only in memory.
    ///
    /// # Errors
    /// Returns an error if loading or storing the persisted keypairs fails
    async fn resolve_jwks_manager(
        provided: Option<Arc<JwksManager>>,
        database: &Database,
        rsa_key_size_bits: usize,
    ) -> AppResult<Arc<JwksManager>> {
        if let Some(mgr) = provided {
            return Ok(mgr);
        }
        Ok(Arc::new(
            Self::load_or_create_jwks_manager(database, rsa_key_size_bits).await?,
        ))
    }

    /// Load the persisted RSA keypairs, storing the first one when the table
    /// is empty; see [`load_or_store_first_keypair`] (carnet#696). A database
    /// with no free connection is waited out until the boot deadline
    /// (carnet#1129).
    ///
    /// # Errors
    /// Returns an error if a database read or write, or key import, fails
    pub async fn load_or_create_jwks_manager(
        database: &Database,
        rsa_key_size_bits: usize,
    ) -> AppResult<JwksManager> {
        let mut queue = KeyStoreQueue::new(database.boot_deadline(), "rsa_keypairs");
        loop {
            match load_or_store_first_keypair(database.as_security_repository(), rsa_key_size_bits)
                .await
            {
                Err(e) if queue.wait_after(&e).await => {}
                done => break done,
            }
        }
    }
}
