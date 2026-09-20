// ABOUTME: Parses the shared agent seeder SQL and checks columns/values/placeholders/binds agree
// ABOUTME: A column added to one clause but not another is invisible until a live seed run
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

//! Why this reads source text instead of exercising the seeder.
//!
//! `seed_insert_agent` / `seed_update_agent` have no test callers and need a
//! live database to run, so a widened column list can disagree with its VALUES
//! clause and nothing notices until a real seed against `PostgreSQL` — which
//! gates the deploy. That happened on 2026-08-14: `visuals` was added to the PG
//! column list while the VALUES clause kept `$26` as its last placeholder,
//! giving 29 columns against 28 values. The two backends were written twice
//! then; both statements now live once in `repositories/seeder.rs` and the
//! one bind chain that executes each in `repositories/seeder_body.rs`, so
//! this reads those two files and checks each statement against its chain.
//!
//! Counting the two statements mechanically costs nothing and fails at compile
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

/// Extract the statement a `const` holds, from its opening quote to the
/// `";` that closes the const.
fn statement(source: &str, const_name: &str) -> String {
    let decl = format!("const {const_name}: &str = \"");
    let start = source
        .find(&decl)
        .unwrap_or_else(|| panic!("const {const_name} not found"))
        + decl.len();
    let rest = &source[start..];
    let end = rest
        .find("\";")
        .unwrap_or_else(|| panic!("unterminated statement in {const_name}"));
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

/// Count `.bind(` calls between `sqlx::query(<const>)` and the following
/// `.execute(`: the chain that executes the statement.
fn binds_for(source: &str, const_name: &str) -> usize {
    let anchor = format!("sqlx::query({const_name})");
    let start = source.find(&anchor).expect("query anchor not found");
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

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The one file the statements are declared in.
fn statements() -> String {
    read("src/repositories/seeder.rs")
}

/// The one file both backends' seeder bodies are emitted from.
fn body() -> String {
    read("src/repositories/seeder_body.rs")
}

#[test]
fn agent_insert_statement_is_internally_consistent() {
    let stmt = statement(&statements(), "INSERT_AGENT_SQL");

    let columns = count_group(&stmt, "INSERT INTO agents");
    let values = count_group(&stmt, "VALUES");
    let (max_param, contiguous) = placeholders(&stmt);
    let binds = binds_for(&body(), "INSERT_AGENT_SQL");

    assert_eq!(
        columns, values,
        "INSERT names {columns} columns but supplies {values} values"
    );
    assert!(contiguous, "INSERT placeholders are not $1..=$N");
    assert_eq!(
        binds, max_param,
        "INSERT has {binds} binds for {max_param} placeholders"
    );
}

#[test]
fn agent_update_statement_is_internally_consistent() {
    let stmt = statement(&statements(), "UPDATE_AGENT_SQL");
    let (max_param, contiguous) = placeholders(&stmt);
    let binds = binds_for(&body(), "UPDATE_AGENT_SQL");

    assert!(contiguous, "UPDATE placeholders are not $1..=$N");
    assert_eq!(
        binds, max_param,
        "UPDATE has {binds} binds for {max_param} placeholders"
    );
    // The WHERE key must be its own placeholder. Reusing an assignment's
    // number silently matches on the wrong value — the $22/$23 collision
    // this change introduced once already.
    assert!(
        stmt.contains(&format!("WHERE id = ${max_param}")),
        "UPDATE must key on the last placeholder, not reuse an assignment's"
    );
}
