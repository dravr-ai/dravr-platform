// ABOUTME: The full resolution ladder with an environment pin in play: user > tenant > env > global
// ABOUTME: One test in its own binary — it mutates process env, which is not safe to share

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # An environment pin beats the system-wide row, and loses to a narrower one
//!
//! `QUOTA_DAILY_MESSAGE_CAP` and its siblings were advertised by the config
//! catalogue for months while nothing read them: setting one in `.envrc`
//! changed nothing, silently. This file is the guard on that being fixed, and
//! on the precedence the fix chose.
//!
//! A pin is a deploy-time, fleet-wide decision, so it outranks a runtime
//! admin edit at the same (system-wide) scope — the posture `GUARDIAN_*`
//! already takes over the persisted guardian document. A tenant or per-user
//! override is narrower than the fleet and still wins.
//!
//! This is deliberately the only test in its own binary. `cargo test` runs
//! integration test files as separate processes but the tests *within* one
//! file share a process, and `set_var` would race any sibling that builds an
//! `AdminConfigService` — construction is exactly when pins are captured.

use std::env;

use pierre_config::admin_types::{ConfigDataType, ConfigScope};
use pierre_core::models::User;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::admin::postgres_manager::PostgresAdminConfigManager;
use pierre_mcp_server::config::admin::repository::SetOverrideParams;
use pierre_mcp_server::config::admin::{
    AdminConfigManager, AdminConfigRepository, AdminConfigService,
};
use pierre_runtime_context::ConfigLookupScope;
use uuid::Uuid;

const CATEGORY: &str = "usage_quotas";
const KEY: &str = "usage_quotas.daily_message_cap";
const ENV_VAR: &str = "QUOTA_DAILY_MESSAGE_CAP";

/// Value pinned through the environment for this test.
const PINNED: i64 = 300;
/// Value written to the system-wide row, which the pin must outrank.
const GLOBAL: i64 = 75;
/// Value written to the per-user row, which must outrank the pin.
const PER_USER: i64 = 900;

/// Build the service and repository against whichever backend
/// [`create_test_db`] opened.
async fn backend(db: &Database) -> (AdminConfigService, Box<dyn AdminConfigRepository>) {
    #[cfg(feature = "postgresql")]
    if let Some(pg) = db.postgres_pool() {
        let service = AdminConfigService::from_postgres(pg.clone())
            .await
            .expect("PostgreSQL admin config service");
        return (
            service,
            Box::new(PostgresAdminConfigManager::new(pg.clone())),
        );
    }

    let pool = db
        .sqlite_pool()
        .expect("test database exposes neither a PostgreSQL nor a SQLite pool")
        .clone();
    let service = AdminConfigService::new(pool.clone())
        .await
        .expect("SQLite admin config service");
    (service, Box::new(AdminConfigManager::new(pool)))
}

#[tokio::test]
async fn env_pin_outranks_the_global_row_but_yields_to_a_user_override() {
    env::set_var(ENV_VAR, PINNED.to_string());

    let db = create_test_db().await.unwrap();

    let admin = User::new(
        format!("env-pin-admin-{}@dravr.test", Uuid::new_v4()),
        "hash_not_verified_in_tests".to_owned(),
        Some("Env Pin Admin".to_owned()),
    );
    let admin_id = admin.id.to_string();
    db.repositories().users.create(&admin).await.unwrap();

    let member = User::new(
        format!("env-pin-member-{}@dravr.test", Uuid::new_v4()),
        "hash_not_verified_in_tests".to_owned(),
        Some("Env Pin Member".to_owned()),
    );
    let member_id = member.id.to_string();
    db.repositories().users.create(&member).await.unwrap();

    // Constructed after set_var: pins are captured once, at construction.
    // Both halves follow whichever backend `create_test_db` opened, so the
    // precedence walk is exercised against PostgreSQL under ci-postgres too.
    let (service, repo) = backend(&db).await;

    // 1. Pin alone, no stored row anywhere.
    let resolved = service
        .get_override_value(KEY, ConfigLookupScope::global())
        .await
        .unwrap()
        .and_then(|v| v.as_i64());
    assert_eq!(
        resolved,
        Some(PINNED),
        "with no stored row, the environment pin must supply the value"
    );

    // 2. A system-wide row exists — the pin still wins.
    let global_json = serde_json::json!(GLOBAL);
    repo.set_override(SetOverrideParams {
        category: CATEGORY,
        key: KEY,
        value: &global_json,
        data_type: ConfigDataType::Integer,
        admin_user_id: &admin_id,
        scope: ConfigScope::Global,
        reason: Some("runtime admin edit"),
    })
    .await
    .unwrap();

    let resolved = service
        .get_override_value(KEY, ConfigLookupScope::global())
        .await
        .unwrap()
        .and_then(|v| v.as_i64());
    assert_eq!(
        resolved,
        Some(PINNED),
        "a fleet-wide pin must outrank a runtime admin edit at the same scope"
    );

    // 3. A per-user row is narrower than the fleet — it wins.
    let user_json = serde_json::json!(PER_USER);
    repo.set_override(SetOverrideParams {
        category: CATEGORY,
        key: KEY,
        value: &user_json,
        data_type: ConfigDataType::Integer,
        admin_user_id: &admin_id,
        scope: ConfigScope::User(&member_id),
        reason: Some("comp account"),
    })
    .await
    .unwrap();

    let resolved = service
        .get_override_value(KEY, ConfigLookupScope::user(&member_id, "tenant-unused"))
        .await
        .unwrap()
        .and_then(|v| v.as_i64());
    assert_eq!(
        resolved,
        Some(PER_USER),
        "a per-user exemption must beat the environment pin"
    );

    // 4. Another user still sees the pin, not the exemption.
    let other = Uuid::new_v4().to_string();
    let resolved = service
        .get_override_value(KEY, ConfigLookupScope::user(&other, "tenant-unused"))
        .await
        .unwrap()
        .and_then(|v| v.as_i64());
    assert_eq!(
        resolved,
        Some(PINNED),
        "one user's exemption must not raise everyone else's cap"
    );

    env::remove_var(ENV_VAR);
}
