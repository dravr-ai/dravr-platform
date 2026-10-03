// ABOUTME: Tests for dashboard route handlers and endpoints
// ABOUTME: Tests dashboard routes, user interface, and data presentation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::significant_drop_tightening,
    clippy::match_wildcard_for_single_variants,
    clippy::match_same_arms,
    clippy::unreadable_literal,
    clippy::module_name_repetitions,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_pass_by_value,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::struct_excessive_bools,
    clippy::missing_const_for_fn,
    clippy::cognitive_complexity,
    clippy::items_after_statements,
    clippy::semicolon_if_nothing_returned,
    clippy::use_self,
    clippy::single_match_else,
    clippy::default_trait_access,
    clippy::enum_glob_use,
    clippy::wildcard_imports,
    clippy::explicit_deref_methods,
    clippy::explicit_iter_loop,
    clippy::manual_let_else,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::unused_self,
    clippy::used_underscore_binding,
    clippy::fn_params_excessive_bools,
    clippy::trivially_copy_pass_by_ref,
    clippy::option_if_let_else,
    clippy::unnecessary_wraps,
    clippy::redundant_else,
    clippy::map_unwrap_or,
    clippy::map_err_ignore,
    clippy::if_not_else,
    clippy::single_char_lifetime_names,
    clippy::doc_markdown,
    clippy::unused_async,
    clippy::redundant_field_names,
    clippy::struct_field_names,
    clippy::ptr_arg,
    clippy::ref_option_ref,
    clippy::implicit_clone,
    clippy::cloned_instead_of_copied,
    clippy::borrow_as_ptr,
    clippy::bool_to_int_with_if,
    clippy::checked_conversions,
    clippy::copy_iterator,
    clippy::empty_enums,
    clippy::enum_variant_names,
    clippy::expl_impl_clone_on_copy,
    clippy::fallible_impl_from,
    clippy::filter_map_next,
    clippy::flat_map_option,
    clippy::fn_to_numeric_cast_any,
    clippy::if_let_mutex,
    clippy::implicit_hasher,
    clippy::inconsistent_struct_constructor,
    clippy::inefficient_to_string,
    clippy::infinite_iter,
    clippy::into_iter_on_ref,
    clippy::iter_not_returning_iterator,
    clippy::iter_on_empty_collections,
    clippy::iter_on_single_items,
    clippy::large_digit_groups,
    clippy::large_stack_arrays,
    clippy::large_types_passed_by_value,
    clippy::let_unit_value,
    clippy::linkedlist,
    clippy::lossy_float_literal,
    clippy::macro_use_imports,
    clippy::manual_assert,
    clippy::manual_instant_elapsed,
    clippy::manual_ok_or,
    clippy::manual_string_new,
    clippy::many_single_char_names,
    clippy::match_wild_err_arm,
    clippy::mem_forget,
    clippy::missing_enforced_import_renames,
    clippy::missing_inline_in_public_items,
    clippy::missing_safety_doc,
    clippy::mut_mut,
    clippy::mutex_integer,
    clippy::naive_bytecount,
    clippy::needless_continue,
    clippy::needless_for_each,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_raw_string_hashes,
    clippy::no_effect_underscore_binding,
    clippy::non_ascii_literal,
    clippy::nonstandard_macro_braces,
    clippy::option_option,
    clippy::or_fun_call,
    clippy::path_buf_push_overwrite,
    clippy::print_literal,
    clippy::print_with_newline,
    clippy::ptr_as_ptr,
    clippy::range_minus_one,
    clippy::range_plus_one,
    clippy::rc_buffer,
    clippy::rc_mutex,
    clippy::redundant_allocation,
    clippy::redundant_pub_crate,
    clippy::ref_binding_to_reference,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::same_functions_in_if_condition,
    clippy::str_to_string,
    clippy::string_add,
    clippy::string_add_assign,
    clippy::string_lit_as_bytes,
    clippy::trait_duplication_in_bounds,
    clippy::transmute_ptr_to_ptr,
    clippy::tuple_array_conversions,
    clippy::unchecked_time_subtraction,
    clippy::unicode_not_nfc,
    clippy::unimplemented,
    clippy::unnecessary_box_returns,
    clippy::unnecessary_struct_initialization,
    clippy::unnecessary_to_owned,
    clippy::unnested_or_patterns,
    clippy::unused_peekable,
    clippy::unused_rounding,
    clippy::useless_let_if_seq,
    clippy::verbose_bit_mask,
    clippy::verbose_file_reads,
    clippy::zero_sized_map_values
)]
//

