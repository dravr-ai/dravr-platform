// ABOUTME: PostgreSQL-lane test for the seeded account locale — the demo-user INSERT carries `locale`
// ABOUTME: The PG seeder is a separate statement from SQLite's, so an unbound column hides on SQLite

//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` seeded-locale test.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use pierre_seeders::bootstrap::{self, SeedArgs};

mod common;

/// `users.locale` is `NOT NULL DEFAULT 'fr'` on both engines, and the two seeder
/// backends write their own INSERT. A locale bound on `SQLite` but missing from the PG
/// statement would leave every deployed seeded account French while the `SQLite` test
/// stayed green — the shape of this repo's PG regressions.
#[tokio::test]
async fn test_pg_seeded_accounts_are_english() {
    let isolated = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };
    let db = isolated.get_database().await.unwrap();
    let repos = db.repositories();

    bootstrap::run(
        SeedArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        &repos,
    )
    .await
    .unwrap();

    let operator = repos
        .users
        .get_by_email("operator@dravr.ai")
        .await
        .unwrap()
        .expect("operator seeded on PG");
    assert_eq!(operator.locale, "en", "the PG-seeded operator is English");

    let alice = repos
        .users
        .get_by_email("alice@demo.pierre.dev")
        .await
        .unwrap()
        .expect("demo user seeded on PG");
    assert_eq!(alice.locale, "en", "PG-seeded demo users are English");

    // The upsert branch is a second statement: prove it rewrites rather than skips.
    repos.users.update_locale(alice.id, "fr").await.unwrap();
    bootstrap::run(
        SeedArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        &repos,
    )
    .await
    .unwrap();
    let reseeded = repos
        .users
        .get_by_email("alice@demo.pierre.dev")
        .await
        .unwrap()
        .expect("demo user still there");
    assert_eq!(
        reseeded.locale, "en",
        "the PG re-run rewrites the locale to English"
    );
}
