// ABOUTME: Unit tests for OAuth routes module
// ABOUTME: Tests OAuth route handlers and endpoint functionality
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for OAuth routes module

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use common::placeholder_llm_chain;
use secrecy::{ExposeSecret, SecretString};

use pierre_auth::auth::AuthManager;
use pierre_config::environment::{
    AppBehaviorConfig, AuthConfig, BackupConfig, CacheConfig, CorsConfig, DatabaseConfig,
    DatabaseUrl, Environment, ExternalServicesConfig, FirebaseConfig, GoalManagementConfig,
    HttpClientConfig, LogLevel, LoggingConfig, McpConfig, MonitoringConfig, OAuth2ServerConfig,
    OAuthConfig, OAuthProviderConfig, PostgresPoolConfig, ProtocolConfig, RateLimitConfig,
    SecurityConfig, SecurityHeadersConfig, ServerConfig, SessionCookieConfig,
    SleepToolParamsConfig, StravaApiConfig, TlsConfig, TokioRuntimeConfig, TrainingZonesConfig,
};
use pierre_database::database::generate_encryption_key;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_auth::{AuthService, LoginRequest, RegisterRequest};
use pierre_test_support::db::create_test_db_with_key;
use std::sync::Arc;

#[tokio::test]
async fn test_email_validation() {
    common::init_server_config();
    let encryption_key = generate_encryption_key().to_vec();

    let database = create_test_db_with_key(encryption_key).await.unwrap();
    let auth_manager = AuthManager::new(24);

    let temp_dir = tempfile::tempdir().unwrap();
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
                directory: temp_dir.path().to_path_buf(),
            },
            postgres_pool: PostgresPoolConfig::default(),
        },
        auth: AuthConfig {
            jwt_expiry_hours: 24,
            refresh_token_expiry_days: 30,
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
            ..Default::default()
        },
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
        external_services: ExternalServicesConfig {
            strava_api: StravaApiConfig::default(),
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            max_activities_fetch: 100,
            default_activities_limit: 20,
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
        session_cookie: SessionCookieConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        host: "localhost".to_owned(),
        base_url: "http://localhost:8081".to_owned(),
        mcp: McpConfig {
            protocol_version: "2025-06-18".to_owned(),
            server_name: "pierre-mcp-server-test".to_owned(),
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
        monitoring: MonitoringConfig::default(),
        frontend_url: None,
        resend_api_key: None,
        resend_from_email: None,
        llm_chain: placeholder_llm_chain(),
        website_base_url: "https://dravr.ai".to_owned(),
        ..ServerConfig::default()
    });

    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            database,
            auth_manager,
            "test_jwt_secret",
            common::offline_config(config),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
                archive_imports: None,
                live_records: None,
                blobs: None,
                lab_reader: None,
                pdf_text_bin: None,
                contremaitre: None,
                #[cfg(feature = "client-messaging")]
                channel_adapters: common::offline_adapters(),
            },
        )
        .await
        .unwrap(),
    );

    let auth_routes = AuthService::new(
        server_resources.auth.auth_manager.clone(),
        server_resources.auth.jwks_manager.clone(),
        server_resources.common.config.clone(),
        server_resources.data(),
    );

    // Valid emails
    let valid_emails = vec![
        "test@example.com",
        "user.name@domain.com",
        "user+tag@example.co.uk",
        "123@numbers.com",
    ];

    for email in valid_emails {
        let request = RegisterRequest {
            analytics_consent: None,
            email: email.to_owned(),
            password: SecretString::from("password123"),
            display_name: None,
        };

        // Should not fail on email validation
        let result = auth_routes.self_register(request).await;
        // May fail on duplicate email, but not on validation
        if let Err(e) = result {
            let err = e.to_string();
            assert!(
                !err.contains("Invalid email format"),
                "Email {email} should be valid"
            );
        }
    }

    // Invalid emails
    let invalid_emails = vec![
        "@domain.com",
        "user@",
        "nodomain",
        "missing@dotcom",
        "",
        "a@b",
    ];

    for email in invalid_emails {
        let request = RegisterRequest {
            analytics_consent: None,
            email: email.to_owned(),
            password: SecretString::from("password123"),
            display_name: None,
        };

        let result = auth_routes.self_register(request).await;
        assert!(result.is_err(), "Email {email} should be invalid");
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid email format"));
    }
}

