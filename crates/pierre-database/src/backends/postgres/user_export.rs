// ABOUTME: PostgreSQL-backed UserExportRepository: one page of an athlete's rows per call, each row built by the database as jsonb
// ABOUTME: The columns the table list withholds are removed in the statement itself, so they never reach the server process
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::user_export::{
    UserExportPage, UserExportRepository, UserExportTable, USER_EXPORT_KEY_ALIAS_SQL,
};

#[async_trait]
impl UserExportRepository for PostgresDatabase {
    async fn export_page(
        &self,
        table: &UserExportTable,
        user_id: Uuid,
        after: &str,
    ) -> AppResult<UserExportPage> {
        let excluded: Vec<String> = table.excluded.iter().map(|c| (*c).to_owned()).collect();
        let rows = sqlx::query(table.postgres_page_sql)
            .bind(user_id.to_string())
            .bind(table.page_rows)
            .bind(after)
            .bind(excluded)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to export {}: {e}", table.name)))?;
        let decode_error = |e: sqlx::Error| {
            AppError::database(format!(
                "Failed to decode an export row of {}: {e}",
                table.name
            ))
        };
        let last_key = rows
            .last()
            .map(|row| row.try_get::<String, _>(USER_EXPORT_KEY_ALIAS_SQL))
            .transpose()
            .map_err(decode_error)?;
        let rows = rows
            .iter()
            .map(|row| row.try_get::<Value, _>("row").map_err(decode_error))
            .collect::<AppResult<_>>()?;
        Ok(UserExportPage { rows, last_key })
    }
}
