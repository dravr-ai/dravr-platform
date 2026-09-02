// ABOUTME: Parses the coach seeder SQL and checks columns/values/placeholders/binds agree
// ABOUTME: A column added to one clause but not another is invisible until a live seed run
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

//! Why this reads source text instead of exercising the seeder.
//!
//! `seed_insert_coach` / `seed_update_coach` have no test callers and need a
//! live database to run, so a widened column list can disagree with its VALUES
//! clause and nothing notices until a real seed against `PostgreSQL` — which
//! gates the deploy. That happened on 2026-08-14: `visuals` was added to the PG
//! column list while the VALUES clause kept `$26` as its last placeholder,
//! giving 29 columns against 28 values. The `SQLite` twin was correct, so the
//! two backends silently disagreed.
//!
//! Counting the four statements mechanically costs nothing and fails at compile
//! time rather than at deploy time.

use std::fs;
use std::path::Path;

/// Collapse Rust string-literal line continuations so a statement is one line.
fn flatten(sql: &str) -> String {
    sql.replace("\\\n", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract the first statement starting at `needle` up to the closing quote.
fn statement(source: &str, needle: &str) -> String {
    let start = source
        .find(needle)
        .unwrap_or_else(|| panic!("statement starting {needle:?} not found"));
    let rest = &source[start..];
    let end = rest
        .find("\",")
        .unwrap_or_else(|| panic!("unterminated statement at {needle:?}"));
    flatten(&rest[..end])
}

/// Highest `$N` placeholder in a statement, and whether `$1..=N` are all used.
fn placeholders(stmt: &str) -> (usize, bool) {
    let mut seen: Vec<usize> = stmt
        .split('$')
        .skip(1)
        .filter_map(|tail| {
            let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .collect();
    seen.sort_unstable();
    seen.dedup();
    let max = seen.last().copied().unwrap_or(0);
    (max, seen == (1..=max).collect::<Vec<_>>())
}

/// Count `.bind(` calls between `from` and the following `.execute(`.
fn binds_after(source: &str, from: &str) -> usize {
    let start = source.find(from).expect("anchor not found");
    let rest = &source[start..];
    let end = rest.find(".execute(").expect("no .execute after statement");
    rest[..end].matches(".bind(").count()
}

/// Comma-separated items inside the first parenthesised group after `after`.
fn count_group(stmt: &str, after: &str) -> usize {
    let tail = &stmt[stmt.find(after).expect("marker not found") + after.len()..];
    let open = tail.find('(').expect("no open paren");
    let close = tail.find(')').expect("no close paren");
    tail[open + 1..close]
        .split(',')
        .filter(|item| !item.trim().is_empty())
        .count()
}

fn source(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn coach_insert_statements_are_internally_consistent() {
    for rel in ["src/database/seeder.rs", "src/backends/postgres/seeder.rs"] {
        let src = source(rel);
        let stmt = statement(&src, "INSERT INTO coaches");

        let columns = count_group(&stmt, "INSERT INTO coaches");
        // `Values` in one backend, `VALUES` in the other — the casing difference
        // is exactly what let a search-and-replace miss one of them.
        let marker = if stmt.contains("VALUES (") {
            "VALUES"
        } else {
            "Values"
        };
        let values = count_group(&stmt, marker);
        let (max_param, contiguous) = placeholders(&stmt);
        let binds = binds_after(&src, "INSERT INTO coaches");

        assert_eq!(
            columns, values,
            "{rel}: INSERT names {columns} columns but supplies {values} values"
        );
        assert!(contiguous, "{rel}: INSERT placeholders are not $1..=$N");
        assert_eq!(
            binds, max_param,
            "{rel}: INSERT has {binds} binds for {max_param} placeholders"
        );
    }
}

#[test]
fn coach_update_statements_are_internally_consistent() {
    for rel in ["src/database/seeder.rs", "src/backends/postgres/seeder.rs"] {
        let src = source(rel);
        // The coach update is the statement whose SET list starts on the next
        // source line; `seed_take_catalogue_ownership` also opens with
        // `UPDATE coaches SET`, on one line, and is not the one under test.
        let needle = "UPDATE coaches SET \\";
        let stmt = statement(&src, needle);
        let (max_param, contiguous) = placeholders(&stmt);
        let binds = binds_after(&src, needle);

        assert!(contiguous, "{rel}: UPDATE placeholders are not $1..=$N");
        assert_eq!(
            binds, max_param,
            "{rel}: UPDATE has {binds} binds for {max_param} placeholders"
        );
        // The WHERE key must be its own placeholder. Reusing an assignment's
        // number silently matches on the wrong value — the $22/$23 collision
        // this change introduced once already.
        assert!(
            stmt.contains(&format!("WHERE id = ${max_param}")),
            "{rel}: UPDATE must key on the last placeholder, not reuse an assignment's"
        );
    }
}

#[test]
fn both_backends_write_the_same_coach_columns() {
    let sqlite = statement(&source("src/database/seeder.rs"), "INSERT INTO coaches");
    let postgres = statement(
        &source("src/backends/postgres/seeder.rs"),
        "INSERT INTO coaches",
    );

    let cols = |stmt: &str| -> Vec<String> {
        let tail = &stmt[stmt.find("INSERT INTO coaches").unwrap() + 19..];
        let open = tail.find('(').unwrap();
        let close = tail.find(')').unwrap();
        tail[open + 1..close]
            .split(',')
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty())
            .collect()
    };

    assert_eq!(
        cols(&sqlite),
        cols(&postgres),
        "the two backends must seed the same coach columns in the same order"
    );
}
