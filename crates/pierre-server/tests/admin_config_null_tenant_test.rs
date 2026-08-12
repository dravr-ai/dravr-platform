// ABOUTME: Regression test — system-wide admin config saves must upsert in place, never append
// ABOUTME: Runs on whichever backend DATABASE_URL names, so PostgreSQL's own upsert is exercised
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # System-wide config overrides must not accumulate duplicate rows
//!
//! A system-wide override is stored with `tenant_id` set to `NULL`. Both
//! `SQLite` and `PostgreSQL` treat `NULL` as distinct from every other `NULL`
//! inside a `UNIQUE` constraint, so `UNIQUE(category, config_key, tenant_id)`
//! never arbitrated a tenant-less write. The `ON CONFLICT` clause naming those
//! three columns therefore never fired, and every save through
//! `PUT /api/admin/config` without a `tenant_id` appended another row.
//!
//! The read side compounded it: `get_override` issues an unordered
//! `fetch_optional`, which in practice returns the oldest surviving row. The
//! effective value stayed pinned to the first-ever write while the admin UI
//! reported each later save as successful.
//!
//! The fix pairs a partial unique index over the tenant-less rows with an
//! `ON CONFLICT (category, config_key) WHERE tenant_id IS NULL` conflict
//! target. This file fails if either half is missing: without the index the
//! conflict target matches no arbiter and the statement errors, and without the
//! new conflict target the second save either appends a second row (before the
//! migration) or violates the index (after it).
//!
//! [`create_test_db`] honours a `PostgreSQL` `DATABASE_URL`, so under
//! `ci-postgres` these assertions run against `PostgresAdminConfigManager`'s own
//! SQL rather than a `SQLite` stand-in.

use pierre_config::admin_types::ConfigDataType;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::admin::postgres_manager::PostgresAdminConfigManager;
use pierre_mcp_server::config::admin::repository::SetOverrideParams;
use pierre_mcp_server::config::admin::{AdminConfigManager, AdminConfigRepository};
use uuid::Uuid;

/// Category of a real catalogued parameter, so the row under test is shaped
/// exactly like one the admin console writes.
const CATEGORY: &str = "group_permissions";
/// Key of that same catalogued parameter.
const KEY: &str = "group_creation_policy";

/// Build the repository for whichever backend [`create_test_db`] opened.
fn repository(db: &Database) -> Box<dyn AdminConfigRepository> {
    #[cfg(feature = "postgresql")]
    if let Some(pg) = db.postgres_pool() {
        return Box::new(PostgresAdminConfigManager::new(pg.clone()));
    }

    Box::new(AdminConfigManager::new(
        db.sqlite_pool()
            .expect("test database exposes neither a PostgreSQL nor a SQLite pool")
            .clone(),
    ))
}

/// Persist a real user: `admin_config_overrides.created_by` carries a foreign
/// key to `users(id)` that `PostgreSQL` enforces.
async fn seed_admin(db: &Database) -> String {
    let user = User::new(
        format!("config-admin-{}@dravr.test", Uuid::new_v4()),
        "hash_not_verified_in_tests".to_owned(),
        Some("Config Admin".to_owned()),
    );
    let id = user.id;
    db.repositories().users.create(&user).await.unwrap();
    id.to_string()
}

