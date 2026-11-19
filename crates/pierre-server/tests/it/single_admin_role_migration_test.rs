// ABOUTME: carnet#1567 — the migration that folds the super_admin user role into admin
// ABOUTME: Runs the shipped file: each stored super_admin becomes an admin in both columns, and it refuses a flag-only admin

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_test_support::server::{create_test_admin_user, create_test_user};
use uuid::Uuid;

use crate::common::create_test_database;

/// The shipped migration, per backend — read as a file because the migration
/// is the data under test, and the whole file is executed, as the migrator
/// executes it: its precondition, then its one `UPDATE`.
const SQLITE_MIGRATION_SQL: &str =
    include_str!("../../../../migrations/20261114113700_single_admin_role.sql");
#[cfg(feature = "postgresql")]
const POSTGRES_MIGRATION_SQL: &str =
    include_str!("../../../../migrations_pg/20261114113700_single_admin_role.sql");
#[cfg(not(feature = "postgresql"))]
const POSTGRES_MIGRATION_SQL: &str = "";

/// What the migration says when it refuses, on either backend.
const REFUSAL: &str = "is_admin without the admin role";

/// Run `sqlite` or `postgres`, whichever backend the test database is on.
async fn try_execute(
    db: &Database,
    sqlite: &str,
    #[cfg_attr(not(feature = "postgresql"), expect(unused_variables))] postgres: &str,
) -> Result<(), sqlx::Error> {
    match db.backend() {
        DatabaseBackend::SQLite(backend) => {
            sqlx::raw_sql(sqlite).execute(backend.pool()).await?;
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => {
            sqlx::raw_sql(postgres).execute(backend.pool()).await?;
        }
    }
    Ok(())
}

async fn execute(db: &Database, sqlite: &str, postgres: &str) {
    try_execute(db, sqlite, postgres).await.unwrap();
}

/// Run the migration file; its refusal is the error.
async fn try_fold(db: &Database) -> Result<(), sqlx::Error> {
    try_execute(db, SQLITE_MIGRATION_SQL, POSTGRES_MIGRATION_SQL).await
}

async fn run_fold(db: &Database) {
    try_fold(db).await.unwrap();
}

/// The `(role, is_admin)` pair as stored, read past the repository: the
/// repository reads the role and never this column, so only this read shows it.
async fn stored_pair(db: &Database, email: &str) -> (String, bool) {
    const STORED_PAIR_SQL: &str = "SELECT role, is_admin FROM users WHERE email = $1";
    match db.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_as(STORED_PAIR_SQL)
            .bind(email)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_as(STORED_PAIR_SQL)
            .bind(email)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
    }
}

/// Take the database back to how 20261114113700 finds it: without the
/// triggers of 20261114113701, which store a written `'super_admin'` as
/// `'admin'` and would leave the fold nothing to rewrite.
async fn drop_the_deriving_triggers(db: &Database) {
    execute(
        db,
        "DROP TRIGGER IF EXISTS trg_users_is_admin_follows_role_insert; \
         DROP TRIGGER IF EXISTS trg_users_is_admin_follows_role_update;",
        "DROP TRIGGER IF EXISTS trg_users_is_admin_follows_role ON users;",
    )
    .await;
}

