// ABOUTME: Server implementation for serving users with isolated data access
// ABOUTME: Production-ready server with authentication and user isolation capabilities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![recursion_limit = "256"]
#![deny(unsafe_code)]

//! # Dravr API Server Binary
//!
//! This binary starts the multi-protocol Dravr API with user authentication,
//! secure token storage, and database management.

use clap::{error::ErrorKind, Parser};
use dravr_tronc::mcp::transport::stdio;
use pierre_auth::auth::AuthManager;
use pierre_auth::key_management::KeyManager;
use pierre_cache::Cache;
use pierre_config::environment::{LlmProviderType, ServerConfig, TokioRuntimeConfig};
use pierre_core::billing::stripe::StripeProvider;
use pierre_core::billing::BillingProvider;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::analytics_sink::{PierreAnalyticsProvider, PierreNotifyEnricher};
use pierre_mcp_server::cache::cache_from_env;
use pierre_mcp_server::{
    constants::init_server_config,
    features::FeatureConfig,
    mcp::{
        host_seams::build_mcp_server,
        multitenant::ProviderToolRouter,
        resources::{ServerContext, ServerContextOptions},
    },
    utils::{http_client::initialize_http_clients, route_timeout::initialize_route_timeouts},
};
use pierre_services::chat_provider_factory::spawn_llm_health_probe;
use pierre_services::server_lifecycle;
use pierre_tool_runtime::guardian;

type Result<T> = AppResult<T>;
use std::{env, sync::Arc};
use tokio::runtime::{Builder, Runtime};
// SIGTERM-based shutdown notification is Unix-only: Cloud Run (Linux) delivers
// SIGTERM on scale-down/redeploy, and `tokio::signal::unix` does not exist on
// Windows. The cross-platform build compiles this binary on Windows too, so the
// handler and its supporting imports are gated to Unix.
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};
#[cfg(unix)]
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Command-line arguments for the Dravr MCP server
#[derive(Parser)]
#[command(name = "pierre-mcp-server")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Dravr API - Multi-protocol fitness data API for LLMs")]
pub struct Args {
    /// Configuration file path for providers
    #[arg(short, long)]
    config: Option<String>,

    /// Override HTTP port
    #[arg(long)]
    http_port: Option<u16>,

    /// Run in stdio-only mode (disables HTTP/SSE transports)
    #[arg(long)]
    stdio: bool,
}

// Provider-synthetic defense-in-depth: the actual prod safety is
// 1. `default_provider()` returns `Option<String>` (no implicit fallback to synthetic),
// 2. The `production-providers` Cargo feature explicitly excludes `provider-synthetic`.
// A `compile_error!` gate here turned out to be too aggressive — CI integration tests
// build `--release` with default features (which include `provider-synthetic`), and
// the gate fired on every CI run blocking the actual fix from shipping. Re-add as a
// runtime warning if a dedicated `production` cfg lands.

fn main() -> Result<()> {
    let args = parse_args_or_default();

    // Load runtime config first to build the Tokio runtime
    let runtime_config = TokioRuntimeConfig::from_env();
    let runtime = build_tokio_runtime(&runtime_config)?;

    // Run the async server on our configured runtime
    runtime.block_on(async {
        let config = setup_configuration(&args)?;
        let result = bootstrap_server(config, args.stdio).await;
        // Flush buffered OTLP spans/metrics before exit. No-op unless the
        // `telemetry` feature is compiled in and the pipeline activated.
        // Runs inside the runtime so batch exporters can drain their workers.
        pierre_logging::shutdown_telemetry();
        result
    })
}

/// Build a Tokio runtime with configurable worker threads
fn build_tokio_runtime(config: &TokioRuntimeConfig) -> Result<Runtime> {
    let mut builder = Builder::new_multi_thread();

    // Configure worker threads if specified
    if let Some(workers) = config.worker_threads {
        builder.worker_threads(workers);
        eprintln!("Tokio runtime: {workers} worker threads (from TOKIO_WORKER_THREADS)");
    }

    // Configure thread stack size if specified
    if let Some(stack_size) = config.thread_stack_size {
        builder.thread_stack_size(stack_size);
        let stack_kb = stack_size / 1024;
        eprintln!("Tokio runtime: {stack_kb}KB thread stack (from TOKIO_THREAD_STACK_SIZE)");
    }

    // Configure thread naming
    builder.thread_name(&config.thread_name);

    // Enable drivers
    if config.enable_io {
        builder.enable_io();
    }
    if config.enable_time {
        builder.enable_time();
    }

    builder
        .build()
        .map_err(|e| AppError::internal(format!("Failed to build Tokio runtime: {e}")))
}

/// Parse command line arguments or use defaults on failure
fn parse_args_or_default() -> Args {
    match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            // Handle --version and --help specially - they should print and exit
            if e.kind() == ErrorKind::DisplayVersion || e.kind() == ErrorKind::DisplayHelp {
                e.exit();
            }
            // For actual errors, use defaults
            eprintln!("Argument parsing failed: {e}");
            eprintln!("Using default configuration for production mode");
            Args {
                config: None,
                http_port: None,
                stdio: false,
            }
        }
    }
}

