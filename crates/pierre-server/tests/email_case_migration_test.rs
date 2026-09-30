// ABOUTME: The email-case migration: every stored email lowercased and trimmed, case variants refused from then on
// ABOUTME: Replays each lane's migrations up to it, plants legacy rows, and proves a collision fails it by name
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Emails are case-insensitive across the product, and 20260928163101 brings
//! the rows stored before that to the one form the server now writes and
//! compares: trimmed and lowercase. It is proven on a real database of each
//! lane rather than trusted from the SQL, because the refusal it carries is
//! the part that matters: when lowercasing would put two accounts (or two
//! pre-approvals) on one address, the migration must fail and name every
//! address involved, merging and dropping nothing, so an operator can merge
//! them by hand and migrate again.
//!
//! Each lane has its own migration set, so the factory's URL decides which
//! one is replayed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::fmt::Debug;
use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
#[cfg(feature = "postgresql")]
use uuid::Uuid;

/// The migration under test; everything before it builds the legacy shape.
const EMAIL_CASE_MIGRATION: i64 = 20_260_928_163_101;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// Every planted account: its legacy spelling and the stored form it must
/// read as afterwards.
const ACCOUNTS: [(&str, &str); 2] = [
    ("Jane@Case.Test", "jane@case.test"),
    ("  Bob@CASE.test ", "bob@case.test"),
];

#[tokio::test]
async fn every_stored_email_is_lowercased_and_a_case_variant_is_refused() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        lowercases_on_postgres(&database.url).await;
        return;
    }
    lowercases_on_sqlite(&database.url).await;
}

#[tokio::test]
async fn a_case_collision_fails_the_migration_and_names_every_address() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        collision_on_postgres(&database.url).await;
        return;
    }
    collision_on_sqlite(&database.url).await;
}

/// The error of applying `sql`, which must fail.
fn refusal<T: Debug>(result: Result<T, sqlx::Error>) -> String {
    match result {
        Ok(done) => panic!("the migration ran over a collision: {done:?}"),
        Err(e) => e.to_string(),
    }
}

