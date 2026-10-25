// ABOUTME: Shared test utilities and setup functions for integration tests
// ABOUTME: Provides common database, auth, and user creation helpers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    dead_code,
    clippy::wildcard_in_or_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args,
    clippy::redundant_closure_for_method_calls
)]
//! Shared test utilities for `pierre_mcp_server`
//!
//! This module provides common test setup functions to reduce duplication
//! across integration tests.

use anyhow::Result;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::api_keys::{ApiKey, ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
use pierre_auth::auth::{AuthManager, DelegatedGrant};
use pierre_auth::key_management::{KeyManager, LocalKekProvider, MasterEncryptionKey};
use pierre_cache::{Cache, CacheConfig};
use pierre_config::environment::{
    GuardianSettings, HttpClientConfig, OAuthConfig, OAuthProviderConfig, ProviderEndpointOverride,
    SciotteServiceConfig, ServerConfig,
};
use pierre_core::billing::BillingProvider;
use pierre_core::llm::LlmProvider;
use pierre_core::models::ConnectionType;
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_llm::config::LlmChain;
use pierre_llm::ChatProvider;
use pierre_mcp_server::services::turn_runner::TurnRunner;
use pierre_mcp_server::{
    constants,
    mcp::resources::{ServerContext, ServerContextOptions},
    routes::mcp::McpRoutes,
    utils,
};
use pierre_middleware::McpAuthMiddleware;
use pierre_test_support::db::create_test_db_with_key;
use pierre_tool_runtime::RuntimeTool;
use rand::Rng;
use secrecy::SecretString;
use std::future::Future;
use std::pin::Pin;
use std::{
    env,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::{Arc, LazyLock, Once},
    time::Duration as StdDuration,
};
use tokio::{net::TcpListener as TokioTcpListener, task::JoinHandle, time::sleep as tokio_sleep};
use uuid::Uuid;

static INIT_LOGGER: Once = Once::new();
static INIT_HTTP_CLIENTS: Once = Once::new();
static INIT_SERVER_CONFIG: Once = Once::new();

/// The model [`test_server_config`] gives the LLM chain's head
/// (`gemini=mock-model`, see [`placeholder_llm_chain`]).
///
/// Exported so a suite that DOES reach a provider can refuse to run on it
/// rather than discovering the placeholder in a pricing warning: no backend
/// serves this id, so a live turn built with it is graded against whatever the
/// provider fell back to and costed at zero.
pub(crate) const PLACEHOLDER_LLM_MODEL: &str = "mock-model";

/// Bcrypt cost the fixture users below are hashed at.
///
/// Production picks its own cost at each of its own call sites; this value
/// reaches nothing but test fixtures. `bcrypt::verify` reads the cost back out
/// of the hash, so the suites that log in as a fixture user with "password123"
/// authenticate exactly as before — they just stop paying
/// `bcrypt::DEFAULT_COST` (12), which is about a second per user at the test
/// profile's opt-level, once per created user in over a hundred test files.
pub(crate) const TEST_BCRYPT_COST: u32 = 4;

/// Initialize server configuration for tests (call once per test process)
pub(crate) fn init_server_config() {
    INIT_SERVER_CONFIG.call_once(|| {
        // Force CI mode for the whole test process. Tests that boot a live
        // server via `ServerConfig::from_env()` (mcp_protocol_comprehensive,
        // mcp_multitenant_specific, configuration_e2e) call this before
        // `from_env()`, so `ci_mode` resolves to true. Combined with
        // `ServerConfig::outbound_email_credentials()` — which returns None in
        // CI mode — this stops a developer's real RESEND_API_KEY (inherited
        // from .envrc) from building a live email service and sending real
        // mail to throwaway addresses on every registration/approval/reset
        // flow a test exercises. Do NOT remove this line; it is the local
        // half of the outbound-email flood guard.
        env::set_var("CI", "true");

        // Point the messaging command loader at the repo-root `commands/`
        // directory so command handlers are populated in the test
        // `ServerContext`. Without this, the loader resolves relative to
        // the test process CWD (`crates/pierre-server/`) and finds nothing.
        let commands_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("commands"));
        if let Some(path) = commands_path {
            env::set_var("PIERRE_COMMANDS_DIR", path);
        }

        // Set OAuth environment variables for testing
        env::set_var("PIERRE_STRAVA_CLIENT_ID", TEST_STRAVA_CLIENT_ID);
        env::set_var("PIERRE_STRAVA_CLIENT_SECRET", "test_strava_client_secret");
        env::set_var("PIERRE_GARMIN_CLIENT_ID", "test_garmin_client_id");
        env::set_var("PIERRE_GARMIN_CLIENT_SECRET", "test_garmin_client_secret");

        // Err when another caller set it first; either way it must be set now.
        drop(constants::init_server_config());
        assert!(
            constants::get_server_config().is_some(),
            "server config did not initialize from the test environment"
        );
    });
}

/// Shared JWKS manager for all tests (generated once, reused everywhere)
/// This eliminates expensive RSA key generation (100ms+ per key) in every test
static SHARED_TEST_JWKS: LazyLock<Arc<JwksManager>> = LazyLock::new(|| {
    let mut jwks = JwksManager::new();
    jwks.generate_rsa_key_pair_with_size("shared_test_key", 2048)
        .expect("Failed to generate shared test JWKS key");
    Arc::new(jwks)
});

/// Initialize quiet logging for tests (call once per test process)
pub(crate) fn init_test_logging() {
    INIT_LOGGER.call_once(|| {
        // Check for TEST_LOG environment variable to control test logging level
        let log_level = match env::var("TEST_LOG").as_deref() {
            Ok("TRACE") => tracing::Level::TRACE,
            Ok("DEBUG") => tracing::Level::DEBUG,
            Ok("INFO") => tracing::Level::INFO,
            Ok("WARN" | "ERROR") | _ => tracing::Level::WARN, // Default to WARN for quiet tests
        };

        tracing_subscriber::fmt()
            .with_max_level(log_level)
            .with_test_writer()
            .try_init()
            // One test binary hosts every suite, so a suite that installed
            // its own global subscriber first keeps it.
            .ok();
    });
}

/// Initialize HTTP clients for tests (call once per test process)
///
/// This function ensures HTTP client configuration is initialized exactly once
/// across all tests in the process. It uses default configuration suitable for testing.
///
/// Call this function at the start of any test that uses HTTP clients, either directly
/// or indirectly through providers or other components that make HTTP requests.
///
/// Safe to call multiple times - initialization happens only once due to `Once` guard.
pub(crate) fn init_test_http_clients() {
    INIT_HTTP_CLIENTS.call_once(|| {
        utils::http_client::initialize_http_clients(HttpClientConfig::default());
    });
}

/// Standard test database setup.
///
/// Opens whatever backend `DATABASE_URL` names — a private `PostgreSQL`
/// database on the CI lane, in-memory `SQLite` locally — through the one
/// factory in `pierre_test_support::db`. Tests never spell a
/// database URL themselves; that is how this suite ran `SQLite` under the
/// `PostgreSQL` banner for fourteen months.
pub(crate) async fn create_test_database() -> Result<Arc<Database>> {
    create_test_database_with_key(generate_encryption_key().to_vec()).await
}

