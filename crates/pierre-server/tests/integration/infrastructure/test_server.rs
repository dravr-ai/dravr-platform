// ABOUTME: Integration test server lifecycle management
// ABOUTME: Spawns real HTTP server with synthetic provider for E2E testing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used)]
// Allow dead code in test infrastructure - methods designed for future test expansion
#![allow(dead_code)]

use anyhow::Result;
use pierre_core::models::CoachingPersona;
use pierre_mcp_server::{
    cache::{factory::Cache, CacheConfig, CacheTtlConfig},
    config::environment::{
        AppBehaviorConfig, AuthConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment,
        ExternalServicesConfig, HttpClientConfig, LogLevel, LoggingConfig, OAuth2ServerConfig,
        OAuthConfig, PostgresPoolConfig, ProtocolConfig, RedisConnectionConfig, RouteTimeoutConfig,
        SecurityConfig, SecurityHeadersConfig, ServerConfig, SseConfig, TlsConfig,
    },
    mcp::{
        multitenant::MultiTenantMcpServer,
        resources::{ServerResources, ServerResourcesOptions},
    },
    models::{Tenant, TenantId, User, UserStatus, UserTier},
    permissions::UserRole,
    providers::synthetic_provider::set_synthetic_test_seed,
};
use rand::Rng;
use std::{env, net::TcpListener, path::PathBuf, sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time::sleep};
use uuid::Uuid;

use crate::common::{
    create_test_auth_manager, create_test_database, get_shared_test_jwks, init_server_config,
    init_test_http_clients, init_test_logging,
};

/// Default seed for deterministic test data generation (reserved for future use)
pub const DEFAULT_TEST_SEED: u64 = 12345;

/// Integration test server that manages the full HTTP server lifecycle
pub struct IntegrationTestServer {
    port: u16,
    resources: Arc<ServerResources>,
    server_handle: Option<JoinHandle<()>>,
}

impl IntegrationTestServer {
    /// Create a new test server
    ///
    /// Uses the standard synthetic provider which is always registered.
    /// The synthetic provider is seeded with `DEFAULT_TEST_SEED` for
    /// deterministic test data generation.
    pub async fn new() -> Result<Self> {
        // Enable seeded synthetic provider for deterministic test data
        set_synthetic_test_seed(DEFAULT_TEST_SEED);

        init_test_logging();
        init_test_http_clients();
        init_server_config();

        let port = find_available_port();
        let database = create_test_database().await?;
        let auth_manager = create_test_auth_manager();
        let jwks_manager = get_shared_test_jwks();

        let config = Arc::new(create_test_server_config(port));
        let cache = Cache::new(CacheConfig {
            max_entries: 1000,
            redis_url: None,
            cleanup_interval: Duration::from_mins(1),
            enable_background_cleanup: false,
            redis_connection: RedisConnectionConfig::default(),
            ttl: CacheTtlConfig::default(),
        })
        .await?;

        let resources = Arc::new(
            ServerResources::new(
                (*database).clone(),
                (*auth_manager).clone(),
                "integration_test_jwt_secret",
                config,
                cache,
                ServerResourcesOptions {
                    rsa_key_size_bits: Some(2048),
                    jwks_manager: Some(jwks_manager),
                    llm_provider: None,
                    extra_tools: Vec::new(),
                },
            )
            .await,
        );

        Ok(Self {
            port,
            resources,
            server_handle: None,
        })
    }

    /// Alias for `new()` - creates test server with default configuration
    pub async fn with_defaults() -> Result<Self> {
        Self::new().await
    }

    /// Start the HTTP server
    pub async fn start(&mut self) -> Result<()> {
        let resources = Arc::clone(&self.resources);
        let port = self.port;

        let handle = tokio::spawn(async move {
            let server = MultiTenantMcpServer::new(resources);
            let _ = server.run(port).await;
        });

        self.server_handle = Some(handle);

        // Wait for server to be ready
        self.wait_for_health().await?;

        Ok(())
    }

    /// Wait for the server to be healthy
    async fn wait_for_health(&self) -> Result<()> {
        let client = reqwest::Client::new();
        let url = format!("{}/health", self.base_url());

        // 600 iterations × 100ms = 60 seconds timeout.
        // PostgreSQL CI needs more headroom: the PG container + multiple concurrent
        // test servers under load can exceed 30 seconds (observed on 2026-04-27 main).
        for _ in 0..600 {
            match client.get(&url).send().await {
                Ok(response) if response.status().is_success() => return Ok(()),
                _ => sleep(Duration::from_millis(100)).await,
            }
        }

        Err(anyhow::anyhow!(
            "Server failed to become healthy within 60 seconds"
        ))
    }

