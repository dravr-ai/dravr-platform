// ABOUTME: SQLite-backed UserExportRepository: one page of an athlete's rows per call, each row decoded into a JSON object
// ABOUTME: Binary values never leave as bytes, and the columns the table list withholds are dropped before a row is built
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use serde_json::{json, Map, Number, Value};
use sqlx::sqlite::SqliteRow;
use sqlx::{Column, Row, TypeInfo, ValueRef};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::user_export::{
    UserExportPage, UserExportRepository, UserExportTable, USER_EXPORT_KEY_ALIAS_SQL,
};

/// A column whose text is a JSON document, by its declared type or its name.
fn holds_json(declared: &str, name: &str) -> bool {
    declared.contains("JSON") || name.ends_with("_json")
}

/// One column of `row` as JSON. Integers, reals and text keep their type, a
/// column declared boolean reads as one, a column holding a JSON document is
/// parsed, and bytes become their length, never their content.
fn column_value(row: &SqliteRow, index: usize, name: &str) -> AppResult<Value> {
    let raw = row
        .try_get_raw(index)
        .map_err(|e| AppError::database(format!("Failed to read export column {name}: {e}")))?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    let declared = row.column(index).type_info().name().to_ascii_uppercase();
    if declared.contains("BOOL") {
        if let Ok(flag) = row.try_get::<bool, _>(index) {
            return Ok(Value::Bool(flag));
        }
    }
    if let Ok(integer) = row.try_get::<i64, _>(index) {
        return Ok(Value::from(integer));
    }
    if let Ok(real) = row.try_get::<f64, _>(index) {
        return Ok(Number::from_f64(real).map_or(Value::Null, Value::Number));
    }
    if let Ok(text) = row.try_get::<String, _>(index) {
        if holds_json(&declared, name) {
            if let Ok(document) = serde_json::from_str::<Value>(&text) {
                return Ok(document);
            }
        }
        return Ok(Value::String(text));
    }
    if let Ok(bytes) = row.try_get::<Vec<u8>, _>(index) {
        return Ok(json!({ "binary_bytes": bytes.len() }));
    }
    Err(AppError::database(format!(
        "Export column {name} holds a value of no type the export reads"
    )))
}

/// The row as a JSON object of its columns, without those `excluded`.
fn row_to_json(row: &SqliteRow, excluded: &[&str]) -> AppResult<Value> {
    let mut object = Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        let name = column.name();
        if name == USER_EXPORT_KEY_ALIAS_SQL || excluded.contains(&name) {
            continue;
        }
        object.insert(name.to_owned(), column_value(row, index, name)?);
    }
    Ok(Value::Object(object))
}

#[async_trait]
impl UserExportRepository for Database {
    async fn export_page(
        &self,
        table: &UserExportTable,
        user_id: Uuid,
        after: &str,
    ) -> AppResult<UserExportPage> {
        let rows = sqlx::query(table.sqlite_page_sql)
            .bind(user_id.to_string())
            .bind(table.page_rows)
            .bind(after)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to export {}: {e}", table.name)))?;
        let last_key = rows
            .last()
            .map(|row| row.try_get::<String, _>(USER_EXPORT_KEY_ALIAS_SQL))
            .transpose()
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to read the export key of {}: {e}",
                    table.name
                ))
            })?;
        let rows = rows
            .iter()
            .map(|row| row_to_json(row, table.excluded))
            .collect::<AppResult<_>>()?;
        Ok(UserExportPage { rows, last_key })
    }
}