/// Setup server configuration from environment and arguments
fn setup_configuration(args: &Args) -> Result<ServerConfig> {
    // Validate required environment variables before loading config
    validate_required_environment()?;

    let mut config = ServerConfig::from_env()?;

    if let Some(http_port) = args.http_port {
        config.http_port = http_port;
    }

    // Register the PostHog analytics provider before logging init so the
    // NotifyLayer can attach the sink (active only when POSTHOG_API_KEY is set).
    pierre_logging::set_analytics_provider(Arc::new(PierreAnalyticsProvider));
    // Register the notify enricher so every Slack/PostHog event carries the
    // user's email (from the identity cache) and a display emoji.
    pierre_logging::set_notify_enricher(Arc::new(PierreNotifyEnricher));
    pierre_logging::init_from_env()?;
    info!("Starting Dravr API - Production Mode");
    info!("{}", config.summary());

    validate_oauth_providers(&config);

    Ok(config)
}

/// Environment variable validation result
struct EnvValidation {
    name: &'static str,
    value: Option<String>,
    required: bool,
    description: &'static str,
}

/// Validate required environment variables at startup
///
/// Fails fast with clear error messages if critical variables are missing.
/// This prevents cryptic errors later in the startup process.
fn validate_required_environment() -> Result<()> {
    // The env-backed master key (LocalKekProvider) is unnecessary once the GCP Cloud
    // KMS KEK is compiled in and configured: the DEK is wrapped by KMS, so
    // KeyManager::configured_kek_provider never reads PIERRE_MASTER_ENCRYPTION_KEY.
    // Mirror that selection so KMS-only deployments don't demand the master key.
    #[cfg(feature = "gcp-kms")]
    let master_key_required = env::var("PIERRE_KMS_KEY_RESOURCE").is_err();
    #[cfg(not(feature = "gcp-kms"))]
    let master_key_required = true;

    let mut validations = vec![
        EnvValidation {
            name: "DATABASE_URL",
            value: env::var("DATABASE_URL").ok(),
            required: true,
            description: "Database connection string (e.g., sqlite:./data/users.db)",
        },
        EnvValidation {
            name: "PIERRE_MASTER_ENCRYPTION_KEY",
            value: env::var("PIERRE_MASTER_ENCRYPTION_KEY").ok(),
            required: master_key_required, // Required unless the GCP Cloud KMS KEK is active
            description:
                "Base64-encoded 32-byte master encryption key (generate: openssl rand -base64 32)",
        },
        EnvValidation {
            name: "BASE_URL",
            value: env::var("BASE_URL").ok(),
            required: false,
            description: "Public base URL of this server (e.g. https://app.pierre.ai). Used to construct hosted-login URLs for channel-initiated provider connections. Falls back to http://localhost:8081 for local development.",
        },
        EnvValidation {
            name: "PROVIDER_LINK_WEBHOOK_URL",
            value: env::var("PROVIDER_LINK_WEBHOOK_URL").ok(),
            required: false,
            description: "Optional outbound webhook URL for channel-initiated provider connections. When set, Dravr POSTs a `provider.linked` event to this URL on successful connection so the channel bot can post a confirmation.",
        },
        EnvValidation {
            name: "STRAVA_CLIENT_ID",
            value: env::var("STRAVA_CLIENT_ID").ok(),
            required: false,
            description: "Strava OAuth application client ID",
        },
        EnvValidation {
            name: "STRAVA_CLIENT_SECRET",
            value: env::var("STRAVA_CLIENT_SECRET").ok(),
            required: false,
            description: "Strava OAuth application client secret",
        },
        EnvValidation {
            name: "WHOOP_CLIENT_ID",
            value: env::var("WHOOP_CLIENT_ID").ok(),
            required: false,
            description: "Whoop OAuth application client ID for the shared Dravr-owned Whoop app. When set (with WHOOP_CLIENT_SECRET), Whoop connects via one-tap OAuth consent; when unset, Whoop falls back to bring-your-own OAuth app.",
        },
        EnvValidation {
            name: "WHOOP_CLIENT_SECRET",
            value: env::var("WHOOP_CLIENT_SECRET").ok(),
            required: false,
            description: "Whoop OAuth application client secret for the shared Dravr-owned Whoop app.",
        },
    ];

    // Append LLM provider-specific validations.
    // Only enforce as required when PIERRE_LLM_PROVIDER is explicitly set —
    // CI environments don't set it and don't need LLM to start the server.
    let explicitly_configured = env::var(LlmProviderType::ENV_VAR).is_ok();
    let llm_provider = LlmProviderType::from_env();
    validations.extend(llm_provider_validations(
        llm_provider,
        explicitly_configured,
    ));

    // Pre-flight Copilot token format check — diagnoses today's outage
    // pattern (wrong token type rotated into Secret Manager) at boot
    // instead of as a 5xx on every chat call.
    validate_copilot_token_format(llm_provider, explicitly_configured)?;

    let mut missing_required = Vec::new();
    let mut missing_optional = Vec::new();

    for validation in &validations {
        if validation.value.is_none() {
            if validation.required {
                missing_required.push(validation);
            } else {
                missing_optional.push(validation);
            }
        }
    }

    // Print warnings for missing optional variables
    for validation in &missing_optional {
        eprintln!(
            "WARNING: {} not set - {}",
            validation.name, validation.description
        );
    }

    // Fail fast if required variables are missing
    if !missing_required.is_empty() {
        eprintln!();
        eprintln!(
            "ERROR: Required environment variables are missing! (LLM provider: {llm_provider})"
        );
        eprintln!();
        for validation in &missing_required {
            eprintln!("  {} - {}", validation.name, validation.description);
        }
        eprintln!();
        eprintln!("Have you sourced your .envrc? Run: source .envrc");
        eprintln!();
        let missing_names = missing_required
            .iter()
            .map(|v| v.name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::config(format!(
            "Missing required environment variables: {missing_names}"
        )));
    }

    // Fail the boot on a mis-typed GUARDIAN_* security-posture variable rather
    // than silently serve traffic under the laxer default (e.g. an operator
    // arming `GUARDIAN_MODE=enforce` who typed `enfroce` would otherwise run
    // `observe`). The Guardian is a security control; an unrecognized value is a
    // configuration error, not something to paper over with a default.
    let bad_guardian = guardian::validate_env();
    if !bad_guardian.is_empty() {
        let detail = bad_guardian
            .iter()
            .map(|(name, value)| format!("{name}={value:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::config(format!(
            "Unrecognized GUARDIAN_* value(s): {detail}. Check the spelling — a \
             security-posture typo must not silently revert to the laxer default."
        )));
    }

    Ok(())
}

