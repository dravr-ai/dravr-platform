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
#[cfg(feature = "protocol-a2a")]
use crate::a2a::system_user::A2ASystemUserService;
use crate::agui::RunRegistry as AgUiRunRegistry;
use crate::config::admin::AdminConfigService;
#[cfg(feature = "client-messaging")]
use crate::services::backfill_notifier::{ChatReentry, ServerBackfillNotifier};
use crate::services::photograveur_client::PhotograveurClient;
use crate::services::turn_lifecycle::InFlightTurns;
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
    calibration::CalibrateHandler,
    coach::{
        CoachAddHandler, CoachAssignHandler, CoachInviteHandler, CoachListHandler,
        CoachRemoveHandler,
    },
    coach_create::CoachCreateHandler,
    discover::{DiscoverHandler, DiscoverInstallHandler},
    group::{
        GroupCoachHandler, GroupConsentHandler, GroupInviteHandler, GroupLeaveHandler,
        GroupListHandler, GroupMembersHandler, GroupRespondHandler, GroupStatusHandler,
    },
    group_membership::{GroupCreateHandler, GroupJoinHandler},
    guardian_confirm::{ConfirmHandler, DenyHandler},
    help::HelpHandler,
    onboarding::PillarsHandler,
    plan::{PlanShareHandler, PlanShowHandler},
    privacy::{PrivacyOffHandler, PrivacyOnHandler, PrivacyStatusHandler},
    reset::ResetHandler,
    status::StatusHandler,
    timezone::TimezoneHandler,
    CommandHandler, CommandHandlerRegistry,
};
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
#[cfg(feature = "client-notifications")]
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
#[cfg(feature = "client-messaging")]
use pierre_messaging::commands::CommandDefinition;

