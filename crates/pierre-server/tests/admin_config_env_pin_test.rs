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
//! The ladder is asserted twice, against a quota parameter and against a
//! monitoring one, because the rule is a property of the config layer and not
//! of `usage_quotas`: every `EnvSource::ConfigLayer` variable behaves the same
//! way. A rule that only held for the keys it was written against would be a
//! coincidence, not a design.
//!
//! Shadowing is also asserted to be *reported*, at both moments an operator is
//! watching: `update_config` names the outranked keys in its response, and the
//! boot path warns once per key. A pin that silently swallows a saved override
//! is the same class of failure as a variable nothing reads.
//!
//! This is deliberately the only test in its own binary. `cargo test` runs
//! integration test files as separate processes but the tests *within* one
//! file share a process, and `set_var` would race any sibling that builds an
//! `AdminConfigService` — construction is exactly when pins are captured.

use std::collections::HashMap;
use std::env;

use pierre_config::admin_types::{ConfigDataType, ConfigScope, UpdateConfigRequest};
use pierre_core::models::User;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::admin::postgres_manager::PostgresAdminConfigManager;
use pierre_mcp_server::config::admin::repository::SetOverrideParams;
use pierre_mcp_server::config::admin::{
    AdminConfigManager, AdminConfigRepository, AdminConfigService, UpdateConfigContext,
};
use pierre_runtime_context::ConfigLookupScope;
use uuid::Uuid;

const CATEGORY: &str = "usage_quotas";
const KEY: &str = "usage_quotas.daily_message_cap";
const ENV_VAR: &str = "QUOTA_DAILY_MESSAGE_CAP";

/// Second parameter, deliberately outside `usage_quotas`: the precedence rule
/// belongs to the config layer, so it must hold here identically.
const OTHER_CATEGORY: &str = "monitoring";
const OTHER_KEY: &str = "monitoring.latency_warn_ms";
const OTHER_ENV_VAR: &str = "MONITORING_LATENCY_WARN_MS";

/// Value pinned through the environment for this test.
const PINNED: i64 = 300;
/// Value written to the system-wide row, which the pin must outrank.
const GLOBAL: i64 = 75;
/// Value written to the per-user row, which must outrank the pin.
const PER_USER: i64 = 900;

/// The same three values for the monitoring parameter, inside its own
/// catalogued range (100..=5000).
const OTHER_PINNED: i64 = 1200;
const OTHER_GLOBAL: i64 = 250;
const OTHER_PER_USER: i64 = 4000;

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

/// Write one integer override at `scope`.
async fn write(
    repo: &dyn AdminConfigRepository,
    admin_id: &str,
    category: &str,
    key: &str,
    scope: ConfigScope<'_>,
    value: i64,
) {
    let json = serde_json::json!(value);
    repo.set_override(SetOverrideParams {
        category,
        key,
        value: &json,
        data_type: ConfigDataType::Integer,
        admin_user_id: admin_id,
        scope,
        reason: Some("env pin precedence test"),
    })
    .await
    .unwrap_or_else(|e| panic!("write {key} at {} scope: {e}", scope.label()));
}

async fn resolved(service: &AdminConfigService, key: &str, scope: ConfigLookupScope<'_>) -> i64 {
    service
        .get_override_value(key, scope)
        .await
        .unwrap()
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("{key} must resolve to an integer"))
}

/// One parameter's three values, so the ladder assertion takes a case rather
/// than a long positional argument list.
struct Case<'a> {
    category: &'a str,
    key: &'a str,
    /// What the environment pins it to.
    pinned: i64,
    /// What the system-wide row holds — outranked by the pin.
    global: i64,
    /// What the per-user row holds — outranks the pin.
    per_user: i64,
}

