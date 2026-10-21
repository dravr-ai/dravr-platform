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

#[test]
fn redacts_credential_query_parameters_case_insensitively() {
    let out = redact_url(
        "https://api.example.com/v1/foods?query=apple&API_KEY=k-1&Access_Token=a-2\
         &refresh_token=r-3&client_secret=s-4&password=p-5&token=t-6&key=k-7&secret=s-8&pageSize=10",
    );
    assert_eq!(
        out,
        "https://api.example.com/v1/foods?query=apple&API_KEY=***&Access_Token=***\
         &refresh_token=***&client_secret=***&password=***&token=***&key=***&secret=***&pageSize=10"
    );
}

#[test]
fn redacts_query_credentials_beside_userinfo_and_keeps_the_fragment() {
    assert_eq!(
        redact_url("postgres://u:pw@db:5432/app?sslmode=require&X-Api-Key=x9#frag"),
        "postgres://***:***@db:5432/app?sslmode=require&X-Api-Key=***#frag"
    );
    assert_eq!(
        redact_url("postgres://db/app?password=hunter2#frag"),
        "postgres://db/app?password=***#frag"
    );
}

#[test]
fn keeps_parameters_that_only_contain_a_sensitive_word() {
    let input = "https://example.com/search?keyword=run&tokenizer=bpe&flag&monkey=1";
    assert_eq!(redact_url(input), input);
}

#[test]
fn an_address_in_the_query_or_path_is_not_userinfo() {
    let input = "https://example.com/users/a@b.example?email=c@d.example";
    assert_eq!(redact_url(input), input);
}

#[test]
fn a_password_holding_a_question_mark_or_hash_is_still_redacted() {
    let out = redact_url("postgres://dravr:pa?ss#wd@db:5432/app?sslmode=require");
    assert_eq!(out, "postgres://***:***@db:5432/app?sslmode=require");
    let out = redact_url("postgres://dravr:p#w@db/app#frag");
    assert_eq!(out, "postgres://***:***@db/app#frag");
}