/// Standard test database setup with custom encryption key
pub(crate) async fn create_test_database_with_key(
    encryption_key: Vec<u8>,
) -> Result<Arc<Database>> {
    init_test_logging();
    Ok(Arc::new(create_test_db_with_key(encryption_key).await?))
}

/// Get shared test JWKS manager (reused across all tests for performance)
pub(crate) fn get_shared_test_jwks() -> Arc<JwksManager> {
    SHARED_TEST_JWKS.clone()
}

/// Create test authentication manager
pub(crate) fn create_test_auth_manager() -> Arc<AuthManager> {
    Arc::new(AuthManager::new(24))
}

/// Create test authentication middleware
pub(crate) fn create_test_auth_middleware(
    auth_manager: &Arc<AuthManager>,
    database: &Database,
) -> Arc<McpAuthMiddleware> {
    // Use shared JWKS manager instead of generating new keys
    let jwks_manager = get_shared_test_jwks();
    let repos = Arc::clone(database.repositories());
    Arc::new(McpAuthMiddleware::new(
        (**auth_manager).clone(),
        repos,
        jwks_manager,
    ))
}

/// Create test cache with background cleanup disabled
pub(crate) async fn create_test_cache() -> Result<Cache> {
    let cache_config = CacheConfig {
        max_entries: 1000,
        redis_url: None,
        cleanup_interval: StdDuration::from_mins(1),
        enable_background_cleanup: false, // Disable background cleanup for tests
        ..Default::default()
    };
    Cache::new(cache_config)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create test cache: {e}"))
}

/// Create a standard test user
pub(crate) async fn create_test_user(database: &Database) -> Result<(Uuid, User)> {
    // Create a proper bcrypt hash for the default test password "password123"
    let password_hash = bcrypt::hash("password123", TEST_BCRYPT_COST)?;

    let mut user = User::new(
        "test@example.com".to_owned(),
        password_hash,
        Some("Test User".to_owned()),
    );

    // Activate the user for testing (bypass admin approval)
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id); // Self-approved for testing
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    let repos = database.repositories();
    repos.users.create(&user).await?;

    // Create the tenant with this user as owner
    // The create_tenant function automatically adds the owner to tenant_users
    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Test Tenant".to_owned(),
        slug: format!("test-tenant-{}", tenant_id),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    repos.tenants.create(&tenant).await?;

    // Also update legacy tenant_id column for backward compatibility
    repos.users.update_tenant_id(user_id, tenant_id).await?;

    Ok((user_id, user))
}

/// Create a test user with custom email
pub(crate) async fn create_test_user_with_email(
    database: &Database,
    email: &str,
) -> Result<(Uuid, User)> {
    let (user_id, user, _tenant_id) =
        create_test_user_with_plan(database, email, "starter").await?;
    Ok((user_id, user))
}

/// Create an active test user owning a tenant on the given billing `plan`.
///
/// Returns the tenant id too — group tests need a plan whose tier enables
/// group coaching (`professional`/`enterprise`); `starter` has groups disabled.
pub(crate) async fn create_test_user_with_plan(
    database: &Database,
    email: &str,
    plan: &str,
) -> Result<(Uuid, User, TenantId)> {
    // Create a proper bcrypt hash for the default test password "password123"
    let password_hash = bcrypt::hash("password123", TEST_BCRYPT_COST)?;

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Test User".to_owned()),
    );

    // Activate the user for testing (bypass admin approval)
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id); // Self-approved for testing
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    let repos = database.repositories();
    repos.users.create(&user).await?;

    // Create the tenant with this user as owner
    // The create_tenant function automatically adds the owner to tenant_users
    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Test Tenant for {}", email),
        slug: format!("test-tenant-{}", tenant_id),
        domain: None,
        plan: plan.to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    repos.tenants.create(&tenant).await?;

    // Also update legacy tenant_id column for backward compatibility
    repos.users.update_tenant_id(user_id, tenant_id).await?;

    Ok((user_id, user, tenant_id))
}

/// Generate a JWT token for a test user that includes their `active_tenant_id`.
///
/// Route handlers require `active_tenant_id` in the JWT claims to resolve the
/// user's tenant. This helper looks up the user's tenant from the database and
/// includes it in the generated token.
pub(crate) async fn generate_test_token(resources: &Arc<ServerContext>, user: &User) -> String {
    let repos = resources.agent.database.repositories();
    let tenants = repos.tenants.list_for_user(user.id).await.unwrap();
    let tenant_id = tenants.first().map(|t| t.id.to_string());
    resources
        .auth
        .auth_manager
        .generate_token_with_tenant(user, &resources.auth.jwks_manager, tenant_id)
        .unwrap()
}

/// Mint the credential an MCP or A2A client holds for a test user: an OAuth
/// access token carrying every delegable scope, with the user's tenant active.
///
/// `/mcp` and A2A refuse a first-party session token (carnet#768), so a test
/// that drives them as an integration authenticates with this instead of
/// [`generate_test_token`]. It never carries `admin`, which is not delegable:
/// a test that needs admin tools over `/mcp` uses [`issue_test_api_key`].
pub(crate) async fn generate_delegated_test_token(
    resources: &ServerContext,
    user: &User,
) -> String {
    let repos = resources.agent.database.repositories();
    let tenants = repos.tenants.list_for_user(user.id).await.unwrap();
    let tenant_id = tenants.first().map(|t| t.id.to_string());
    delegated_test_token_for_tenant(resources, user.id, tenant_id)
}

/// [`generate_delegated_test_token`] with an explicit active tenant (`None`
/// leaves the user's default tenant to be resolved per request).
pub(crate) fn delegated_test_token_for_tenant(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: Option<String>,
) -> String {
    let scopes: Vec<String> = OAuthScope::delegable_as_str()
        .into_iter()
        .map(str::to_owned)
        .collect();
    resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: "mcp_client_test",
                scopes: &scopes,
                audience: None,
            },
            &user_id,
            &[],
            tenant_id,
        )
        .unwrap()
}

/// Store a fresh API key for `user_id` and return the full key a client sends.
///
/// The athlete's own credential, so it carries the self grant: the one way a
/// test reaches an admin tool over `/mcp` for a global admin.
pub(crate) async fn issue_test_api_key(
    database: &Database,
    user_id: Uuid,
    name: &str,
) -> Result<String> {
    let request = CreateApiKeyRequest {
        name: name.to_owned(),
        description: Some("Test API key".to_owned()),
        tier: ApiKeyTier::Professional,
        rate_limit_requests: Some(100_000),
        expires_in_days: None,
    };
    let (api_key, full_key) = ApiKeyManager::new().create_api_key(user_id, request)?;
    database.repositories().api_keys.create(&api_key).await?;
    Ok(full_key)
}

