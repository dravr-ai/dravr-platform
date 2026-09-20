// ABOUTME: How each backend stores a list-of-strings column: TEXT[] on Postgres, one text encoding per column on SQLite
// ABOUTME: One codec per backend, handed to a shared repository body so the list bind and decode are written once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The other thing the two backends cannot share: a column that holds a
//! list of strings.
//!
//! Postgres declares `a2a_clients.capabilities`, `a2a_clients.redirect_uris`,
//! `a2a_sessions.granted_scopes`, `a2a_usage.client_capabilities` and
//! `a2a_usage.granted_scopes` as `TEXT[]`, which sqlx binds and reads as a
//! `Vec<String>` natively. `SQLite` has no array type and holds each of
//! those as `TEXT`, in the encoding its rows were written with: a JSON
//! array for the client and usage columns, and a comma-joined string for
//! `a2a_sessions.granted_scopes`. A shared repository body takes one of
//! these codecs as its macro argument and names the encoding each column
//! uses; the statements and the row parsers around it are written once.

use std::fmt::Display;

use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

#[cfg(feature = "postgresql")]
use sqlx::postgres::PgRow;

fn list_column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("Invalid {col} list: {e}"))
}

/// The `SQLite` codec: a list binds as text in the column's encoding and
/// reads back from it.
pub struct TextList;

impl TextList {
    /// Bind a list into a column that holds a JSON array: the array as a
    /// [`Value`], which sqlx stores as its text.
    pub(crate) fn bind_json(list: &[String]) -> Value {
        Value::Array(list.iter().cloned().map(Value::String).collect())
    }

    /// Read a list from a column that holds a JSON array.
    ///
    /// # Errors
    /// Returns a database error when the column is missing, NULL, or does
    /// not hold a JSON array of strings.
    pub(crate) fn read_json(row: &SqliteRow, col: &str) -> AppResult<Vec<String>> {
        let raw: String = row.try_get(col).map_err(|e| list_column_error(col, e))?;
        serde_json::from_str(&raw).map_err(|e| list_column_error(col, e))
    }

    /// Bind a list into a column that holds it comma-joined.
    pub(crate) fn bind_csv(list: &[String]) -> String {
        list.join(",")
    }

    /// Read a list from a column that holds it comma-joined. An empty
    /// column is an empty list, not a list of one empty string.
    ///
    /// # Errors
    /// Returns a database error when the column is missing or NULL.
    pub(crate) fn read_csv(row: &SqliteRow, col: &str) -> AppResult<Vec<String>> {
        let raw: String = row.try_get(col).map_err(|e| list_column_error(col, e))?;
        Ok(raw
            .split(',')
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

/// The Postgres codec: a list binds and reads as the `TEXT[]` the column
/// is, whichever text encoding `SQLite` would use for it.
///
/// Gated with the backend that uses it: without the `postgresql` feature the
/// Postgres shells are not compiled, so neither is their half of the seam.
#[cfg(feature = "postgresql")]
pub struct NativeList;

#[cfg(feature = "postgresql")]
impl NativeList {
    /// Bind a list into a `TEXT[]` column.
    pub(crate) const fn bind_json(list: &[String]) -> &[String] {
        list
    }

    /// Read a list from a `TEXT[]` column.
    ///
    /// # Errors
    /// Returns a database error when the column is missing or NULL.
    pub(crate) fn read_json(row: &PgRow, col: &str) -> AppResult<Vec<String>> {
        row.try_get(col).map_err(|e| list_column_error(col, e))
    }

    /// Bind a list into a `TEXT[]` column.
    pub(crate) const fn bind_csv(list: &[String]) -> &[String] {
        list
    }

    /// Read a list from a `TEXT[]` column.
    ///
    /// # Errors
    /// Returns a database error when the column is missing or NULL.
    pub(crate) fn read_csv(row: &PgRow, col: &str) -> AppResult<Vec<String>> {
        row.try_get(col).map_err(|e| list_column_error(col, e))
    }
}
