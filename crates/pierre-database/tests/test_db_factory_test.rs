// ABOUTME: Proves the test-database factory opens the backend DATABASE_URL names and isolates callers
// ABOUTME: On a PostgreSQL URL every call must land on a private PostgreSQL database, never on SQLite
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The `PostgreSQL` CI lane's whole verdict rests on the factory in
//! `pierre_database::database::test_utils` honouring `DATABASE_URL`. This file
//! asserts that contract from the outside, against whichever server the
//! environment provides: on a `PostgreSQL` URL every database it hands out is
//! `PostgreSQL` and private to its caller; anywhere else it is in-memory
//! `SQLite`. Both branches assert — neither is a skip.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::User;
use pierre_database::backends::factory::{Database, DatabaseType};
use pierre_database::database::test_utils::{
    create_sqlite_test_db, create_test_db, create_test_db_url, create_test_db_with_key,
};
use pierre_database::DatabaseProvider;
use std::env;

fn expected_backend() -> DatabaseType {
    match env::var("DATABASE_URL") {
        Ok(url) if url.starts_with("postgres://") || url.starts_with("postgresql://") => {
            DatabaseType::PostgreSQL
        }
        _ => DatabaseType::SQLite,
    }
}

fn backend_of(db: &Database) -> DatabaseType {
    match db {
        Database::SQLite(_) => DatabaseType::SQLite,
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(_) => DatabaseType::PostgreSQL,
    }
}

#[tokio::test]
async fn factory_opens_the_backend_database_url_names() {
    let db = create_test_db().await.expect("factory opens a database");
    assert_eq!(backend_of(&db), expected_backend());

    let keyed = create_test_db_with_key(vec![7u8; 32])
        .await
        .expect("factory opens a database under a caller key");
    assert_eq!(backend_of(&keyed), expected_backend());
}

#[tokio::test]
async fn every_call_is_a_private_database() {
    let first = create_test_db().await.expect("first database");
    let second = create_test_db().await.expect("second database");

    let email = "isolation@example.com";
    first
        .repositories()
        .users
        .create(&User::new(
            email.to_owned(),
            "hash".to_owned(),
            Some("Isolation".to_owned()),
        ))
        .await
        .expect("insert into the first database");

    assert!(
        first
            .repositories()
            .users
            .get_by_email(email)
            .await
            .expect("lookup in the first database")
            .is_some(),
        "the row is visible where it was written"
    );
    assert!(
        second
            .repositories()
            .users
            .get_by_email(email)
            .await
            .expect("lookup in the second database")
            .is_none(),
        "a row written to one test database must not be visible from another"
    );
}

#[tokio::test]
async fn a_database_url_opens_a_migrated_database() {
    let handle = create_test_db_url().await.expect("factory hands out a URL");
    assert_eq!(
        handle.is_reserved(),
        expected_backend() == DatabaseType::PostgreSQL,
        "a PostgreSQL database stays reserved while the handle lives; SQLite needs no reservation"
    );
    let opened = open(&handle.url).await;
    assert_eq!(backend_of(&opened), expected_backend());

    // Migrations already applied: a repository call succeeds without any
    // further setup, which is what a server booted from this URL relies on.
    let users = opened
        .repositories()
        .users
        .get_by_email("nobody@example.com")
        .await
        .expect("a migrated schema answers queries");
    assert!(users.is_none());
}

#[tokio::test]
async fn explicit_sqlite_ignores_database_url() {
    let db = create_sqlite_test_db().await.expect("sqlite opens");
    assert_eq!(backend_of(&db), DatabaseType::SQLite);
}

/// The `SQLite` fast path clones a serialized image instead of migrating, so
/// this proves the clone carries the complete migration ledger: `migrate()`
/// re-validates every applied migration's checksum and would fail on a
/// missing ledger (re-running DDL against existing tables) or on bytes that
/// diverge from the embedded set.
#[tokio::test]
async fn a_factory_database_carries_the_full_migration_ledger() {
    let db = create_test_db().await.expect("factory opens a database");
    db.migrate()
        .await
        .expect("every embedded migration is recorded as applied");
}

async fn open(url: &str) -> Database {
    #[cfg(feature = "postgresql")]
    {
        Database::new(url, vec![0u8; 32], &PostgresPoolConfig::default())
            .await
            .expect("the URL opens")
    }
    #[cfg(not(feature = "postgresql"))]
    {
        Database::new(url, vec![0u8; 32])
            .await
            .expect("the URL opens")
    }
}