/// Create a test API key for a user (returns API key string)
pub(crate) fn create_test_api_key(
    _database: &Database,
    user_id: Uuid,
    name: &str,
) -> Result<String> {
    let request = CreateApiKeyRequest {
        name: name.to_owned(),
        description: Some("Test API key".to_owned()),
        tier: ApiKeyTier::Starter,
        rate_limit_requests: Some(1000),
        expires_in_days: None,
    };

    let manager = ApiKeyManager::new();
    let (_, api_key_string) = manager.create_api_key(user_id, request)?;
    Ok(api_key_string)
}

/// Create a test API key and store it in the database (returns `ApiKey` object)
pub(crate) async fn create_and_store_test_api_key(
    database: &Database,
    user_id: Uuid,
    name: &str,
) -> Result<ApiKey> {
    let request = CreateApiKeyRequest {
        name: name.to_owned(),
        description: Some("Test API key".to_owned()),
        tier: ApiKeyTier::Starter,
        rate_limit_requests: Some(1000),
        expires_in_days: None,
    };

    let manager = ApiKeyManager::new();
    let (api_key, _) = manager.create_api_key(user_id, request)?;
    let repos = database.repositories();
    repos.api_keys.create(&api_key).await?;
    Ok(api_key)
}

/// Complete test environment setup
/// Returns (database, `auth_manager`, `auth_middleware`, `user_id`, `api_key`)
pub(crate) async fn setup_test_environment() -> Result<(
    Arc<Database>,
    Arc<AuthManager>,
    Arc<McpAuthMiddleware>,
    Uuid,
    String,
)> {
    let database = create_test_database().await?;
    let auth_manager = create_test_auth_manager();
    let auth_middleware = create_test_auth_middleware(&auth_manager, &database);

    let (user_id, _user) = create_test_user(&database).await?;
    let api_key = create_test_api_key(&database, user_id, "test-key")?;

    Ok((database, auth_manager, auth_middleware, user_id, api_key))
}

/// Lightweight test environment for simple tests
/// Returns (database, `user_id`)
pub(crate) async fn setup_simple_test_environment() -> Result<(Arc<Database>, Uuid)> {
    let database = create_test_database().await?;
    let (user_id, _user) = create_test_user(&database).await?;
    Ok((database, user_id))
}

/// Test environment with custom user tier
pub(crate) async fn setup_test_environment_with_tier(
    tier: UserTier,
) -> Result<(Arc<Database>, Uuid)> {
    let database = create_test_database().await?;
    let mut user = User::new(
        "test@example.com".to_owned(),
        "test_hash".to_owned(),
        Some("Test User".to_owned()),
    );
    user.tier = tier;
    let user_id = user.id;

    let repos = database.repositories();
    repos.users.create(&user).await?;
    Ok((database, user_id))
}

/// Create test `ServerContext` with all components properly initialized
/// This replaces individual resource creation for proper architectural patterns
pub(crate) async fn create_test_server_resources() -> Result<Arc<ServerContext>> {
    Box::pin(create_test_server_resources_inner(None, Vec::new(), None)).await
}

/// Same as [`create_test_server_resources`] but injects a caller-supplied
/// [`LlmProvider`] implementation into [`ServerContext::llm_provider`].
///
/// Tests that need the chat pipeline to actually run (for example the
/// conversation-turn E2E) use this to wire a deterministic mock so the
/// pipeline writes real `llm_usage` rows without touching the network.
pub(crate) async fn create_test_server_resources_with_llm(
    provider: Arc<dyn LlmProvider + 'static>,
) -> Result<Arc<ServerContext>> {
    Box::pin(create_test_server_resources_inner(
        Some(provider),
        Vec::new(),
        None,
    ))
    .await
}

/// Same as [`create_test_server_resources_with_llm`] plus caller-supplied
/// MCP tools injected into the default registry.
///
/// Used by messaging-eval integration tests that need a no-auth stub
/// tool to exercise the tool-execution loop end-to-end: the mock LLM
/// emits a `<tool_call>` block for the stub's name, the pipeline
/// routes to it via the registry, the stub returns canned data, and
/// the summary row records `tools_called` with the real pipeline-
/// populated value rather than a direct `llm_usage` seed.
pub(crate) async fn create_test_server_resources_with_llm_and_tools(
    provider: Arc<dyn LlmProvider + 'static>,
    extra_tools: Vec<Arc<dyn RuntimeTool>>,
) -> Result<Arc<ServerContext>> {
    Box::pin(create_test_server_resources_inner(
        Some(provider),
        extra_tools,
        None,
    ))
    .await
}

/// Same as [`create_test_server_resources`] but with a caller-supplied config.
///
/// For tests that assert config-driven wiring — for example the MCP `Origin`
/// allowlist, which is empty under [`ServerConfig::default`] and so cannot
/// distinguish a wired server from an unwired one.
///
/// Returns a boxed future: a `ServerConfig` is about 16 KB, so an `async fn`
/// holding one by value sits past clippy's `large_futures` limit in every
/// caller.
pub(crate) fn create_test_server_resources_with_config(config: ServerConfig) -> BoxedServerContext {
    Box::pin(create_test_server_resources_inner(
        None,
        Vec::new(),
        Some(config),
    ))
}

/// The future a config-taking helper returns, boxed so callers stay small.
pub(crate) type BoxedServerContext =
    Pin<Box<dyn Future<Output = Result<Arc<ServerContext>>> + Send>>;

/// A key manager over a local KEK holding `mek`, so a test supplies its key
/// material directly instead of through the process environment.
#[must_use]
pub(crate) fn bootstrap_keys(mek: [u8; 32]) -> (KeyManager, [u8; 32]) {
    KeyManager::bootstrap_with(Box::new(LocalKekProvider::from_mek(
        MasterEncryptionKey::from_bytes(mek),
    )))
}

/// The LLM chain every test server runs with: one Gemini entry on
/// [`PLACEHOLDER_LLM_MODEL`].
///
/// The messaging session resolver and the turn pipeline read the chain's head
/// model from the server config, so a test server needs one whatever the
/// developer's `.envrc` or the CI job sets — an unset chain's Gemini head runs
/// `PIERRE_LLM_DEFAULT_MODEL`, which nothing sets on the coverage lane. The
/// placeholder is safe ONLY for a suite that calls no model: a turn asks the
/// head for this model id, which no backend serves. A lane driving real
/// providers builds its config from the environment's chain and refuses the
/// placeholder — `live_incident_corpus_test` does.
#[must_use]
pub(crate) fn placeholder_llm_chain() -> LlmChain {
    LlmChain::parse(&format!("gemini={PLACEHOLDER_LLM_MODEL}"))
}

/// The config every test server runs with unless the test supplies its own.
#[must_use]
pub(crate) fn test_server_config() -> ServerConfig {
    ServerConfig {
        llm_chain: placeholder_llm_chain(),
        oauth: test_oauth_config(),
        usda_api_key: env::var("USDA_API_KEY").ok().map(SecretString::from),
        // ServerConfig::default() leaves this at 0 (the derived usize default);
        // from_env() uses DEFAULT_ACTIVITIES_LIMIT in production. A 0 limit makes
        // every activity-fetch path (group snapshots, AllProvidersMerge) read
        // LIMIT 0 and return nothing, so give tests a realistic value.
        activity_fetch_limit: 100,
        ..ServerConfig::default()
    }
}

