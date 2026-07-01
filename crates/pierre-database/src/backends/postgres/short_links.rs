// ABOUTME: PostgreSQL-backed ShortLinkRepository for the channel-agnostic URL shortener
// ABOUTME: Mirrors the SQLite impl with PG-native binds ($1.., BIGINT epoch, TEXT target_url)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::ShortLinkRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::postgres::PgRow;
use sqlx::Row;

/// Extract the `target_url` column via `try_get` (never `Row::get`, which is
/// `try_get().unwrap()` and panics on a live PG connection on a type/NULL
/// surprise) so a corrupt row surfaces as a recoverable error, not a crash.
fn pg_target_url(row: &PgRow) -> AppResult<String> {
    row.try_get::<String, _>("target_url")
        .map_err(|e| AppError::database(format!("short_link target_url: {e}")))
}

#[async_trait]
impl ShortLinkRepository for PostgresDatabase {
    async fn create_short_link(
        &self,
        code: &str,
        target_url: &str,
        tenant_id: &str,
        user_id: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO short_links (code, target_url, tenant_id, user_id, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            ",
        )
        .bind(code)
        .bind(target_url)
        .bind(tenant_id)
        .bind(user_id)
        .bind(expires_at.timestamp())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to insert short_link: {e}")))?;

        Ok(())
    }

    async fn resolve_short_link(&self, code: &str) -> AppResult<Option<String>> {
        let row = sqlx::query(
            r"
            SELECT target_url
            FROM short_links
            WHERE code = $1 AND expires_at > $2
            ",
        )
        .bind(code)
        .bind(Utc::now().timestamp())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to read short_link: {e}")))?;

        row.map(|r| pg_target_url(&r)).transpose()
    }

    async fn delete_expired_short_links(&self) -> AppResult<u64> {
        let result = sqlx::query(r"DELETE FROM short_links WHERE expires_at <= $1")
            .bind(Utc::now().timestamp())
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to sweep short_links: {e}")))?;
        Ok(result.rows_affected())
    }
}