/// Return provider-specific environment variable validations for the configured LLM provider
///
/// When `required` is true (`PIERRE_LLM_PROVIDER` explicitly set), missing keys are fatal.
/// When false (env var not set, e.g. CI), missing keys produce warnings only.
fn llm_provider_validations(provider: LlmProviderType, required: bool) -> Vec<EnvValidation> {
    match provider {
        LlmProviderType::Groq => vec![EnvValidation {
            name: "GROQ_API_KEY",
            value: env::var("GROQ_API_KEY").ok(),
            required,
            description: "Groq API key (get one at https://console.groq.com/keys)",
        }],
        LlmProviderType::Gemini => vec![
            EnvValidation {
                name: "GEMINI_API_KEY",
                value: env::var("GEMINI_API_KEY").ok(),
                required,
                description:
                    "Google Gemini API key (get one at https://makersuite.google.com/app/apikey)",
            },
            EnvValidation {
                name: "PIERRE_LLM_DEFAULT_MODEL",
                value: env::var("PIERRE_LLM_DEFAULT_MODEL").ok(),
                required,
                description: "Default Gemini model name (e.g., gemini-2.0-flash)",
            },
            EnvValidation {
                name: "PIERRE_LLM_FALLBACK_MODEL",
                value: env::var("PIERRE_LLM_FALLBACK_MODEL").ok(),
                required,
                description: "Fallback Gemini model name (e.g., gemini-2.0-flash-lite)",
            },
        ],
        LlmProviderType::Local => vec![EnvValidation {
            name: "LOCAL_LLM_BASE_URL",
            value: env::var("LOCAL_LLM_BASE_URL").ok(),
            required: false,
            description: "Local LLM base URL (defaults to http://localhost:11434/v1 for Ollama)",
        }],
        LlmProviderType::OpenRouter => vec![EnvValidation {
            name: "OPENROUTER_API_KEY",
            value: env::var("OPENROUTER_API_KEY").ok(),
            required,
            description: "OpenRouter API key (get one at https://openrouter.ai/keys)",
        }],
        LlmProviderType::Cohere => vec![EnvValidation {
            name: "COHERE_API_KEY",
            value: env::var("COHERE_API_KEY").ok(),
            required,
            description: "Cohere API key (get one at https://dashboard.cohere.com/api-keys)",
        }],
        LlmProviderType::ClaudeCode
        | LlmProviderType::Copilot
        | LlmProviderType::CursorAgent
        | LlmProviderType::OpenCode
        | LlmProviderType::CopilotHeadless
        | LlmProviderType::GeminiCli
        | LlmProviderType::CodexCli
        | LlmProviderType::GooseCli
        | LlmProviderType::ClineCli
        | LlmProviderType::ContinueCli
        | LlmProviderType::WarpCli
        | LlmProviderType::KiroCli
        | LlmProviderType::KiloCli => {
            // CLI/ACP providers handle their own auth; no env vars required at startup
            vec![]
        }
        LlmProviderType::OpenAiApi => vec![
            EnvValidation {
                name: "OPENAI_API_BASE_URL",
                value: env::var("OPENAI_API_BASE_URL").ok(),
                required,
                description: "OpenAI-compatible API base URL (e.g., https://api.openai.com/v1)",
            },
            EnvValidation {
                name: "OPENAI_API_KEY",
                value: env::var("OPENAI_API_KEY").ok(),
                required,
                description: "API key for the OpenAI-compatible endpoint",
            },
        ],
    }
}