/// The server-level OAuth apps every test server holds unless the test
/// configures its own: a Strava app and a Garmin app, with each provider's
/// default scopes.
#[must_use]
pub(crate) fn test_oauth_config() -> OAuthConfig {
    OAuthConfig {
        strava: OAuthProviderConfig::for_provider(
            "strava",
            Some(TEST_STRAVA_CLIENT_ID.to_owned()),
            Some("test_strava_client_secret".to_owned()),
        ),
        garmin: OAuthProviderConfig::for_provider(
            "garmin",
            Some("test_garmin_client_id".to_owned()),
            Some("test_garmin_client_secret".to_owned()),
        ),
        ..OAuthConfig::default()
    }
}

/// The `client_id` of the Strava app [`test_oauth_config`] gives every test
/// server.
pub(crate) const TEST_STRAVA_CLIENT_ID: &str = "test_strava_client_id";

/// [`test_server_config`] with `provider`'s server-level app holding these
/// credentials and the provider's default scopes, every other setting as the
/// default test server has it.
#[must_use]
pub(crate) fn test_server_config_with_app(
    provider: &str,
    client_id: &str,
    client_secret: &str,
) -> ServerConfig {
    let mut config = test_server_config();
    set_server_app(&mut config, provider, client_id, client_secret);
    config
}

/// Give `config` a server-level app for `provider` with these credentials.
///
/// # Panics
///
/// Panics for a provider the server holds no app for.
pub(crate) fn set_server_app(
    config: &mut ServerConfig,
    provider: &str,
    client_id: &str,
    client_secret: &str,
) {
    let app = OAuthProviderConfig::for_provider(
        provider,
        Some(client_id.to_owned()),
        Some(client_secret.to_owned()),
    );
    let oauth = &mut config.oauth;
    let slot = match provider {
        "strava" => &mut oauth.strava,
        "garmin" => &mut oauth.garmin,
        "whoop" => &mut oauth.whoop,
        "terra" => &mut oauth.terra,
        "intervals_icu" => &mut oauth.intervals_icu,
        "wahoo" => &mut oauth.wahoo,
        other => panic!("no server-level app slot for provider {other}"),
    };
    *slot = app;
}

/// Point `provider`'s endpoints in `config` at a stand-in: each `Some` replaces
/// the provider's default when the server builds its provider registry.
pub(crate) fn set_provider_endpoints(
    config: &mut ServerConfig,
    provider: &str,
    endpoints: ProviderEndpointOverride,
) {
    config
        .oauth
        .endpoints
        .insert(provider.to_owned(), endpoints);
}

/// [`test_server_config`] under the operator's `GUARDIAN_*` overrides
/// `guardian`, as startup would have read them.
#[must_use]
pub(crate) fn test_server_config_with_guardian(guardian: GuardianSettings) -> ServerConfig {
    ServerConfig {
        guardian,
        ..test_server_config()
    }
}

/// [`test_server_config`] with `provider`'s API reached at `api_base`.
#[must_use]
pub(crate) fn test_server_config_with_api_base(provider: &str, api_base: &str) -> ServerConfig {
    let mut config = test_server_config();
    set_provider_endpoints(
        &mut config,
        provider,
        ProviderEndpointOverride {
            api_base_url: Some(api_base.to_owned()),
            ..ProviderEndpointOverride::default()
        },
    );
    config
}

/// [`test_server_config`] with the sciotte scraper service at `url`, for a
/// test that stands the service in on loopback (where no audience is needed).
#[must_use]
pub(crate) fn test_server_config_with_sciotte(url: &str) -> ServerConfig {
    let mut config = test_server_config();
    config.external_services.sciotte = SciotteServiceConfig {
        remote_url: Some(url.to_owned()),
        audience: None,
    };
    config
}

/// [`create_test_server_resources_with_config`] with the sciotte scraper
/// service at `url`.
pub(crate) fn create_test_server_resources_with_sciotte(url: &str) -> BoxedServerContext {
    create_test_server_resources_with_config(test_server_config_with_sciotte(url))
}

/// [`create_test_server_resources_with_llm`] over a caller-supplied config.
///
/// Returns a boxed future for the same reason as
/// [`create_test_server_resources_with_config`].
pub(crate) fn create_test_server_resources_with_llm_and_config(
    provider: Arc<dyn LlmProvider + 'static>,
    config: ServerConfig,
) -> BoxedServerContext {
    Box::pin(create_test_server_resources_inner(
        Some(provider),
        Vec::new(),
        Some(config),
    ))
}

/// [`create_test_server_resources_with_chat_provider`] over a caller-supplied
/// config, for a chat-driven test that points an outbound API at a stub.
///
/// Returns a boxed future for the same reason as
/// [`create_test_server_resources_with_config`].
pub(crate) fn create_test_server_resources_with_chat_provider_and_config(
    provider: Arc<dyn LlmProvider + 'static>,
    config: ServerConfig,
) -> BoxedServerContext {
    Box::pin(create_test_server_resources_inner_full(
        None,
        Vec::new(),
        Some(config),
        Some(provider),
    ))
}

/// Wire the mock the way PRODUCTION wires a provider: `chat_provider` set,
/// `llm_provider` left `None`.
///
/// [`create_test_server_resources_with_llm`] does the opposite, and the
/// difference is not cosmetic. `pierre-mcp-server.rs` sets
/// `llm_provider: None` and supplies a `ChatProvider`, so any pipeline code
/// that reads `ctx.llm_provider` directly is dead in production while passing
/// every test that uses the `with_llm` helper. That is exactly how the bounded
/// identity re-ask shipped inert in 48175a705 — two green e2e tests, zero live
/// executions. Tests covering code that resolves a provider should use THIS
/// helper.
pub(crate) async fn create_test_server_resources_with_chat_provider(
    provider: Arc<dyn LlmProvider + 'static>,
) -> Result<Arc<ServerContext>> {
    Box::pin(create_test_server_resources_inner_full(
        None,
        Vec::new(),
        None,
        Some(provider),
    ))
    .await
}

/// [`create_test_server_resources_with_chat_provider`] plus extra runtime
/// tools, for a turn on the chat route that has to run a tool round.
pub(crate) async fn create_test_server_resources_with_chat_provider_and_tools(
    provider: Arc<dyn LlmProvider + 'static>,
    extra_tools: Vec<Arc<dyn RuntimeTool>>,
) -> Result<Arc<ServerContext>> {
    Box::pin(create_test_server_resources_inner_full(
        None,
        extra_tools,
        None,
        Some(provider),
    ))
    .await
}

async fn create_test_server_resources_inner(
    llm_provider: Option<Arc<dyn LlmProvider + 'static>>,
    extra_tools: Vec<Arc<dyn RuntimeTool>>,
    config_override: Option<ServerConfig>,
) -> Result<Arc<ServerContext>> {
    create_test_server_resources_inner_full(llm_provider, extra_tools, config_override, None).await
}

