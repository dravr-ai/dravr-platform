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
use sqlx::Row;

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

        Ok(row.map(|r| r.get::<String, _>("target_url")))
    }
}