/// Validate `COPILOT_GITHUB_TOKEN` format when a Copilot CLI runner is the
/// active LLM provider.
///
/// Copilot CLI (`@github/copilot-linux-x64`) accepts three token types
/// (per `copilot login --help`):
///
///   1. Fine-grained PATs (v2) with the "Copilot Requests" permission —
///      `github_pat_…` prefix. This is what production uses today.
///   2. OAuth tokens from the GitHub Copilot CLI app — `ghu_…` prefix.
///   3. OAuth tokens from the GitHub CLI (`gh`) app — `gho_…` prefix.
///
/// Classic personal access tokens (`ghp_…`) and gh refresh tokens
/// (`ghr_…`) are not supported and cause runtime
/// `Authentication required` errors on every chat call.
///
/// This pre-check fails fast at boot with a specific diagnostic so a
/// bad rotation surfaces as "container did not start" instead of a 5xx
/// on every user message. Only enforced when:
///
///   * `PIERRE_LLM_PROVIDER` is explicitly set to a Copilot variant
///     (`required == true` path in [`validate_environment`]).
///   * `COPILOT_GITHUB_TOKEN` is present (callers may rely on
///     `~/.config/github-copilot/` credentials instead — in that case
///     the env var is empty and we let the CLI handle auth).
fn validate_copilot_token_format(provider: LlmProviderType, required: bool) -> AppResult<()> {
    if !required {
        return Ok(());
    }
    if !matches!(
        provider,
        LlmProviderType::Copilot | LlmProviderType::CopilotHeadless
    ) {
        return Ok(());
    }
    let Some(token) = env::var("COPILOT_GITHUB_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        // Empty env var is OK — Copilot CLI falls back to the on-disk
        // credential store. We can't validate that store here.
        info!(
            provider = %provider,
            "COPILOT_GITHUB_TOKEN not set; Copilot CLI will use on-disk credentials"
        );
        return Ok(());
    };

    let prefix_kind = if token.starts_with("github_pat_") {
        "fine_grained_pat"
    } else if token.starts_with("ghu_") {
        "copilot_oauth"
    } else if token.starts_with("gho_") {
        "gh_cli_oauth"
    } else if token.starts_with("ghp_") {
        "classic_pat_REJECTED"
    } else if token.starts_with("ghr_") {
        "gh_refresh_token_REJECTED"
    } else {
        "unknown_REJECTED"
    };

    info!(
        provider = %provider,
        prefix_kind,
        token_len = token.len(),
        "COPILOT_GITHUB_TOKEN prefix detected"
    );

    if prefix_kind.ends_with("_REJECTED") {
        return Err(AppError::config(format!(
            "COPILOT_GITHUB_TOKEN has unsupported prefix ({prefix_kind}); Copilot CLI accepts \
             github_pat_v2 fine-grained PATs (with Copilot Requests permission), gh_ OAuth \
             tokens, or ghu_ Copilot OAuth tokens. Classic ghp_ PATs and ghr_ refresh tokens \
             are rejected by Copilot's API and will fail every chat call. Generate a new \
             fine-grained PAT at https://github.com/settings/personal-access-tokens/new with \
             Account permissions → Copilot Chat: Read."
        )));
    }
    Ok(())
}

/// Validate OAuth provider credentials at startup
fn validate_oauth_providers(config: &ServerConfig) {
    info!("Validating OAuth provider credentials...");
    let all_valid = validate_all_providers(config);
    log_validation_result(all_valid);
}

/// Validate all OAuth providers and return combined result
fn validate_all_providers(config: &ServerConfig) -> bool {
    let strava_valid = config.oauth.strava.validate_and_log("strava");
    let fitbit_valid = config.oauth.fitbit.validate_and_log("fitbit");
    let garmin_valid = config.oauth.garmin.validate_and_log("garmin");
    strava_valid && fitbit_valid && garmin_valid
}

/// Log OAuth validation result
fn log_validation_result(all_valid: bool) {
    if all_valid {
        info!("OAuth credential validation passed for all enabled providers");
    } else {
        error!("OAuth credential validation failed - check client_id/client_secret configuration");
        error!("Hint: Compare secret_fingerprint with known good values to detect mismatches");
    }
}

/// Bootstrap the complete server with all dependencies
async fn bootstrap_server(config: ServerConfig, stdio_only: bool) -> Result<()> {
    // Validate feature configuration before any expensive initialization
    // The validate() function logs all enabled features
    FeatureConfig::validate()?;

    initialize_global_configs(&config)?;

    let (database, auth_manager, jwt_secret) = initialize_core_systems(&config).await?;
    let cache = initialize_cache().await?;
    let server = create_server(database, auth_manager, &jwt_secret, &config, cache).await;
    run_server(server, &config, stdio_only).await
}

fn initialize_global_configs(config: &ServerConfig) -> Result<()> {
    initialize_http_clients(config.http_client.clone());
    info!("HTTP client configuration initialized");

    initialize_route_timeouts(config.route_timeouts.clone());
    info!("Route timeout configuration initialized");

    init_server_config()?;
    info!("Static server configuration initialized");

    Ok(())
}

async fn initialize_cache() -> Result<Cache> {
    let cache = cache_from_env().await?;
    info!("Cache initialized successfully");
    Ok(cache)
}

/// Initialize core systems (key management, database, auth)
async fn initialize_core_systems(config: &ServerConfig) -> Result<(Database, AuthManager, String)> {
    let (mut key_manager, database_encryption_key) = bootstrap_key_management()?;
    let mut database = initialize_database(config, database_encryption_key).await?;
    key_manager.complete_initialization(&mut database).await?;
    info!("Two-tier key management system fully initialized");

    let repos = database.repositories();
    let jwt_secret_string = initialize_jwt_secret(&database, &repos, config).await?;
    let auth_manager = create_auth_manager(config);

    Ok((database, auth_manager, jwt_secret_string))
}

