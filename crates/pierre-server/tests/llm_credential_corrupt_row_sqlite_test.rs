// ABOUTME: Proves a corrupt identifier column in user_llm_credentials fails the read
// ABOUTME: instead of decoding to a nil/default id that silently matches nothing.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_credentials` used to decode identifier columns with `unwrap_or_default()`
//! (and `tenant_id` with `unwrap_or_else(|_| TenantId::nil())`). A malformed
//! value therefore became a *valid-looking* all-zeros id, which then quietly
//! matched no rows anywhere downstream — a data-integrity fault laundered into
//! an ordinary empty result.
//!
//! Scope note, stated rather than implied: the `tenant_id` column cannot be
//! exercised through this path. `get_credentials` filters on `WHERE tenant_id =
//! ?1`, binding a well-formed `TenantId`, so a row whose stored `tenant_id` is
//! unparseable can never be returned in the first place. The reachable columns
//! are `id` and `created_by`, and those are what this test drives. The
//! `tenant_id` arm is defense in depth for any future caller that does not
//! filter on it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_sqlite_test_db;
use pierre_database::database::Database as SqliteDatabase;
use pierre_database::repositories::LlmCredentialRepository;
use sqlx::Executor;
use uuid::Uuid;

/// The `SQLite` backend itself: the corruptions below are shapes only its
/// untyped TEXT columns accept (`PostgreSQL` refuses a non-UUID in a UUID
/// column and a non-timestamp in a TIMESTAMPTZ column at the INSERT), and
/// `PRAGMA foreign_keys` is a `SQLite` switch.
async fn open_in_memory_db() -> Result<SqliteDatabase> {
    match create_sqlite_test_db().await? {
        Database::SQLite(db) => Ok(db),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(_) => unreachable!("create_sqlite_test_db opens SQLite"),
    }
}

/// Seed the user and tenant rows the `user_llm_credentials` foreign keys need.
async fn seed_user_and_tenant(db: &SqliteDatabase) -> Result<(TenantId, Uuid)> {
    let user_uuid = Uuid::new_v4();
    let tenant_uuid = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let pool = db.pool();

    pool.execute(
        sqlx::query(
            r"
            INSERT INTO users (id, email, password_hash, created_at, last_active)
            VALUES ($1, $2, $3, $4, $4)
            ",
        )
        .bind(user_uuid.to_string())
        .bind(format!("creds-{user_uuid}@test.com"))
        .bind("hash")
        .bind(&now),
    )
    .await?;

    pool.execute(
        sqlx::query(
            r"
            INSERT INTO tenants (id, name, slug, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $4)
            ",
        )
        .bind(tenant_uuid.to_string())
        .bind("Test Tenant")
        .bind(format!("tenant-{tenant_uuid}"))
        .bind(&now),
    )
    .await?;

    Ok((TenantId::from_uuid(tenant_uuid), user_uuid))
}

