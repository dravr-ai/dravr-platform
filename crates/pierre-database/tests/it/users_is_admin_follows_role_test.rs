// ABOUTME: carnet#1574 — the database derives users.is_admin from users.role on every write, whatever the writer sent
// ABOUTME: Raw SQL past the repository, and the repository's own writes, which name the role alone

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::seed_models::SeedDemoUser;
use pierre_test_support::db::create_test_db;
use pierre_test_support::server::{create_test_admin_user, create_test_user};
use uuid::Uuid;

/// A row written with the flag and no role: `role` takes its DEFAULT.
const INSERT_FLAG_ONLY_SQL: &str = "INSERT INTO users \
     (id, email, password_hash, created_at, last_active, is_admin) \
     VALUES ($1, $2, 'not-a-hash', $3, $3, $4)";

/// A row written with the role and no flag: `is_admin` takes its DEFAULT.
const INSERT_ROLE_ONLY_SQL: &str = "INSERT INTO users \
     (id, email, password_hash, created_at, last_active, role) \
     VALUES ($1, $2, 'not-a-hash', $3, $3, $4)";

/// A row written with both columns, as given.
const INSERT_BOTH_SQL: &str = "INSERT INTO users \
     (id, email, password_hash, created_at, last_active, is_admin, role) \
     VALUES ($1, $2, 'not-a-hash', $3, $3, $4, $5)";

/// The upsert shape the boot seeder of a revision before carnet#1574 writes,
/// and may still write while it serves during a deploy: an existing email
/// takes the incoming role and flag as sent.
const UPSERT_BOTH_SQL: &str = "INSERT INTO users \
     (id, email, password_hash, created_at, last_active, is_admin, role) \
     VALUES ($1, $2, 'not-a-hash', $3, $3, $4, $5) \
     ON CONFLICT(email) DO UPDATE SET role = EXCLUDED.role, is_admin = EXCLUDED.is_admin";

const SET_FLAG_SQL: &str = "UPDATE users SET is_admin = $2 WHERE id = $1";
const SET_ROLE_SQL: &str = "UPDATE users SET role = $2 WHERE id = $1";
const SET_BOTH_SQL: &str = "UPDATE users SET is_admin = $2, role = $3 WHERE id = $1";
/// A write that names neither column.
const TOUCH_SQL: &str = "UPDATE users SET display_name = $2 WHERE id = $1";
const STORED_PAIR_SQL: &str = "SELECT role, is_admin FROM users WHERE id = $1";

/// One value of a raw statement; the id column is `uuid` on `PostgreSQL` and
/// TEXT on `SQLite`, every other value binds the same way on both.
#[derive(Clone, Copy)]
enum Arg<'a> {
    Id(Uuid),
    Text(&'a str),
    Flag(bool),
    Now,
}