fn bootstrap_key_management() -> Result<(KeyManager, [u8; 32])> {
    let (key_manager, database_encryption_key) = KeyManager::bootstrap()?;
    info!("Two-tier key management system bootstrapped");
    Ok((key_manager, database_encryption_key))
}

async fn initialize_database(
    config: &ServerConfig,
    database_encryption_key: [u8; 32],
) -> Result<Database> {
    let database = Database::new(
        &config.database.url.to_connection_string(),
        database_encryption_key.to_vec(),
        #[cfg(feature = "postgresql")]
        &config.database.postgres_pool,
    )
    .await?;
    info!(
        "Database initialized successfully: {}",
        database.backend_info()
    );
    info!(
        "Database URL: {}",
        redact_url(&config.database.url.to_connection_string())
    );
    Ok(database)
}

/// Log startup warnings when no admin user is configured
fn log_missing_admin_warning(auto_approve: bool) {
    let oauth_behavior = if auto_approve {
        "auto-approve"
    } else {
        "pending"
    };
    error!(
        "No admin user configured! Email/password login unavailable. \
         Firebase/OAuth creates {oauth_behavior} users. Fix: cargo run --bin pierre-cli -- user create --email admin@example.com --password <password>"
    );
}

/// Check admin user status and log appropriate startup message
async fn check_admin_status(repos: &RepositoryRegistry, auto_approve: bool) {
    match repos.users.get_by_status("active", None).await {
        Ok(users) => {
            let admin_exists = users.iter().any(|u| u.is_admin);
            if admin_exists {
                info!("Admin user configured - server ready for authentication");
            } else {
                log_missing_admin_warning(auto_approve);
            }
        }
        Err(e) => {
            error!("Failed to check admin user status: {e}");
        }
    }
}

async fn initialize_jwt_secret(
    database: &Database,
    repos: &RepositoryRegistry,
    config: &ServerConfig,
) -> Result<String> {
    let jwt_secret_string = repos
        .security
        .get_or_create_system_secret("admin_jwt_secret")
        .await?;

    info!("Admin JWT secret ready for secure token generation");

    // Check auto-approval setting
    // Precedence: env var (if set) > database > default
    let auto_approve = if config.app_behavior.auto_approve_users_from_env {
        config.app_behavior.auto_approve_users
    } else {
        database
            .is_auto_approval_enabled()
            .await
            .ok()
            .flatten()
            .unwrap_or(config.app_behavior.auto_approve_users)
    };

    check_admin_status(repos, auto_approve).await;

    let domains = &config.app_behavior.auto_approve_domains;
    if !domains.is_empty() {
        info!(
            "Auto-approve domains: {} (users from these domains bypass approval)",
            domains.join(", ")
        );
    }

    Ok(jwt_secret_string)
}

fn create_auth_manager(config: &ServerConfig) -> AuthManager {
    #[allow(clippy::cast_possible_wrap)]
    {
        let auth_manager = AuthManager::new(config.auth.jwt_expiry_hours as i64);
        info!("Authentication manager initialized with RS256");
        auth_manager
    }
}

/// Build the production [`pierre_llm::ChatProvider`] singleton ONCE at startup.
///
/// Every chat / coach / social / memory / health-probe caller shares this one
/// instance. For the Copilot Headless backend this is what keeps a single
/// `copilot --acp` subprocess + its in-memory GitHub→Copilot OAuth token
/// cache warm across the process lifetime — without it, each call (and each
/// 5-minute probe tick) would respawn the subprocess and re-run the token
/// exchange.
///
/// Returns `None` on initialization failure so the binary keeps booting;
/// `/ready` surfaces the failure via the LLM health probe, and the per-call
/// fallback in `chat_provider_from_resources_arc` preserves the old
/// build-per-call behaviour for request paths.
async fn build_chat_provider_singleton() -> Option<Arc<pierre_llm::ChatProvider>> {
    match pierre_llm::ChatProvider::from_env().await {
        Ok(p) => {
            info!("ChatProvider singleton built; reused across all callers");
            Some(Arc::new(p))
        }
        Err(e) => {
            error!(
                error = %e,
                "ChatProvider::from_env failed at startup; chat callers will fall back to per-call build"
            );
            None
        }
    }
}

/// Build the production billing provider from environment configuration.
///
/// When `STRIPE_SECRET_KEY` and `STRIPE_WEBHOOK_SECRET` are both present, wires
/// the real `StripeProvider` (which additionally requires the
/// `STRIPE_PRICE_PROFESSIONAL` / `STRIPE_PRICE_ENTERPRISE` price ids). When the
/// Stripe environment is absent, returns `None` so resources init falls back to
/// the in-tree `DummyProvider` for local dev. A partial / malformed Stripe
/// configuration is logged and also falls back rather than crashing startup.
fn build_billing_provider() -> Option<Arc<dyn BillingProvider>> {
    let stripe_configured = env::var("STRIPE_SECRET_KEY").is_ok_and(|v| !v.trim().is_empty())
        && env::var("STRIPE_WEBHOOK_SECRET").is_ok_and(|v| !v.trim().is_empty());

    if !stripe_configured {
        info!("Stripe billing not configured; using DummyProvider");
        return None;
    }

    match StripeProvider::from_env() {
        Ok(provider) => {
            // secret_fingerprint() is a SHA-256 prefix, never the raw key.
            info!(
                secret_fingerprint = %provider.client_fingerprint(),
                "Stripe billing provider configured"
            );
            Some(Arc::new(provider) as Arc<dyn BillingProvider>)
        }
        Err(e) => {
            error!(
                error = %e,
                "Stripe env partially configured but StripeProvider::from_env failed; falling back to DummyProvider"
            );
            None
        }
    }
}

