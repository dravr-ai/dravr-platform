// ABOUTME: Pure drift-classification logic shared by pierre-cli check-drift coaches
// ABOUTME: I/O-free comparison primitives so unit tests can drive every branch without a DB
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach Drift Classification
//!
//! Pure functions and value types consumed by the `pierre-cli check-drift
//! coaches` binary subcommand. Lives in the library (not the binary) so the
//! unit tests can sit in `crates/pierre-server/tests/` per the workspace
//! convention that forbids inline test modules under `src/` — the binary
//! itself stays free of test scaffolding.
//!
//! Background: on 2026-05-25 we discovered the prod `dravr-mcp-server-seed-coaches`
//! Cloud Run job had been silently exit-1 for 3.5 weeks. The drift sub-command
//! and the Cloud Run job that runs it daily exist to make sure the next
//! contremaitre→DB regression surfaces within 24 hours, not 24 days.

/// Snapshot of the `(source, content_hash)` columns for one coach row.
///
/// `content_hash` is `None` when the column is NULL (pre-2026-04 rows
/// seeded before content hashing landed); `source` defaults to
/// `'custom'` per migration `20260401000001_coach_source_column.sql`,
/// so it is always present.
#[derive(Debug, Clone)]
pub struct DbCoachRow {
    /// The `coaches.source` column value: `'contremaitre'`, `'seed'`, or `'custom'`.
    pub source: String,
    /// The `coaches.content_hash` column value, NULL on pre-2026-04 rows.
    pub content_hash: Option<String>,
}

/// Outcome classifier for one contremaitre file vs the matching DB row.
#[derive(Debug, PartialEq, Eq)]
pub enum DriftOutcome {
    /// Hashes match (or both are unset). DB row stays in sync with the file.
    InSync,
    /// DB row has `source = 'contremaitre'` but a different `content_hash`.
    /// This is the failure mode: contremaitre edits never reached prod.
    HashDrift {
        /// The hash recomputed from the contremaitre file.
        expected: String,
        /// The hash currently stored in the DB (or `None` on pre-2026-04 rows).
        actual: Option<String>,
    },
    /// DB row exists but `source` is something other than `'contremaitre'`
    /// (e.g. transitional `'seed'`). The contremaitre file isn't authoritative
    /// for that coach yet.
    SourceMismatch {
        /// The unexpected `coaches.source` value.
        actual_source: String,
    },
    /// No DB row at all for this contremaitre slug.
    Missing,
}

/// Compare one contremaitre file's hash to a DB row snapshot.
///
/// Pure function — no I/O — so unit tests can drive every branch without
/// a database. The semantics match the docstring on
/// [`crate::bin::pierre_cli`]'s drift sub-command (and on the inline
/// module-level docs in that binary): only `source = 'contremaitre'`
/// rows are subject to hash-drift checks; everything else is a warning,
/// not an error.
#[must_use]
pub fn classify_drift(file_hash: &str, db_row: Option<&DbCoachRow>) -> DriftOutcome {
    let Some(row) = db_row else {
        return DriftOutcome::Missing;
    };
    if row.source != "contremaitre" {
        return DriftOutcome::SourceMismatch {
            actual_source: row.source.clone(),
        };
    }
    if row.content_hash.as_deref() == Some(file_hash) {
        DriftOutcome::InSync
    } else {
        DriftOutcome::HashDrift {
            expected: file_hash.to_owned(),
            actual: row.content_hash.clone(),
        }
    }
}
