// ABOUTME: Bench-only ServerContext fixture — in-memory SQLite, seeded users/tenants, user tool overrides.
// ABOUTME: Replicates the minimal recipe from tests/common.rs (unreachable from bench targets).
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Builds the in-process dispatch surface the tool-dispatch benches measure:
//! a real [`ServerContext`] over `sqlite::memory:` (auto-migrated, tool
//! catalog seeded), plus users/tenants persisted through the production
//! repositories and per-user tool overrides written via
//! `pierre_services::admin_ops` — the same write path the admin API uses.
//!
//! Bench users are `UserTier::Enterprise`: Starter's daily tool-call quota
//! (200) would trip within the first second of a criterion loop and silently
//! turn every later iteration into a quota-error measurement. Enterprise
//! keeps the quota-check reads on the measured path without ever tripping.

#![allow(
    clippy::missing_docs_in_private_items,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::unwrap_used,
    missing_docs
)]

use std::env;
use std::path::Path;
use std::sync::{Arc, Once};

use chrono::Utc;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_cache::{Cache, CacheConfig};
#[cfg(feature = "postgresql")]
use pierre_config::environment::PostgresPoolConfig;
use pierre_config::environment::{HttpClientConfig, ServerConfig};
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_mcp_server::constants;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_mcp_server::utils;
use pierre_services::admin_ops;
use uuid::Uuid;

static INIT: Once = Once::new();

/// Process-global init the [`ServerContext`] constructor expects (mirrors
/// `tests/common.rs`): CI mode, provider OAuth env, command-loader path,
/// HTTP clients, and the global server config.
pub fn init_bench_env() {
    INIT.call_once(|| {
        env::set_var("CI", "true");
        env::set_var("DATABASE_URL", "sqlite::memory:");

        let commands_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("commands"));
        if let Some(path) = commands_path {
            env::set_var("PIERRE_COMMANDS_DIR", path);
        }

        env::set_var("PIERRE_STRAVA_CLIENT_ID", "bench_strava_client_id");
        env::set_var("PIERRE_STRAVA_CLIENT_SECRET", "bench_strava_client_secret");
        env::set_var("PIERRE_GARMIN_CLIENT_ID", "bench_garmin_client_id");
        env::set_var("PIERRE_GARMIN_CLIENT_SECRET", "bench_garmin_client_secret");
        env::set_var("PIERRE_FITBIT_CLIENT_ID", "bench_fitbit_client_id");
        env::set_var("PIERRE_FITBIT_CLIENT_SECRET", "bench_fitbit_client_secret");

        if env::var("PIERRE_LLM_MODEL").is_err() {
            env::set_var("PIERRE_LLM_MODEL", "mock-model");
        }

        utils::http_client::initialize_http_clients(HttpClientConfig::default());
        let _ = constants::init_server_config();
    });
}

/// Full in-process server resources over in-memory `SQLite`.
///
/// `Database::new` auto-migrates and seeds the tool catalog; the 2048-bit
/// RSA key is generated once per process (benches build the context once,
/// outside the timed loop).
pub async fn build_server_context() -> Arc<ServerContext> {
    init_bench_env();

    let encryption_key = generate_encryption_key().to_vec();

    #[cfg(feature = "postgresql")]
    let database = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await
    .unwrap();

    #[cfg(not(feature = "postgresql"))]
    let database = Database::new("sqlite::memory:", encryption_key)
        .await
        .unwrap();

    let auth_manager = AuthManager::new(24);
    let config = Arc::new(ServerConfig {
        activity_fetch_limit: 100,
        ..ServerConfig::default()
    });

    let cache_config = CacheConfig {
        max_entries: 1000,
        redis_url: None,
        enable_background_cleanup: false,
        ..Default::default()
    };
    let cache = Cache::new(cache_config).await.unwrap();

    let mut jwks = JwksManager::new();
    jwks.generate_rsa_key_pair_with_size("bench_key", 2048)
        .unwrap();

    Arc::new(
        ServerContext::new(
            database,
            auth_manager,
            "bench_admin_secret",
            config,
            cache,
            ServerContextOptions::testing().with_jwks_manager(Arc::new(jwks)),
        )
        .await,
    )
}

/// Persist an active Enterprise-tier user owning a fresh starter-plan tenant
/// (tenant plan governs tool visibility; user tier governs call quotas).
///
/// Skips bcrypt (the bench never logs in via password) — the stored hash is
/// an inert placeholder.
pub async fn seed_user(resources: &ServerContext, email: &str) -> (User, TenantId) {
    let mut user = User::new(
        email.to_owned(),
        "bench-not-a-real-hash".to_owned(),
        Some("Bench User".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    user.tier = UserTier::Enterprise;

    let repos = resources.coach.database.repositories();
    repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::new();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Bench Tenant for {email}"),
        slug: format!("bench-tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user.id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    repos.tenants.create(&tenant).await.unwrap();
    repos
        .users
        .update_tenant_id(user.id, tenant_id)
        .await
        .unwrap();

    (user, tenant_id)
}

/// Disable the given catalogued tools for one user through the production
/// admin write path, exercising the `UserOverride` rung on later dispatches.
/// Unknown tool names fail loudly here (catalog-validated), not mid-bench.
pub async fn seed_user_overrides(resources: &ServerContext, user_id: Uuid, tools: &[&str]) {
    let repos = resources.coach.database.repositories();
    for tool_name in tools {
        admin_ops::set_user_tool_override(
            &repos,
            user_id,
            tool_name,
            false,
            None,
            Some("bench fixture".to_owned()),
        )
        .await
        .unwrap();
    }
}
