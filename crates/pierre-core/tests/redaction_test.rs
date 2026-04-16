// ABOUTME: Regression tests for URL credential redaction
// ABOUTME: Pins the exact production case that leaked a DB URL on 2026-04-16
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_core::redaction::redact_url;

#[test]
fn redacts_postgres_url_with_url_encoded_password() {
    // The exact shape that leaked in Cloud Run prod logs on 2026-04-16:
    // password contains % (URL-encoded), cloudsql host param attached as query string.
    let input =
        "postgresql://dravr:5EEP4Vw%25OtzZ@localhost/dravr?host=/cloudsql/dravr-dev:region:instance";
    let out = redact_url(input);

    assert!(
        !out.contains("5EEP4Vw"),
        "raw password must not appear in redacted URL: {out}"
    );
    assert!(
        !out.contains("%25OtzZ"),
        "URL-encoded password suffix must not appear: {out}"
    );
    assert!(
        out.contains("***:***@localhost"),
        "host and redaction marker must be preserved: {out}"
    );
    assert!(
        out.contains("?host=/cloudsql/"),
        "cloudsql query param must be preserved for ops visibility: {out}"
    );
}

#[test]
fn leaves_sqlite_paths_unchanged() {
    assert_eq!(
        redact_url("sqlite:./data/users.db"),
        "sqlite:./data/users.db"
    );
}

#[test]
fn redacts_when_username_is_empty() {
    // Some postgres URLs omit the user: postgres://:password@host
    let out = redact_url("postgres://:only_password@host:5432/db");
    assert!(!out.contains("only_password"), "got: {out}");
    assert!(out.contains("@host:5432/db"), "got: {out}");
}

#[test]
fn leaves_postgres_url_without_credentials_unchanged() {
    let input = "postgres://localhost:5432/dravr";
    assert_eq!(redact_url(input), input);
}
