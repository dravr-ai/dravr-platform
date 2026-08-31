// ABOUTME: Covers config override scopes (global/tenant/user), env pins, and the resolution order
// ABOUTME: Asserts concrete resolved values — a stub returning the tier default would fail every case

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Config override scopes and environment pins
//!
//! A config value resolves through five rungs, most specific first:
//!
//! ```text
//! per-user row -> per-tenant row -> environment pin -> system-wide row -> parameter default
//! ```
//!
//! Every assertion here pins a concrete number rather than checking that a
//! call succeeded: the failure mode these guard against is a lookup that
//! silently falls through to the compiled-in default, which is itself a
//! perfectly valid-looking value.
//!
//! The environment cases drive [`EnvConfigPins::capture_from`] with an
//! explicit variable map. Mutating the real process environment is not safe
//! from tests that share a process with other tests running in parallel.

use std::collections::HashMap;

use pierre_config::admin_definitions::{
    register_heart_rate_zones, register_usage_quotas, ParameterDefinition,
};
use pierre_config::admin_env::EnvConfigPins;
use pierre_config::admin_types::{ConfigDataType, ConfigScope};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::admin::postgres_manager::PostgresAdminConfigManager;
use pierre_mcp_server::config::admin::repository::SetOverrideParams;
use pierre_mcp_server::config::admin::{
    AdminConfigManager, AdminConfigRepository, AdminConfigService,
};
use uuid::Uuid;

const CATEGORY: &str = "usage_quotas";
const KEY: &str = "usage_quotas.daily_message_cap";
const ENV_VAR: &str = "QUOTA_DAILY_MESSAGE_CAP";

fn quota_definitions() -> HashMap<String, ParameterDefinition> {
    let mut defs = HashMap::new();
    register_usage_quotas(&mut defs);
    defs
}

/// Build the repository for whichever backend [`create_test_db`] opened.
///
/// `create_test_db` honours a `PostgreSQL` `DATABASE_URL`, so under
/// `ci-postgres` these assertions must run against
/// `PostgresAdminConfigManager`'s own SQL — the per-user upsert names a
/// partial unique index, and index inference is exactly the kind of thing a
/// `SQLite` stand-in would not exercise.
fn repository(db: &Database) -> Box<dyn AdminConfigRepository> {
    match db {
        Database::SQLite(sqlite) => Box::new(AdminConfigManager::new(sqlite.pool().clone())),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => Box::new(PostgresAdminConfigManager::new(pg.pool().clone())),
    }
}

/// Build the config service against the same backend.
async fn config_service(db: &Database) -> AdminConfigService {
    AdminConfigService::for_database(db)
        .await
        .expect("admin config service")
}

/// Count stored per-user rows for the parameter under test.
async fn count_user_rows(db: &Database, user: &str) -> i64 {
    match db {
        Database::SQLite(sqlite) => sqlx::query_scalar(
            "SELECT COUNT(*) FROM admin_config_overrides \
             WHERE category = $1 AND config_key = $2 AND user_id = $3",
        )
        .bind(CATEGORY)
        .bind(KEY)
        .bind(user)
        .fetch_one(sqlite.pool())
        .await
        .unwrap(),
        // `user_id` is a `uuid` column on PostgreSQL, so the text id is cast.
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => sqlx::query_scalar(
            "SELECT COUNT(*) FROM admin_config_overrides \
             WHERE category = $1 AND config_key = $2 AND user_id = $3::uuid",
        )
        .bind(CATEGORY)
        .bind(KEY)
        .bind(user)
        .fetch_one(pg.pool())
        .await
        .unwrap(),
    }
}

async fn seed_user(db: &Database, label: &str) -> String {
    let user = User::new(
        format!("{label}-{}@dravr.test", Uuid::new_v4()),
        "hash_not_verified_in_tests".to_owned(),
        Some(format!("Scope {label}")),
    );
    let id = user.id;
    db.repositories().users.create(&user).await.unwrap();
    id.to_string()
}