/// Wire a REAL [`ChatProvider`] through untouched, variant and all.
///
/// [`create_test_server_resources_with_chat_provider`] takes a trait object and
/// wraps it in [`ChatProvider::Custom`], which is right for a mock and wrong for
/// anything that has to be recognised by variant. The headless tool loop reaches
/// the Copilot ACP runner through `ChatProvider::as_embacle_provider`, and `Custom`
/// answers `None` to that — so a test handing in a real CLI provider as a trait
/// object gets "Headless tool loop requires a Copilot turn provider but
/// provider is not a CLI provider" and silently exercises a different path than
/// production. Anything driving the real ACP path must come through here.
pub(crate) async fn create_test_server_resources_with_real_chat_provider(
    provider: Arc<ChatProvider>,
) -> Result<Arc<ServerContext>> {
    Box::pin(create_test_server_resources_inner_typed(
        None,
        Vec::new(),
        None,
        Some(provider),
    ))
    .await
}

/// [`create_test_server_resources_with_real_chat_provider`] over a
/// caller-supplied config, for a lane whose turns must ask the provider for
/// the model its own chain names rather than [`placeholder_llm_chain`]'s.
///
/// Returns a boxed future for the same reason as
/// [`create_test_server_resources_with_config`].
pub(crate) fn create_test_server_resources_with_real_chat_provider_and_config(
    provider: Arc<ChatProvider>,
    config: ServerConfig,
) -> BoxedServerContext {
    Box::pin(create_test_server_resources_inner_typed(
        None,
        Vec::new(),
        Some(config),
        Some(provider),
    ))
}

async fn create_test_server_resources_inner_full(
    llm_provider: Option<Arc<dyn LlmProvider + 'static>>,
    extra_tools: Vec<Arc<dyn RuntimeTool>>,
    config_override: Option<ServerConfig>,
    as_chat_provider: Option<Arc<dyn LlmProvider + 'static>>,
) -> Result<Arc<ServerContext>> {
    create_test_server_resources_inner_typed(
        llm_provider,
        extra_tools,
        config_override,
        as_chat_provider.map(|p| Arc::new(ChatProvider::Custom(p))),
    )
    .await
}

async fn create_test_server_resources_inner_typed(
    llm_provider: Option<Arc<dyn LlmProvider + 'static>>,
    extra_tools: Vec<Arc<dyn RuntimeTool>>,
    config_override: Option<ServerConfig>,
    chat_provider: Option<Arc<ChatProvider>>,
) -> Result<Arc<ServerContext>> {
    let database = create_test_db_with_key(generate_encryption_key().to_vec()).await?;
    Box::pin(create_test_server_resources_over(
        database,
        llm_provider,
        extra_tools,
        config_override,
        chat_provider,
        None,
        None,
    ))
    .await
}

/// A second server context over the database an existing one already runs on.
///
/// The shape of a Cloud Run rollout or scaledown: the instance that was
/// running a turn is gone, and a fresh process — its own in-flight tracker,
/// its own drain token, the same rows — is what picks the turn up. Tests of
/// the resume path (registre#126) need that second process, because a drain
/// token stays cancelled for the life of the process that drained, exactly
/// as it does in production where that process exits.
///
/// `chat_provider` is passed rather than copied so a stateful stub (one that
/// hangs on its first call and answers afterwards) keeps its count across the
/// two instances.
///
/// # Errors
///
/// Returns an error if the context cannot be assembled.
pub(crate) async fn create_sibling_server_resources_with_chat_provider(
    sibling_of: &ServerContext,
    chat_provider: Arc<dyn LlmProvider + 'static>,
) -> Result<Arc<ServerContext>> {
    create_sibling_server_resources_with_chat_provider_and_runner(sibling_of, chat_provider, None)
        .await
}

/// A sibling context with the messaging turn runner chosen.
///
/// A sibling on Cloud Tasks mounts the turn-run route, which is how a test
/// delivers a task to "the next instance".
///
/// # Errors
///
/// Returns an error if the context cannot be assembled.
pub(crate) async fn create_sibling_server_resources_with_chat_provider_and_runner(
    sibling_of: &ServerContext,
    chat_provider: Arc<dyn LlmProvider + 'static>,
    turn_runner: Option<Arc<TurnRunner>>,
) -> Result<Arc<ServerContext>> {
    let database = (*sibling_of.agent.database).clone();
    Box::pin(create_test_server_resources_over(
        database,
        None,
        Vec::new(),
        None,
        Some(Arc::new(ChatProvider::Custom(chat_provider))),
        turn_runner,
        None,
    ))
    .await
}

/// A test context with the messaging turn runner chosen (registre#126).
///
/// Tests of the Cloud Tasks path build a runner over a local stand-in for the
/// Cloud Tasks API and pass it here; everything else leaves it in-process.
///
/// # Errors
///
/// Returns an error if the context cannot be assembled.
pub(crate) async fn create_test_server_resources_with_chat_provider_and_runner(
    provider: Arc<dyn LlmProvider + 'static>,
    turn_runner: Arc<TurnRunner>,
) -> Result<Arc<ServerContext>> {
    let database = create_test_db_with_key(generate_encryption_key().to_vec()).await?;
    Box::pin(create_test_server_resources_over(
        database,
        None,
        Vec::new(),
        None,
        Some(Arc::new(ChatProvider::Custom(provider))),
        Some(turn_runner),
        None,
    ))
    .await
}

/// [`create_test_server_resources`] with a caller-supplied billing provider in
/// place of the in-tree `DummyProvider`, for a test that reads what the
/// platform asked the provider to bill.
pub(crate) async fn create_test_server_resources_with_billing_provider(
    billing_provider: Arc<dyn BillingProvider>,
) -> Result<Arc<ServerContext>> {
    let database = create_test_db_with_key(generate_encryption_key().to_vec()).await?;
    Box::pin(create_test_server_resources_over(
        database,
        None,
        Vec::new(),
        None,
        None,
        None,
        Some(billing_provider),
    ))
    .await
}

async fn create_test_server_resources_over(
    database: Database,
    llm_provider: Option<Arc<dyn LlmProvider + 'static>>,
    extra_tools: Vec<Arc<dyn RuntimeTool>>,
    config_override: Option<ServerConfig>,
    chat_provider: Option<Arc<ChatProvider>>,
    turn_runner: Option<Arc<TurnRunner>>,
    billing_provider: Option<Arc<dyn BillingProvider>>,
) -> Result<Arc<ServerContext>> {
    init_test_logging();
    init_test_http_clients();
    init_server_config();

    let auth_manager = AuthManager::new(24);

    let admin_jwt_secret = "test_admin_secret";
    let config = Arc::new(config_override.unwrap_or_else(test_server_config));

    // Create test cache with background cleanup disabled for tests
    let cache_config = CacheConfig {
        max_entries: 1000,
        redis_url: None,
        cleanup_interval: StdDuration::from_mins(1),
        enable_background_cleanup: false, // Disable background cleanup for tests
        ..Default::default()
    };
    let cache = Cache::new(cache_config).await?;

    // Use shared JWKS manager to eliminate expensive RSA key generation (250-350ms per test)
    let jwks_manager = get_shared_test_jwks();

    Ok(Arc::new(
        ServerContext::new(
            database,
            auth_manager,
            admin_jwt_secret,
            config,
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(jwks_manager),
                llm_provider,
                chat_provider,
                extra_tools,
                billing_provider,
                turn_runner,
            },
        )
        .await
        .unwrap(),
    ))
}