/// The whole ladder for one parameter. Run against a `usage_quotas` key and a
/// `monitoring` key so the rule is shown to belong to the config layer.
async fn assert_ladder(
    service: &AdminConfigService,
    repo: &dyn AdminConfigRepository,
    admin_id: &str,
    member_id: &str,
    case: &Case<'_>,
) {
    let Case {
        category,
        key,
        pinned,
        global,
        per_user,
    } = *case;
    // 1. Pin alone, no stored row anywhere.
    assert_eq!(
        resolved(service, key, ConfigLookupScope::global()).await,
        pinned,
        "{key}: with no stored row the environment pin must supply the value"
    );

    // 2. A system-wide row exists — the pin still wins.
    write(repo, admin_id, category, key, ConfigScope::Global, global).await;
    assert_eq!(
        resolved(service, key, ConfigLookupScope::global()).await,
        pinned,
        "{key}: a fleet-wide pin must outrank a runtime admin edit at the same scope"
    );

    // 3. A per-user row is narrower than the fleet — it wins.
    write(
        repo,
        admin_id,
        category,
        key,
        ConfigScope::User(member_id),
        per_user,
    )
    .await;
    assert_eq!(
        resolved(
            service,
            key,
            ConfigLookupScope::user(member_id, "tenant-unused")
        )
        .await,
        per_user,
        "{key}: a per-user exemption must beat the environment pin"
    );

    // 4. Another user still sees the pin, not the exemption.
    let other = Uuid::new_v4().to_string();
    assert_eq!(
        resolved(
            service,
            key,
            ConfigLookupScope::user(&other, "tenant-unused")
        )
        .await,
        pinned,
        "{key}: one user's exemption must not raise everyone else's value"
    );
}

#[tokio::test]
async fn env_pin_outranks_the_global_row_but_yields_to_a_user_override() {
    env::set_var(ENV_VAR, PINNED.to_string());
    env::set_var(OTHER_ENV_VAR, OTHER_PINNED.to_string());

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

    // The same ladder, twice: a quota parameter and a monitoring one. If the
    // second diverged, the precedence rule would be a property of the keys it
    // was written against rather than of the config layer.
    for case in [
        Case {
            category: CATEGORY,
            key: KEY,
            pinned: PINNED,
            global: GLOBAL,
            per_user: PER_USER,
        },
        Case {
            category: OTHER_CATEGORY,
            key: OTHER_KEY,
            pinned: OTHER_PINNED,
            global: OTHER_GLOBAL,
            per_user: OTHER_PER_USER,
        },
    ] {
        assert_ladder(&service, repo.as_ref(), &admin_id, &member_id, &case).await;
    }

    // A global write of a pinned key is saved but outranked, and says so.
    let mut parameters = HashMap::new();
    parameters.insert(KEY.to_owned(), serde_json::json!(90));
    parameters.insert(OTHER_KEY.to_owned(), serde_json::json!(400));
    let response = service
        .update_config(
            &UpdateConfigRequest {
                parameters,
                reason: Some("shadow report".to_owned()),
            },
            UpdateConfigContext {
                admin_user_id: &admin_id,
                admin_email: "env-pin-admin@dravr.test",
                scope: ConfigScope::Global,
                ip_address: None,
                user_agent: None,
            },
        )
        .await
        .expect("global update must succeed even when a pin outranks it");

    assert!(response.success, "the row is still written");
    let mut reported = response.shadowed_by_env.clone();
    reported.sort();
    let mut expected = vec![KEY.to_owned(), OTHER_KEY.to_owned()];
    expected.sort();
    assert_eq!(
        reported, expected,
        "both pinned keys must be reported as outranked, not silently stored"
    );

    // The same write scoped to a user is NOT shadowed — it beats the pin.
    let mut parameters = HashMap::new();
    parameters.insert(KEY.to_owned(), serde_json::json!(120));
    let response = service
        .update_config(
            &UpdateConfigRequest {
                parameters,
                reason: Some("user scope is narrower than the fleet".to_owned()),
            },
            UpdateConfigContext {
                admin_user_id: &admin_id,
                admin_email: "env-pin-admin@dravr.test",
                scope: ConfigScope::User(&member_id),
                ip_address: None,
                user_agent: None,
            },
        )
        .await
        .expect("per-user update must succeed");
    assert!(
        response.shadowed_by_env.is_empty(),
        "a per-user override outranks the pin, so nothing is shadowed"
    );
    assert_eq!(
        resolved(
            &service,
            KEY,
            ConfigLookupScope::user(&member_id, "tenant-unused")
        )
        .await,
        120,
        "the per-user write must be the value that applies"
    );

    env::remove_var(ENV_VAR);
    env::remove_var(OTHER_ENV_VAR);
}