    /// Get the server port
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Get the base URL for the server
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Get the MCP endpoint URL
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url())
    }

    /// Get shared server resources
    pub const fn resources(&self) -> &Arc<ServerResources> {
        &self.resources
    }

    /// Create a test user and return (`user_id`, `jwt_token`)
    pub async fn create_test_user(&self, email: &str) -> Result<(Uuid, String)> {
        let user_id = Uuid::new_v4();
        let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;

        let user = User {
            id: user_id,
            email: email.to_owned(),
            display_name: Some("Integration Test User".to_owned()),
            password_hash,
            tier: UserTier::Professional,
            strava_token: None,
            fitbit_token: None,
            is_active: true,
            user_status: UserStatus::Active,
            is_admin: false,
            role: UserRole::User,
            approved_by: Some(user_id),
            approved_at: Some(chrono::Utc::now()),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            firebase_uid: None,
            auth_provider: String::new(),
            analytics_consent: false,
            analytics_consent_at: None,
            locale: "fr".to_owned(),
            default_coach_id: None,
            coaching_persona: CoachingPersona::Casual,
            manages_roster: false,
        };

        let repos = self.resources.database.repositories();
        repos.users.create(&user).await?;

        // Create tenant for user
        // Use enterprise plan to enable all tools for integration testing
        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Tenant for {email}"),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "enterprise".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repos.tenants.create(&tenant).await?;

        // Update user with tenant ID
        repos.users.update_tenant_id(user_id, tenant_id).await?;

        // Generate JWT token with active_tenant_id so route handlers can resolve tenant
        let jwt_token = self.resources.auth_manager.generate_token_with_tenant(
            &user,
            &self.resources.jwks_manager,
            Some(tenant_id.to_string()),
        )?;

        Ok((user_id, jwt_token))
    }

    /// Create a test user and return (`user_id`, `tenant_id`, `jwt_token`)
    ///
    /// Like `create_test_user` but also returns the tenant ID for tests that
    /// need to interact with tenant-scoped data (counters, config, etc.).
    pub async fn create_test_user_with_tenant(
        &self,
        email: &str,
    ) -> Result<(Uuid, TenantId, String)> {
        let user_id = Uuid::new_v4();
        let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;

        let user = User {
            id: user_id,
            email: email.to_owned(),
            display_name: Some("Integration Test User".to_owned()),
            password_hash,
            tier: UserTier::Professional,
            strava_token: None,
            fitbit_token: None,
            is_active: true,
            user_status: UserStatus::Active,
            is_admin: false,
            role: UserRole::User,
            approved_by: Some(user_id),
            approved_at: Some(chrono::Utc::now()),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            firebase_uid: None,
            auth_provider: String::new(),
            analytics_consent: false,
            analytics_consent_at: None,
            locale: "fr".to_owned(),
            default_coach_id: None,
            coaching_persona: CoachingPersona::Casual,
            manages_roster: false,
        };

        let repos = self.resources.database.repositories();
        repos.users.create(&user).await?;

        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Tenant for {email}"),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "enterprise".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repos.tenants.create(&tenant).await?;

        repos.users.update_tenant_id(user_id, tenant_id).await?;

        let jwt_token = self.resources.auth_manager.generate_token_with_tenant(
            &user,
            &self.resources.jwks_manager,
            Some(tenant_id.to_string()),
        )?;

        Ok((user_id, tenant_id, jwt_token))
    }

    /// Stop the server gracefully
    pub fn stop(&mut self) {
        if let Some(handle) = self.server_handle.take() {
            handle.abort();
        }
    }
}

impl Drop for IntegrationTestServer {
    fn drop(&mut self) {
        self.stop();
        // Reset the test seed to avoid affecting other tests
        set_synthetic_test_seed(0);
    }
}

/// Find an available TCP port
fn find_available_port() -> u16 {
    let mut rng = rand::rng();
    for _ in 0..100 {
        let port = rng.random_range(20000..50000);
        if TcpListener::bind(format!("127.0.0.1:{port}")).is_ok() {
            return port;
        }
    }
    panic!("Could not find an available port after 100 attempts");
}

/// Create test server configuration
fn create_test_server_config(port: u16) -> ServerConfig {
    ServerConfig {
        http_port: port,
        oauth_callback_port: 35535,
        log_level: LogLevel::Warn,
        logging: LoggingConfig::default(),
        http_client: HttpClientConfig::default(),
        database: DatabaseConfig {
            url: DatabaseUrl::Memory,
            auto_migrate: true,
            backup: BackupConfig {
                enabled: false,
                interval_seconds: 3600,
                retention_count: 7,
                directory: PathBuf::from("test_backups"),
            },
            postgres_pool: PostgresPoolConfig::default(),
        },
        auth: AuthConfig {
            jwt_expiry_hours: 24,
            enable_refresh_tokens: false,
            ..AuthConfig::default()
        },
        oauth: OAuthConfig::default(),
        security: SecurityConfig {
            cors_origins: vec!["*".to_owned()],
            allowed_mobile_redirect_origins: vec![],
            tls: TlsConfig {
                enabled: false,
                cert_path: None,
                key_path: None,
            },
            headers: SecurityHeadersConfig {
                environment: Environment::Testing,
            },
        },
        external_services: ExternalServicesConfig::default(),
        usda_api_key: env::var("USDA_API_KEY").ok(),
        app_behavior: AppBehaviorConfig {
            max_activities_fetch: 100,
            default_activities_limit: 20,
            ci_mode: true,
            auto_approve_users: false,
            auto_approve_users_from_env: false,
            auto_approve_domains: vec![],
            protocol: ProtocolConfig {
                mcp_version: "2025-06-18".to_owned(),
                server_name: "pierre-integration-test".to_owned(),
                server_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        },
        sse: SseConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        route_timeouts: RouteTimeoutConfig::default(),
        ..Default::default()
    }
}