/// One `setMyCommands` call: the entries, the scope and the `language_code` they are for.
#[cfg(feature = "client-messaging")]
type TelegramMenuList = (Vec<(String, String)>, CommandScope, Option<&'static str>);

use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::ContremaitreConfig;
use pierre_core::billing::{dummy::DummyProvider, BillingProvider};
use pierre_core::errors::{AppError, AppResult};
#[cfg(feature = "client-messaging")]
use pierre_core::models::SUPPORTED_LOCALES;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
#[cfg(feature = "tools-groups")]
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_intelligence::{
    ActivityIntelligence, ContextualFactors, PerformanceMetrics, TimeOfDay, TrendDirection,
    TrendIndicators,
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
#[cfg(feature = "health-sync")]
use pierre_services::health_sync::PierreSyncStorage;
#[cfg(feature = "client-messaging")]
use pierre_services::messenger_persistent_menu::publish_messenger_menu;
#[cfg(all(feature = "client-notifications", feature = "client-messaging"))]
use pierre_services::notification_channel_sink::MessagingChannelSink;
#[cfg(feature = "client-notifications")]
use pierre_services::notification_localizer::UserLocaleNotificationLocalizer;
#[cfg(feature = "client-notifications")]
use pierre_services::persona_notification_policy_gate::PersonaNotificationPolicyGate;
use pierre_services::pricing_loader;
#[cfg(feature = "client-messaging")]
use pierre_services::telegram_bot_commands::{
    publish_telegram_commands, CommandScope, PERSONAL_MARKER,
};
use pierre_services::tenant_chat_provider::TenantChatProviderCache;
use pierre_services::usage_pruning::start_usage_pruning_task;
#[cfg(feature = "transport-sse")]
use pierre_sse::SseManager;
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_selection::ToolSelectionService;
use std::collections::HashMap;
// Only `CommandRegistries` carries a HashSet, and that whole type is
// client-messaging-gated.
#[cfg(feature = "client-messaging")]
use std::collections::HashSet;
// Its one reader is the commands-directory override, which is
// client-messaging-gated like the catalogue it points at.
#[cfg(feature = "client-messaging")]
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

/// Everything one parse of the `commands/*.md` catalogue produces: the command
/// definitions, the handlers keyed by command name, the argument signatures
/// `/help` renders beside each, and the names of the commands that act on
/// their caller alone. `None` in a host built without the catalogue.
#[cfg(feature = "client-messaging")]
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

        // Load the Guardian policy snapshot from `system_settings.guardian_config`
        // (defaults ← persisted document ← GUARDIAN_* env, env wins per field).
        // Every dispatch point reads the effective Arc<Guardian> through the
        // ServerContext ToolRuntime override, and `PUT /admin/settings/guardian`
        // calls `install` on it after persisting a new document.
        let guardian_config_registry =
            Arc::new(GuardianConfigRegistry::from_database(&database_arc).await);

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

        // Create group coaching service (before struct construction to avoid borrow-after-move)
        #[cfg(feature = "tools-groups")]
        let group_service = Arc::new(pierre_groups::GroupService::new(
            repos.groups.clone(),
            tier_strategy_for("professional"),
        ));

        // Load messaging slash commands + handlers (see `build_command_registries`).
        #[cfg(feature = "client-messaging")]
        let (command_registry, command_handler_registry, command_arg_specs, personal) =
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

        // Push that same catalogue to Telegram's `/` menu, per locale and as
        // two lists each: the default scope every chat falls back to, and a
        // group list carrying the SAME commands with the personal ones
        // marked, so a shared room can tell which entries act on one member
        // alone without losing the ability to discover them. Descriptions
        // come from the five-locale strings registry, so the menu reads in
        // the athlete's language; a list with no `language_code` is the
        // fallback for a locale the registry does not speak. Detached because
        // a slow or unreachable Telegram API must not stall the bind path;
        // the publisher no-ops when TELEGRAM_BOT_TOKEN is unset.
        #[cfg(feature = "client-messaging")]
        if let Some(registry) = command_registry.as_ref() {
            let strings = Arc::clone(&contremaitre_messaging_strings_registry);
            let mut lists: Vec<TelegramMenuList> = Vec::new();
            // The `/` menu is published in every locale the platform speaks —
            // the one list, not a copy of it.
            for locale in SUPPORTED_LOCALES {
                let describe = |d: &CommandDefinition| {
                    strings.command_description(&d.name, &d.description, locale)
                };
                let all_commands = registry.bot_command_list_described(describe);
                let group_commands = registry.bot_command_list_described(|d| {
                    if personal.contains(&d.name) {
                        format!("{PERSONAL_MARKER}{}", describe(d))
                    } else {
                        describe(d)
                    }
                });
                lists.push((all_commands, CommandScope::Default, Some(locale)));
                lists.push((group_commands, CommandScope::AllGroupChats, Some(locale)));
            }
            // The frontmatter's own English line is the fallback list.
            lists.push((registry.bot_command_list(), CommandScope::Default, None));
            lists.push((
                registry.bot_command_list_described(|d| {
                    if personal.contains(&d.name) {
                        format!("{PERSONAL_MARKER}{}", d.description)
                    } else {
                        d.description.clone()
                    }
                }),
                CommandScope::AllGroupChats,
                None,
            ));
            // Messenger's menu is the plain DM list in the frontmatter's language.
            let messenger_commands = registry.bot_command_list();
            let publish = tokio::spawn(async move {
                for (commands, scope, language_code) in &lists {
                    publish_telegram_commands(commands, *scope, *language_code).await;
                }
                // Messenger has no group thread, so its menu is the DM list —
                // the same one, unmarked.
                // LIMITATION(registre#129): `publish_messenger_menu` and
                // `publish_telegram_commands` are the only always-on menu
                // publishers. WhatsApp has no persistent-menu API; Slack's
                // only programmatic path replaces the whole app config
                // app-globally behind a 12h human-bootstrapped token, and its
                // slash commands cannot be invoked in threads, where the
                // coach conversation lives; Discord is deprioritized.
                publish_messenger_menu(&messenger_commands).await;
            });
            // A panic in a detached task is otherwise swallowed, leaving a
            // stale menu with nothing in the logs to explain it.
            tokio::spawn(async move {
                if let Err(e) = publish.await {
                    warn!(error = %e, "Telegram command-menu publish task failed");
                }
            });
        }

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
            Arc::new(registry)
        };

        // Sync tool_catalog table with registry so tenant filtering always has complete data
        Self::run_tool_catalog_sync(&tool_registry, &repos).await;

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
        ));

        // Start the background notification scheduler if service is available
        #[cfg(feature = "client-notifications")]
        let scheduler_abort_handle = notification_service.as_ref().map(|s| s.start_scheduler());

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
        #[cfg(feature = "client-messaging")]
        let base_url = config.base_url.clone();
        let common = super::slices::CommonSlice {
            repos,
            cache: cache_arc,
            turns: Arc::new(InFlightTurns::new()),
            // Reads PHOTOGRAVEUR_URL; absent means messaging charts stay off.
            photograveur: Arc::new(PhotograveurClient::from_env(reqwest::Client::new())),
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
            #[cfg(feature = "client-messaging")]
            command_registry,
            #[cfg(feature = "client-messaging")]
            command_handler_registry,
            #[cfg(feature = "client-messaging")]
            command_arg_specs,
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
            guardian_config_registry,
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
            // The in-app arm's ping. Cloned from the already-assembled common
            // slice, the same handle the commitment sweep is given.
            #[cfg(feature = "client-notifications")]
            common.notification_service.clone(),
        ));

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
        let handlers: HashMap<String, Arc<dyn CommandHandler>> = [
            ("status", Arc::new(StatusHandler) as Arc<dyn CommandHandler>),
            ("logout", Arc::new(LogoutHandler)),
            ("group", Arc::new(GroupListHandler)),
            ("group-status", Arc::new(GroupStatusHandler)),
            ("group-members", Arc::new(GroupMembersHandler)),
            ("group-invite", Arc::new(GroupInviteHandler)),
            ("group-coach", Arc::new(GroupCoachHandler)),
            ("group-respond", Arc::new(GroupRespondHandler)),
            ("group-leave", Arc::new(GroupLeaveHandler)),
            ("group-consent", Arc::new(GroupConsentHandler)),
            ("group-create", Arc::new(GroupCreateHandler)),
            ("group-join", Arc::new(GroupJoinHandler)),
            ("discover", Arc::new(DiscoverHandler)),
            ("discover-install", Arc::new(DiscoverInstallHandler)),
            ("coach-list", Arc::new(CoachListHandler)),
            ("coach-add", Arc::new(CoachAddHandler)),
            ("coach-remove", Arc::new(CoachRemoveHandler)),
            ("coach-create", Arc::new(CoachCreateHandler)),
            ("coach-assign", Arc::new(CoachAssignHandler)),
            ("coach-invite", Arc::new(CoachInviteHandler)),
            ("privacy", Arc::new(PrivacyStatusHandler)),
            ("reset", Arc::new(ResetHandler)),
            ("privacy-on", Arc::new(PrivacyOnHandler)),
            ("privacy-off", Arc::new(PrivacyOffHandler)),
            ("timezone", Arc::new(TimezoneHandler)),
            ("confirm", Arc::new(ConfirmHandler)),
            ("deny", Arc::new(DenyHandler)),
            ("pillars", Arc::new(PillarsHandler)),
            ("plan", Arc::new(PlanShowHandler)),
            ("plan-share", Arc::new(PlanShareHandler)),
            ("calibrate", Arc::new(CalibrateHandler)),
        ]
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
    #[cfg(feature = "client-notifications")]
    fn create_notification_service(
        database: &Arc<Database>,
        repos: &Arc<RepositoryRegistry>,
        messaging_strings: &Arc<MessagingStringsRegistry>,
        persona_contracts: &Arc<PersonaContractRegistry>,
    ) -> Arc<NotificationService> {
        let service = match database.as_ref() {
            Database::SQLite(db) => NotificationService::from_sqlite(db.pool().clone()),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => NotificationService::from_postgres(db.pool().clone()),
        };
        #[cfg(feature = "client-messaging")]
        let service = service.with_channel_sink(Arc::new(MessagingChannelSink::new(
            Arc::clone(repos),
            Arc::clone(messaging_strings),
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