/// Create server instance with all resources
async fn create_server(
    database: Database,
    auth_manager: AuthManager,
    jwt_secret: &str,
    config: &ServerConfig,
    cache: Cache,
) -> ProviderToolRouter {
    let rsa_key_size = get_rsa_key_size();
    info!("Using {}-bit RSA keys for JWT signing", rsa_key_size);

    // Seed messaging channel configs from environment variables (idempotent upsert)
    #[cfg(feature = "client-messaging")]
    {
        use pierre_mcp_server::messaging_seed;
        let repos = database.repositories();
        messaging_seed::seed_from_env(&repos).await;
    }

    // In-memory (test) databases run headless: skip building the ChatProvider so a
    // test-spawned server never starts an interactive CLI provider (e.g. `copilot
    // --acp`, which blocks on browser OAuth). Tests that exercise chat inject a mock.
    let chat_provider_singleton = if config.database.url.is_memory() {
        info!(
            "In-memory (test) database detected — skipping ChatProvider startup build (headless)"
        );
        None
    } else {
        build_chat_provider_singleton().await
    };
    let billing_provider = build_billing_provider();

    let resources_instance = ServerContext::new(
        database,
        auth_manager,
        jwt_secret,
        Arc::new(config.clone()),
        cache,
        ServerContextOptions {
            rsa_key_size_bits: Some(rsa_key_size),
            jwks_manager: None, // Generate new JWKS manager for production
            llm_provider: None, // Use ChatProvider::from_env() for LLM in production
            chat_provider: chat_provider_singleton,
            extra_tools: Vec::new(),
            billing_provider,
        },
    )
    .await;

    let resources = spawn_background_workers(resources_instance);

    server_lifecycle::notify_started();

    // The shutdown notice races Cloud Run's ~10s SIGTERM grace window; the short
    // flush sleep gives it a chance to send before the process is killed.
    #[cfg(unix)]
    tokio::spawn(async {
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
                server_lifecycle::notify_stopping();
                sleep(Duration::from_secs(3)).await;
            }
            Err(e) => {
                warn!(error = %e, "Failed to install SIGTERM handler for shutdown notification");
            }
        }
    });

    // Initialize product analytics (PostHog or noop)
    pierre_mcp_server::init_analytics();

    ProviderToolRouter::new(resources)
}