/// Insert a credentials row directly, so `id` and `created_by` can be written in
/// shapes the repository's own writer would never produce.
async fn insert_credentials_row(
    db: &SqliteDatabase,
    raw_id: &str,
    tenant_id: TenantId,
    raw_created_by: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    db.pool()
        .execute(
            sqlx::query(
                r"
                INSERT INTO user_llm_credentials (
                    id, tenant_id, user_id, provider, api_key_encrypted,
                    base_url, default_model, is_active, created_at, updated_at, created_by
                ) VALUES ($1, $2, NULL, 'gemini', 'ciphertext', NULL, NULL, 1, $3, $3, $4)
                ",
            )
            .bind(raw_id)
            .bind(tenant_id.to_string())
            .bind(&now)
            .bind(raw_created_by),
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn wellformed_row_is_returned_with_its_real_identifiers() {
    let db = open_in_memory_db().await.unwrap();
    let (tenant_id, user_id) = seed_user_and_tenant(&db).await.unwrap();

    let real_id = Uuid::new_v4();
    insert_credentials_row(&db, &real_id.to_string(), tenant_id, &user_id.to_string())
        .await
        .unwrap();

    let record = db
        .get_credentials(tenant_id, None, "gemini")
        .await
        .expect("a well-formed row must read back cleanly")
        .expect("the row was just inserted for this tenant");

    // Assert the decoded values, not merely that the call succeeded — a stub
    // returning a default-constructed record would pass an `is_ok()` check.
    assert_eq!(record.id, real_id, "id must decode to the stored value");
    assert_eq!(record.tenant_id, tenant_id);
    assert_eq!(record.created_by, user_id);
    assert_eq!(record.provider, "gemini");
    assert_eq!(record.api_key_encrypted, "ciphertext");
    assert!(
        !record.id.is_nil(),
        "a real id must never decode to the nil UUID"
    );
}

#[tokio::test]
async fn corrupt_id_column_fails_the_read_and_names_the_column() {
    let db = open_in_memory_db().await.unwrap();
    let (tenant_id, user_id) = seed_user_and_tenant(&db).await.unwrap();

    insert_credentials_row(&db, "not-a-uuid", tenant_id, &user_id.to_string())
        .await
        .unwrap();

    let err = db
        .get_credentials(tenant_id, None, "gemini")
        .await
        .expect_err(
            "a malformed id must fail the read — decoding it to the nil UUID hides \
             the corruption behind a valid-looking record",
        );

    let message = err.to_string();
    assert!(
        message.contains("user_llm_credentials.id"),
        "the error must name the offending column so the bad row can be found, got: {message}"
    );
}

#[tokio::test]
async fn corrupt_created_by_column_fails_the_read_and_names_the_column() {
    let db = open_in_memory_db().await.unwrap();
    let (tenant_id, _user_id) = seed_user_and_tenant(&db).await.unwrap();

    // `created_by` carries a foreign key to users(id). `PRAGMA foreign_keys` is
    // per-connection, so the pragma and the write must ride the SAME pooled
    // connection — issuing them separately against the pool lets the pool hand
    // the INSERT a different connection that still enforces the key.
    let now = Utc::now().to_rfc3339();
    let mut conn = db.pool().acquire().await.unwrap();
    conn.execute(sqlx::query("PRAGMA foreign_keys = OFF"))
        .await
        .unwrap();
    conn.execute(
        sqlx::query(
            r"
            INSERT INTO user_llm_credentials (
                id, tenant_id, user_id, provider, api_key_encrypted,
                base_url, default_model, is_active, created_at, updated_at, created_by
            ) VALUES ($1, $2, NULL, 'gemini', 'ciphertext', NULL, NULL, 1, $3, $3, $4)
            ",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(tenant_id.to_string())
        .bind(&now)
        .bind("also-not-a-uuid"),
    )
    .await
    .unwrap();
    drop(conn);

    let err = db
        .get_credentials(tenant_id, None, "gemini")
        .await
        .expect_err("a malformed created_by must fail the read, not default to the nil UUID");

    let message = err.to_string();
    assert!(
        message.contains("user_llm_credentials.created_by"),
        "the error must name the offending column, got: {message}"
    );
}

// ---------------------------------------------------------------------------
// Same class, different table: coaching_groups row mappers used to decode a bad
// UUID to nil and a bad timestamp to `Utc::now()`. Both now fail the read.
// ---------------------------------------------------------------------------

/// A corrupt `created_at` is the sharper of the two: the old code substituted
/// `Utc::now()`, which is a plausible value nothing downstream could tell apart
/// from a real one.
#[tokio::test]
async fn corrupt_coaching_group_timestamp_fails_the_read() {
    use pierre_database::repositories::CoachingGroupRepository;

    let db = open_in_memory_db().await.unwrap();
    let (tenant_id, user_id) = seed_user_and_tenant(&db).await.unwrap();
    let group_id = Uuid::new_v4();

    let mut conn = db.pool().acquire().await.unwrap();
    conn.execute(sqlx::query("PRAGMA foreign_keys = OFF"))
        .await
        .unwrap();
    conn.execute(
        sqlx::query(
            r"
            INSERT INTO coaching_groups (
                id, tenant_id, name, description, coach_id, owner_id,
                peer_data_sharing, max_members, is_active, created_at, updated_at
            ) VALUES ($1, $2, 'Test Group', NULL, 'coach-1', $3, 0, 20, 1, $4, $4)
            ",
        )
        .bind(group_id.to_string())
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind("not-a-timestamp"),
    )
    .await
    .unwrap();
    drop(conn);

    let err = db
        .get_group(&group_id.to_string(), tenant_id)
        .await
        .expect_err("a malformed created_at must fail the read, not become Utc::now()");

    let message = err.to_string();
    assert!(
        message.contains("created_at"),
        "the error must name the offending column, got: {message}"
    );
}