/// Complete test environment setup using `ServerContext` pattern
/// Returns (`server_resources`, `user_id`, `api_key`)
pub(crate) async fn setup_server_resources_test_environment(
) -> Result<(Arc<ServerContext>, Uuid, String)> {
    let resources = create_test_server_resources().await?;
    let (user_id, _user) = create_test_user(&resources.agent.database).await?;
    let api_key = create_test_api_key(&resources.agent.database, user_id, "test-key")?;

    Ok((resources, user_id, api_key))
}

// Test databases
//
// A test never spells a database URL. `create_test_database()` and every
// `create_test_server_resources*` helper above go through
// `pierre_test_support::db`, which honours DATABASE_URL: on the
// PostgreSQL CI lane each call is a private PostgreSQL database cloned from a
// migrated template and reclaimed once the test is done; anywhere else it is
// in-memory SQLite, which leaves no files behind. A test that opened SQLite by
// hand would run SQLite on every lane whatever DATABASE_URL says, which is why
// `scripts/ci/architectural-validation.sh` rejects it outside `*_sqlite_test.rs`.

// ============================================================================
// USDA Mock Client for Testing (No Real API Calls)
// ============================================================================

use async_trait::async_trait;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_external::{FoodDetails, FoodNutrient, FoodSearchResult};
use pierre_llm::{ChatRequest, ChatResponse, ChatStream, LlmCapabilities, StreamChunk};
use std::collections::HashMap;

/// Mock USDA client for testing (no API calls)
pub(crate) struct MockUsdaClient {
    mock_foods: HashMap<u64, FoodDetails>,
}

impl MockUsdaClient {
    /// Create a new mock client with predefined test data
    #[must_use]
    pub(crate) fn new() -> Self {
        let mut mock_foods = HashMap::new();

        // Mock food: Chicken breast (FDC ID: 171_477)
        mock_foods.insert(
            171_477,
            FoodDetails {
                fdc_id: 171_477,
                description: "Chicken, breast, meat only, cooked, roasted".to_owned(),
                data_type: "SR Legacy".to_owned(),
                food_nutrients: vec![
                    FoodNutrient {
                        nutrient_id: 1003,
                        nutrient_name: "Protein".to_owned(),
                        unit_name: "g".to_owned(),
                        amount: 31.02,
                    },
                    FoodNutrient {
                        nutrient_id: 1004,
                        nutrient_name: "Total lipid (fat)".to_owned(),
                        unit_name: "g".to_owned(),
                        amount: 3.57,
                    },
                    FoodNutrient {
                        nutrient_id: 1005,
                        nutrient_name: "Carbohydrate, by difference".to_owned(),
                        unit_name: "g".to_owned(),
                        amount: 0.0,
                    },
                    FoodNutrient {
                        nutrient_id: 1008,
                        nutrient_name: "Energy".to_owned(),
                        unit_name: "kcal".to_owned(),
                        amount: 165.0,
                    },
                ],
                serving_size: Some(100.0),
                serving_size_unit: Some("g".to_owned()),
            },
        );

        // Mock food: Apple (FDC ID: 171_688)
        mock_foods.insert(
            171_688,
            FoodDetails {
                fdc_id: 171_688,
                description: "Apples, raw, with skin".to_owned(),
                data_type: "SR Legacy".to_owned(),
                food_nutrients: vec![
                    FoodNutrient {
                        nutrient_id: 1003,
                        nutrient_name: "Protein".to_owned(),
                        unit_name: "g".to_owned(),
                        amount: 0.26,
                    },
                    FoodNutrient {
                        nutrient_id: 1004,
                        nutrient_name: "Total lipid (fat)".to_owned(),
                        unit_name: "g".to_owned(),
                        amount: 0.17,
                    },
                    FoodNutrient {
                        nutrient_id: 1005,
                        nutrient_name: "Carbohydrate, by difference".to_owned(),
                        unit_name: "g".to_owned(),
                        amount: 13.81,
                    },
                    FoodNutrient {
                        nutrient_id: 1008,
                        nutrient_name: "Energy".to_owned(),
                        unit_name: "kcal".to_owned(),
                        amount: 52.0,
                    },
                ],
                serving_size: Some(182.0),
                serving_size_unit: Some("g".to_owned()),
            },
        );

        Self { mock_foods }
    }

    /// Mock search implementation
    ///
    /// # Errors
    /// Returns `AppError::InvalidInput` if query is empty
    pub(crate) fn search_foods(
        &self,
        query: &str,
        _page_size: u32,
    ) -> Result<Vec<FoodSearchResult>, AppError> {
        if query.is_empty() {
            return Err(AppError::invalid_input("Search query cannot be empty"));
        }

        let query_lower = query.to_lowercase();
        let results: Vec<FoodSearchResult> = self
            .mock_foods
            .values()
            .filter(|food| food.description.to_lowercase().contains(&query_lower))
            .map(|food| FoodSearchResult {
                fdc_id: food.fdc_id,
                description: food.description.clone(),
                data_type: food.data_type.clone(),
                publication_date: None,
                brand_owner: None,
            })
            .collect();

        Ok(results)
    }

    /// Mock details implementation
    ///
    /// # Errors
    /// Returns `AppError::NotFound` if food with given FDC ID doesn't exist
    pub(crate) fn get_food_details(&self, fdc_id: u64) -> Result<FoodDetails, AppError> {
        self.mock_foods
            .get(&fdc_id)
            .cloned()
            .ok_or_else(|| AppError::not_found(format!("Food with FDC ID {fdc_id}")))
    }
}

impl Default for MockUsdaClient {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// SDK Bridge Helpers for Multi-Tenant E2E Testing
// ============================================================================

use std::process::{Child, Command, Stdio};
use tokio::time::{sleep, Duration};

/// Handle for SDK bridge process that cleans up automatically on drop
/// Ensures subprocess is terminated when test completes
pub(crate) struct SdkBridgeHandle {
    process: Child,
    port: u16,
}

impl SdkBridgeHandle {
    /// Get the server port this bridge is connected to
    pub(crate) const fn port(&self) -> u16 {
        self.port
    }

    /// Get mutable reference to stdin for sending requests
    pub(crate) fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.process.stdin.as_mut()
    }

    /// Get mutable reference to stdout for reading responses
    pub(crate) fn stdout(&mut self) -> Option<&mut ChildStdout> {
        self.process.stdout.as_mut()
    }

