// ABOUTME: Integration tests for HTTP API endpoints (user management, OAuth, API keys)
// ABOUTME: Tests complete user workflows through REST API routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use anyhow::Result;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use pierre_auth::{
    api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest},
    auth::AuthManager,
    tenant::TenantOAuthCredentials,
};
use pierre_cache::{Cache, CacheConfig as MemoryCacheConfig};
use pierre_config::environment::{
    AppBehaviorConfig, AuthConfig, BackupConfig, CacheConfig, CorsConfig, DatabaseConfig,
    DatabaseUrl, Environment, ExternalServicesConfig, FirebaseConfig, GarminApiConfig,
    GeocodingServiceConfig, GoalManagementConfig, HttpClientConfig, LogLevel, LoggingConfig,
    McpConfig, MonitoringConfig, OAuth2ServerConfig, OAuthConfig, OAuthProviderConfig,
    PostgresPoolConfig, ProtocolConfig, RateLimitConfig, RouteTimeoutConfig, SecurityConfig,
    SecurityHeadersConfig, ServerConfig, SleepToolParamsConfig, SqlxConfig, SseConfig,
    StravaApiConfig, TlsConfig, TokioRuntimeConfig, TrainingZonesConfig, WeatherServiceConfig,
};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::{
    backends::{factory::Database, DatabaseProvider},
    database::generate_encryption_key,
};
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_mcp_server::routes::mcp::McpRoutes;
use pierre_routes_auth::{AuthService, LoginRequest, OAuthService, RegisterRequest};
use pierre_services::oauth_flow::AuthUrlOptions;
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tower::ServiceExt;

/// Test setup for SDK integration tests: the services built over [`setup_test_context`].
async fn setup_test_environment() -> Result<(Arc<Database>, AuthService, OAuthService, TenantId)> {
    let (database, server_resources, tenant_id) = setup_test_context().await?;

    let auth_routes = AuthService::new(
        server_resources.auth.auth_manager.clone(),
        server_resources.auth.jwks_manager.clone(),
        server_resources.common.config.clone(),
        server_resources.data(),
    );
    let oauth_routes = OAuthService::new(
        server_resources.data(),
        server_resources.common.config.clone(),
    );

    Ok((database, auth_routes, oauth_routes, tenant_id))
}