/// Put `user_id` in the shape the retired tier stored: `role = 'super_admin'`,
/// with the admin flag as given (20260518000001 found rows where the flag and
/// the role disagreed, so both shapes are folded).
async fn store_as_super_admin(db: &Database, user_id: Uuid, flag: bool) {
    const STORE_AS_SUPER_ADMIN_SQL: &str =
        "UPDATE users SET role = 'super_admin', is_admin = $1 WHERE id = $2";
    // The id column is `uuid` on `PostgreSQL` and TEXT on `SQLite`.
    match db.backend() {
        DatabaseBackend::SQLite(backend) => {
            sqlx::query(STORE_AS_SUPER_ADMIN_SQL)
                .bind(flag)
                .bind(user_id.to_string())
                .execute(backend.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => {
            sqlx::query(STORE_AS_SUPER_ADMIN_SQL)
                .bind(flag)
                .bind(user_id)
                .execute(backend.pool())
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn every_stored_super_admin_becomes_an_admin_and_nobody_else_moves() {
    let db = create_test_database().await.unwrap();
    let users = &db.repositories().users;
    let flagged = create_test_user("was-super-flagged@example.com", None);
    let unflagged = create_test_user("was-super-unflagged@example.com", None);
    let admin = create_test_admin_user("already-admin@example.com", None);
    let athlete = create_test_user("athlete@example.com", None);
    for user in [&flagged, &unflagged, &admin, &athlete] {
        users.create(user).await.unwrap();
    }
    drop_the_deriving_triggers(&db).await;
    store_as_super_admin(&db, flagged.id, true).await;
    store_as_super_admin(&db, unflagged.id, false).await;

    // Before the fold, this build does not know the stored value: any
    // 'super_admin' row reads as a plain user, whatever its flag. That is why
    // the migration must run before the server serves.
    for id in [flagged.id, unflagged.id] {
        let before = users.get_global(id).await.unwrap().unwrap();
        assert_eq!(before.role, UserRole::User);
    }

    run_fold(&db).await;

    // The stored columns, read past the repository and with the deriving
    // triggers dropped: the fold alone sets the flag with the role.
    for email in [
        "was-super-flagged@example.com",
        "was-super-unflagged@example.com",
    ] {
        assert_eq!(
            stored_pair(&db, email).await,
            ("admin".to_owned(), true),
            "{email}: the fold writes the role and the flag"
        );
    }
    for (id, was) in [(flagged.id, "flagged"), (unflagged.id, "unflagged")] {
        let folded = users.get_global(id).await.unwrap().unwrap();
        assert_eq!(folded.role, UserRole::Admin, "{was}");
        assert!(folded.is_active_admin(), "{was}");
    }
    let listed: Vec<Uuid> = users
        .list_admins()
        .await
        .unwrap()
        .iter()
        .map(|u| u.id)
        .collect();
    assert_eq!(listed.len(), 3, "the two folded accounts and the admin");
    assert!(listed.contains(&flagged.id) && listed.contains(&unflagged.id));

    assert_eq!(
        stored_pair(&db, "already-admin@example.com").await,
        ("admin".to_owned(), true)
    );
    assert_eq!(
        stored_pair(&db, "athlete@example.com").await,
        ("user".to_owned(), false),
        "a plain user is not promoted"
    );

    // Idempotent: a second run finds nothing to fold.
    run_fold(&db).await;
    assert_eq!(users.list_admins().await.unwrap().len(), 3);
}

/// The build this migration replaces treated `is_admin = true, role = 'user'`
/// as an admin; this one decides from the role, and the next migration clears
/// such a flag. Rather than demote an admin without a word, as 20260518000001
/// did to both founders, the fold refuses to run while such a row exists and
/// writes nothing, so the revision still serving keeps what it reads. Once
/// the row is settled by hand, it runs.
#[tokio::test]
async fn the_fold_refuses_to_run_while_an_account_holds_the_flag_without_the_role() {
    let db = create_test_database().await.unwrap();
    let users = &db.repositories().users;
    let flag_only = create_test_user("flag-only@example.com", None);
    let was_super = create_test_user("was-super@example.com", None);
    for user in [&flag_only, &was_super] {
        users.create(user).await.unwrap();
    }
    drop_the_deriving_triggers(&db).await;
    store_as_super_admin(&db, was_super.id, true).await;
    execute(
        &db,
        "UPDATE users SET is_admin = 1 WHERE email = 'flag-only@example.com'",
        "UPDATE users SET is_admin = TRUE WHERE email = 'flag-only@example.com'",
    )
    .await;
    assert_eq!(
        stored_pair(&db, "flag-only@example.com").await,
        ("user".to_owned(), true)
    );

    let refused = try_fold(&db)
        .await
        .expect_err("a flag-only admin stops the migration");
    assert!(refused.to_string().contains(REFUSAL), "{refused}");
    assert_eq!(
        stored_pair(&db, "was-super@example.com").await,
        ("super_admin".to_owned(), true),
        "the refused migration folded nobody"
    );
    assert_eq!(
        stored_pair(&db, "flag-only@example.com").await,
        ("user".to_owned(), true),
        "and decided nothing about the row it refused over"
    );

    // Settled as an admin by hand, the row no longer stops it.
    execute(
        &db,
        "UPDATE users SET role = 'admin' WHERE email = 'flag-only@example.com'",
        "UPDATE users SET role = 'admin' WHERE email = 'flag-only@example.com'",
    )
    .await;
    run_fold(&db).await;
    for email in ["flag-only@example.com", "was-super@example.com"] {
        assert_eq!(stored_pair(&db, email).await, ("admin".to_owned(), true));
    }
}