/// Spawn background workers (messaging outbound, Discord, Slack), returning shared resources
fn spawn_background_workers(resources_instance: ServerContext) -> Arc<ServerContext> {
    let resources = Arc::new(resources_instance);

    // Install the MCP protocol-stream factory on the SSE manager now that
    // the composition-root Arc<ServerContext> exists. The SSE crate
    // doesn't know about the concrete McpProtocolStream type
    // (ToolHandlers dispatch lives in pierre-server); the factory closes
    // over the resources handle so each per-session stream gets it.
    #[cfg(feature = "transport-sse")]
    {
        use pierre_mcp_server::sse::protocol::McpProtocolStreamFactory;
        let factory = Arc::new(McpProtocolStreamFactory {
            resources: Arc::clone(&resources),
        });
        // Ignore the result: a second install attempt is a startup-time
        // logic bug, not a runtime condition worth aborting on (the first
        // factory has already won). Surface via tracing for visibility.
        if resources
            .sse
            .sse_manager
            .install_protocol_factory(factory)
            .is_err()
        {
            warn!("SseManager protocol factory already installed; skipping");
        }
    }

    // Install the chat-pipeline re-entry handle on the backfill-completion
    // notifier now that the composition-root Arc<ServerContext> exists. The
    // notifier is built pre-Arc (inside ServerContext::new), so the handle —
    // which needs the Arc to drive `pierre_chat_pipeline::run` — is plumbed in
    // here, holding a Weak<ServerContext> to avoid a DI-graph cycle. Mirrors the
    // SSE protocol-factory install above. Lets a finished historical backfill
    // push a real in-persona coach answer instead of a templated activity list.
    #[cfg(feature = "client-messaging")]
    {
        use pierre_mcp_server::services::backfill_notifier::install_backfill_reentry;
        install_backfill_reentry(&resources);
    }

    // Install the AuthService-backed credential refresher on the health-sync
    // storage now that the composition-root Arc<ServerContext> exists. Routes
    // scheduled-sync credential reads through the same refresh-and-persist
    // path live tool calls use, so an expired provider token no longer fails
    // every sync until a user happens to trigger a live fetch.
    #[cfg(feature = "health-sync")]
    {
        use pierre_mcp_server::services::health_sync_refresher::install_health_sync_refresher;
        install_health_sync_refresher(&resources);
    }

    // Spawn LLM startup health probe — populates resources.llm_health so
    // /ready and /health/llm reflect the boot-time round-trip. Fire-and-
    // forget; failures only surface via the dedicated readiness route
    // (Cloud Run startup probe is /health by default and stays unaffected).
    //
    // Skipped for in-memory (test) servers: probing would exercise the CLI
    // provider and spawn `copilot --acp`, which blocks on browser OAuth.
    if resources.common.config.database.url.is_memory() {
        info!("In-memory (test) database detected — skipping LLM health probe (headless)");
    } else {
        spawn_llm_health_probe(
            Arc::clone(&resources.common.llm_health),
            resources.common.chat_provider.as_ref().map(Arc::clone),
        );
    }

    // Start messaging outbound retry worker (polls queue every 5 seconds)
    #[cfg(feature = "client-messaging")]
    {
        use pierre_mcp_server::start_outbound_worker;
        start_outbound_worker(Arc::clone(&resources.common.repos.messaging));
        info!("Messaging outbound retry worker started");
    }

    // Start coach followup scheduler (polls coach_followups every 60 seconds)
    {
        use pierre_mcp_server::start_followup_scheduler;
        start_followup_scheduler(
            Arc::clone(&resources.common.repos.memory),
            #[cfg(feature = "client-notifications")]
            resources.common.notification_service.clone(),
        );
    }

    // Start the short-link sweeper (deletes expired reconnect/connect links
    // every few hours). The shortener mints a row per link and the chat
    // reconnect path mints on every expired-session turn by design, so without
    // this reclaim the short_links table grows unbounded.
    {
        use pierre_mcp_server::start_short_link_sweeper;
        start_short_link_sweeper(Arc::clone(&resources.common.repos.short_links));
    }

    // Coaching background workers: the outcome evaluator, archetype aggregation
    // and the commitment sweep. Skipped for in-memory test servers.
    {
        use pierre_mcp_server::start_coaching_workers;
        start_coaching_workers(&resources);
    }

    // Start group weekly-digest scheduler (weekly cadence). Reads the
    // per-tenant `weekly_digest` tier flag to decide eligibility, computes
    // the same weekly report the on-demand /report endpoint returns, and
    // pushes it to each group's owner/admins. The spawned task is fire-and-
    // forget: the scheduler is best-effort and a restart re-arms the timer.
    #[cfg(feature = "client-groups")]
    {
        use pierre_routes_social::group_digest_scheduler::start_digest_scheduler;
        start_digest_scheduler(
            Arc::clone(&resources),
            #[cfg(feature = "client-notifications")]
            resources.common.notification_service.clone(),
        );
    }

    // Start Discord Gateway WebSocket client for real-time message delivery
    #[cfg(feature = "client-messaging")]
    {
        use pierre_mcp_server::start_discord_gateway;
        start_discord_gateway(&resources);
    }

    // Start Slack Socket Mode WebSocket client for real-time Slack event delivery
    // (no-op when SLACK_APP_TOKEN is unset; webhook path remains the default)
    #[cfg(feature = "client-messaging")]
    {
        use pierre_mcp_server::start_slack_socket_mode;
        start_slack_socket_mode(&resources);
    }

    resources
}

/// Get RSA key size from environment or use production default
fn get_rsa_key_size() -> usize {
    env::var("PIERRE_RSA_KEY_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4096)
}

/// Run the server after displaying endpoints
async fn run_server(
    server: ProviderToolRouter,
    config: &ServerConfig,
    stdio_only: bool,
) -> Result<()> {
    if stdio_only {
        run_stdio_only_mode(server).await
    } else {
        run_http_mode(server, config).await
    }
}

/// Run server in stdio-only mode (no HTTP/SSE transports)
///
/// Drives the shared `dravr_tronc` MCP engine over stdin/stdout. The stdio
/// transport carries no HTTP bearer, so requests run under the anonymous
/// context — auth-gated tools reject; this mode suits a trusted local client.
async fn run_stdio_only_mode(server: ProviderToolRouter) -> Result<()> {
    info!("Starting in stdio-only mode (HTTP/SSE transports disabled)");
    info!("Listening on stdin/stdout for MCP JSON-RPC messages");

    let mcp_server = build_mcp_server(server.resources());
    stdio::run(mcp_server).await.map_err(|e| {
        error!("Stdio transport error: {}", e);
        AppError::internal(format!("Stdio transport error: {e}"))
    })?;

    Ok(())
}

/// Run server in HTTP mode with all transports
async fn run_http_mode(server: ProviderToolRouter, config: &ServerConfig) -> Result<()> {
    info!(
        "Server starting on port {} (unified MCP and HTTP)",
        config.http_port
    );
    display_available_endpoints(config);
    info!("Ready to serve fitness data!");

    server.run(config.http_port).await.map_err(|e| {
        error!("Server error: {}", e);
        e
    })?;

    Ok(())
}

/// Display all available API endpoints with their ports
fn display_available_endpoints(config: &ServerConfig) {
    // Default to 127.0.0.1 for local development - production uses reverse proxy
    let host = "127.0.0.1";

    info!("=== Available API Endpoints ===");
    display_mcp_endpoints(host, config.http_port);
    display_auth_endpoints(host, config.http_port);
    display_oauth2_endpoints(host, config.http_port);
    display_oauth_callback_urls(host, config);
    display_admin_endpoints(host, config.http_port);
    display_api_key_endpoints(host, config.http_port);
    display_tenant_endpoints(host, config.http_port);
    display_dashboard_endpoints(host, config.http_port);
    display_a2a_endpoints(host, config.http_port);
    display_config_endpoints(host, config.http_port);
    display_fitness_endpoints(host, config.http_port);
    display_notification_endpoints(host, config.http_port);
    info!("=== End of Endpoint List ===");
}

/// Endpoint category definition for structured display
struct EndpointCategory {
    name: &'static str,
    endpoints: &'static [(&'static str, &'static str, &'static str)], // (description, method, path)
}