/// The database, the server context built over it, and the test tenant.
// Long function: Defines complete test environment setup including database, auth, config, and test data
async fn setup_test_context() -> Result<(Arc<Database>, Arc<ServerContext>, TenantId)> {
    // Initialize server config for tests
    common::init_server_config();

    let database = Arc::new(create_test_db_with_key(generate_encryption_key().to_vec()).await?);
    database.migrate().await?;

    let auth_manager = Arc::new(AuthManager::new(24));

    // Create admin user first
    let admin_user = User {
        id: uuid::Uuid::new_v4(),
        email: "admin@example.com".to_owned(),
        display_name: Some("Admin".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        strava_token: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    let admin_id = database.repositories().users.create(&admin_user).await?;

    // Create tenant
    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Test Tenant".to_owned(),
        slug: "test-tenant".to_owned(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: admin_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    database.repositories().tenants.create(&tenant).await?;

    // Store tenant OAuth credentials
    let strava_credentials = TenantOAuthCredentials {
        tenant_id,
        provider: "strava".to_owned(),
        client_id: "test_client_id".to_owned(),
        client_secret: "test_client_secret".to_owned(),
        redirect_uri: "http://localhost:8080/oauth/callback/strava".to_owned(),
        scopes: vec!["read".to_owned(), "activity:read_all".to_owned()],
        rate_limit_per_day: 15000,
    };
    database
        .repositories()
        .tenants
        .store_oauth_credentials(&strava_credentials)
        .await?;

    // Create basic config with correct structure
    let config = Arc::new(ServerConfig {
        http_port: 8081,
        log_level: LogLevel::Info,
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
            refresh_token_expiry_days: 30,
            ..AuthConfig::default()
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: Some("test_client_id".to_owned()),
                client_secret: Some("test_client_secret".to_owned()),
                redirect_uri: Some("http://localhost:8081/oauth/callback/strava".to_owned()),
                scopes: vec!["read".to_owned(), "activity:read_all".to_owned()],
                enabled: true,
            },
            garmin: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
            whoop: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
            terra: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
        },
        security: SecurityConfig {
            cors_origins: vec!["http://localhost:3000".to_owned()],
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
        external_services: ExternalServicesConfig {
            weather: WeatherServiceConfig {
                api_key: None,
                base_url: "https://api.openweathermap.org/data/2.5".to_owned(),
                enabled: false,
            },
            geocoding: GeocodingServiceConfig {
                base_url: "https://nominatim.openstreetmap.org".to_owned(),
                enabled: false,
            },
            strava_api: StravaApiConfig {
                base_url: "https://www.strava.com/api/v3".to_owned(),
                auth_url: "https://www.strava.com/oauth/authorize".to_owned(),
                token_url: "https://www.strava.com/oauth/token".to_owned(),
                revoke_url: "https://www.strava.com/oauth/revoke".to_owned(),
                ..Default::default()
            },
            garmin_api: GarminApiConfig {
                base_url: "https://apis.garmin.com".to_owned(),
                auth_url: "https://connect.garmin.com/oauthConfirm".to_owned(),
                token_url: "https://connect.garmin.com/oauth-service/oauth/access_token".to_owned(),
                revoke_url: "https://connect.garmin.com/oauth-service/oauth/revoke".to_owned(),
                ..Default::default()
            },
        },
        app_behavior: AppBehaviorConfig {
            max_activities_fetch: 200,
            default_activities_limit: 50,
            ci_mode: true,
            auto_approve_users: false,
            auto_approve_users_from_env: false,
            auto_approve_domains: vec![],
            protocol: ProtocolConfig {
                mcp_version: "2025-06-18".to_owned(),
                server_name: "pierre-mcp-server-test".to_owned(),
                server_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        },
        sse: SseConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        route_timeouts: RouteTimeoutConfig::default(),
        host: "localhost".to_owned(),
        base_url: "http://localhost:8081".to_owned(),
        mcp: McpConfig {
            protocol_version: "2025-06-18".to_owned(),
            server_name: "pierre-mcp-server-test".to_owned(),
            session_cache_size: 1000,
            ..Default::default()
        },
        cors: CorsConfig {
            allowed_origins: "*".to_owned(),
            allow_localhost_dev: true,
        },
        cache: CacheConfig {
            redis_url: None,
            max_entries: 10000,
            cleanup_interval_secs: 300,
            ..Default::default()
        },
        usda_api_key: None,
        rate_limiting: RateLimitConfig::default(),
        sleep_tool_params: SleepToolParamsConfig::default(),
        activity_fetch_limit: 100,
        goal_management: GoalManagementConfig::default(),
        training_zones: TrainingZonesConfig::default(),
        firebase: FirebaseConfig::default(),
        tokio_runtime: TokioRuntimeConfig::default(),
        sqlx: SqlxConfig::default(),
        monitoring: MonitoringConfig::default(),
        frontend_url: None,
        resend_api_key: None,
        resend_from_email: None,
    });

    let cache_config = MemoryCacheConfig {
        max_entries: 1000,
        redis_url: None,
        cleanup_interval: Duration::from_mins(1),
        enable_background_cleanup: false,
        ..Default::default()
    };
    let cache = Cache::new(cache_config).await.unwrap();

    let server_resources = Arc::new(
        ServerContext::new(
            (*database).clone(),
            (*auth_manager).clone(),
            "test_jwt_secret",
            config,
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
            },
        )
        .await,
    );

    Ok((database, server_resources, tenant_id))
}

/// Helper to create and approve a test user
async fn create_approved_test_user(
    database: &Database,
    email: &str,
    password: &str,
) -> Result<String> {
    let user = User::new(
        email.to_owned(),
        bcrypt::hash(password, bcrypt::DEFAULT_COST)?,
        Some("Test User".to_owned()),
    );
    let user_id = user.id;

    // Create user with active status
    let mut active_user = user;
    active_user.user_status = UserStatus::Active;
    active_user.approved_at = Some(chrono::Utc::now());

    database.repositories().users.create(&active_user).await?;
    Ok(user_id.to_string())
}

#[tokio::test]
async fn test_sdk_user_registration_flow() -> Result<()> {
    let (database, auth_routes, _oauth_routes, _tenant_id) = setup_test_environment().await?;

    // Test 1: Register user (should create with pending status)
    let register_request = RegisterRequest {
        email: "sdk_test@example.com".to_owned(),
        password: "TestPassword123".to_owned(),
        display_name: Some("SDK Test User".to_owned()),
    };

    let register_response = auth_routes.register(register_request).await?;
    assert!(!register_response.user_id.is_empty());
    assert!(register_response.message.contains("pending admin approval"));

    // Test 2: Verify user is created with pending status
    let user_id = uuid::Uuid::parse_str(&register_response.user_id)?;
    let user = database
        .repositories()
        .users
        .get_global(user_id)
        .await?
        .unwrap();
    assert_eq!(user.user_status, UserStatus::Pending);

    // Test 3: Login succeeds for pending user but returns pending status
    // (Backend authenticates; frontend handles access control based on user_status)
    let pending_login_response = auth_routes
        .login(LoginRequest {
            email: "sdk_test@example.com".to_owned(),
            password: "TestPassword123".to_owned(),
            timezone: None,
        })
        .await?;
    assert!(
        pending_login_response
            .jwt_token
            .as_ref()
            .is_some_and(|t| !t.is_empty()),
        "JWT token should be present even for pending user"
    );
    assert_eq!(
        pending_login_response.user.user_status, "pending",
        "User status should be 'pending' so frontend can restrict access"
    );

    // Test 4: Approve user and verify status changes to active
    database
        .repositories()
        .users
        .update_status(user_id, UserStatus::Active, None)
        .await?;

    let active_login_response = auth_routes
        .login(LoginRequest {
            email: "sdk_test@example.com".to_owned(),
            password: "TestPassword123".to_owned(),
            timezone: None,
        })
        .await?;
    assert!(
        active_login_response
            .jwt_token
            .as_ref()
            .is_some_and(|t| !t.is_empty()),
        "JWT token should be present and non-empty"
    );
    assert_eq!(active_login_response.user.email, "sdk_test@example.com");
    assert_eq!(
        active_login_response.user.user_status, "active",
        "User status should be 'active' after approval"
    );

    Ok(())
}

#[tokio::test]
async fn test_sdk_oauth_credentials_storage() -> Result<()> {
    let (database, _auth_routes, oauth_routes, tenant_id) = setup_test_environment().await?;

    // Create and approve a test user
    let user_id_str =
        create_approved_test_user(&database, "oauth_test@example.com", "TestPassword123").await?;
    let user_id = uuid::Uuid::parse_str(&user_id_str)?;

    // Test 1: Get OAuth authorization URL (uses tenant credentials)
    // This tests the OAuth flow with tenant-based credentials

    // Test 2: Get OAuth authorization URL
    let auth_url_response = oauth_routes
        .get_auth_url(user_id, tenant_id, "strava", AuthUrlOptions::default())
        .await?;
    assert!(auth_url_response.authorization_url.contains("strava.com"));
    assert!(auth_url_response
        .authorization_url
        .contains("test_client_id")); // From tenant credentials
    assert!(!auth_url_response.state.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_sdk_api_key_management() -> Result<()> {
    let (database, _auth_routes, _oauth_routes, _tenant_id) = setup_test_environment().await?;

    // Create and approve a test user
    let user_id_str =
        create_approved_test_user(&database, "apikey_test@example.com", "TestPassword123").await?;
    let user_id = uuid::Uuid::parse_str(&user_id_str)?;

    // Test 1: Create API key
    let api_key_manager = ApiKeyManager::new();
    let create_request = CreateApiKeyRequest {
        name: "SDK Test Key".to_owned(),
        description: Some("Test API key for SDK integration".to_owned()),
        tier: ApiKeyTier::Starter,
        rate_limit_requests: Some(1000),
        expires_in_days: Some(365),
    };

    let (api_key, api_key_string) = api_key_manager.create_api_key(user_id, create_request)?;
    database.repositories().api_keys.create(&api_key).await?;

    // Test 2: Verify API key is created
    assert!(!api_key_string.is_empty());
    assert!(api_key_string.starts_with("pk_"));
    assert_eq!(api_key.name, "SDK Test Key");
    assert_eq!(api_key.user_id, user_id);

    // Test 3: Verify API key in database
    let api_key_details = database
        .repositories()
        .api_keys
        .get_by_id(&api_key.id, None)
        .await?
        .unwrap();
    assert_eq!(api_key_details.name, "SDK Test Key");
    assert_eq!(api_key_details.user_id, user_id);

    Ok(())
}

#[tokio::test]
async fn test_sdk_error_handling() -> Result<()> {
    let (database, auth_routes, oauth_routes, tenant_id) = setup_test_environment().await?;

    // Test 1: Invalid email format
    let invalid_email_request = RegisterRequest {
        email: "invalid-email".to_owned(),
        password: "TestPassword123".to_owned(),
        display_name: Some("Test User".to_owned()),
    };

    let result = auth_routes.register(invalid_email_request).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid email format"));

    // Test 2: Password too short
    let short_password_request = RegisterRequest {
        email: "test@example.com".to_owned(),
        password: "short".to_owned(),
        display_name: Some("Test User".to_owned()),
    };

    let result = auth_routes.register(short_password_request).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("at least 8 characters"));

    // Test 3: Duplicate user registration
    let valid_request = RegisterRequest {
        email: "duplicate@example.com".to_owned(),
        password: "TestPassword123".to_owned(),
        display_name: Some("Test User".to_owned()),
    };

    auth_routes.register(valid_request.clone()).await?;
    let result = auth_routes.register(valid_request).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));

    // Test 4: Login with non-existent user
    let login_request = LoginRequest {
        email: "nonexistent@example.com".to_owned(),
        password: "TestPassword123".to_owned(),
        timezone: None,
    };

    let result = auth_routes.login(login_request).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid email or password"));

    // Test 5: OAuth URL for unsupported provider
    let user_id_str =
        create_approved_test_user(&database, "oauth_error_test@example.com", "TestPassword123")
            .await?;
    let user_id = uuid::Uuid::parse_str(&user_id_str)?;

    let result = oauth_routes
        .get_auth_url(
            user_id,
            tenant_id,
            "unsupported_provider",
            AuthUrlOptions::default(),
        )
        .await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Unsupported provider"));

    Ok(())
}

#[tokio::test]
async fn test_sdk_complete_onboarding_simulation() -> Result<()> {
    let (database, auth_routes, oauth_routes, tenant_id) = setup_test_environment().await?;

    // Simulate complete SDK onboarding flow

    // Step 1: Register user
    let register_request = RegisterRequest {
        email: "complete_test@example.com".to_owned(),
        password: "CompleteTest123".to_owned(),
        display_name: Some("Complete Test User".to_owned()),
    };

    let register_response = auth_routes.register(register_request).await?;
    let user_id = uuid::Uuid::parse_str(&register_response.user_id)?;

    // Step 2: Admin approves user (simulated, service token approval)
    database
        .repositories()
        .users
        .update_status(user_id, UserStatus::Active, None)
        .await?;

    // Step 3: User logs in
    let login_request = LoginRequest {
        email: "complete_test@example.com".to_owned(),
        password: "CompleteTest123".to_owned(),
        timezone: None,
    };

    let login_response = auth_routes.login(login_request).await?;
    assert!(
        login_response
            .jwt_token
            .as_ref()
            .is_some_and(|t| !t.is_empty()),
        "JWT token should be present and non-empty"
    );

    // Step 4: Test OAuth URL generation (uses env vars for client credentials)

    // Step 5: Create API key
    let api_key_manager = ApiKeyManager::new();
    let create_request = CreateApiKeyRequest {
        name: "Complete Test API Key".to_owned(),
        description: Some("Complete onboarding test".to_owned()),
        tier: ApiKeyTier::Professional,
        rate_limit_requests: Some(5000),
        expires_in_days: None,
    };

    let (api_key, api_key_string) = api_key_manager.create_api_key(user_id, create_request)?;
    database.repositories().api_keys.create(&api_key).await?;

    // Step 6: Verify complete setup
    let user = database
        .repositories()
        .users
        .get_global(user_id)
        .await?
        .unwrap();
    assert_eq!(user.user_status, UserStatus::Active);
    assert_eq!(user.email, "complete_test@example.com");

    // Skip OAuth app verification for now due to type mismatch
    // The OAuth URLs are generated successfully using environment variables

    let stored_api_key = database
        .repositories()
        .api_keys
        .get_by_id(&api_key.id, None)
        .await?
        .unwrap();
    assert_eq!(stored_api_key.name, "Complete Test API Key");
    assert_eq!(stored_api_key.user_id, user_id);

    // Step 7: Test OAuth URL generation
    let auth_url = oauth_routes
        .get_auth_url(user_id, tenant_id, "strava", AuthUrlOptions::default())
        .await?;
    assert!(auth_url.authorization_url.contains("test_client_id")); // Uses tenant credentials

    // Step 8: Test connection status
    let connections = oauth_routes.get_connection_status(user_id).await?;
    assert!(!connections.is_empty());

    // Find strava connection
    let strava_connection = connections
        .iter()
        .find(|c| c.provider == "strava")
        .expect("Should have strava connection");
    assert!(!strava_connection.connected); // Not connected yet (no tokens exchanged)

    println!("Complete SDK onboarding simulation successful!");
    println!("User: complete_test@example.com");
    println!("API Key: {}", &api_key_string[..20]);
    println!("OAuth App: strava (complete_test_client_id)");

    Ok(())
}

/// An MCP host reaches `/mcp` with a Dravr API key the way the SDK bridge
/// sends it (`PIERRE_API_KEY`): as the bearer credential of the
/// `Authorization` header. The same key under `X-API-Key`, a header
/// authentication never reads, is no credential at all, so that request is
/// refused as unauthenticated.
#[tokio::test]
async fn test_mcp_host_presents_an_api_key_as_its_bearer_not_x_api_key() -> Result<()> {
    let (database, server_resources, _tenant_id) = setup_test_context().await?;
    let (user_id, _) = common::create_test_user(&database).await?;

    let (api_key, api_key_string) = ApiKeyManager::new().create_api_key(
        user_id,
        CreateApiKeyRequest {
            name: "MCP host key".to_owned(),
            description: None,
            tier: ApiKeyTier::Starter,
            rate_limit_requests: None,
            expires_in_days: None,
        },
    )?;
    database.repositories().api_keys.create(&api_key).await?;

    let tools_list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    let post_mcp = |header: &'static str, value: String| {
        McpRoutes::routes(server_resources.clone()).oneshot(
            Request::post("/mcp")
                .header("content-type", "application/json")
                .header(header, value)
                .body(Body::from(tools_list.to_string()))
                .unwrap(),
        )
    };

    let as_bearer = post_mcp("authorization", format!("Bearer {api_key_string}")).await?;
    assert_eq!(as_bearer.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(as_bearer.into_body(), usize::MAX).await?)?;
    let tools = body["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list answers the key's tool set: {body}"));
    assert!(
        tools.iter().any(|tool| tool["name"] == "get_activities"),
        "the key's tool set includes get_activities: {body}"
    );

    let as_x_api_key = post_mcp("x-api-key", api_key_string).await?;
    assert_eq!(as_x_api_key.status(), StatusCode::UNAUTHORIZED);
    let challenge = as_x_api_key
        .headers()
        .get("www-authenticate")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        challenge.starts_with("Bearer"),
        "the refusal asks for a bearer credential: {challenge}"
    );

    Ok(())
}