/// Assert `refused` reports a case collision in `column` naming every address
/// in `named` and none in `unnamed`.
fn assert_names(refused: &str, column: &str, named: &[&str], unnamed: &[&str]) {
    assert!(
        refused.contains(&format!("{column} case collision")),
        "{refused}"
    );
    for address in named {
        assert!(refused.contains(address), "names {address}: {refused}");
    }
    for address in unnamed {
        assert!(
            !refused.contains(address),
            "names only the colliding addresses, not {address}: {refused}"
        );
    }
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. Foreign keys off: the
/// planted rows name owners the test does not build.
async fn sqlite_before_migration(url: &str) -> (sqlx::SqlitePool, String) {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for migration in MIGRATOR.iter() {
        if migration.version == EMAIL_CASE_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the email-case migration is in migrations/");
}

async fn plant_sqlite_user(pool: &sqlx::SqlitePool, id: &str, email: &str) {
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, last_active)
         VALUES ($1, $2, 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .bind(email)
    .execute(pool)
    .await
    .unwrap();
}

async fn plant_sqlite_allow(pool: &sqlx::SqlitePool, email: &str) {
    sqlx::query(
        "INSERT INTO pre_approved_emails (email, created_at) VALUES ($1, '2026-01-01T00:00:00Z')",
    )
    .bind(email)
    .execute(pool)
    .await
    .unwrap();
}

async fn sqlite_texts(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
    sqlx::query(sql)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>(0))
        .collect()
}

async fn lowercases_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;

    for (i, (legacy, _)) in ACCOUNTS.iter().enumerate() {
        plant_sqlite_user(&pool, &format!("u-{i}"), legacy).await;
    }
    plant_sqlite_allow(&pool, "Allowed@Case.Test").await;
    sqlx::raw_sql(
        "INSERT INTO messaging_link_states (id, tenant_id, channel_type, code, method, expires_at, email)
         VALUES ('ls-1', 't-1', 'whatsapp', 'code-1', 'otp', '2026-01-01T00:10:00Z', ' Otp@Case.Test');
         INSERT INTO delegated_connections (id, provider, group_id, coach_user_id, coach_tenant_id,
             member_user_id, provider_athlete_id, provider_athlete_email, status, proposed_at)
         VALUES ('dc-1', 'trainingpeaks', 'g-1', 'u-0', 't-1', 'u-1', '900001', 'Athlete@Case.Test',
             'proposed', '2026-01-01T00:00:00Z');
         INSERT INTO admin_config_audit (id, timestamp, admin_user_id, admin_email, category,
             config_key, new_value, data_type)
         VALUES ('audit-1', '2026-01-01T00:00:00Z', 'u-0', 'Jane@Case.Test', 'c', 'k', '1', 'integer');
         INSERT INTO admin_provisioned_keys (admin_token_id, api_key_id, user_email, requested_tier,
             provisioned_at, provisioned_by_service, rate_limit_requests, rate_limit_period)
         VALUES ('tok-1', 'key-1', 'Bob@Case.Test', 'starter', '2026-01-01T00:00:00Z', 'svc', 10, 'month');
         INSERT INTO a2a_clients (client_id, user_id, name, client_secret_hash, api_key_hash,
             contact_email, created_at, updated_at)
         VALUES ('a2a-1', 'u-0', 'client', 'secret', 'hash', 'Ops@Case.Test',
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let mut wanted: Vec<String> = ACCOUNTS.iter().map(|(_, s)| (*s).to_owned()).collect();
    wanted.sort();
    assert_eq!(
        sqlite_texts(&pool, "SELECT email FROM users ORDER BY email").await,
        wanted
    );
    for (sql, stored) in [
        ("SELECT email FROM pre_approved_emails", "allowed@case.test"),
        ("SELECT email FROM messaging_link_states", "otp@case.test"),
        (
            "SELECT provider_athlete_email FROM delegated_connections",
            "athlete@case.test",
        ),
        (
            "SELECT admin_email FROM admin_config_audit",
            "jane@case.test",
        ),
        (
            "SELECT user_email FROM admin_provisioned_keys",
            "bob@case.test",
        ),
        ("SELECT contact_email FROM a2a_clients", "ops@case.test"),
    ] {
        assert_eq!(sqlite_texts(&pool, sql).await, vec![stored], "{sql}");
    }

    // A case variant of a stored address is refused by the schema itself.
    let variant = sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, last_active)
         VALUES ('u-variant', 'JANE@case.test', 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await;
    assert!(
        variant
            .unwrap_err()
            .to_string()
            .contains("idx_users_email_lower"),
        "the lower(email) index refuses a case variant of an account"
    );
    let variant = sqlx::query(
        "INSERT INTO pre_approved_emails (email, created_at) VALUES ('ALLOWED@Case.Test', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await;
    assert!(
        variant
            .unwrap_err()
            .to_string()
            .contains("idx_pre_approved_emails_email_lower"),
        "the lower(email) index refuses a case variant of a pre-approval"
    );

    // Applying it again finds nothing to do.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_eq!(
        sqlite_texts(&pool, "SELECT email FROM users ORDER BY email").await,
        wanted
    );
}

async fn collision_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;

    plant_sqlite_user(&pool, "u-jane", "Jane@Case.Test").await;
    plant_sqlite_user(&pool, "u-jane-lower", "jane@case.test").await;
    plant_sqlite_user(&pool, "u-other", "Other@Case.Test").await;
    plant_sqlite_allow(&pool, "Allowed@Case.Test").await;
    plant_sqlite_allow(&pool, " allowed@case.test").await;

    let refused = refusal(sqlx::raw_sql(&sql).execute(&pool).await);
    assert_names(
        &refused,
        "users.email",
        &["Jane@Case.Test", "jane@case.test"],
        &["Other@Case.Test"],
    );
    // Nothing merged, dropped or rewritten.
    assert_eq!(
        sqlite_texts(&pool, "SELECT email FROM users ORDER BY id").await,
        vec!["Jane@Case.Test", "jane@case.test", "Other@Case.Test"]
    );

    // The operator merges the two accounts by hand; the pre-approvals collide
    // next, and are named the same way.
    sqlx::query("DELETE FROM users WHERE id = 'u-jane'")
        .execute(&pool)
        .await
        .unwrap();
    let refused = refusal(sqlx::raw_sql(&sql).execute(&pool).await);
    assert_names(
        &refused,
        "pre_approved_emails.email",
        &["Allowed@Case.Test", " allowed@case.test"],
        &[],
    );
    assert_eq!(
        sqlite_texts(&pool, "SELECT email FROM users ORDER BY id").await,
        vec!["jane@case.test", "Other@Case.Test"]
    );

    // Once both are resolved by hand, the migration runs.
    sqlx::query("DELETE FROM pre_approved_emails WHERE email = 'Allowed@Case.Test'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_eq!(
        sqlite_texts(&pool, "SELECT email FROM users ORDER BY id").await,
        vec!["jane@case.test", "other@case.test"]
    );
    assert_eq!(
        sqlite_texts(&pool, "SELECT email FROM pre_approved_emails").await,
        vec!["allowed@case.test"]
    );
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. The factory hands out a clone of the migrated
/// template; the rows are only rewritten as the migration runs, so start
/// again from an empty schema.
#[cfg(feature = "postgresql")]
async fn postgres_before_migration(url: &str) -> (sqlx::PgPool, String) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    for migration in PG_MIGRATOR.iter() {
        if migration.version == EMAIL_CASE_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the email-case migration is in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn plant_postgres_user(pool: &sqlx::PgPool, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'x')")
        .bind(id)
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
    id
}

#[cfg(feature = "postgresql")]
async fn plant_postgres_allow(pool: &sqlx::PgPool, email: &str) {
    sqlx::query("INSERT INTO pre_approved_emails (email, created_at) VALUES ($1, now())")
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
}

/// The first column of every row `sql` reads, in byte order: the database's
/// collation is the server's, and would order mixed case its own way.
#[cfg(feature = "postgresql")]
async fn postgres_texts(pool: &sqlx::PgPool, sql: &str) -> Vec<String> {
    let mut texts: Vec<String> = sqlx::query(sql)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>(0))
        .collect();
    texts.sort();
    texts
}

#[cfg(feature = "postgresql")]
async fn lowercases_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;

    let mut accounts = Vec::new();
    for (legacy, _) in ACCOUNTS {
        accounts.push(plant_postgres_user(&pool, legacy).await);
    }
    let admin = accounts[0];
    plant_postgres_allow(&pool, "Allowed@Case.Test").await;
    let tenant = Uuid::new_v4();
    sqlx::query("INSERT INTO tenants (id, name, slug) VALUES ($1, 'Case', 'email-case')")
        .bind(tenant)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO messaging_link_states (id, tenant_id, channel_type, code, method, expires_at, email)
         VALUES ('ls-1', $1, 'whatsapp', 'code-1', 'otp', now(), ' Otp@Case.Test')",
    )
    .bind(tenant)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO admin_config_audit (id, timestamp, admin_user_id, admin_email, category,
             config_key, new_value, data_type)
         VALUES ('audit-1', now(), $1, 'Jane@Case.Test', 'c', 'k', '1', 'integer')",
    )
    .bind(admin)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let mut wanted: Vec<String> = ACCOUNTS.iter().map(|(_, s)| (*s).to_owned()).collect();
    wanted.sort();
    assert_eq!(
        postgres_texts(&pool, "SELECT email FROM users").await,
        wanted
    );
    for (sql, stored) in [
        ("SELECT email FROM pre_approved_emails", "allowed@case.test"),
        ("SELECT email FROM messaging_link_states", "otp@case.test"),
        (
            "SELECT admin_email FROM admin_config_audit",
            "jane@case.test",
        ),
    ] {
        assert_eq!(postgres_texts(&pool, sql).await, vec![stored], "{sql}");
    }

    let variant = sqlx::query(
        "INSERT INTO users (id, email, password_hash) VALUES ($1, 'JANE@case.test', 'x')",
    )
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;
    assert!(
        variant
            .unwrap_err()
            .to_string()
            .contains("idx_users_email_lower"),
        "the lower(email) index refuses a case variant of an account"
    );
    let variant = sqlx::query(
        "INSERT INTO pre_approved_emails (email, created_at) VALUES ('ALLOWED@Case.Test', now())",
    )
    .execute(&pool)
    .await;
    assert!(
        variant
            .unwrap_err()
            .to_string()
            .contains("idx_pre_approved_emails_email_lower"),
        "the lower(email) index refuses a case variant of a pre-approval"
    );

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_eq!(
        postgres_texts(&pool, "SELECT email FROM users").await,
        wanted
    );
}