/// Display a category of endpoints with consistent formatting
fn display_endpoint_category(category: &EndpointCategory, host: &str, port: u16) {
    info!("{}", category.name);
    for (description, method, path) in category.endpoints {
        info!("   {description:18} {method} http://{host}:{port}{path}");
    }
}

fn display_mcp_endpoints(host: &str, port: u16) {
    let endpoints = [
        "MCP Protocol:",
        &format!("   HTTP Transport:    http://{host}:{port}/mcp"),
        &format!("   Server-Sent Events: http://{host}:{port}/mcp/sse"),
    ];
    for line in &endpoints {
        info!("{}", line);
    }
}

fn display_auth_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Authentication & OAuth:",
        endpoints: &[
            ("User Registration:", "POST", "/auth/register"),
            ("User Login:", "POST", "/auth/login"),
            ("OAuth Authorize:", "GET", "/api/oauth/authorize/{provider}"),
            ("OAuth Callback:", "GET", "/api/oauth/callback/{provider}"),
            ("OAuth Status:", "GET", "/api/oauth/status"),
            (
                "OAuth Disconnect:",
                "POST",
                "/api/oauth/disconnect/{provider}",
            ),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_oauth2_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "OAuth 2.0 Server:",
        endpoints: &[
            ("Authorization:", "GET", "/oauth2/authorize"),
            ("Token Exchange:", "POST", "/oauth2/token"),
            ("Client Registration:", "POST", "/oauth2/register"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_oauth_callback_urls(_host: &str, config: &ServerConfig) {
    let endpoints = [
        "OAuth Callback URLs (MCP Bridge):",
        &format!(
            "   Bridge Callback:   http://localhost:{}/oauth/callback",
            config.oauth_callback_port
        ),
        &format!(
            "   Focus Recovery:    http://localhost:{}/oauth/focus-recovery",
            config.oauth_callback_port
        ),
        &format!(
            "   Provider Callback: http://localhost:{}/oauth/provider-callback/{{provider}}",
            config.oauth_callback_port
        ),
    ];
    for line in &endpoints {
        info!("{}", line);
    }
}

fn display_admin_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Admin Management:",
        endpoints: &[
            ("Admin Setup:", "POST", "/admin/setup"),
            ("Create User:", "POST", "/admin/users"),
            ("List Users:", "GET", "/admin/users"),
            ("Generate Token:", "POST", "/admin/tokens"),
            ("List Tokens:", "GET", "/admin/tokens"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_api_key_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "API Key Management:",
        endpoints: &[
            ("Create API Key:", "POST", "/api/keys"),
            ("List API Keys:", "GET", "/api/keys"),
            ("Delete API Key:", "DELETE", "/api/keys/{key_id}"),
            ("API Key Usage:", "GET", "/api/keys/usage"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_tenant_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Tenant Management:",
        endpoints: &[
            ("Create Tenant:", "POST", "/tenants"),
            ("List Tenants:", "GET", "/tenants"),
            ("Get Tenant:", "GET", "/tenants/{tenant_id}"),
            ("Update Tenant:", "PUT", "/tenants/{tenant_id}"),
            ("Delete Tenant:", "DELETE", "/tenants/{tenant_id}"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_dashboard_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Dashboard & Monitoring:",
        endpoints: &[
            ("Health Check:", "GET", "/health"),
            ("Plugin Status:", "GET", "/health/plugins"),
            ("System Status:", "GET", "/dashboard/status"),
            ("User Dashboard:", "GET", "/dashboard/user"),
            ("Admin Dashboard:", "GET", "/dashboard/admin"),
            ("Detailed Stats:", "GET", "/dashboard/detailed"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_a2a_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "A2A Protocol:",
        endpoints: &[
            ("A2A Status:", "GET", "/a2a/status"),
            ("Agent Card:", "GET", "/.well-known/agent-card.json"),
            ("Clients (list/create):", "GET/POST", "/a2a/clients"),
            ("Client (get/delete):", "GET/DELETE", "/a2a/clients/{id}"),
            ("Client Usage:", "GET", "/a2a/clients/{id}/usage"),
            ("Client Rate Limit:", "GET", "/a2a/clients/{id}/rate-limit"),
            ("Dashboard Overview:", "GET", "/a2a/dashboard/overview"),
            ("Dashboard Analytics:", "GET", "/a2a/dashboard/analytics"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_config_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Configuration:",
        endpoints: &[
            ("Get Config:", "GET", "/config"),
            ("Update Config:", "PUT", "/config"),
            ("User Config:", "GET", "/config/user"),
            ("Update User Config:", "PUT", "/config/user"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_fitness_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Fitness Configuration:",
        endpoints: &[
            ("Get Fitness Config:", "GET", "/fitness/config"),
            ("Update Fitness Config:", "PUT", "/fitness/config"),
            ("Delete Fitness Config:", "DELETE", "/fitness/config"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_notification_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Real-time Notifications:",
        endpoints: &[("SSE Stream:", "GET", "/notifications/sse?user_id={user_id}")],
    };
    display_endpoint_category(&category, host, port);
}