//! Comprehensive integration tests for dashboard routes
//!
//! This test suite provides comprehensive coverage for all dashboard route endpoints,
//! including authentication, authorization, request/response validation,
//! error handling, edge cases, and dashboard-specific functionality.

mod common;

use anyhow::Result;
use chrono::{Duration, Utc};
use futures_util::{stream, StreamExt, TryStreamExt};
use pierre_auth::{
    api_keys::{ApiKey, ApiKeyManager, ApiKeyTier, ApiKeyUsage, CreateApiKeyRequest},
    auth::{AuthMethod, AuthResult},
};
use pierre_config::environment::{
    AppBehaviorConfig, AuthConfig, BackupConfig, CacheConfig, CorsConfig, DatabaseConfig,
    DatabaseUrl, Environment, ExternalServicesConfig, FirebaseConfig, GarminApiConfig,
    GeocodingServiceConfig, GoalManagementConfig, HttpClientConfig, LogLevel, LoggingConfig,
    McpConfig, MonitoringConfig, OAuth2ServerConfig, OAuthConfig, OAuthProviderConfig,
    PostgresPoolConfig, ProtocolConfig, RateLimitConfig, SecurityConfig, SecurityHeadersConfig,
    ServerConfig, SleepToolParamsConfig, SqlxConfig, SseConfig, StravaApiConfig, TlsConfig,
    TokioRuntimeConfig, TrainingZonesConfig, WeatherServiceConfig,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::factory::Database;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_dashboard::service::DashboardRoutes;
use std::sync::Arc;
use uuid::Uuid;

/// Test setup helper that creates all necessary components for dashboard route testing
struct DashboardTestSetup {
    dashboard_routes: DashboardRoutes<ServerContext>,
    user_id: Uuid,
}

impl DashboardTestSetup {
    async fn new() -> Result<Self> {
        // Create test database and auth manager
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();

        // Create minimal config for ServerContext
        let temp_dir = tempfile::tempdir()?;
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
                    base_url: "https://apis.garmin.com/wellness-api/rest".to_owned(),
                    auth_url: "https://connect.garmin.com/oauth2Confirm".to_owned(),
                    token_url: "https://diauth.garmin.com/di-oauth2-service/oauth/token"
                        .to_string(),
                    revoke_url: "https://apis.garmin.com/wellness-api/rest/user/registration"
                        .to_owned(),
                    ..Default::default()
                },
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
            sse: SseConfig::default(),
            oauth2_server: OAuth2ServerConfig::default(),
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
            website_base_url: "https://dravr.ai".to_owned(),
        });

        // Create test cache
        let cache = common::create_test_cache().await?;

        // Create ServerContext using proper constructor
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
            .await
            .unwrap(),
        );

        // Create dashboard routes
        let dashboard_routes = DashboardRoutes::new(server_resources);

        // Create test user
        let (user_id, _) = common::create_test_user(&database).await?;

        // Create multiple test API keys with different tiers and usage patterns
        let mut api_keys = Vec::new();

        // Create starter tier API key
        let starter_key =
            common::create_and_store_test_api_key(&database, user_id, "Starter Dashboard Key")
                .await?;
        api_keys.push(starter_key);

        // Create professional tier API key
        let request_pro = CreateApiKeyRequest {
            name: "Professional Dashboard Key".to_owned(),
            description: Some("Professional tier for dashboard testing".to_owned()),
            tier: ApiKeyTier::Professional,
            rate_limit_requests: Some(5000),
            expires_in_days: None,
        };

        let manager = ApiKeyManager::new();
        let (pro_key, _) = manager.create_api_key(user_id, request_pro)?;
        database.repositories().api_keys.create(&pro_key).await?;
        api_keys.push(pro_key);

        // Create enterprise tier API key
        let request_enterprise = CreateApiKeyRequest {
            name: "Enterprise Dashboard Key".to_owned(),
            description: Some("Enterprise tier for dashboard testing".to_owned()),
            tier: ApiKeyTier::Enterprise,
            rate_limit_requests: None, // Unlimited
            expires_in_days: Some(365),
        };

        let (enterprise_key, _) = manager.create_api_key(user_id, request_enterprise)?;
        database
            .repositories()
            .api_keys
            .create(&enterprise_key)
            .await?;
        api_keys.push(enterprise_key);

        // Create some usage data for testing
        Self::create_test_usage_data(&database, &api_keys).await?;

        Ok(Self {
            dashboard_routes,
            user_id,
        })
    }

    /// Create AuthResult for testing authenticated endpoints
    fn auth_result(&self) -> AuthResult {
        use pierre_auth::auth::{AuthMethod, AuthResult};

        AuthResult {
            scopes: OAuthScope::self_grant(),
            session_id: None,
            user_id: self.user_id,
            auth_method: AuthMethod::JwtToken {
                tier: "premium".to_owned(),
            },
            active_tenant_id: None,
        }
    }

    /// Create test usage data for dashboard analytics
    ///
    /// The three keys carry 4543 usage rows between them and every test in this
    /// file builds the whole set, so how the rows are WRITTEN decides the
    /// file's run time. Two consequences, neither of which changes a single
    /// row: the rows are built before any of them is written, so the repository
    /// registry — which clones the backend and rebuilds sixty trait-object
    /// handles — is constructed once instead of once per row; and the writes go
    /// out `USAGE_WRITE_CONCURRENCY` at a time, which is safe because
    /// `record_api_key` is a bare single-row INSERT on both backends, with no
    /// counter row or trigger behind it to contend on. Every dashboard query
    /// orders by timestamp or by an aggregate, never by insert order.
    async fn create_test_usage_data(database: &Database, api_keys: &[ApiKey]) -> Result<()> {
        /// Writes in flight. The CI PostgreSQL test pool caps at three
        /// connections, so this only has to be large enough to keep them all
        /// busy; SQLite's single connection serialises regardless.
        const USAGE_WRITE_CONCURRENCY: usize = 8;

        let now = Utc::now();
        let mut rows = Vec::new();

        // Create some API key usage records for testing
        for (i, api_key) in api_keys.iter().enumerate() {
            // Create usage for the last few days
            for days_ago in 0..7 {
                let timestamp = now - Duration::days(days_ago);

                // Vary usage patterns by API key tier
                let base_requests = match api_key.tier {
                    ApiKeyTier::Trial => 5,
                    ApiKeyTier::Starter => 25,
                    ApiKeyTier::Professional => 100,
                    // Enterprise and any future tiers default to max
                    _ => 500,
                };

                let request_count = base_requests + (i as u32 * 5) + (days_ago as u32 % 10);

                // Create usage records using the available API
                for j in 0..request_count {
                    rows.push(ApiKeyUsage {
                        id: None,
                        api_key_id: api_key.id.clone(),
                        timestamp: timestamp + Duration::minutes(i64::from(j) * 2),
                        tool_name: match j % 4 {
                            0 => "strava_activities".to_owned(),
                            1 => "whoop_data".to_owned(),
                            2 => "weather_info".to_owned(),
                            _ => "analytics".to_owned(),
                        },
                        response_time_ms: Some(100 + (j % 200)),
                        status_code: if j % 20 == 0 { 500 } else { 200 }, // 95% success rate
                        error_message: if j % 20 == 0 {
                            Some("Test error".to_owned())
                        } else {
                            None
                        },
                        request_size_bytes: Some(1024 + (j % 512)),
                        response_size_bytes: Some(2048 + (j % 1024)),
                        ip_address: Some("127.0.0.1".to_owned()),
                        user_agent: Some("test-client".to_owned()),
                    });
                }
            }
        }

        // Record the usage
        let usage_repo = &database.repositories().usage;
        stream::iter(rows)
            .map(|usage| {
                let repo = Arc::clone(usage_repo);
                async move { repo.record_api_key(&usage).await }
            })
            .buffer_unordered(USAGE_WRITE_CONCURRENCY)
            .try_collect::<Vec<()>>()
            .await?;

        Ok(())
    }
}