#[cfg(feature = "postgresql")]
async fn collision_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;

    let jane = plant_postgres_user(&pool, "Jane@Case.Test").await;
    plant_postgres_user(&pool, "jane@case.test").await;
    plant_postgres_user(&pool, "Other@Case.Test").await;
    plant_postgres_allow(&pool, "Allowed@Case.Test").await;
    plant_postgres_allow(&pool, " allowed@case.test").await;

    let refused = refusal(sqlx::raw_sql(&sql).execute(&pool).await);
    assert_names(
        &refused,
        "users.email",
        &["Jane@Case.Test", "jane@case.test"],
        &["Other@Case.Test"],
    );
    assert_eq!(
        postgres_texts(&pool, "SELECT email FROM users").await,
        vec!["Jane@Case.Test", "Other@Case.Test", "jane@case.test"]
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(jane)
        .execute(&pool)
        .await
        .unwrap();
    let refused = refusal(sqlx::raw_sql(&sql).execute(&pool).await);
    assert_names(
        &refused,
        "pre_approved_emails.email",
        &["Allowed@Case.Test", " allowed@case.test"],
        &[],
    );
    assert_eq!(
        postgres_texts(&pool, "SELECT email FROM users").await,
        vec!["Other@Case.Test", "jane@case.test"]
    );

    sqlx::query("DELETE FROM pre_approved_emails WHERE email = 'Allowed@Case.Test'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_eq!(
        postgres_texts(&pool, "SELECT email FROM users").await,
        vec!["jane@case.test", "other@case.test"]
    );
    assert_eq!(
        postgres_texts(&pool, "SELECT email FROM pre_approved_emails").await,
        vec!["allowed@case.test"]
    );
}
