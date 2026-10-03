// ABOUTME: End-to-end integration test for complete multi-tenant onboarding workflow
// ABOUTME: Tests tenant creation, OAuth app registration, credential management, and tool execution
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # End-to-End Tenant Onboarding Test
//!
//! This test demonstrates the complete multi-tenant onboarding workflow:
//! 1. Create a new tenant with admin user
//! 2. Register OAuth applications for fitness providers
//! 3. Configure tenant-specific OAuth credentials
//! 4. Execute tools using tenant-isolated credentials
//! 5. Verify proper isolation between tenants

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use dravr_cageux::types::{
    ActivityIntelligence, ContextualFactors, PerformanceMetrics, TimeOfDay, TrendDirection,
    TrendIndicators,
};
use pierre_auth::{
    auth::AuthManager,
    tenant::oauth_manager::{
        authorizing_client, default_rate_limit_for_provider, issuing_client, IssuingClient,
        IssuingLookup,
    },
};
use pierre_config::environment::{OAuthProviderConfig, ServerConfig};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{Tenant, TenantId, TenantOAuthCredentials, User, UserStatus, UserTier};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use pierre_test_support::db::create_test_db;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::json;
use std::sync::Arc;
use tracing_subscriber::fmt;
use uuid::Uuid;

mod common;

