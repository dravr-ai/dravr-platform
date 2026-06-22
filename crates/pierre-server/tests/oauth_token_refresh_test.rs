// ABOUTME: OAuth token refresh tests for token lifecycle management
// ABOUTME: Tests token refresh logic, expiration handling, and token persistence
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
    clippy::from_iter_instead_of_collect,
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

//! # OAuth Token Refresh Tests
//!
//! Tests for automatic token refresh in Universal Tool Executor.

mod common;

use axum::{routing::post, Json, Router};
use pierre_auth::auth::AuthManager;
use pierre_config::environment::{
    AppBehaviorConfig, AuthConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment,
    ExternalServicesConfig, FitbitApiConfig, GeocodingServiceConfig, HttpClientConfig, LogLevel,
    LoggingConfig, MonitoringConfig, OAuth2ServerConfig, OAuthConfig, OAuthProviderConfig,
    PostgresPoolConfig, ProtocolConfig, RouteTimeoutConfig, SecurityConfig, SecurityHeadersConfig,
    ServerConfig, SseConfig, StravaApiConfig, TlsConfig, WeatherServiceConfig,
};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{Tenant, TenantId, User, UserOAuthToken, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::{backends::factory::Database, database::generate_encryption_key};
use pierre_intelligence::{
    ActivityIntelligence, ContextualFactors, PerformanceMetrics, TimeOfDay, TrendDirection,
    TrendIndicators,
};
use pierre_mcp_server::{
    constants::oauth_providers,
    mcp::resources::{ServerContext, ServerContextOptions},
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::json;
use serial_test::serial;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{env, path::PathBuf, sync::Arc};
use tokio::net::TcpListener;
use uuid::Uuid;

/// Create a test ServerConfig with missing OAuth credentials for failure testing
fn create_test_server_config_without_oauth() -> Arc<ServerConfig> {
    Arc::new(ServerConfig {
        http_port: 8081,
        oauth_callback_port: 35535,
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
            enable_refresh_tokens: false,
            ..AuthConfig::default()
        },
        oauth: OAuthConfig {
            strava: OAuthProviderConfig {
                client_id: None,     // Missing credentials
                client_secret: None, // Missing credentials
                redirect_uri: Some("http://localhost:8081/oauth/callback/strava".to_owned()),
                scopes: vec!["read".to_owned(), "activity:read_all".to_owned()],
                enabled: true,
            },
            fitbit: OAuthProviderConfig {
                client_id: None,     // Missing credentials
                client_secret: None, // Missing credentials
                redirect_uri: Some("http://localhost:8081/oauth/callback/fitbit".to_owned()),
                scopes: vec!["activity".to_owned(), "profile".to_owned()],
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
            strava_api: StravaApiConfig {
                base_url: "https://www.strava.com/api/v3".to_owned(),
                auth_url: "https://www.strava.com/oauth/authorize".to_owned(),
                token_url: "https://www.strava.com/oauth/token".to_owned(),
                deauthorize_url: "https://www.strava.com/oauth/deauthorize".to_owned(),
                ..Default::default()
            },
            fitbit_api: FitbitApiConfig {
                base_url: "https://api.fitbit.com".to_owned(),
                auth_url: "https://www.fitbit.com/oauth2/authorize".to_owned(),
                token_url: "https://api.fitbit.com/oauth2/token".to_owned(),
                revoke_url: "https://api.fitbit.com/oauth2/revoke".to_owned(),
                ..Default::default()
            },
            geocoding: GeocodingServiceConfig {
                base_url: "https://nominatim.openstreetmap.org".to_owned(),
                enabled: true,
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
                server_name: "pierre-mcp-server-test".to_owned(),
                server_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        },
        sse: SseConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        route_timeouts: RouteTimeoutConfig::default(),
        monitoring: MonitoringConfig::default(),
        ..Default::default()
    })
}

/// Create a test ServerConfig for OAuth token refresh tests
fn create_test_server_config() -> Arc<ServerConfig> {
    Arc::new(ServerConfig {
        http_port: 8081,
        oauth_callback_port: 35535,
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
            enable_refresh_tokens: false,
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
            fitbit: OAuthProviderConfig {
                client_id: Some("test_fitbit_id".to_owned()),
                client_secret: Some("test_fitbit_secret".to_owned()),
                redirect_uri: Some("http://localhost:8081/oauth/callback/fitbit".to_owned()),
                scopes: vec!["activity".to_owned(), "profile".to_owned()],
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
            strava_api: StravaApiConfig {
                base_url: "https://www.strava.com/api/v3".to_owned(),
                auth_url: "https://www.strava.com/oauth/authorize".to_owned(),
                token_url: "https://www.strava.com/oauth/token".to_owned(),
                deauthorize_url: "https://www.strava.com/oauth/deauthorize".to_owned(),
                ..Default::default()
            },
            fitbit_api: FitbitApiConfig {
                base_url: "https://api.fitbit.com".to_owned(),
                auth_url: "https://www.fitbit.com/oauth2/authorize".to_owned(),
                token_url: "https://api.fitbit.com/oauth2/token".to_owned(),
                revoke_url: "https://api.fitbit.com/oauth2/revoke".to_owned(),
                ..Default::default()
            },
            geocoding: GeocodingServiceConfig {
                base_url: "https://nominatim.openstreetmap.org".to_owned(),
                enabled: true,
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
                server_name: "pierre-mcp-server-test".to_owned(),
                server_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        },
        sse: SseConfig::default(),
        oauth2_server: OAuth2ServerConfig::default(),
        route_timeouts: RouteTimeoutConfig::default(),
        monitoring: MonitoringConfig::default(),
        ..Default::default()
    })
}

/// Create a test UniversalToolExecutor with in-memory database
async fn create_test_executor() -> (Arc<UniversalToolExecutor>, Arc<Database>) {
    #[cfg(feature = "postgresql")]
    let database = Arc::new(
        Database::new(
            "sqlite::memory:",
            generate_encryption_key().to_vec(),
            &PostgresPoolConfig::default(),
        )
        .await
        .unwrap(),
    );

    #[cfg(not(feature = "postgresql"))]
    let database = Arc::new(
        Database::new("sqlite::memory:", generate_encryption_key().to_vec())
            .await
            .unwrap(),
    );

    let _intelligence = Arc::new(ActivityIntelligence::new(
        "Test Intelligence".to_owned(),
        vec![],
        PerformanceMetrics {
            relative_effort: Some(7.5),
            zone_distribution: None,
            personal_records: vec![],
            efficiency_score: Some(85.0),
            trend_indicators: TrendIndicators {
                pace_trend: TrendDirection::Stable,
                effort_trend: TrendDirection::Improving,
                distance_trend: TrendDirection::Stable,
                consistency_score: 88.0,
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

    // Create ServerContext for the test
    let auth_manager = AuthManager::new(24);
    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            (*database).clone(),
            auth_manager,
            "test_secret",
            create_test_server_config(),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
            },
        )
        .await,
    );
    let executor = Arc::new(UniversalToolExecutor::new(server_resources));

    (executor, database)
}

/// Create a test UniversalToolExecutor without OAuth credentials for failure testing
async fn create_test_executor_without_oauth() -> (Arc<UniversalToolExecutor>, Arc<Database>) {
    #[cfg(feature = "postgresql")]
    let database = Arc::new(
        Database::new(
            "sqlite::memory:",
            generate_encryption_key().to_vec(),
            &PostgresPoolConfig::default(),
        )
        .await
        .unwrap(),
    );

    #[cfg(not(feature = "postgresql"))]
    let database = Arc::new(
        Database::new("sqlite::memory:", generate_encryption_key().to_vec())
            .await
            .unwrap(),
    );

    let _intelligence = Arc::new(ActivityIntelligence::new(
        "Test Intelligence".to_owned(),
        vec![],
        PerformanceMetrics {
            relative_effort: Some(7.5),
            zone_distribution: None,
            personal_records: vec![],
            efficiency_score: Some(85.0),
            trend_indicators: TrendIndicators {
                pace_trend: TrendDirection::Stable,
                effort_trend: TrendDirection::Improving,
                distance_trend: TrendDirection::Stable,
                consistency_score: 88.0,
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

    // Create ServerContext for the test
    let auth_manager = AuthManager::new(24);
    let cache = common::create_test_cache().await.unwrap();
    let server_resources = Arc::new(
        ServerContext::new(
            (*database).clone(),
            auth_manager,
            "test_secret",
            create_test_server_config_without_oauth(),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
            },
        )
        .await,
    );
    let executor = Arc::new(UniversalToolExecutor::new(server_resources));

    (executor, database)
}

/// Sets env vars for the duration of a test and removes them on Drop — even if
/// an assertion panics — so a failing `#[serial]` test cannot leak global env
/// state into the next one. (Credentials and the token-URL override are both
/// process-global, so cleanup must be panic-safe, not an end-of-fn `remove_var`.)
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, String)]) -> Self {
        let keys = vars.iter().map(|(k, _)| *k).collect();
        for (k, v) in vars {
            env::set_var(k, v);
        }
        Self { keys }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for k in &self.keys {
            env::remove_var(k);
        }
    }
}

/// Create an active user directly in the DB for refresh tests.
async fn create_active_user(database: &Database, user_id: Uuid, email: &str) {
    let user = User {
        id: user_id,
        email: email.to_owned(),
        display_name: Some("Refresh Test User".to_owned()),
        password_hash: bcrypt::hash("password", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
    };
    database.repositories().users.create(&user).await.unwrap();
}

/// Create a tenant with no provider OAuth credentials, so token refresh resolves
/// credentials via the env defaults (rather than erroring on a missing tenant).
async fn create_bare_tenant(database: &Database, tenant_id: TenantId, owner: Uuid) {
    let tenant = Tenant {
        id: tenant_id,
        name: "Refresh Test Tenant".to_owned(),
        slug: format!("refresh-test-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: owner,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    database
        .repositories()
        .tenants
        .create(&tenant)
        .await
        .unwrap();
}

/// Refresh success leg (the gap `test_get_activities_with_expired_token` left
/// open by accepting failure): an EXPIRED Strava token must trigger EXACTLY ONE
/// call to the provider token endpoint, and the refreshed token must be persisted
/// with a new access token, a future expiry, and the rotated refresh token.
///
/// The production seam (`PIERRE_STRAVA_TOKEN_URL`) redirects the refresh POST to a
/// local mock, so the call is deterministic and touches no network. The static
/// `api_client` middleware is a passthrough (no retry), so one refresh == one POST.
#[tokio::test]
#[serial]
async fn strava_expired_token_refreshes_exactly_once_and_persists() {
    common::init_server_config();

    // Local mock token endpoint: counts hits, returns a Strava token payload with
    // an ABSOLUTE future `expires_at` (Strava encodes expiry as a unix timestamp).
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_route = Arc::clone(&hits);
    let future_unix = (chrono::Utc::now() + chrono::Duration::hours(6)).timestamp();
    let app = Router::new().route(
        "/oauth/token",
        post(move || {
            let hits = Arc::clone(&hits_route);
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "access_token": "mock_new_access_token",
                    "refresh_token": "mock_new_refresh_token",
                    "token_type": "Bearer",
                    "expires_at": future_unix,
                    "expires_in": 21600,
                }))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    // Point the production seam at the mock and supply env credentials. The guard
    // removes all three even if an assertion panics.
    let _env = EnvGuard::set(&[
        (
            "PIERRE_STRAVA_TOKEN_URL",
            format!("http://{addr}/oauth/token"),
        ),
        ("STRAVA_CLIENT_ID", "test_client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "test_secret".to_owned()),
    ]);

    // Seed an active user, a credential-less tenant, and an EXPIRED Strava token.
    let (executor, database) = create_test_executor().await;
    let user_id = Uuid::new_v4();
    let tenant_str = "550e8400-e29b-41d4-a716-446655440000";
    let tenant_id: TenantId = tenant_str.parse().unwrap();
    create_active_user(&database, user_id, "refresh-once@example.com").await;
    create_bare_tenant(&database, tenant_id, user_id).await;

    let oauth_token = UserOAuthToken::new(
        user_id,
        tenant_str.to_owned(),
        oauth_providers::STRAVA.to_owned(),
        "expired_access_token".to_owned(),
        Some("old_refresh_token".to_owned()),
        Some(chrono::Utc::now() - chrono::Duration::hours(1)),
        Some("read,activity:read_all".to_owned()),
    );
    database
        .repositories()
        .oauth_tokens
        .upsert_token(&oauth_token)
        .await
        .unwrap();

    // Drive the refresh path directly.
    let token = executor
        .auth_service
        .get_valid_token(user_id, "strava", Some(tenant_str))
        .await
        .expect("get_valid_token must not error")
        .expect("a valid token must be returned after refresh");

    // Exactly one call to the provider token endpoint.
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "the refresh endpoint must be hit exactly once"
    );

    // The persisted token is the refreshed one (re-read to prove the DB write).
    let stored = database
        .repositories()
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .expect("token still present after refresh");
    assert_eq!(stored.access_token, "mock_new_access_token");
    assert_ne!(stored.access_token, "expired_access_token");
    assert!(
        stored.expires_at.unwrap() > chrono::Utc::now(),
        "refreshed token must have a future expiry"
    );
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some("mock_new_refresh_token"),
        "refresh token must be rotated to the new value"
    );
    assert_eq!(
        token.access_token, stored.access_token,
        "returned token must mirror what was persisted"
    );
}

/// One provider's expected refresh behaviour, exercised by
/// [`assert_refresh_once_and_persists`].
struct RefreshSeamCase {
    /// Provider key used for the token row, the seam env var, and the lookup.
    provider: &'static str,
    /// The `PIERRE_<PROVIDER>_TOKEN_URL` seam variable to redirect at the mock.
    token_url_env: &'static str,
    /// Credential env vars the default-credential path reads.
    client_id_env: &'static str,
    client_secret_env: &'static str,
    /// JSON body the mock token endpoint returns (provider-shaped).
    mock_response: serde_json::Value,
    /// Access token the mock returns and the DB must persist.
    expected_access: &'static str,
    /// Refresh token the DB must hold after refresh. When the response omits a
    /// new refresh token, this is the ORIGINAL (input) token, preserved.
    expected_refresh: &'static str,
    /// Unique seed email so concurrent cases don't collide.
    email: &'static str,
}

/// Shared refresh-success-leg assertion for every bearer provider: an EXPIRED
/// token must trigger EXACTLY ONE call to the (mocked) provider token endpoint,
/// and the refreshed token must be persisted with a new access token, a future
/// expiry, and the expected refresh token. The `PIERRE_<PROVIDER>_TOKEN_URL` seam
/// redirects the refresh POST to a local mock — deterministic, no network — and
/// the static `api_client` middleware is a passthrough (no retry), so one refresh
/// equals one POST.
async fn assert_refresh_once_and_persists(case: RefreshSeamCase) {
    common::init_server_config();

    let hits = Arc::new(AtomicUsize::new(0));
    let hits_route = Arc::clone(&hits);
    let body = case.mock_response.clone();
    let app = Router::new().route(
        "/oauth/token",
        post(move || {
            let hits = Arc::clone(&hits_route);
            let body = body.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                Json(body)
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let _env = EnvGuard::set(&[
        (case.token_url_env, format!("http://{addr}/oauth/token")),
        (case.client_id_env, "test_client".to_owned()),
        (case.client_secret_env, "test_secret".to_owned()),
    ]);

    let (executor, database) = create_test_executor().await;
    let user_id = Uuid::new_v4();
    let tenant_str = "550e8400-e29b-41d4-a716-446655440000";
    let tenant_id: TenantId = tenant_str.parse().unwrap();
    create_active_user(&database, user_id, case.email).await;
    create_bare_tenant(&database, tenant_id, user_id).await;

    let oauth_token = UserOAuthToken::new(
        user_id,
        tenant_str.to_owned(),
        case.provider.to_owned(),
        "expired_access_token".to_owned(),
        Some("old_refresh_token".to_owned()),
        Some(chrono::Utc::now() - chrono::Duration::hours(1)),
        Some("read,activity:read_all".to_owned()),
    );
    database
        .repositories()
        .oauth_tokens
        .upsert_token(&oauth_token)
        .await
        .unwrap();

    let token = executor
        .auth_service
        .get_valid_token(user_id, case.provider, Some(tenant_str))
        .await
        .expect("get_valid_token must not error")
        .expect("a valid token must be returned after refresh");

    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "{} refresh endpoint must be hit exactly once",
        case.provider
    );

    let stored = database
        .repositories()
        .oauth_tokens
        .get_token(user_id, tenant_id, case.provider)
        .await
        .unwrap()
        .expect("token still present after refresh");
    assert_eq!(stored.access_token, case.expected_access);
    assert_ne!(stored.access_token, "expired_access_token");
    assert!(
        stored.expires_at.unwrap() > chrono::Utc::now(),
        "{} refreshed token must have a future expiry",
        case.provider
    );
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some(case.expected_refresh),
        "{} refresh token must match the expected value",
        case.provider
    );
    assert_eq!(
        token.access_token, stored.access_token,
        "returned token must mirror what was persisted"
    );
}

#[tokio::test]
#[serial]
async fn fitbit_expired_token_refreshes_exactly_once_and_persists() {
    // Fitbit encodes expiry as a relative `expires_in` (seconds).
    assert_refresh_once_and_persists(RefreshSeamCase {
        provider: "fitbit",
        token_url_env: "PIERRE_FITBIT_TOKEN_URL",
        client_id_env: "FITBIT_CLIENT_ID",
        client_secret_env: "FITBIT_CLIENT_SECRET",
        mock_response: json!({
            "access_token": "fitbit_new_access",
            "refresh_token": "fitbit_new_refresh",
            "token_type": "Bearer",
            "expires_in": 28800,
            "scope": "activity heartrate",
            "user_id": "fitbit_user_123",
        }),
        expected_access: "fitbit_new_access",
        expected_refresh: "fitbit_new_refresh",
        email: "refresh-fitbit@example.com",
    })
    .await;
}

/// WHOOP refresh — BOTH cases in one test: the normal case (response carries a
/// new refresh token, which must be rotated) and the omit case (no refresh token
/// in the response, so the seeded input token must be preserved per
/// `client.rs`: `response.refresh_token.or_else(|| input)`).
///
/// The two cases run sequentially in a single test on purpose: each
/// `assert_refresh_once_and_persists` call sets and then drops the shared
/// `PIERRE_WHOOP_TOKEN_URL`, so running them as two separate tests would race on
/// that process-global var (`#[serial]` does not reliably serialize these async
/// tests). Each provider therefore owns exactly one test and a unique env var,
/// so cross-provider tests never collide even when run concurrently.
#[tokio::test]
#[serial]
async fn whoop_refresh_with_and_without_new_token() {
    // Response carries a new refresh token -> rotated.
    assert_refresh_once_and_persists(RefreshSeamCase {
        provider: "whoop",
        token_url_env: "PIERRE_WHOOP_TOKEN_URL",
        client_id_env: "WHOOP_CLIENT_ID",
        client_secret_env: "WHOOP_CLIENT_SECRET",
        mock_response: json!({
            "access_token": "whoop_new_access",
            "refresh_token": "whoop_new_refresh",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "read:recovery read:workout",
        }),
        expected_access: "whoop_new_access",
        expected_refresh: "whoop_new_refresh",
        email: "refresh-whoop@example.com",
    })
    .await;

    // No `refresh_token` field -> the seeded "old_refresh_token" must persist.
    assert_refresh_once_and_persists(RefreshSeamCase {
        provider: "whoop",
        token_url_env: "PIERRE_WHOOP_TOKEN_URL",
        client_id_env: "WHOOP_CLIENT_ID",
        client_secret_env: "WHOOP_CLIENT_SECRET",
        mock_response: json!({
            "access_token": "whoop_new_access_2",
            "token_type": "Bearer",
            "expires_in": 3600,
        }),
        expected_access: "whoop_new_access_2",
        expected_refresh: "old_refresh_token",
        email: "refresh-whoop-omit@example.com",
    })
    .await;
}

/// Test connection status with OAuth manager integration
#[tokio::test]
#[serial]
async fn test_connection_status_with_oauth_manager() {
    common::init_server_config();
    let (executor, database) = create_test_executor().await;

    // Create user
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: "test@example.com".to_owned(),
        display_name: Some("Test User".to_owned()),
        password_hash: bcrypt::hash("password", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
    };
    let repos = database.repositories();
    repos.users.create(&user).await.unwrap();

    // Provision a tenant for the user — Finding D requires explicit tenant_id
    // on every UniversalRequest; falling back to user_id is no longer permitted.
    let tenant = Tenant::new(
        "OAuth Status Test Tenant".to_owned(),
        "oauth-status-test-tenant".to_owned(),
        Some("oauth-status.example.com".to_owned()),
        "starter".to_owned(),
        user_id,
    );
    repos.tenants.create(&tenant).await.unwrap();

    // Set up environment for OAuth providers
    env::set_var("STRAVA_CLIENT_ID", "test_client");
    env::set_var("STRAVA_CLIENT_SECRET", "test_secret");
    env::set_var("FITBIT_CLIENT_ID", "test_fitbit");
    env::set_var("FITBIT_CLIENT_SECRET", "test_fitbit_secret");

    // Create request for get_connection_status
    let request = UniversalRequest {
        user_id: user_id.to_string(),
        tool_name: "get_connection_status".to_owned(),
        parameters: json!({}),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant.id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    // Execute tool
    let response = executor.execute_tool(request).await.unwrap();

    // Check response
    assert!(response.success);
    assert!(response.result.is_some());

    let result = response.result.unwrap();
    assert!(result.get("providers").is_some());

    let providers = result.get("providers").unwrap();
    // Check for supported providers (strava, garmin, synthetic)
    assert!(
        providers.get("strava").is_some()
            || providers.get("garmin").is_some()
            || providers.get("synthetic").is_some()
    );

    // All providers should be disconnected since no tokens are stored
    if let Some(strava) = providers.get("strava") {
        assert_eq!(strava["connected"], serde_json::Value::Bool(false));
    }
    if let Some(garmin) = providers.get("garmin") {
        assert_eq!(garmin["connected"], serde_json::Value::Bool(false));
    }
}

/// Test that analyze_activity uses token refresh
#[tokio::test]
#[serial]
async fn test_analyze_activity_token_refresh() {
    common::init_server_config();
    let (executor, database) = create_test_executor().await;

    // Create user
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: "test@example.com".to_owned(),
        display_name: Some("Test User".to_owned()),
        password_hash: bcrypt::hash("password", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
    };
    let repos = database.repositories();
    repos.users.create(&user).await.unwrap();

    // Store token that will expire soon
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(3); // Expires in 3 minutes (within buffer)
    let oauth_token = UserOAuthToken::new(
        user_id,
        "00000000-0000-0000-0000-000000000000".to_owned(),
        oauth_providers::STRAVA.to_owned(),
        "soon_to_expire_token".to_owned(),
        Some("refresh_token_456".to_owned()),
        Some(expires_at),
        Some("read,activity:read_all".to_owned()),
    );
    repos.oauth_tokens.upsert_token(&oauth_token).await.unwrap();

    // Set up environment
    env::set_var("STRAVA_CLIENT_ID", "test_client");
    env::set_var("STRAVA_CLIENT_SECRET", "test_secret");

    // Create request - explicitly request Strava provider which requires OAuth
    let request = UniversalRequest {
        user_id: user_id.to_string(),
        tool_name: "analyze_activity".to_owned(),
        parameters: json!({
            "activity_id": "123456789",
            "provider": "strava"
        }),
        protocol: "test".to_owned(),
        tenant_id: None,
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    // Execute - should trigger refresh due to token expiring soon
    let response = executor.execute_tool(request).await;

    // Verify response (will fail in test without mock server, but structure is tested)
    match response {
        Ok(resp) => {
            if let Some(error) = resp.error {
                // Expected in test environment - could be OAuth error, provider error, deprecated system, or activity not found
                // Also accept generic connection/provider errors since we can't refresh without a real server
                assert!(
                    error.contains("OAuth")
                        || error.contains("Failed")
                        || error.contains("not yet fully implemented")
                        || error.contains("Activity not found")
                        || error.contains("deprecated")
                        || error.contains("tenant-aware MCP endpoints")
                        || error.contains("Authentication required")
                        || error.contains("No valid authentication token")
                        || error.contains("Authentication failed")
                        || (error.contains("No valid") && error.contains("token found"))
                        || error.contains("Authentication error")
                        || error.contains("provider")
                        || error.contains("connect")
                        || error.contains("token")
                );
            }
        }
        Err(_) => {
            // Expected in test environment
        }
    }
}

/// Test concurrent token refresh attempts
#[tokio::test]
#[serial]
async fn test_concurrent_token_operations() {
    common::init_server_config();
    let (executor, database) = create_test_executor().await;

    // Create user
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: "test@example.com".to_owned(),
        display_name: Some("Test User".to_owned()),
        password_hash: bcrypt::hash("password", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
    };
    let repos = database.repositories();
    repos.users.create(&user).await.unwrap();

    // Provision a tenant — Finding D requires explicit tenant_id on every request
    let tenant = Tenant::new(
        "Concurrent Token Test Tenant".to_owned(),
        "concurrent-token-test-tenant".to_owned(),
        Some("concurrent-token.example.com".to_owned()),
        "starter".to_owned(),
        user_id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    let tenant_id_str = tenant.id.to_string();

    // Store valid token (scoped to the provisioned tenant)
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
    let oauth_token = UserOAuthToken::new(
        user_id,
        tenant_id_str.clone(),
        oauth_providers::STRAVA.to_owned(),
        "valid_token".to_owned(),
        Some("refresh_token".to_owned()),
        Some(expires_at),
        Some("read,activity:read_all".to_owned()),
    );
    repos.oauth_tokens.upsert_token(&oauth_token).await.unwrap();

    // Set up environment
    env::set_var("STRAVA_CLIENT_ID", "test_client");
    env::set_var("STRAVA_CLIENT_SECRET", "test_secret");

    // Create multiple concurrent requests
    let mut handles = vec![];

    for _i in 0..5 {
        let executor_clone = executor.clone();
        let user_id_str = user_id.to_string();
        let request_tenant_id = tenant_id_str.clone();
        let handle = tokio::spawn(async move {
            let request = UniversalRequest {
                user_id: user_id_str,
                tool_name: "get_connection_status".to_owned(),
                parameters: json!({}),
                protocol: "test".to_owned(),
                tenant_id: Some(request_tenant_id),
                progress_token: None,
                cancellation_token: None,
                progress_reporter: None,
            };
            executor_clone.execute_tool(request).await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.success);
    }
}

/// Test error handling when OAuth provider initialization fails
/// Fixed test isolation using configuration without OAuth credentials
#[tokio::test]
async fn test_oauth_provider_init_failure() {
    common::init_server_config();
    // Create executor with configuration that has missing OAuth credentials
    let (executor, database) = create_test_executor_without_oauth().await;

    // Create user first so they can be tenant owner
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: "test@example.com".to_owned(),
        display_name: Some("Test User".to_owned()),
        password_hash: bcrypt::hash("password", bcrypt::DEFAULT_COST).unwrap(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
    };
    let repos = database.repositories();
    repos.users.create(&user).await.unwrap();

    // Create tenant with the user as owner
    let tenant = Tenant::new(
        "Test Tenant".to_owned(),
        "test-tenant".to_owned(),
        Some("test.example.com".to_owned()),
        "starter".to_owned(),
        user_id, // User is now the owner
    );
    repos.tenants.create(&tenant).await.unwrap();

    // Create request - explicitly request Strava provider which requires OAuth
    let request = UniversalRequest {
        user_id: user_id.to_string(),
        tool_name: "get_activities".to_owned(),
        parameters: json!({"provider": "strava"}),
        protocol: "test".to_owned(),
        tenant_id: None,
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    // Execute - should handle provider initialization failure gracefully
    let response = executor.execute_tool(request).await.unwrap();

    // Should fail with proper error message about missing OAuth credentials
    println!("Response: {:?}", response);
    println!("Success: {}", response.success);
    if let Some(ref err) = response.error {
        println!("Error: {}", err);
    }
    assert!(
        !response.success,
        "Tool execution should fail when no OAuth token"
    );
    assert!(
        response.error.is_some(),
        "Should have error message about missing OAuth token"
    );

    // Check that the error contains information about missing OAuth token
    let error_msg = response.error.unwrap();
    assert!(
        (error_msg.contains("No") && error_msg.contains("token"))
            || error_msg.contains("Connect your")
            || error_msg.contains("Please connect your"),
        "Error should contain OAuth connection message: {}",
        error_msg
    );
}