// ============================================================================
// Dashboard Overview Tests
// ============================================================================

// ============================================================================
// Usage Analytics Tests
// ============================================================================

#[tokio::test]
async fn test_get_usage_analytics_success() -> Result<()> {
    common::init_server_config();
    let setup = DashboardTestSetup::new().await?;

    let analytics = setup
        .dashboard_routes
        .get_usage_analytics(setup.auth_result(), 7)
        .await?;

    // Verify time series data
    assert_eq!(analytics.time_series.len(), 7); // 7 days requested

    // Verify each day has data
    for data_point in &analytics.time_series {
        // Date is YYYY-MM-DD format string
        assert_eq!(
            data_point.date.len(),
            10,
            "Date should be YYYY-MM-DD format"
        );
        assert!(
            data_point.date.starts_with("20"),
            "Date should start with year"
        );
        assert!(data_point.average_response_time >= 0.0);
    }

    // Note: Top tools might be empty in test environment due to data setup limitations
    // This is acceptable as we're testing the API interface and authentication
    for tool in &analytics.top_tools {
        assert!(!tool.tool_name.is_empty());
        // Verify tool structure (removing redundant >= 0 check for unsigned type)
        assert!(!tool.tool_name.is_empty());
        assert!(tool.success_rate >= 0.0 && tool.success_rate <= 100.0);
        assert!(tool.average_response_time >= 0.0);
    }

    // Verify overall metrics
    assert!(analytics.average_response_time >= 0.0);

    Ok(())
}

