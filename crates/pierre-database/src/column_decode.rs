// ABOUTME: Shared decoders for identifier and timestamp TEXT columns
// ABOUTME: Every one fails closed — a value that will not parse is never defaulted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Column decoders that fail closed.
//!
//! A stored value that will not parse is a data-integrity fault. Substituting a
//! default turns it into something worse than an error: a plausible value. A nil
//! UUID is a *wrong* identity rather than a missing one — it silently matches
//! nothing, or matches another row that also defaulted — and a timestamp
//! defaulted to `Utc::now()` is indistinguishable from a real reading.
//!
//! These live here rather than in each backend because `SQLite` and `Postgres`
//! decode the same columns and must agree on what a bad one means.

use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

/// Decode a UUID stored in a TEXT column, naming the column on failure.
///
/// # Errors
/// Returns a database error when `raw` is not a valid UUID.
pub fn uuid_column(table_and_column: &str, raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw)
        .map_err(|e| AppError::database(format!("{table_and_column} is not a valid UUID: {e}")))
}