async fn seed_tenant(db: &Database, owner_user_id: &str) -> String {
    let tenant_id = TenantId::generate();
    let owner = Uuid::parse_str(owner_user_id).expect("seeded user id is a uuid");
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Scope Tenant {tenant_id}"),
        slug: format!("scope-tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: owner,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    db.repositories().tenants.create(&tenant).await.unwrap();
    tenant_id.to_string()
}

async fn write(repo: &dyn AdminConfigRepository, admin: &str, scope: ConfigScope<'_>, value: i64) {
    let json = serde_json::json!(value);
    repo.set_override(SetOverrideParams {
        category: CATEGORY,
        key: KEY,
        value: &json,
        data_type: ConfigDataType::Integer,
        admin_user_id: admin,
        scope,
        reason: Some("scope test"),
    })
    .await
    .unwrap_or_else(|e| panic!("write at {} scope must succeed: {e}", scope.label()));
}

// ---------------------------------------------------------------------------
// Environment pins
// ---------------------------------------------------------------------------

#[test]
fn env_pin_parses_each_declared_type() {
    let defs = quota_definitions();
    let vars: HashMap<&str, &str> = [
        (ENV_VAR, "500"),
        ("QUOTA_BURST_MULTIPLIER", "2.5"),
        // Underscore separators, so a token budget reads the way it does in Rust.
        ("QUOTA_WEEKLY_TOKEN_BUDGET", "20_000_000"),
    ]
    .into_iter()
    .collect();

    let (pins, errors) =
        EnvConfigPins::capture_from(&defs, |name| vars.get(name).map(|v| (*v).to_owned()));

    assert!(
        errors.is_empty(),
        "well-formed pins must not error: {errors:?}"
    );
    assert_eq!(
        pins.get(KEY).and_then(serde_json::Value::as_i64),
        Some(500),
        "an integer pin must land as a JSON integer, not a string"
    );
    assert_eq!(
        pins.get("usage_quotas.burst_multiplier")
            .and_then(serde_json::Value::as_f64),
        Some(2.5),
        "a float pin must keep its fractional part"
    );
    assert_eq!(
        pins.get("usage_quotas.weekly_token_budget")
            .and_then(serde_json::Value::as_i64),
        Some(20_000_000),
        "underscore digit separators must be accepted"
    );
}

#[test]
fn env_pin_rejects_unparseable_and_out_of_range_values() {
    let defs = quota_definitions();

    let (pins, errors) =
        EnvConfigPins::capture_from(&defs, |name| (name == ENV_VAR).then(|| "lots".to_owned()));
    assert!(
        pins.get(KEY).is_none(),
        "an unparseable pin must not install a value"
    );
    assert_eq!(
        errors.len(),
        1,
        "the rejection must be reported, not swallowed"
    );
    assert_eq!(errors[0].env_variable, ENV_VAR);
    assert!(
        errors[0].describe().contains("expected an integer"),
        "the error must say what was wrong: {}",
        errors[0].describe()
    );

    // 5..=1000 is the catalogued range for the daily message cap.
    let (pins, errors) =
        EnvConfigPins::capture_from(&defs, |name| (name == ENV_VAR).then(|| "9000".to_owned()));
    assert!(
        pins.get(KEY).is_none(),
        "an out-of-range pin must be refused, never clamped into range"
    );
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].message.contains("between"),
        "the range must be named in the error: {}",
        errors[0].message
    );

    let (_, errors) =
        EnvConfigPins::capture_from(&defs, |name| (name == ENV_VAR).then(String::new));
    assert_eq!(
        errors.len(),
        1,
        "an empty assignment is a mistake and must be reported, not read as unset"
    );
}

