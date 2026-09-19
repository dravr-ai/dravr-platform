// ABOUTME: How each backend stores a uuid column: hyphenated TEXT on SQLite, a native uuid on Postgres
// ABOUTME: One codec per backend, handed to a shared repository body so the id bind and decode are written once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one thing the two backends cannot share on a uuid column.
//!
//! Postgres declares `users.id` and every column that references it as
//! `uuid`; `SQLite` declares them `TEXT` holding the hyphenated form. sqlx
//! cannot paper over that: on `SQLite` a bare [`Uuid`] encodes as a 16-byte
//! BLOB and decodes from one, which is not what the TEXT rows hold, and on
//! Postgres a `String` bound against a `uuid` column is a type error. So a
//! shared repository body takes one of these codecs as its macro argument
//! and spells every id bind and read through it; the statements, the row
//! parser and the error strings around it are written once.

use std::fmt::Display;

use pierre_core::errors::{AppError, AppResult};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

#[cfg(feature = "postgresql")]
use sqlx::postgres::PgRow;

fn uuid_column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("Invalid {col} uuid: {e}"))
}

/// The `SQLite` codec: a uuid binds as its hyphenated text and reads back
/// from a `TEXT` column, matching the schema those tables declare there.
pub struct TextUuid;

impl TextUuid {
    /// Bind an id the way `SQLite` stores it.
    pub(crate) fn bind(id: Uuid) -> String {
        id.to_string()
    }

    /// Bind an optional id; `None` binds as SQL NULL.
    pub(crate) fn bind_opt(id: Option<Uuid>) -> Option<String> {
        id.map(|u| u.to_string())
    }

    /// Read a NOT NULL uuid column.
    ///
    /// # Errors
    /// Returns a database error when the column is missing, NULL, or not a uuid.
    pub(crate) fn read(row: &SqliteRow, col: &str) -> AppResult<Uuid> {
        let raw: String = row.try_get(col).map_err(|e| uuid_column_error(col, e))?;
        Uuid::parse_str(&raw).map_err(|e| uuid_column_error(col, e))
    }

    /// Read a nullable uuid column.
    ///
    /// # Errors
    /// Returns a database error when the column is missing or holds text that
    /// is not a uuid.
    pub(crate) fn read_opt(row: &SqliteRow, col: &str) -> AppResult<Option<Uuid>> {
        let raw: Option<String> = row.try_get(col).map_err(|e| uuid_column_error(col, e))?;
        raw.map(|s| Uuid::parse_str(&s).map_err(|e| uuid_column_error(col, e)))
            .transpose()
    }
}

/// The Postgres codec: a uuid binds natively into the `uuid` column and
/// reads back as one, with no textual round-trip.
///
/// Gated with the backend that uses it: without the `postgresql` feature the
/// Postgres shells are not compiled, so neither is their half of the seam.
#[cfg(feature = "postgresql")]
pub struct NativeUuid;

#[cfg(feature = "postgresql")]
impl NativeUuid {
    /// Bind an id the way Postgres stores it.
    pub(crate) const fn bind(id: Uuid) -> Uuid {
        id
    }

    /// Bind an optional id; `None` binds as SQL NULL.
    pub(crate) const fn bind_opt(id: Option<Uuid>) -> Option<Uuid> {
        id
    }

    /// Read a NOT NULL uuid column.
    ///
    /// # Errors
    /// Returns a database error when the column is missing, NULL, or not a uuid.
    pub(crate) fn read(row: &PgRow, col: &str) -> AppResult<Uuid> {
        row.try_get(col).map_err(|e| uuid_column_error(col, e))
    }

    /// Read a nullable uuid column.
    ///
    /// # Errors
    /// Returns a database error when the column is missing or not a uuid.
    pub(crate) fn read_opt(row: &PgRow, col: &str) -> AppResult<Option<Uuid>> {
        row.try_get(col).map_err(|e| uuid_column_error(col, e))
    }
}