#[tokio::test]
async fn test_password_validation() {
    common::init_server_config();
    let encryption_key = generate_encryption_key().to_vec();

    let database = create_test_db_with_key(encryption_key).await.unwrap();
    let auth_manager = AuthManager::new(24);

    let temp_dir = tempfile::tempdir().unwrap();
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
                directory: temp_dir.path().to_path_buf(),
            },
            postgres_pool: PostgresPoolConfig::default(),
        },
        auth: AuthConfig {
            jwt_expiry_hours: 24,
            refresh_token_expiry_days: 30,
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
            ..Default::default()
        },
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
        external_services: ExternalServicesConfig {
            strava_api: StravaApiConfig::default(),
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            max_activities_fetch: 100,
            default_activities_limit: 20,
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
        session_cookie: SessionCookieConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        host: "localhost".to_owned(),
        base_url: "http://localhost:8081".to_owned(),
        mcp: McpConfig {
            protocol_version: "2025-06-18".to_owned(),
            server_name: "pierre-mcp-server-test".to_owned(),
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
        monitoring: MonitoringConfig::default(),
        frontend_url: None,
        resend_api_key: None,
        resend_from_email: None,
        llm_chain: placeholder_llm_chain(),
        website_base_url: "https://dravr.ai".to_owned(),
        ..ServerConfig::default()
    });

    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            database,
            auth_manager,
            "test_jwt_secret",
            common::offline_config(config),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
                archive_imports: None,
                live_records: None,
                blobs: None,
                lab_reader: None,
                pdf_text_bin: None,
                contremaitre: None,
                #[cfg(feature = "client-messaging")]
                channel_adapters: common::offline_adapters(),
            },
        )
        .await
        .unwrap(),
    );

    let auth_routes = AuthService::new(
        server_resources.auth.auth_manager.clone(),
        server_resources.auth.jwks_manager.clone(),
        server_resources.common.config.clone(),
        server_resources.data(),
    );

    // Test short password
    let request = RegisterRequest {
        analytics_consent: None,
        email: "test@example.com".to_owned(),
        password: SecretString::from("short"),
        display_name: None,
    };

    let result = auth_routes.self_register(request).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Password must be at least 8 characters"));

    // Test valid password
    let request = RegisterRequest {
        analytics_consent: None,
        email: "test2@example.com".to_owned(),
        password: SecretString::from("validpassword123"),
        display_name: None,
    };

    let result = auth_routes.self_register(request).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_duplicate_user_registration() {
    common::init_server_config();
    let encryption_key = generate_encryption_key().to_vec();

    let database = create_test_db_with_key(encryption_key).await.unwrap();
    let auth_manager = AuthManager::new(24);

    let temp_dir = tempfile::tempdir().unwrap();
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
                directory: temp_dir.path().to_path_buf(),
            },
            postgres_pool: PostgresPoolConfig::default(),
        },
        auth: AuthConfig {
            jwt_expiry_hours: 24,
            refresh_token_expiry_days: 30,
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
            ..Default::default()
        },
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
        external_services: ExternalServicesConfig {
            strava_api: StravaApiConfig::default(),
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            max_activities_fetch: 100,
            default_activities_limit: 20,
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
        session_cookie: SessionCookieConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        host: "localhost".to_owned(),
        base_url: "http://localhost:8081".to_owned(),
        mcp: McpConfig {
            protocol_version: "2025-06-18".to_owned(),
            server_name: "pierre-mcp-server-test".to_owned(),
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
        monitoring: MonitoringConfig::default(),
        frontend_url: None,
        resend_api_key: None,
        resend_from_email: None,
        llm_chain: placeholder_llm_chain(),
        website_base_url: "https://dravr.ai".to_owned(),
        ..ServerConfig::default()
    });

    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            database,
            auth_manager,
            "test_jwt_secret",
            common::offline_config(config),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
                archive_imports: None,
                live_records: None,
                blobs: None,
                lab_reader: None,
                pdf_text_bin: None,
                contremaitre: None,
                #[cfg(feature = "client-messaging")]
                channel_adapters: common::offline_adapters(),
            },
        )
        .await
        .unwrap(),
    );

    let auth_routes = AuthService::new(
        server_resources.auth.auth_manager.clone(),
        server_resources.auth.jwks_manager.clone(),
        server_resources.common.config.clone(),
        server_resources.data(),
    );

    let request = RegisterRequest {
        analytics_consent: None,
        email: "duplicate@example.com".to_owned(),
        password: SecretString::from("password123"),
        display_name: Some("Test User".to_owned()),
    };

    // First registration should succeed
    let result1 = auth_routes.self_register(request.clone()).await;
    assert!(result1.is_ok());

    // Second registration with same email should fail
    let result2 = auth_routes.self_register(request).await;
    assert!(result2.is_err());
    assert!(result2.unwrap_err().to_string().contains("already exists"));
}