/// Create a real tenant row and return its id.
///
/// `admin_config_overrides.tenant_id` carries a foreign key, so a scoped
/// override cannot reference an invented uuid — it fails with `SQLite` error 787
/// rather than exercising the upsert this test is about.
async fn seed_tenant(db: &Database, owner_user_id: &str) -> String {
    let tenant_id = TenantId::generate();
    let owner = Uuid::parse_str(owner_user_id).expect("seeded admin id is a uuid");
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Config Tenant {tenant_id}"),
        slug: format!("config-tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: owner,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    db.repositories().tenants.create(&tenant).await.unwrap();
    tenant_id.to_string()
}

/// Count stored rows for the parameter under test, scoped to `tenant` — or to
/// the system-wide `NULL` scope when `tenant` is `None`.
async fn count_rows(db: &Database, tenant: Option<&str>) -> i64 {
    #[cfg(feature = "postgresql")]
    if let Some(pg) = db.postgres_pool() {
        let sql = if tenant.is_some() {
            "SELECT COUNT(*) FROM admin_config_overrides \
             WHERE category = $1 AND config_key = $2 AND tenant_id = $3"
        } else {
            "SELECT COUNT(*) FROM admin_config_overrides \
             WHERE category = $1 AND config_key = $2 AND tenant_id IS NULL"
        };
        let mut q = sqlx::query_scalar(sql).bind(CATEGORY).bind(KEY);
        if let Some(t) = tenant {
            q = q.bind(t);
        }
        return q.fetch_one(pg).await.unwrap();
    }

    let sql = if tenant.is_some() {
        "SELECT COUNT(*) FROM admin_config_overrides \
         WHERE category = ?1 AND config_key = ?2 AND tenant_id = ?3"
    } else {
        "SELECT COUNT(*) FROM admin_config_overrides \
         WHERE category = ?1 AND config_key = ?2 AND tenant_id IS NULL"
    };
    let mut q = sqlx::query_scalar(sql).bind(CATEGORY).bind(KEY);
    if let Some(t) = tenant {
        q = q.bind(t);
    }
    let pool = db
        .sqlite_pool()
        .expect("test database exposes neither a PostgreSQL nor a SQLite pool");
    q.fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn system_wide_save_updates_in_place_instead_of_appending_a_row() {
    let db = create_test_db().await.unwrap();
    let admin = seed_admin(&db).await;
    let repo = repository(&db);

    let everyone = serde_json::json!("everyone");
    repo.set_override(SetOverrideParams {
        category: CATEGORY,
        key: KEY,
        value: &everyone,
        data_type: ConfigDataType::Enum,
        admin_user_id: &admin,
        tenant_id: None,
        reason: Some("open group creation to every member"),
    })
    .await
    .expect("first system-wide save must succeed");

    let admins_only = serde_json::json!("admins_only");
    let returned = repo
        .set_override(SetOverrideParams {
            category: CATEGORY,
            key: KEY,
            value: &admins_only,
            data_type: ConfigDataType::Enum,
            admin_user_id: &admin,
            tenant_id: None,
            reason: Some("lock group creation down to admins"),
        })
        .await
        .expect("second system-wide save must upsert, not collide or append");

    assert_eq!(
        returned.config_value, admins_only,
        "set_override must hand back the value just written, not a stale row"
    );
    assert_eq!(
        returned.tenant_id, None,
        "a system-wide override must stay tenant-less"
    );

    let read_back = repo
        .get_override(CATEGORY, KEY, None)
        .await
        .unwrap()
        .expect("the system-wide override must exist after two saves");
    assert_eq!(
        read_back.config_value, admins_only,
        "the read must return the operator's latest value, not the first one written"
    );

    let effective = repo
        .get_effective_override(CATEGORY, KEY, None)
        .await
        .unwrap()
        .expect("the effective system-wide override must exist");
    assert_eq!(
        effective.config_value, admins_only,
        "the effective value backs enforcement and must track the latest save"
    );

    assert_eq!(
        count_rows(&db, None).await,
        1,
        "two system-wide saves of one parameter must leave exactly one row"
    );
}

#[tokio::test]
async fn tenant_override_still_upserts_and_leaves_the_system_wide_row_alone() {
    let db = create_test_db().await.unwrap();
    let admin = seed_admin(&db).await;
    let repo = repository(&db);
    let tenant = seed_tenant(&db, &admin).await;

    let everyone = serde_json::json!("everyone");
    repo.set_override(SetOverrideParams {
        category: CATEGORY,
        key: KEY,
        value: &everyone,
        data_type: ConfigDataType::Enum,
        admin_user_id: &admin,
        tenant_id: None,
        reason: Some("system-wide default"),
    })
    .await
    .expect("system-wide save must succeed");

    let admins_only = serde_json::json!("admins_only");
    for reason in ["tenant opts into lockdown", "tenant reconfirms lockdown"] {
        repo.set_override(SetOverrideParams {
            category: CATEGORY,
            key: KEY,
            value: &admins_only,
            data_type: ConfigDataType::Enum,
            admin_user_id: &admin,
            tenant_id: Some(&tenant),
            reason: Some(reason),
        })
        .await
        .expect("tenant-scoped save must upsert against the table's UNIQUE constraint");
    }

    assert_eq!(
        count_rows(&db, Some(&tenant)).await,
        1,
        "two tenant-scoped saves must leave exactly one tenant row"
    );
    assert_eq!(
        count_rows(&db, None).await,
        1,
        "tenant-scoped saves must not add or duplicate the system-wide row"
    );

    let system_wide = repo
        .get_override(CATEGORY, KEY, None)
        .await
        .unwrap()
        .expect("the system-wide override must survive tenant writes");
    assert_eq!(
        system_wide.config_value, everyone,
        "a tenant override must not overwrite the system-wide value"
    );

    let effective = repo
        .get_effective_override(CATEGORY, KEY, Some(&tenant))
        .await
        .unwrap()
        .expect("the tenant must resolve an effective override");
    assert_eq!(
        effective.config_value, admins_only,
        "a tenant override must win over the system-wide value for that tenant"
    );
}
