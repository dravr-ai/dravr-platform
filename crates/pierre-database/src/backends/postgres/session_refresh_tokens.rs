// ABOUTME: PostgreSQL operations for first-party refresh tokens — store, exchange once, revoke a family or a user
// ABOUTME: Mirrors the SQLite impl with PG-native binds (UUID user_id, TIMESTAMPTZ)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::backends::postgres::PostgresDatabase;
use crate::backends::shared::encryption::HasEncryption;
use crate::repositories::SessionRefreshTokenRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::SessionRefreshToken;
use sqlx::Row;
use uuid::Uuid;

#[async_trait]
impl SessionRefreshTokenRepository for PostgresDatabase {
    async fn store_token(&self, token: &str, record: &SessionRefreshToken) -> AppResult<()> {
        let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

        // `user_id` is native UUID here, so the Uuid goes in unwrapped —
        // binding its string form is the recurring PG decode failure.
        sqlx::query(
            r"
            INSERT INTO session_refresh_tokens
                (token_hash, family_id, user_id, tenant_id, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(&token_hash)
        .bind(&record.family_id)
        .bind(record.user_id)
        .bind(&record.tenant_id)
        .bind(record.created_at)
        .bind(record.expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store session refresh token: {e}")))?;

        Ok(())
    }

    async fn consume_token(
        &self,
        token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<SessionRefreshToken>> {
        let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

        // Check-and-revoke in one statement: a second exchange of the same
        // token, even a concurrent one, matches zero rows.
        let row = sqlx::query(
            r"
            UPDATE session_refresh_tokens
            SET revoked_at = $2
            WHERE token_hash = $1
              AND revoked_at IS NULL
              AND expires_at > $2
            RETURNING family_id, user_id, tenant_id, created_at, expires_at
            ",
        )
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to consume session refresh token: {e}")))?;

        row.map(|row| {
            Ok(SessionRefreshToken {
                family_id: row
                    .try_get("family_id")
                    .map_err(|e| AppError::database(format!("Failed to get family_id: {e}")))?,
                user_id: row
                    .try_get("user_id")
                    .map_err(|e| AppError::database(format!("Failed to get user_id: {e}")))?,
                tenant_id: row
                    .try_get("tenant_id")
                    .map_err(|e| AppError::database(format!("Failed to get tenant_id: {e}")))?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| AppError::database(format!("Failed to get created_at: {e}")))?,
                expires_at: row
                    .try_get("expires_at")
                    .map_err(|e| AppError::database(format!("Failed to get expires_at: {e}")))?,
            })
        })
        .transpose()
    }

    async fn revoke_token_family(&self, token: &str, now: DateTime<Utc>) -> AppResult<u64> {
        let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

        let result = sqlx::query(
            r"
            UPDATE session_refresh_tokens
            SET revoked_at = $2
            WHERE revoked_at IS NULL
              AND family_id = (
                  SELECT family_id FROM session_refresh_tokens WHERE token_hash = $1
              )
            ",
        )
        .bind(&token_hash)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to revoke session refresh token family: {e}"
            ))
        })?;

        Ok(result.rows_affected())
    }

    async fn revoke_user_tokens(&self, user_id: Uuid, now: DateTime<Utc>) -> AppResult<u64> {
        let result = sqlx::query(
            r"
            UPDATE session_refresh_tokens
            SET revoked_at = $2
            WHERE user_id = $1 AND revoked_at IS NULL
            ",
        )
        .bind(user_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to revoke user session refresh tokens: {e}"))
        })?;

        Ok(result.rows_affected())
    }
}