#[tokio::test]
async fn test_login_with_correct_credentials() {
    common::init_server_config();
    let encryption_key = generate_encryption_key().to_vec();

    let database = create_test_db_with_key(encryption_key).await.unwrap();
    let auth_manager = AuthManager::new(24);

    let temp_dir = tempfile::tempdir().unwrap();
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
                directory: temp_dir.path().to_path_buf(),
            },
            postgres_pool: PostgresPoolConfig::default(),
        },
        auth: AuthConfig {
            jwt_expiry_hours: 24,
            refresh_token_expiry_days: 30,
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
            ..Default::default()
        },
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
        external_services: ExternalServicesConfig {
            strava_api: StravaApiConfig::default(),
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            max_activities_fetch: 100,
            default_activities_limit: 20,
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
        session_cookie: SessionCookieConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        host: "localhost".to_owned(),
        base_url: "http://localhost:8081".to_owned(),
        mcp: McpConfig {
            protocol_version: "2025-06-18".to_owned(),
            server_name: "pierre-mcp-server-test".to_owned(),
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
        monitoring: MonitoringConfig::default(),
        frontend_url: None,
        resend_api_key: None,
        resend_from_email: None,
        llm_chain: placeholder_llm_chain(),
        website_base_url: "https://dravr.ai".to_owned(),
        ..ServerConfig::default()
    });

    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            database,
            auth_manager,
            "test_jwt_secret",
            common::offline_config(config),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
                archive_imports: None,
                live_records: None,
                blobs: None,
                lab_reader: None,
                pdf_text_bin: None,
                contremaitre: None,
                #[cfg(feature = "client-messaging")]
                channel_adapters: common::offline_adapters(),
            },
        )
        .await
        .unwrap(),
    );

    let auth_routes = AuthService::new(
        server_resources.auth.auth_manager.clone(),
        server_resources.auth.jwks_manager.clone(),
        server_resources.common.config.clone(),
        server_resources.data(),
    );

    common::create_test_user_with_email(&server_resources.agent.database, "login_test@example.com")
        .await
        .unwrap();

    // Login with correct credentials
    let login_request = LoginRequest {
        email: "login_test@example.com".to_owned(),
        password: SecretString::from("password123"),
        timezone: None,
    };

    let result = auth_routes.login(login_request).await;
    assert!(result.is_ok());

    let response = result.unwrap();
    assert!(response
        .jwt_token
        .as_ref()
        .is_some_and(|t| !t.expose_secret().is_empty()));
    assert_eq!(response.user.email, "login_test@example.com");
    assert_eq!(response.user.display_name, Some("Test User".to_owned()));
}