    /// Get mutable reference to stderr for reading errors
    pub(crate) fn stderr(&mut self) -> Option<&mut ChildStderr> {
        self.process.stderr.as_mut()
    }
}

impl Drop for SdkBridgeHandle {
    fn drop(&mut self) {
        // Kill the SDK bridge process when handle is dropped; it may already
        // have exited, so both calls are best-effort.
        drop(self.process.kill());
        drop(self.process.wait());
    }
}

/// Spawn SDK bridge process for testing
/// Returns RAII handle that automatically cleans up subprocess on drop
///
/// # Arguments
/// * `jwt_token` - Valid JWT token for authentication
/// * `server_port` - Port where Pierre server is running
///
/// # Errors
/// Returns error if SDK bridge binary not found or process fails to start
pub(crate) async fn spawn_sdk_bridge(jwt_token: &str, server_port: u16) -> Result<SdkBridgeHandle> {
    // Find SDK CLI entry point (dist/cli.js - built from TypeScript)
    // SDK lives at workspace root, not in the crate directory
    let sdk_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map_or_else(
            || PathBuf::from("sdk/dist/cli.js"),
            |ws| ws.join("sdk").join("dist").join("cli.js"),
        );

    if !sdk_path.exists() {
        return Err(anyhow::Error::msg(format!(
            "SDK entry point not found at: {}",
            sdk_path.display()
        )));
    }

    // Spawn Node.js process running SDK bridge in stdio mode
    // Increase connection timeouts for CI runners where startup can be slow
    let mut process = Command::new("node")
        .arg(sdk_path)
        .env(
            "PIERRE_SERVER_URL",
            format!("http://localhost:{}", server_port),
        )
        .env("PIERRE_JWT_TOKEN", jwt_token)
        .env("MCP_TRANSPORT", "stdio")
        .env("PIERRE_PROACTIVE_CONNECTION_TIMEOUT_MS", "10000")
        .env("PIERRE_PROACTIVE_TOOLS_LIST_TIMEOUT_MS", "10000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Wait for process to initialize (longer on CI where startup can be slow)
    sleep(Duration::from_secs(2)).await;

    // Check if process is still alive
    if let Ok(Some(status)) = process.try_wait() {
        // Capture stderr to understand why the process failed
        use std::io::Read;
        let stderr_output = process.stderr.take().map_or_else(
            || String::from("(stderr not available)"),
            |mut stderr| {
                let mut output = String::new();
                stderr.read_to_string(&mut output).unwrap_or_default();
                output
            },
        );
        return Err(anyhow::Error::msg(format!(
            "SDK bridge process exited immediately with status: {}\nStderr: {}",
            status, stderr_output
        )));
    }

    Ok(SdkBridgeHandle {
        process,
        port: server_port,
    })
}

/// Send MCP request via SDK stdio bridge
/// Writes JSON-RPC request to stdin and reads response from stdout
///
/// # Arguments
/// * `sdk_bridge` - Mutable reference to SDK bridge handle
/// * `method` - MCP method name (e.g., "tools/list", "tools/call")
/// * `params` - JSON parameters for the method
///
/// # Errors
/// Returns error if stdio communication fails or server returns error
pub(crate) fn send_sdk_stdio_request(
    sdk_bridge: &mut SdkBridgeHandle,
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use std::io::{BufRead, BufReader, Write};

    // Build JSON-RPC 2.0 request
    let request_id = Uuid::new_v4().to_string();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params
    });

    // Get stdin handle
    let stdin = sdk_bridge
        .stdin()
        .ok_or_else(|| anyhow::Error::msg("SDK bridge stdin not available"))?;

    // Write request to stdin (MCP protocol expects newline-delimited JSON)
    let request_str = serde_json::to_string(&request)?;
    writeln!(stdin, "{}", request_str)?;
    stdin.flush()?;

    // Get stdout handle
    let stdout = sdk_bridge
        .stdout()
        .ok_or_else(|| anyhow::Error::msg("SDK bridge stdout not available"))?;

    // Read response from stdout
    let mut reader = BufReader::new(stdout);
    let mut response_line = String::new();
    reader.read_line(&mut response_line)?;

    // Parse JSON-RPC response
    let response: serde_json::Value = serde_json::from_str(response_line.trim())?;

    // Validate JSON-RPC 2.0 response structure
    if response.get("jsonrpc") != Some(&serde_json::json!("2.0")) {
        return Err(anyhow::Error::msg(
            "Invalid JSON-RPC response (missing jsonrpc field)",
        ));
    }

    if response.get("id").and_then(|v| v.as_str()) != Some(&request_id) {
        return Err(anyhow::Error::msg("Response ID mismatch"));
    }

    // Return the result or error
    if let Some(error) = response.get("error") {
        return Err(anyhow::Error::msg(format!("SDK returned error: {error}")));
    }

    response
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow::Error::msg("Response missing result field"))
}

/// Send HTTP MCP request directly to server
/// Bypasses SDK to test HTTP transport directly
///
/// # Arguments
/// * `url` - Full URL to MCP endpoint (e.g., `http://localhost:8081/mcp`)
/// * `method` - MCP method name (e.g., "tools/list", "tools/call")
/// * `params` - JSON parameters for the method
/// * `jwt_token` - Valid JWT token for authentication
///
/// # Errors
/// Returns error if HTTP request fails or server returns error
pub(crate) async fn send_http_mcp_request(
    url: &str,
    method: &str,
    params: serde_json::Value,
    jwt_token: &str,
) -> Result<serde_json::Value> {
    let client = reqwest::Client::new();

    let request_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params});

    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {}", jwt_token))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::Error::msg(format!(
            "HTTP request failed with status: {}",
            response.status()
        )));
    }

    let response_json: serde_json::Value = response.json().await?;

    // Check for JSON-RPC error
    if let Some(error) = response_json.get("error") {
        return Err(anyhow::Error::msg(format!("MCP error: {}", error)));
    }

    // Extract result field
    response_json
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow::Error::msg("Response missing 'result' field".to_owned()))
}

/// Create test tenant with user and JWT token
/// Combines user creation and token generation for multi-tenant tests
///
/// # Arguments
/// * `resources` - Server resources containing database and auth
/// * `email` - Email address for the test user
///
/// # Errors
/// Returns error if user creation or token generation fails
pub(crate) async fn create_test_tenant(
    resources: &ServerContext,
    email: &str,
) -> Result<(User, String)> {
    // Create test user with specified email
    let (_user_id, user) = create_test_user_with_email(&resources.agent.database, email).await?;

    // Look up user's tenant to include active_tenant_id in the JWT.
    // Route handlers require active_tenant_id in JWT claims.
    let repos = resources.agent.database.repositories();
    let tenants = repos
        .tenants
        .list_for_user(user.id)
        .await
        .map_err(|e| anyhow::Error::msg(format!("Failed to get user tenants: {e}")))?;
    let tenant_id = tenants.first().map(|t| t.id.to_string());

    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(&user, &resources.auth.jwks_manager, tenant_id)
        .map_err(|e| anyhow::Error::msg(format!("Failed to generate JWT: {e}")))?;

    Ok((user, token))
}

