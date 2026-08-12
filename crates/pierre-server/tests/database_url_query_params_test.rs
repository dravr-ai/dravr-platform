// ABOUTME: A SQLite URL that already carries query parameters must be honoured as-is
// ABOUTME: Guards against re-appending ?mode=rwc and producing an unparseable URL

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `Database::new` appends `?mode=rwc` so SQLite creates the file when missing.
//! It used to append unconditionally, so a caller that spelled out its own
//! parameters got them mangled: `sqlite:x.db?mode=rwc` became
//! `sqlite:x.db?mode=rwc?mode=rwc`, which SQLx rejects with
//! `unknown value "rwc?mode=rwc" for mode`.
//!
//! That is a startup failure, not a query failure — the process dies before
//! serving anything. It took down `CI: TypeScript SDK` on main the moment a
//! workflow set an explicit `?mode=rwc`, because the SDK job boots a real server
//! to generate types against.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_database::database::{generate_encryption_key, Database};

/// The exact shape that broke CI: the caller already asked for `mode=rwc`.
#[tokio::test]
async fn sqlite_url_with_explicit_mode_rwc_connects() {
    let key = generate_encryption_key().to_vec();
    Database::new("sqlite::memory:?mode=rwc", key)
        .await
        .expect("a URL that already specifies mode=rwc must be used as-is");
}

/// Any other caller-supplied parameter must survive too — the rule is "has a
/// query string", not "happens to mention mode".
#[tokio::test]
async fn sqlite_url_with_other_query_params_connects() {
    let key = generate_encryption_key().to_vec();
    Database::new("sqlite::memory:?cache=shared", key)
        .await
        .expect("caller-supplied query parameters must be preserved");
}

/// The original behaviour still holds: no query string means we add the one
/// that makes SQLite create a missing file.
#[tokio::test]
async fn sqlite_url_without_query_params_still_connects() {
    let key = generate_encryption_key().to_vec();
    Database::new("sqlite::memory:", key)
        .await
        .expect("a bare SQLite URL must still connect");
}

// --- Direct coverage of the pure builder -----------------------------------

use pierre_database::backends::shared::connection_url::sqlite_connection_options;

#[test]
fn builder_adds_mode_rwc_only_when_no_query_string() {
    // Bare URL: we add the parameter that makes SQLite create a missing file.
    assert_eq!(
        sqlite_connection_options("sqlite:./x.db"),
        "sqlite:./x.db?mode=rwc"
    );

    // Caller already specified parameters — taken at its word, not appended to.
    assert_eq!(
        sqlite_connection_options("sqlite:./x.db?mode=rwc"),
        "sqlite:./x.db?mode=rwc",
        "must not produce the ?mode=rwc?mode=rwc that SQLx rejects"
    );
    assert_eq!(
        sqlite_connection_options("sqlite:./x.db?cache=shared"),
        "sqlite:./x.db?cache=shared"
    );

    // Non-SQLite URLs pass through untouched.
    assert_eq!(
        sqlite_connection_options("postgres://localhost/db"),
        "postgres://localhost/db"
    );
}
