// ABOUTME: Builds the SQLite connection string, preserving caller-supplied query params
// ABOUTME: Pure string logic, kept out of the connection path so it can be tested directly

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Build the SQLite connection string, adding `mode=rwc` so SQLite creates a
/// missing file.
///
/// Only added when the caller supplied no query string of its own. Appending
/// unconditionally turned a URL that already carried `?mode=rwc` into
/// `...?mode=rwc?mode=rwc`, which SQLx rejects with
/// `unknown value "rwc?mode=rwc" for mode` — a startup failure, not a query
/// failure, so the process dies before serving anything. A caller that spells
/// out its own parameters is taken at its word.
///
/// Non-SQLite URLs are returned unchanged.
#[must_use]
pub fn sqlite_connection_options(database_url: &str) -> String {
    if database_url.starts_with("sqlite:") && !database_url.contains('?') {
        format!("{database_url}?mode=rwc")
    } else {
        database_url.to_owned()
    }
}