#[test]
fn boot_loader_owned_variables_are_not_read_by_the_config_layer() {
    // FITNESS_ZONE_* and friends are loaded into their own structs before a
    // database exists. A second reader here would produce two values for one
    // variable that silently disagree.
    let mut defs = HashMap::new();
    register_heart_rate_zones(&mut defs);

    let (pins, errors) = EnvConfigPins::capture_from(&defs, |name| {
        (name == "FITNESS_ZONE_RECOVERY_MAX").then(|| "55.0".to_owned())
    });

    assert!(errors.is_empty());
    assert!(
        pins.is_empty(),
        "a boot-loader-owned variable must stay invisible to the config layer"
    );
}

// ---------------------------------------------------------------------------
// Stored scopes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn each_scope_stores_its_own_row_and_the_narrowest_wins() {
    let db = create_test_db().await.unwrap();
    let admin = seed_user(&db, "admin").await;
    let tenant = seed_tenant(&db, &admin).await;
    let user = seed_user(&db, "member").await;
    let repo = repository(&db);

    write(repo.as_ref(), &admin, ConfigScope::Global, 60).await;
    write(repo.as_ref(), &admin, ConfigScope::Tenant(&tenant), 120).await;
    write(repo.as_ref(), &admin, ConfigScope::User(&user), 400).await;

    // All three coexist — a narrower write must not overwrite a broader row.
    for (scope, expected) in [
        (ConfigScope::Global, 60),
        (ConfigScope::Tenant(&tenant), 120),
        (ConfigScope::User(&user), 400),
    ] {
        let row = repo
            .get_override(CATEGORY, KEY, scope)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{} scope must have its own row", scope.label()));
        assert_eq!(
            row.config_value.as_i64(),
            Some(expected),
            "{} scope must read back its own value",
            scope.label()
        );
    }

    let user_row = repo
        .get_override(CATEGORY, KEY, ConfigScope::User(&user))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        user_row.user_id.as_deref(),
        Some(user.as_str()),
        "a per-user row must carry the subject user, not just the author"
    );
    assert_eq!(
        user_row.tenant_id, None,
        "a per-user row is not a tenant row"
    );
}

#[tokio::test]
async fn a_user_override_is_invisible_to_another_user() {
    let db = create_test_db().await.unwrap();
    let admin = seed_user(&db, "admin").await;
    let alice = seed_user(&db, "alice").await;
    let bob = seed_user(&db, "bob").await;
    let repo = repository(&db);

    write(repo.as_ref(), &admin, ConfigScope::Global, 50).await;
    write(repo.as_ref(), &admin, ConfigScope::User(&alice), 900).await;

    let bobs = repo
        .get_override(CATEGORY, KEY, ConfigScope::User(&bob))
        .await
        .unwrap();
    assert!(
        bobs.is_none(),
        "Alice's exemption must not appear on Bob's scope"
    );

    let listed = repo
        .get_overrides_at(ConfigScope::User(&bob))
        .await
        .unwrap();
    let leaked = listed
        .iter()
        .any(|o| o.user_id.as_deref() == Some(alice.as_str()));
    assert!(
        !leaked,
        "a scoped listing must not carry another user's override rows"
    );
}

#[tokio::test]
async fn a_tenant_override_is_invisible_to_another_tenant() {
    let db = create_test_db().await.unwrap();
    let admin = seed_user(&db, "admin").await;
    let tenant_a = seed_tenant(&db, &admin).await;
    let tenant_b = seed_tenant(&db, &admin).await;
    let repo = repository(&db);

    write(repo.as_ref(), &admin, ConfigScope::Global, 50).await;
    write(repo.as_ref(), &admin, ConfigScope::Tenant(&tenant_a), 750).await;

    let b_row = repo
        .get_override(CATEGORY, KEY, ConfigScope::Tenant(&tenant_b))
        .await
        .unwrap();
    assert!(
        b_row.is_none(),
        "tenant A's cap must not be readable as tenant B's own row"
    );

    let b_listing = repo
        .get_overrides_at(ConfigScope::Tenant(&tenant_b))
        .await
        .unwrap();
    let a_value_present = b_listing
        .iter()
        .filter(|o| o.config_key == KEY)
        .any(|o| o.config_value.as_i64() == Some(750));
    assert!(
        !a_value_present,
        "tenant B must see only its own rows and the system-wide ones"
    );
}