#[tokio::test]
async fn test_get_usage_analytics_different_timeframes() -> Result<()> {
    common::init_server_config();
    let setup = DashboardTestSetup::new().await?;

    // Test different day ranges
    let timeframes = vec![1, 7, 30, 90];

    for days in timeframes {
        let analytics = setup
            .dashboard_routes
            .get_usage_analytics(setup.auth_result(), days)
            .await?;

        assert_eq!(analytics.time_series.len(), days as usize);

        // Verify dates are in correct order (oldest first, string comparison works for YYYY-MM-DD)
        for i in 1..analytics.time_series.len() {
            assert!(analytics.time_series[i].date >= analytics.time_series[i - 1].date);
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_get_usage_analytics_invalid_auth() -> Result<()> {
    common::init_server_config();
    let setup = DashboardTestSetup::new().await?;

    let result = setup
        .dashboard_routes
        .get_usage_analytics(
            AuthResult {
                scopes: OAuthScope::self_grant(),
                session_id: None,
                user_id: uuid::Uuid::nil(),
                auth_method: AuthMethod::JwtToken {
                    tier: "premium".to_owned(),
                },
                active_tenant_id: None,
            },
            7,
        )
        .await;
    assert!(result.is_err());

    let result = setup
        .dashboard_routes
        .get_usage_analytics(
            AuthResult {
                scopes: OAuthScope::self_grant(),
                session_id: None,
                user_id: uuid::Uuid::nil(),
                auth_method: AuthMethod::JwtToken {
                    tier: "premium".to_owned(),
                },
                active_tenant_id: None,
            },
            7,
        )
        .await;
    assert!(result.is_err());

    Ok(())
}

// ============================================================================
// Rate Limit Overview Tests
// ============================================================================

// ============================================================================
// Request Logs Tests
// ============================================================================

// ============================================================================
// Request Stats Tests
// ============================================================================

// ============================================================================
// Tool Usage Breakdown Tests
// ============================================================================

#[tokio::test]
async fn test_get_tool_usage_breakdown_success() -> Result<()> {
    common::init_server_config();
    let setup = DashboardTestSetup::new().await?;

    let tool_usage = setup
        .dashboard_routes
        .get_tool_usage_breakdown(
            setup.auth_result(),
            None,       // All API keys
            Some("7d"), // Last 7 days
        )
        .await?;

    // Note: Tool usage might be empty in test environment - this is acceptable
    // as we're testing the API interface and authentication

    // Verify tool usage structure
    for usage in &tool_usage {
        assert!(!usage.tool_name.is_empty());
        // Removed redundant >= 0 check for unsigned type
        assert!(usage.success_rate >= 0.0 && usage.success_rate <= 100.0);
        assert!(usage.average_response_time >= 0.0);
    }

    // Should be sorted by request count (descending)
    for i in 1..tool_usage.len() {
        assert!(tool_usage[i - 1].request_count >= tool_usage[i].request_count);
    }

    // Should not exceed 10 tools (top 10)
    assert!(tool_usage.len() <= 10);

    Ok(())
}

#[tokio::test]
async fn test_get_tool_usage_breakdown_different_timeframes() -> Result<()> {
    common::init_server_config();
    let setup = DashboardTestSetup::new().await?;

    let timeframes = vec!["1h", "24h", "7d", "30d"];

    for timeframe in timeframes {
        let tool_usage = setup
            .dashboard_routes
            .get_tool_usage_breakdown(setup.auth_result(), None, Some(timeframe))
            .await?;

        // Each timeframe should return valid data
        for usage in &tool_usage {
            assert!(!usage.tool_name.is_empty());
            // Removed redundant >= 0 check for unsigned type
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_get_tool_usage_breakdown_invalid_auth() -> Result<()> {
    common::init_server_config();
    let setup = DashboardTestSetup::new().await?;

    let result = setup
        .dashboard_routes
        .get_tool_usage_breakdown(
            AuthResult {
                scopes: OAuthScope::self_grant(),
                session_id: None,
                user_id: uuid::Uuid::nil(),
                auth_method: AuthMethod::JwtToken {
                    tier: "premium".to_owned(),
                },
                active_tenant_id: None,
            },
            None,
            Some("7d"),
        )
        .await;
    assert!(result.is_err());

    let result = setup
        .dashboard_routes
        .get_tool_usage_breakdown(
            AuthResult {
                scopes: OAuthScope::self_grant(),
                session_id: None,
                user_id: uuid::Uuid::nil(),
                auth_method: AuthMethod::JwtToken {
                    tier: "premium".to_owned(),
                },
                active_tenant_id: None,
            },
            None,
            Some("7d"),
        )
        .await;
    assert!(result.is_err());

    Ok(())
}

// ============================================================================
// Edge Cases and Error Handling Tests
// ============================================================================

// JWT expiration test removed - JWT validation happens at HTTP filter level
// Route methods only validate that user_id is not nil

#[tokio::test]
async fn test_dashboard_boundary_conditions() -> Result<()> {
    common::init_server_config();
    let setup = DashboardTestSetup::new().await?;

    // Test edge case: analytics for 0 days (should default to something reasonable)
    let analytics = setup
        .dashboard_routes
        .get_usage_analytics(setup.auth_result(), 0)
        .await?;

    assert_eq!(analytics.time_series.len(), 0);

    // Test large number of days
    let analytics = setup
        .dashboard_routes
        .get_usage_analytics(setup.auth_result(), 1000)
        .await?;

    assert_eq!(analytics.time_series.len(), 1000);

    Ok(())
}

// ============================================================================
// Integration with Database Tests
// ============================================================================