#[tokio::test]
async fn test_login_with_wrong_password() {
    common::init_server_config();
    let encryption_key = generate_encryption_key().to_vec();

    let database = create_test_db_with_key(encryption_key).await.unwrap();
    let auth_manager = AuthManager::new(24);

    let temp_dir = tempfile::tempdir().unwrap();
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
                directory: temp_dir.path().to_path_buf(),
            },
            postgres_pool: PostgresPoolConfig::default(),
        },
        auth: AuthConfig {
            jwt_expiry_hours: 24,
            refresh_token_expiry_days: 30,
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
            ..Default::default()
        },
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
        external_services: ExternalServicesConfig {
            strava_api: StravaApiConfig::default(),
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            max_activities_fetch: 100,
            default_activities_limit: 20,
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
        session_cookie: SessionCookieConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        host: "localhost".to_owned(),
        base_url: "http://localhost:8081".to_owned(),
        mcp: McpConfig {
            protocol_version: "2025-06-18".to_owned(),
            server_name: "pierre-mcp-server-test".to_owned(),
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
        monitoring: MonitoringConfig::default(),
        frontend_url: None,
        resend_api_key: None,
        resend_from_email: None,
        llm_chain: placeholder_llm_chain(),
        website_base_url: "https://dravr.ai".to_owned(),
        ..ServerConfig::default()
    });

    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            database,
            auth_manager,
            "test_jwt_secret",
            common::offline_config(config),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
                archive_imports: None,
                live_records: None,
                blobs: None,
                lab_reader: None,
                pdf_text_bin: None,
                contremaitre: None,
                #[cfg(feature = "client-messaging")]
                channel_adapters: common::offline_adapters(),
            },
        )
        .await
        .unwrap(),
    );

    let auth_routes = AuthService::new(
        server_resources.auth.auth_manager.clone(),
        server_resources.auth.jwks_manager.clone(),
        server_resources.common.config.clone(),
        server_resources.data(),
    );

    common::create_test_user_with_email(&server_resources.agent.database, "wrong_pass@example.com")
        .await
        .unwrap();

    // Login with wrong password
    let login_request = LoginRequest {
        email: "wrong_pass@example.com".to_owned(),
        password: SecretString::from("wrongpassword"),
        timezone: None,
    };

    let result = auth_routes.login(login_request).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid credentials provided"));
}

#[tokio::test]
async fn test_login_with_non_existent_user() {
    common::init_server_config();
    let encryption_key = generate_encryption_key().to_vec();

    let database = create_test_db_with_key(encryption_key).await.unwrap();
    let auth_manager = AuthManager::new(24);

    let temp_dir = tempfile::tempdir().unwrap();
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
                directory: temp_dir.path().to_path_buf(),
            },
            postgres_pool: PostgresPoolConfig::default(),
        },
        auth: AuthConfig {
            jwt_expiry_hours: 24,
            refresh_token_expiry_days: 30,
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: None,
                client_secret: None,
                redirect_uri: None,
                scopes: vec![],
                enabled: false,
            },
            ..Default::default()
        },
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
        external_services: ExternalServicesConfig {
            strava_api: StravaApiConfig::default(),
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            max_activities_fetch: 100,
            default_activities_limit: 20,
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
        session_cookie: SessionCookieConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        host: "localhost".to_owned(),
        base_url: "http://localhost:8081".to_owned(),
        mcp: McpConfig {
            protocol_version: "2025-06-18".to_owned(),
            server_name: "pierre-mcp-server-test".to_owned(),
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
        monitoring: MonitoringConfig::default(),
        frontend_url: None,
        resend_api_key: None,
        resend_from_email: None,
        llm_chain: placeholder_llm_chain(),
        website_base_url: "https://dravr.ai".to_owned(),
        ..ServerConfig::default()
    });

    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            database,
            auth_manager,
            "test_jwt_secret",
            common::offline_config(config),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
                archive_imports: None,
                live_records: None,
                blobs: None,
                lab_reader: None,
                pdf_text_bin: None,
                contremaitre: None,
                #[cfg(feature = "client-messaging")]
                channel_adapters: common::offline_adapters(),
            },
        )
        .await
        .unwrap(),
    );

    let auth_routes = AuthService::new(
        server_resources.auth.auth_manager.clone(),
        server_resources.auth.jwks_manager.clone(),
        server_resources.common.config.clone(),
        server_resources.data(),
    );

    let login_request = LoginRequest {
        email: "nonexistent@example.com".to_owned(),
        password: SecretString::from("password123"),
        timezone: None,
    };

    let result = auth_routes.login(login_request).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid email or password"));
}