/// Test configuration for end-to-end tenant onboarding
#[tokio::test]
async fn test_complete_tenant_onboarding_workflow() -> Result<()> {
    fmt::init();

    // Initialize HTTP clients (only once across all tests)
    common::init_test_http_clients();
    common::init_server_config();

    // Step 1: Create test database and base infrastructure
    let database = Arc::new(
        create_test_db()
            .await
            .expect("Failed to create test database"),
    );

    // Step 2: Create admin users first (required for tenant foreign key constraints)
    let acme_admin_id = Uuid::new_v4();
    let beta_admin_id = Uuid::new_v4();
    let acme_tenant_id = TenantId::generate();
    let beta_tenant_id = TenantId::generate();

    let acme_admin = User {
        id: acme_admin_id,
        email: "admin@acmefitness.com".to_owned(),
        display_name: Some("Acme Admin".to_owned()),
        password_hash: "hashed_password".to_owned(),
        tier: UserTier::Enterprise,
        strava_token: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: true,
        role: UserRole::Admin,
        approved_by: None,
        approved_at: Some(chrono::Utc::now()),
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

    let beta_admin = User {
        id: beta_admin_id,
        email: "admin@betahealth.com".to_owned(),
        display_name: Some("Beta Admin".to_owned()),
        password_hash: "hashed_password".to_owned(),
        tier: UserTier::Professional,
        strava_token: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: true,
        role: UserRole::Admin,
        approved_by: None,
        approved_at: Some(chrono::Utc::now()),
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

    database.repositories().users.create(&acme_admin).await?;
    database.repositories().users.create(&beta_admin).await?;

    // Step 3: Create first tenant ("Acme Fitness Co.")
    let acme_tenant = Tenant {
        id: acme_tenant_id,
        name: "Acme Fitness Co.".to_owned(),
        slug: "acme-fitness".to_owned(),
        domain: None,
        plan: "enterprise".to_owned(),
        owner_user_id: acme_admin_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    database.repositories().tenants.create(&acme_tenant).await?;

    // Step 4: Create second tenant ("Beta Health Inc.") for isolation testing
    let beta_tenant = Tenant {
        id: beta_tenant_id,
        name: "Beta Health Inc.".to_owned(),
        slug: "beta-health".to_owned(),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: beta_admin_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    database.repositories().tenants.create(&beta_tenant).await?;

    // Step 5: Configure each tenant's OAuth credentials

    // Configure Acme's Strava credentials

    database
        .repositories()
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id: acme_tenant_id,
            provider: "strava".to_owned(),
            client_id: "acme_strava_client_123".to_owned(),
            client_secret: "acme_secret_key".to_owned(),
            redirect_uri: "https://acme-fitness.com/oauth/strava/callback".to_owned(),
            scopes: vec!["read".to_owned(), "activity:read_all".to_owned()],
            rate_limit_per_day: default_rate_limit_for_provider("strava"),
        })
        .await?;

    // Configure Beta's Strava credentials

    database
        .repositories()
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id: beta_tenant_id,
            provider: "strava".to_owned(),
            client_id: "beta_strava_client_456".to_owned(),
            client_secret: "beta_secret_key".to_owned(),
            redirect_uri: "https://beta-health.com/oauth/strava/callback".to_owned(),
            scopes: vec!["read".to_owned(), "activity:read_all".to_owned()],
            rate_limit_per_day: default_rate_limit_for_provider("strava"),
        })
        .await?;

    // Step 6: Create Universal Tool Executor with tenant OAuth support
    let _intelligence = Arc::new(ActivityIntelligence::new(
        "E2E Test Intelligence".to_owned(),
        vec![], // No initial insights
        PerformanceMetrics {
            relative_effort: Some(85.0),
            zone_distribution: None,
            personal_records: Vec::new(),
            efficiency_score: Some(90.0),
            trend_indicators: TrendIndicators {
                pace_trend: TrendDirection::Improving,
                effort_trend: TrendDirection::Stable,
                distance_trend: TrendDirection::Improving,
                consistency_score: 85.0,
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
    ));

    let config = Arc::new(create_test_server_config());

    // Create ServerContext for the test
    let auth_manager = AuthManager::new(24);
    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            (*database).clone(),
            auth_manager,
            "test_secret",
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
        .await
        .unwrap(),
    );
    // The executor and the authorize builder (Step 10) share the one context
    let executor_resources: Arc<ServerContext> = Arc::clone(&server_resources);
    let executor =
        UniversalToolExecutor::new(executor_resources).with_scopes(OAuthScope::self_grant());

    // Step 7: Test tenant-aware tool execution for Acme
    let acme_request = UniversalRequest {
        tool_name: "get_connection_status".to_owned(),
        parameters: json!({}),
        user_id: acme_admin_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(acme_tenant_id.to_string()),
    };

    let acme_response = executor.execute_tool(acme_request).await?;
    assert!(acme_response.success);
    println!("Acme tenant tool execution successful");

    // Step 8: Test tenant-aware tool execution for Beta
    let beta_request = UniversalRequest {
        tool_name: "get_connection_status".to_owned(),
        parameters: json!({}),
        user_id: beta_admin_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(beta_tenant_id.to_string()),
    };

    let beta_response = executor.execute_tool(beta_request).await?;
    assert!(beta_response.success);
    println!("Beta tenant tool execution successful");

    // Step 9: Verify tenant isolation - each tenant's grants belong to its own app
    let repos = database.repositories();
    let acme_client = tenant_client(&database, acme_tenant_id, None).await?;
    let beta_client = tenant_client(&database, beta_tenant_id, None).await?;
    assert!(matches!(acme_client, IssuingClient::Tenant(_)));
    assert!(matches!(beta_client, IssuingClient::Tenant(_)));
    assert_eq!(acme_client.client_id(), "acme_strava_client_123");
    assert_eq!(beta_client.client_id(), "beta_strava_client_456");
    assert_ne!(acme_client.client_secret(), beta_client.client_secret());

    println!("Tenant OAuth credential isolation verified");

    // Step 10: A new authorization in each tenant runs under that tenant's app,
    // which no Strava pool app owns
    let server_level = OAuthProviderConfig::default();
    let (acme_authorizing, acme_pool_app) = authorizing_client(
        acme_admin_id,
        acme_tenant_id,
        "strava",
        &server_level,
        &*repos.tenants,
        &*repos.oauth_tokens,
    )
    .await?;
    let (beta_authorizing, beta_pool_app) = authorizing_client(
        beta_admin_id,
        beta_tenant_id,
        "strava",
        &server_level,
        &*repos.tenants,
        &*repos.oauth_tokens,
    )
    .await?;
    assert_eq!(acme_authorizing.client_id(), "acme_strava_client_123");
    assert_eq!(beta_authorizing.client_id(), "beta_strava_client_456");
    assert_eq!(acme_pool_app, None);
    assert_eq!(beta_pool_app, None);

    // The authorize URL each tenant's athlete is sent to names that app
    let oauth_service = OAuthService::new(
        server_resources.data(),
        server_resources.common.config.clone(),
    );
    let acme_auth_url = oauth_service
        .get_auth_url(
            acme_admin_id,
            acme_tenant_id,
            "strava",
            AuthUrlOptions::default(),
        )
        .await?;
    let beta_auth_url = oauth_service
        .get_auth_url(
            beta_admin_id,
            beta_tenant_id,
            "strava",
            AuthUrlOptions::default(),
        )
        .await?;
    assert!(acme_auth_url
        .authorization_url
        .contains("client_id=acme_strava_client_123&"));
    assert!(beta_auth_url
        .authorization_url
        .contains("client_id=beta_strava_client_456&"));

    println!("Tenant-specific OAuth authorizations resolved");

    // Step 12: Comprehensive workflow validation
    println!("\nEND-TO-END TENANT ONBOARDING WORKFLOW COMPLETED SUCCESSFULLY!");
    println!("   Multi-tenant database setup");
    println!("   Tenant creation and user management");
    println!("   OAuth application registration per tenant");
    println!("   Tenant-specific credential configuration");
    println!("   Isolated tool execution per tenant");
    println!("   OAuth credential isolation verification");
    println!("   Tenant-specific OAuth authorizations");

    Ok(())
}

/// Helper function to create test database for tenant context tests
async fn create_tenant_test_database() -> Result<Arc<Database>> {
    let database = Arc::new(create_test_db().await?);

    Ok(database)
}

/// Helper function to setup multi-tenant test scenario
async fn setup_multitenant_scenario(
    database: &Arc<Database>,
) -> Result<(TenantId, TenantId, Uuid)> {
    let tenant1_id = TenantId::generate();
    let tenant2_id = TenantId::generate();
    let user_id = Uuid::new_v4();

    let user = User {
        id: user_id,
        email: "multi-tenant-user@example.com".to_owned(),
        display_name: Some("Multi Tenant User".to_owned()),
        password_hash: "hashed_password".to_owned(),
        tier: UserTier::Professional,
        strava_token: None,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(chrono::Utc::now()),
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

    database.repositories().users.create(&user).await?;

    let tenant1 = Tenant {
        id: tenant1_id,
        name: "Tenant One".to_owned(),
        slug: "tenant-one".to_owned(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let tenant2 = Tenant {
        id: tenant2_id,
        name: "Tenant Two".to_owned(),
        slug: "tenant-two".to_owned(),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    database.repositories().tenants.create(&tenant1).await?;
    database.repositories().tenants.create(&tenant2).await?;

    Ok((tenant1_id, tenant2_id, user_id))
}

/// Test tenant switching and context validation
#[tokio::test]
async fn test_tenant_context_switching() -> Result<()> {
    common::init_test_http_clients();
    let database = create_tenant_test_database().await?;
    let (tenant1_id, tenant2_id, user_id) = setup_multitenant_scenario(&database).await?;

    // Set up different OAuth credentials for each tenant

    database
        .repositories()
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id: tenant1_id,
            provider: "strava".to_owned(),
            client_id: "tenant1_client".to_owned(),
            client_secret: "tenant1_secret".to_owned(),
            redirect_uri: "https://tenant1.com/callback".to_owned(),
            scopes: vec!["read".to_owned()],
            rate_limit_per_day: default_rate_limit_for_provider("strava"),
        })
        .await?;
    database
        .repositories()
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id: tenant2_id,
            provider: "strava".to_owned(),
            client_id: "tenant2_client".to_owned(),
            client_secret: "tenant2_secret".to_owned(),
            redirect_uri: "https://tenant2.com/callback".to_owned(),
            scopes: vec!["read".to_owned(), "write".to_owned()],
            rate_limit_per_day: default_rate_limit_for_provider("strava"),
        })
        .await?;

    // The same user resolves a different OAuth client in each tenant
    let oauth1 = tenant_client(&database, tenant1_id, Some(user_id)).await?;
    let oauth2 = tenant_client(&database, tenant2_id, Some(user_id)).await?;

    assert_eq!(oauth1.client_id(), "tenant1_client");
    assert_eq!(oauth2.client_id(), "tenant2_client");
    assert_ne!(oauth1.client_secret(), oauth2.client_secret());

    println!("Tenant context switching validated");

    Ok(())
}

/// Helper function to create test server configuration
fn create_test_server_config() -> ServerConfig {
    use pierre_config::environment::*;
    use std::path::PathBuf;

    ServerConfig {
        http_port: 4000,
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
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: Some("test_strava_client".to_owned()),
                client_secret: Some("test_strava_secret".to_owned()),
                redirect_uri: Some("http://localhost:3000/oauth/strava/callback".to_owned()),
                scopes: vec!["read".to_owned(), "activity:read_all".to_owned()],
                enabled: true,
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
                environment: Environment::Development,
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
                enabled: true,
            },
            strava_api: StravaApiConfig {
                base_url: "https://www.strava.com/api/v3".to_owned(),
                auth_url: "https://www.strava.com/oauth/authorize".to_owned(),
                token_url: "https://www.strava.com/oauth/token".to_owned(),
                revoke_url: "https://www.strava.com/oauth/revoke".to_owned(),
                ..Default::default()
            },
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
                mcp_version: "2024-11-05".to_owned(),
                server_name: "pierre-mcp-server-e2e-test".to_owned(),
                server_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        },
        sse: SseConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        ..Default::default()
    }
}