/// Run `sql` with `args` straight on the pool, past the repository.
async fn write(db: &Database, sql: &str, args: &[Arg<'_>]) {
    let now = Utc::now();
    match db.backend() {
        DatabaseBackend::SQLite(backend) => {
            let mut query = sqlx::query(sql);
            for arg in args {
                query = match *arg {
                    Arg::Id(id) => query.bind(id.to_string()),
                    Arg::Text(text) => query.bind(text),
                    Arg::Flag(flag) => query.bind(flag),
                    Arg::Now => query.bind(now),
                };
            }
            query.execute(backend.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => {
            let mut query = sqlx::query(sql);
            for arg in args {
                query = match *arg {
                    Arg::Id(id) => query.bind(id),
                    Arg::Text(text) => query.bind(text),
                    Arg::Flag(flag) => query.bind(flag),
                    Arg::Now => query.bind(now),
                };
            }
            query.execute(backend.pool()).await.unwrap();
        }
    }
}

/// The `(role, is_admin)` pair as stored, read past the repository: the
/// repository reads the role and never this column, so only this read shows it.
async fn stored_pair(db: &Database, id: Uuid) -> (String, bool) {
    match db.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_as(STORED_PAIR_SQL)
            .bind(id.to_string())
            .fetch_one(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_as(STORED_PAIR_SQL)
            .bind(id)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
    }
}

fn admin() -> (String, bool) {
    ("admin".to_owned(), true)
}

fn plain_user() -> (String, bool) {
    ("user".to_owned(), false)
}

/// Insert a plain user past the repository and return its id.
async fn insert_plain_user(db: &Database, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    write(
        db,
        INSERT_BOTH_SQL,
        &[
            Arg::Id(id),
            Arg::Text(email),
            Arg::Now,
            Arg::Flag(false),
            Arg::Text("user"),
        ],
    )
    .await;
    assert_eq!(stored_pair(db, id).await, plain_user(), "{email}");
    id
}

/// Insert an admin past the repository and return its id.
async fn insert_admin(db: &Database, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    write(
        db,
        INSERT_BOTH_SQL,
        &[
            Arg::Id(id),
            Arg::Text(email),
            Arg::Now,
            Arg::Flag(true),
            Arg::Text("admin"),
        ],
    )
    .await;
    assert_eq!(stored_pair(db, id).await, admin(), "{email}");
    id
}

#[tokio::test]
async fn an_insert_stores_the_flag_the_role_decides() {
    let db = create_test_db().await.unwrap();

    // The flag without the role: what the first-admin setup wrote for both
    // founders. The role decides, so this is a plain user.
    let flag_only = Uuid::new_v4();
    write(
        &db,
        INSERT_FLAG_ONLY_SQL,
        &[
            Arg::Id(flag_only),
            Arg::Text("flag-only@example.com"),
            Arg::Now,
            Arg::Flag(true),
        ],
    )
    .await;
    assert_eq!(stored_pair(&db, flag_only).await, plain_user());

    // The role without the flag: the flag follows.
    let role_only = Uuid::new_v4();
    write(
        &db,
        INSERT_ROLE_ONLY_SQL,
        &[
            Arg::Id(role_only),
            Arg::Text("role-only@example.com"),
            Arg::Now,
            Arg::Text("admin"),
        ],
    )
    .await;
    assert_eq!(stored_pair(&db, role_only).await, admin());

    // The retired value, as a revision from before the fold writes it: with
    // no flag, with the flag off and with the flag on. Stored as an admin,
    // never refused.
    let retired = Uuid::new_v4();
    write(
        &db,
        INSERT_ROLE_ONLY_SQL,
        &[
            Arg::Id(retired),
            Arg::Text("retired-no-flag@example.com"),
            Arg::Now,
            Arg::Text("super_admin"),
        ],
    )
    .await;
    assert_eq!(stored_pair(&db, retired).await, admin());
    for (email, flag) in [
        ("retired-flag-off@example.com", false),
        ("retired-flag-on@example.com", true),
    ] {
        let id = Uuid::new_v4();
        write(
            &db,
            INSERT_BOTH_SQL,
            &[
                Arg::Id(id),
                Arg::Text(email),
                Arg::Now,
                Arg::Flag(flag),
                Arg::Text("super_admin"),
            ],
        )
        .await;
        assert_eq!(stored_pair(&db, id).await, admin(), "{email}");
    }

    // The two columns contradicting each other, both ways.
    let contradicted = Uuid::new_v4();
    write(
        &db,
        INSERT_BOTH_SQL,
        &[
            Arg::Id(contradicted),
            Arg::Text("admin-flag-off@example.com"),
            Arg::Now,
            Arg::Flag(false),
            Arg::Text("admin"),
        ],
    )
    .await;
    assert_eq!(stored_pair(&db, contradicted).await, admin());

    // What the repository hands out is the same fact.
    let users = &db.repositories().users;
    let read = users.get_global(flag_only).await.unwrap().unwrap();
    assert_eq!(read.role, UserRole::User);
    assert!(!read.is_active_admin());
    let read = users.get_global(retired).await.unwrap().unwrap();
    assert_eq!(read.role, UserRole::Admin);
}

#[tokio::test]
async fn an_update_stores_the_flag_the_role_decides() {
    let db = create_test_db().await.unwrap();

    // The flag without the role promotes nobody.
    let flagged = insert_plain_user(&db, "flagged@example.com").await;
    write(&db, SET_FLAG_SQL, &[Arg::Id(flagged), Arg::Flag(true)]).await;
    assert_eq!(stored_pair(&db, flagged).await, plain_user());

    // The role without the flag promotes, and the flag follows.
    let promoted = insert_plain_user(&db, "promoted@example.com").await;
    write(&db, SET_ROLE_SQL, &[Arg::Id(promoted), Arg::Text("admin")]).await;
    assert_eq!(stored_pair(&db, promoted).await, admin());

    // The retired value is stored as an admin, alone or beside either flag.
    let retired = insert_plain_user(&db, "retired@example.com").await;
    write(
        &db,
        SET_ROLE_SQL,
        &[Arg::Id(retired), Arg::Text("super_admin")],
    )
    .await;
    assert_eq!(stored_pair(&db, retired).await, admin());
    for (email, flag) in [
        ("retired-off@example.com", false),
        ("retired-on@example.com", true),
    ] {
        let id = insert_plain_user(&db, email).await;
        write(
            &db,
            SET_BOTH_SQL,
            &[Arg::Id(id), Arg::Flag(flag), Arg::Text("super_admin")],
        )
        .await;
        assert_eq!(stored_pair(&db, id).await, admin(), "{email}");
    }

    // Clearing the flag demotes nobody; clearing the role does, flag included.
    let kept = insert_admin(&db, "kept@example.com").await;
    write(&db, SET_FLAG_SQL, &[Arg::Id(kept), Arg::Flag(false)]).await;
    assert_eq!(stored_pair(&db, kept).await, admin());
    let demoted = insert_admin(&db, "demoted@example.com").await;
    write(&db, SET_ROLE_SQL, &[Arg::Id(demoted), Arg::Text("user")]).await;
    assert_eq!(stored_pair(&db, demoted).await, plain_user());

    // A write that names neither column leaves both as they were.
    write(&db, TOUCH_SQL, &[Arg::Id(kept), Arg::Text("Kept")]).await;
    write(&db, TOUCH_SQL, &[Arg::Id(demoted), Arg::Text("Demoted")]).await;
    assert_eq!(stored_pair(&db, kept).await, admin());
    assert_eq!(stored_pair(&db, demoted).await, plain_user());
}

/// The repository's writes name the role and never `users.is_admin`: the
/// column a rolled-back build reads is right after each of them because the
/// database derives it, on both engines.
#[tokio::test]
async fn the_repository_writes_the_role_and_the_column_follows() {
    let db = create_test_db().await.unwrap();
    let users = &db.repositories().users;

    // create(): an admin and a plain user.
    let standing = create_test_admin_user("standing@example.com", None);
    users.create(&standing).await.unwrap();
    assert_eq!(stored_pair(&db, standing.id).await, admin());
    let account = create_test_user("account@example.com", None);
    users.create(&account).await.unwrap();
    assert_eq!(stored_pair(&db, account.id).await, plain_user());

    // update(): the whole-row write, promoting and demoting.
    let mut row = users.get_global(account.id).await.unwrap().unwrap();
    row.role = UserRole::Admin;
    users.update(&row).await.unwrap();
    assert_eq!(stored_pair(&db, account.id).await, admin());
    row.role = UserRole::User;
    users.update(&row).await.unwrap();
    assert_eq!(stored_pair(&db, account.id).await, plain_user());

    // set_admin_status(): the single-column write, both ways.
    let promoted = users.set_admin_status(account.id, true).await.unwrap().user;
    assert_eq!(promoted.role, UserRole::Admin);
    assert_eq!(stored_pair(&db, account.id).await, admin());
    let demoted = users
        .set_admin_status(account.id, false)
        .await
        .unwrap()
        .user;
    assert_eq!(demoted.role, UserRole::User);
    assert_eq!(stored_pair(&db, account.id).await, plain_user());

    // The seeder's insert: the bootstrap admin and a demo account.
    for (email, role, expected) in [
        ("seeded-admin@example.com", UserRole::Admin, admin()),
        ("seeded-demo@example.com", UserRole::User, plain_user()),
    ] {
        let seeded = SeedDemoUser {
            id: Uuid::new_v4(),
            email: email.to_owned(),
            display_name: "Seeded".to_owned(),
            password_hash: "not-a-hash".to_owned(),
            tier: "starter".to_owned(),
            status: "active".to_owned(),
            role,
            locale: "en".to_owned(),
            created_at: Utc::now(),
        };
        db.repositories()
            .seeder
            .seed_insert_demo_user(&seeded)
            .await
            .unwrap();
        assert_eq!(stored_pair(&db, seeded.id).await, expected, "{email}");
    }
}

/// The boot seeder of a revision from before the fold upserts its admin by
/// email with `role = 'super_admin'`. The write must succeed, and the row it
/// lands on must be an admin in this build.
#[tokio::test]
async fn an_upsert_onto_an_existing_row_is_derived_too() {
    let db = create_test_db().await.unwrap();
    let existing = insert_plain_user(&db, "seeded@example.com").await;

    write(
        &db,
        UPSERT_BOTH_SQL,
        &[
            Arg::Id(Uuid::new_v4()),
            Arg::Text("seeded@example.com"),
            Arg::Now,
            Arg::Flag(false),
            Arg::Text("super_admin"),
        ],
    )
    .await;
    assert_eq!(stored_pair(&db, existing).await, admin());

    // And back: the flag sent beside a plain role does not keep the admin.
    write(
        &db,
        UPSERT_BOTH_SQL,
        &[
            Arg::Id(Uuid::new_v4()),
            Arg::Text("seeded@example.com"),
            Arg::Now,
            Arg::Flag(true),
            Arg::Text("user"),
        ],
    )
    .await;
    assert_eq!(stored_pair(&db, existing).await, plain_user());
}

/// `SQLite` derives the flag in AFTER triggers that update the row they fired
/// for. The pool leaves `recursive_triggers` off; a connection that turns it
/// on must reach the same rows, and return.
#[tokio::test]
async fn the_sqlite_triggers_terminate_with_recursive_triggers_on() {
    let db = create_test_db().await.unwrap();
    let backend = match db.backend() {
        DatabaseBackend::SQLite(backend) => backend,
        // PostgreSQL sets NEW in a BEFORE trigger: there is nothing to recurse.
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(_) => return,
    };
    let mut conn = backend.pool().acquire().await.unwrap();
    let pragma: i64 = sqlx::query_scalar("PRAGMA recursive_triggers")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(pragma, 0, "the pool runs the SQLite default");
    sqlx::query("PRAGMA recursive_triggers = ON")
        .execute(&mut *conn)
        .await
        .unwrap();

    let now = Utc::now();
    let inserted = Uuid::new_v4();
    sqlx::query(INSERT_BOTH_SQL)
        .bind(inserted.to_string())
        .bind("recursive-insert@example.com")
        .bind(now)
        .bind(false)
        .bind("super_admin")
        .execute(&mut *conn)
        .await
        .unwrap();
    let updated = Uuid::new_v4();
    sqlx::query(INSERT_BOTH_SQL)
        .bind(updated.to_string())
        .bind("recursive-update@example.com")
        .bind(now)
        .bind(false)
        .bind("user")
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query(SET_BOTH_SQL)
        .bind(updated.to_string())
        .bind(true)
        .bind("user")
        .execute(&mut *conn)
        .await
        .unwrap();

    for (id, expected) in [(inserted, admin()), (updated, plain_user())] {
        let pair: (String, bool) = sqlx::query_as(STORED_PAIR_SQL)
            .bind(id.to_string())
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(pair, expected);
    }

    // The connection goes back to the pool as it came.
    sqlx::query("PRAGMA recursive_triggers = OFF")
        .execute(&mut *conn)
        .await
        .unwrap();
}