/// [`create_test_tenant`] with an integration's token instead of a session.
///
/// The token is [`generate_delegated_test_token`]'s delegated grant, since
/// `/mcp` and A2A both refuse a first-party session token (carnet#768).
///
/// # Errors
/// Returns error if user creation fails
pub(crate) async fn create_test_mcp_tenant(
    resources: &ServerContext,
    email: &str,
) -> Result<(User, String)> {
    let (user, _session_token) = create_test_tenant(resources, email).await?;
    let token = generate_delegated_test_token(resources, &user).await;
    Ok((user, token))
}

/// Test tenant + user pre-registered with a `Synthetic` provider connection.
///
/// Same shape as [`create_test_tenant`] but also registers a `Synthetic`
/// provider connection so the user can reach chat / agent / messaging
/// endpoints past the onboarding gate
/// (see `pierre_services::onboarding_gate`).
///
/// Use this whenever a test posts to `/api/chat/conversations/.../messages`
/// or any other endpoint that reads provider state. Tests that
/// exercise the gate itself should keep using [`create_test_tenant`] (no
/// provider).
///
/// # Errors
/// Returns error if user/tenant creation, provider registration, or token
/// generation fails.
pub(crate) async fn create_test_tenant_with_provider(
    resources: &ServerContext,
    email: &str,
) -> Result<(User, String)> {
    let (user, token) = create_test_tenant(resources, email).await?;
    let repos = resources.agent.database.repositories();
    let tenants = repos
        .tenants
        .list_for_user(user.id)
        .await
        .map_err(|e| anyhow::Error::msg(format!("Failed to get user tenants: {e}")))?;
    let tenant_id = tenants
        .first()
        .ok_or_else(|| anyhow::Error::msg("expected tenant for user"))?
        .id;
    repos
        .provider_connections
        .register_connection(user.id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .map_err(|e| anyhow::Error::msg(format!("Failed to register provider: {e}")))?;
    Ok((user, token))
}

/// Handle for HTTP MCP server that cleans up automatically on drop
pub(crate) struct HttpServerHandle {
    task_handle: JoinHandle<()>,
    port: u16,
}

impl HttpServerHandle {
    /// Get the port the server is listening on
    pub(crate) const fn port(&self) -> u16 {
        self.port
    }

    /// Get the base URL for making HTTP requests to this server
    pub(crate) fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for HttpServerHandle {
    fn drop(&mut self) {
        // Abort the server task when handle is dropped (RAII cleanup)
        self.task_handle.abort();
    }
}

/// Check if a TCP port is available for binding
fn is_port_available(port: u16) -> bool {
    TcpListener::bind(format!("127.0.0.1:{port}")).is_ok()
}

/// Find an available port for testing
fn find_available_port() -> u16 {
    let mut rng = rand::rng();
    for _ in 0..100 {
        let port = rng.random_range(10000..60000);
        if is_port_available(port) {
            return port;
        }
    }
    panic!("Could not find an available port after 100 attempts");
}

/// Spawn HTTP MCP server for E2E testing
///
/// Creates an Axum server with MCP routes listening on a random available port.
/// The server runs in the background and is automatically cleaned up when the
/// returned handle is dropped (RAII pattern).
///
/// # Arguments
/// * `resources` - Arc-wrapped server resources with database, auth, and configuration
///
/// # Returns
/// Handle to the running server with port and base URL
///
/// # Errors
/// Returns error if server cannot be started
pub(crate) async fn spawn_http_mcp_server(
    resources: &Arc<ServerContext>,
) -> Result<HttpServerHandle> {
    let port = find_available_port();

    // Clone Arc for moving into spawned task (Arc enables sharing across tasks)
    let resources_for_task = Arc::clone(resources);

    // Spawn server task
    let task_handle = tokio::spawn(async move {
        let app = McpRoutes::routes(resources_for_task);

        let listener = TokioTcpListener::bind(format!("127.0.0.1:{port}"))
            .await
            .expect("Failed to bind to port");

        axum::serve(listener, app)
            .await
            .expect("Server failed to run");
    });

    // Wait for server to be ready
    tokio_sleep(StdDuration::from_millis(500)).await;

    Ok(HttpServerHandle { task_handle, port })
}

// ============================================================================
// Test LLM Provider for Mocking
// ============================================================================

/// Mock LLM provider for testing insight validation logic
///
/// Allows tests to control LLM responses without making actual API calls.
/// Configured with predetermined responses for validation testing.
pub(crate) struct TestLlmProvider {
    /// Pre-configured JSON response that will be returned by `complete()`
    response: String,
    /// Model name to return (stored to satisfy lifetime requirements)
    model_name: String,
    available_models: Vec<String>,
}

impl TestLlmProvider {
    /// Create a provider that returns a "valid" verdict
    #[must_use]
    pub(crate) fn valid() -> Self {
        Self {
            response: r#"{"verdict": "valid", "reason": "Content meets quality standards"}"#
                .to_owned(),
            model_name: "test-model-v1".to_owned(),
            available_models: vec!["test-model-v1".to_owned()],
        }
    }

    /// Create a provider that returns a "rejected" verdict with custom reason
    #[must_use]
    pub(crate) fn rejected(reason: &str) -> Self {
        Self {
            response: format!(r#"{{"verdict": "rejected", "reason": "{reason}"}}"#),
            model_name: "test-model-v1".to_owned(),
            available_models: vec!["test-model-v1".to_owned()],
        }
    }

    /// Create a provider that returns an "improved" verdict with enhanced content
    #[must_use]
    pub(crate) fn improved(improved_content: &str, reason: &str) -> Self {
        Self {
            response: format!(
                r#"{{"verdict": "improved", "reason": "{reason}", "improved_content": "{improved_content}"}}"#
            ),
            model_name: "test-model-v1".to_owned(),
            available_models: vec!["test-model-v1".to_owned()],
        }
    }

    /// Create a provider with a custom JSON response
    #[must_use]
    pub(crate) fn with_response(response: String) -> Self {
        Self {
            response,
            model_name: "test-model-v1".to_owned(),
            available_models: vec!["test-model-v1".to_owned()],
        }
    }
}

#[async_trait]
impl LlmProvider for TestLlmProvider {
    fn name(&self) -> &'static str {
        "test"
    }

    fn display_name(&self) -> &'static str {
        "Test Provider"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES | LlmCapabilities::JSON_MODE
    }

    fn default_model(&self) -> &str {
        &self.model_name
    }

    fn available_models(&self) -> &[String] {
        &self.available_models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: self.response.clone(),
            model: self.model_name.clone(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        // For tests, just delegate to non-streaming complete
        // Tests typically don't need streaming functionality
        let response = self.complete(request).await?;

        // Create a single-item stream
        let chunk = StreamChunk {
            delta: response.content,
            is_final: true,
            finish_reason: response.finish_reason,
        };

        let result_stream = stream::once(async move { Ok(chunk) });
        Ok(Box::pin(result_stream))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}