/// Tenant OAuth credentials live in the `tenants` repository alone, encrypted
/// at rest, so whatever resolves a client after they were stored (a restart,
/// another instance) reads the same app and secret. There is no per-process
/// copy to lose.
#[tokio::test]
async fn tenant_credentials_are_read_from_the_store() -> Result<()> {
    let database = create_tenant_test_database().await?;
    let (tenant_id, _other_tenant, _user_id) = setup_multitenant_scenario(&database).await?;
    let repos = database.repositories();
    repos
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id,
            provider: "whoop".to_owned(),
            client_id: "tenant-whoop-app".to_owned(),
            client_secret: "tenant-whoop-secret".to_owned(),
            redirect_uri: "https://tenant.example.com/api/oauth/callback/whoop".to_owned(),
            scopes: vec!["read:recovery".to_owned()],
            rate_limit_per_day: 4_321,
        })
        .await?;

    let resolved = issuing_client(
        IssuingLookup {
            user_id: None,
            tenant_id: Some(tenant_id),
            provider: "whoop",
            issuing_app: None,
            server_level: &OAuthProviderConfig::default(),
        },
        &*repos.tenants,
        &*repos.oauth_tokens,
    )
    .await?;
    let IssuingClient::Tenant(stored) = resolved else {
        panic!("the tenant's app is read back, got {resolved:?}");
    };
    assert_eq!(stored.client_id, "tenant-whoop-app");
    assert_eq!(stored.client_secret, "tenant-whoop-secret");
    assert_eq!(
        stored.rate_limit_per_day, 4_321,
        "the stored daily limit persists"
    );
    Ok(())
}

/// The client a grant for `user_id` in `tenant_id` belongs to, with no server-level app.
async fn tenant_client(
    database: &Database,
    tenant_id: TenantId,
    user_id: Option<Uuid>,
) -> Result<IssuingClient> {
    let repos = database.repositories();
    Ok(issuing_client(
        IssuingLookup {
            user_id,
            tenant_id: Some(tenant_id),
            provider: "strava",
            issuing_app: None,
            server_level: &OAuthProviderConfig::default(),
        },
        &*repos.tenants,
        &*repos.oauth_tokens,
    )
    .await?)
}
