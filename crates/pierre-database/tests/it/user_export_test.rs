// ABOUTME: Holds the athlete data export against the live schema: every user-keyed table is read or withheld, no credential or binary column is read
// ABOUTME: And against rows: one athlete's export holds no other athlete's rows, nothing from a tenant they are not in, and never a secret

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET /api/user/export` reads [`USER_EXPORT_TABLES`] (carnet#1346). The list
//! is only worth what holds it to the schema, so the first half of this file
//! reads the catalog of whichever backend `DATABASE_URL` names; the second half
//! writes rows for two athletes through the repositories and reads them back
//! through the export.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use chrono::{NaiveDate, Utc};
use pierre_core::models::{ApiKey, ApiKeyTier, Tenant, TenantId, User};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{
    UserExportTable, CREDENTIAL_MARKERS, NOT_CREDENTIAL_COLUMNS, USER_EXPORT_EXCLUDED_TABLES,
    USER_EXPORT_TABLES,
};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use serde_json::{json, Value};
use uuid::Uuid;

/// A column as the catalog reports it.
struct CatalogColumn {
    name: String,
    declared_type: String,
}

/// Every table of the live schema with its columns.
async fn catalog(db: &Database) -> BTreeMap<String, Vec<CatalogColumn>> {
    const SQLITE: &str = "SELECT m.name AS tbl, c.name AS col, c.type AS ty \
        FROM sqlite_master m, pragma_table_info(m.name) c WHERE m.type = 'table'";
    #[cfg(feature = "postgresql")]
    const POSTGRES: &str =
        "SELECT CAST(c.relname AS TEXT) AS tbl, CAST(a.attname AS TEXT) AS col, \
        CAST(format_type(a.atttypid, a.atttypmod) AS TEXT) AS ty \
        FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
        WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind = 'r' \
          AND pg_catalog.pg_table_is_visible(c.oid)";
    use sqlx::Row;
    let rows = match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query(SQLITE)
            .fetch_all(sqlite.pool())
            .await
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("tbl"),
                    r.get::<String, _>("col"),
                    r.get::<String, _>("ty"),
                )
            })
            .collect::<Vec<_>>(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query(POSTGRES)
            .fetch_all(pg.pool())
            .await
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("tbl"),
                    r.get::<String, _>("col"),
                    r.get::<String, _>("ty"),
                )
            })
            .collect::<Vec<_>>(),
    };
    let mut tables: BTreeMap<String, Vec<CatalogColumn>> = BTreeMap::new();
    for (table, name, declared_type) in rows {
        tables.entry(table).or_default().push(CatalogColumn {
            name,
            declared_type: declared_type.to_ascii_lowercase(),
        });
    }
    tables
}

/// A column naming a person: `user_id`, or a role-prefixed `*_user_id`.
fn is_user_keyed(column: &str) -> bool {
    column == "user_id" || column.ends_with("_user_id")
}

#[tokio::test]
async fn every_user_keyed_table_is_exported_or_withheld_with_a_reason() {
    let db = create_test_db().await.unwrap();
    let tables = catalog(&db).await;
    let exported: BTreeSet<&str> = USER_EXPORT_TABLES.iter().map(|t| t.name).collect();
    let withheld: BTreeSet<&str> = USER_EXPORT_EXCLUDED_TABLES
        .iter()
        .map(|(t, _)| *t)
        .collect();

    assert!(
        exported.is_disjoint(&withheld),
        "a table is both exported and withheld: {:?}",
        exported.intersection(&withheld).collect::<Vec<_>>()
    );
    let unclassified: Vec<&String> = tables
        .iter()
        .filter(|(table, columns)| {
            columns.iter().any(|c| is_user_keyed(&c.name))
                && !exported.contains(table.as_str())
                && !withheld.contains(table.as_str())
        })
        .map(|(table, _)| table)
        .collect();
    assert!(
        unclassified.is_empty(),
        "these tables hold a user-keyed column and are neither in USER_EXPORT_TABLES nor in \
         USER_EXPORT_EXCLUDED_TABLES with a reason: {unclassified:?}"
    );
    let missing: Vec<&&str> = exported
        .iter()
        .filter(|t| !tables.contains_key(**t))
        .collect();
    assert!(
        missing.is_empty(),
        "exported tables absent from the schema: {missing:?}"
    );
    for (table, reason) in USER_EXPORT_EXCLUDED_TABLES {
        assert!(!reason.is_empty(), "{table} is withheld without a reason");
    }
}

#[tokio::test]
async fn no_credential_or_binary_column_is_read() {
    let db = create_test_db().await.unwrap();
    let tables = catalog(&db).await;
    let mut leaks = Vec::new();
    for table in USER_EXPORT_TABLES {
        let columns = &tables[table.name];
        let names: BTreeSet<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        for withheld in table.excluded {
            assert!(
                names.contains(withheld),
                "{} withholds {withheld}, which is not a column of it: the withholding does nothing",
                table.name
            );
        }
        for column in columns {
            if table.excluded.contains(&column.name.as_str()) {
                continue;
            }
            let looks_secret = CREDENTIAL_MARKERS.iter().any(|m| column.name.contains(m))
                && !NOT_CREDENTIAL_COLUMNS.contains(&column.name.as_str());
            let is_binary =
                column.declared_type.contains("blob") || column.declared_type.contains("bytea");
            if looks_secret || is_binary {
                leaks.push(format!(
                    "{}.{} ({})",
                    table.name, column.name, column.declared_type
                ));
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "these columns look like credentials or bytes and the export reads them: {leaks:?}"
    );
}

#[tokio::test]
async fn a_table_with_a_tenant_column_is_filtered_by_tenant() {
    let db = create_test_db().await.unwrap();
    let tables = catalog(&db).await;
    let mut unscoped = Vec::new();
    for table in USER_EXPORT_TABLES {
        let has_tenant = tables[table.name].iter().any(|c| c.name == "tenant_id");
        // The account row is the athlete's by its id; its tenant may be null.
        if has_tenant && table.tenant_column.is_none() && table.name != "users" {
            unscoped.push(table.name);
        }
        if let Some(column) = table.tenant_column {
            assert!(
                tables[table.name].iter().any(|c| c.name == column),
                "{} filters on {column}, which it does not have",
                table.name
            );
            assert!(
                table.sqlite_page_sql.contains(column) && table.postgres_page_sql.contains(column),
                "{} names a tenant column its statements do not use",
                table.name
            );
        }
    }
    assert!(
        unscoped.is_empty(),
        "tables with a tenant_id the export does not filter on: {unscoped:?}"
    );
}

/// The primary-key columns of every table of the live schema.
async fn primary_keys(db: &Database) -> BTreeMap<String, BTreeSet<String>> {
    const SQLITE: &str = "SELECT m.name AS tbl, c.name AS col \
        FROM sqlite_master m, pragma_table_info(m.name) c WHERE m.type = 'table' AND c.pk > 0";
    #[cfg(feature = "postgresql")]
    const POSTGRES: &str = "SELECT CAST(c.relname AS TEXT) AS tbl, CAST(a.attname AS TEXT) AS col \
        FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid = i.indrelid \
        JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) \
        WHERE i.indisprimary AND pg_catalog.pg_table_is_visible(c.oid)";
    use sqlx::Row;
    let rows = match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query(SQLITE)
            .fetch_all(sqlite.pool())
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<String, _>("tbl"), r.get::<String, _>("col")))
            .collect::<Vec<_>>(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query(POSTGRES)
            .fetch_all(pg.pool())
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<String, _>("tbl"), r.get::<String, _>("col")))
            .collect::<Vec<_>>(),
    };
    let mut keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (table, column) in rows {
        keys.entry(table).or_default().insert(column);
    }
    keys
}

#[tokio::test]
async fn every_table_pages_by_its_primary_key() {
    let db = create_test_db().await.unwrap();
    let keys = primary_keys(&db).await;
    for table in USER_EXPORT_TABLES {
        let named: BTreeSet<String> = table.key_columns.iter().map(|c| (*c).to_owned()).collect();
        assert_eq!(
            Some(&named),
            keys.get(table.name),
            "{} pages by a key that is not its primary key: a page could skip or repeat a row",
            table.name
        );
    }
}

/// An API key row for `user_id` named `name`.
fn api_key(user_id: Uuid, name: &str) -> ApiKey {
    ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: name.to_owned(),
        key_prefix: format!("pk_{}", &Uuid::new_v4().to_string()[..8]),
        key_hash: format!("hash-{}", Uuid::new_v4()),
        description: None,
        tier: ApiKeyTier::Starter,
        rate_limit_requests: 100,
        rate_limit_window_seconds: 3600,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn a_row_updated_during_an_export_is_neither_dropped_nor_repeated() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let ana = fresh_user(repos, "paging").await;
    own_tenant(repos, ana.id).await;
    for name in ["one", "two", "three", "four"] {
        repos.api_keys.create(&api_key(ana.id, name)).await.unwrap();
    }
    let table = USER_EXPORT_TABLES
        .iter()
        .find(|t| t.name == "api_keys")
        .unwrap();
    let one_at_a_time = UserExportTable {
        page_rows: 1,
        ..*table
    };

    let mut seen = Vec::new();
    let mut after = String::new();
    loop {
        let page = repos
            .user_export
            .export_page(&one_at_a_time, ana.id, &after)
            .await
            .unwrap();
        let Some(last_key) = page.last_key else {
            break;
        };
        seen.extend(
            page.rows
                .iter()
                .map(|r| r["id"].as_str().unwrap().to_owned()),
        );
        // Rewriting every row moves it on disk: an order by position, with an
        // offset, would then skip the rows that shifted under it.
        rewrite_api_keys(&db, ana.id).await;
        after = last_key;
    }

    assert_eq!(seen.len(), 4, "each row once: {seen:?}");
    assert_eq!(seen.iter().collect::<BTreeSet<_>>().len(), 4);
}

/// Update every API key `user_id` holds.
async fn rewrite_api_keys(db: &Database, user_id: Uuid) {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query("UPDATE api_keys SET description = 'rewritten' WHERE user_id = $1")
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query("UPDATE api_keys SET description = 'rewritten' WHERE user_id = $1")
                .bind(user_id)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

async fn fresh_user(repos: &RepositoryRegistry, label: &str) -> User {
    let user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        "argon2-hash-secret-do-not-export".to_owned(),
        Some(label.to_owned()),
    );
    repos.users.create(&user).await.unwrap();
    user
}

async fn own_tenant(repos: &RepositoryRegistry, owner: Uuid) -> TenantId {
    let tenant = Tenant::new(
        "Export Tenant".to_owned(),
        format!("export-tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        owner,
    );
    repos.tenants.create(&tenant).await.unwrap();
    tenant.id
}

/// Every row the export reads for `user_id`, by table.
async fn export(repos: &RepositoryRegistry, user_id: Uuid) -> BTreeMap<&'static str, Vec<Value>> {
    let mut out = BTreeMap::new();
    for table in USER_EXPORT_TABLES {
        let mut after = String::new();
        let mut rows = Vec::new();
        loop {
            let page = repos
                .user_export
                .export_page(table, user_id, &after)
                .await
                .unwrap();
            let done = i64::try_from(page.rows.len()).unwrap() < table.page_rows;
            rows.extend(page.rows);
            match page.last_key {
                Some(last_key) if !done => after = last_key,
                _ => break,
            }
        }
        if !rows.is_empty() {
            out.insert(table.name, rows);
        }
    }
    out
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

#[tokio::test]
async fn an_export_holds_the_athletes_rows_and_no_one_elses() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let ana = fresh_user(repos, "ana").await;
    let ben = fresh_user(repos, "ben").await;
    let ana_tenant = own_tenant(repos, ana.id).await;
    let ben_tenant = own_tenant(repos, ben.id).await;
    // A tenant Ana is not a member of: a row written under it is not hers to carry.
    let foreign_tenant = TenantId::generate();

    for (tenant, user, flow) in [
        (ana_tenant, ana.id, "flow-ana"),
        (foreign_tenant, ana.id, "flow-foreign-tenant"),
        (ben_tenant, ben.id, "flow-ben"),
    ] {
        repos
            .first_run_progress
            .record_first_run_day(tenant, user, flow, day(1), Utc::now())
            .await
            .unwrap();
    }
    repos
        .profiles
        .create_goal(ana.id, json!({"title": "ana-goal"}))
        .await
        .unwrap();
    repos
        .profiles
        .create_goal(ben.id, json!({"title": "ben-goal"}))
        .await
        .unwrap();

    let exported = export(repos, ana.id).await;
    let body = serde_json::to_string(&exported).unwrap();

    let users = &exported["users"];
    assert_eq!(users.len(), 1, "the account row, once: {users:?}");
    assert_eq!(users[0]["email"], json!(ana.email));

    let flows: Vec<&str> = exported["first_run_progress"]
        .iter()
        .map(|r| r["flow_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        flows,
        ["flow-ana"],
        "only the row in a tenant she belongs to"
    );

    assert!(
        body.contains("ana-goal"),
        "her own goal is exported: {body}"
    );
    for other in [
        "flow-ben",
        "flow-foreign-tenant",
        "ben-goal",
        ben.email.as_str(),
    ] {
        assert!(
            !body.contains(other),
            "{other} is not Ana's and is in her export"
        );
    }
    assert!(
        !body.contains(&ben.id.to_string()),
        "Ben's id is in Ana's export"
    );
}

#[tokio::test]
async fn an_export_never_carries_a_credential() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let ana = fresh_user(repos, "cred").await;
    own_tenant(repos, ana.id).await;
    let secret_hash = format!("hash-{}", Uuid::new_v4());
    repos
        .api_keys
        .create(&ApiKey {
            key_hash: secret_hash.clone(),
            ..api_key(ana.id, "my laptop")
        })
        .await
        .unwrap();

    let exported = export(repos, ana.id).await;
    let body = serde_json::to_string(&exported).unwrap();

    assert_eq!(
        exported["api_keys"][0]["name"],
        json!("my laptop"),
        "the key's name is hers"
    );
    assert!(
        !body.contains(&secret_hash),
        "the API key hash is in the export"
    );
    assert!(
        !body.contains("argon2-hash-secret-do-not-export"),
        "the password hash is in the export"
    );
    assert!(
        exported["users"][0].get("password_hash").is_none(),
        "password_hash is a column of the row"
    );
}