#[tokio::test]
async fn clearing_a_user_override_restores_the_broader_scope() {
    let db = create_test_db().await.unwrap();
    let admin = seed_user(&db, "admin").await;
    let user = seed_user(&db, "member").await;
    let repo = repository(&db);

    write(repo.as_ref(), &admin, ConfigScope::Global, 50).await;
    write(repo.as_ref(), &admin, ConfigScope::User(&user), 400).await;

    let removed = repo
        .delete_override(CATEGORY, KEY, ConfigScope::User(&user))
        .await
        .unwrap();
    assert!(removed, "the per-user row must be deletable");

    assert!(
        repo.get_override(CATEGORY, KEY, ConfigScope::User(&user))
            .await
            .unwrap()
            .is_none(),
        "the per-user row must be gone"
    );

    let global = repo
        .get_override(CATEGORY, KEY, ConfigScope::Global)
        .await
        .unwrap()
        .expect("clearing a narrower scope must leave the system-wide row untouched");
    assert_eq!(
        global.config_value.as_i64(),
        Some(50),
        "the system-wide value must survive a per-user reset"
    );
}

#[tokio::test]
async fn repeated_user_writes_upsert_rather_than_accumulate() {
    let db = create_test_db().await.unwrap();
    let admin = seed_user(&db, "admin").await;
    let user = seed_user(&db, "member").await;
    let repo = repository(&db);

    write(repo.as_ref(), &admin, ConfigScope::User(&user), 200).await;
    write(repo.as_ref(), &admin, ConfigScope::User(&user), 650).await;

    let count = count_user_rows(&db, &user).await;

    assert_eq!(
        count, 1,
        "a second per-user save must update in place — the partial unique index is the arbiter"
    );

    let row = repo
        .get_override(CATEGORY, KEY, ConfigScope::User(&user))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.config_value.as_i64(),
        Some(650),
        "the surviving row must hold the operator's latest value"
    );
}

// ---------------------------------------------------------------------------
// Enforcement
// ---------------------------------------------------------------------------

/// The point of the whole feature: a per-user override must change the number
/// the quota gate actually enforces, not merely be stored.
///
/// `UsageCounterService` is what `check_pre_chat_quotas_scoped` consults
/// before every chat turn, so asserting on its resolved `limit` is asserting
/// on what a user is allowed to send.
#[tokio::test]
async fn a_per_user_override_changes_the_enforced_daily_message_limit() {
    use pierre_core::models::UserTier;
    use pierre_services::usage_counter::UsageCounterService;

    let db = create_test_db().await.unwrap();
    let admin = seed_user(&db, "admin").await;
    let tenant = seed_tenant(&db, &admin).await;
    let lifted = seed_user(&db, "lifted").await;
    let ordinary = seed_user(&db, "ordinary").await;

    let service = config_service(&db).await;
    let repo = repository(&db);
    let repos = db.repositories();
    let counters = UsageCounterService::new(repos.usage_counters.as_ref(), &service);

    // Starter's compiled-in cap, before any override exists.
    let before = counters
        .check_limit_for_tier(&tenant, &lifted, "daily_messages", &UserTier::Starter)
        .await
        .unwrap();
    assert_eq!(
        before.limit, 50,
        "with no override the Starter tier default must be enforced"
    );

    write(repo.as_ref(), &admin, ConfigScope::User(&lifted), 400).await;

    let after = counters
        .check_limit_for_tier(&tenant, &lifted, "daily_messages", &UserTier::Starter)
        .await
        .unwrap();
    assert_eq!(
        after.limit, 400,
        "the per-user override must be the limit the quota gate enforces"
    );

    let untouched = counters
        .check_limit_for_tier(&tenant, &ordinary, "daily_messages", &UserTier::Starter)
        .await
        .unwrap();
    assert_eq!(
        untouched.limit, 50,
        "lifting one user's cap must leave every other user on the tier default"
    );
}
