// ABOUTME: PostgreSQL operations for email-verification tokens — issue, consume, rate-limit, stamp verified
// ABOUTME: Mirrors the SQLite impl with PG-native binds (UUID user_id, TIMESTAMPTZ, INTEGER as i32)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::backends::postgres::PostgresDatabase;
use crate::database::email_verification_tokens::VERIFY_MAX_ATTEMPTS;
use crate::repositories::EmailVerificationRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

#[async_trait]
impl EmailVerificationRepository for PostgresDatabase {
    async fn store_token(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::minutes(ttl_minutes);

        // `id` is TEXT and `user_id` is native UUID here — mirroring
        // password_reset_tokens. Binding a stringified uuid into the UUID column
        // is the recurring decode failure, so the Uuid goes in unwrapped.
        sqlx::query(
            r"
            INSERT INTO email_verification_tokens (id, user_id, selector, token_hash, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(id.to_string())
        .bind(user_id)
        .bind(selector)
        .bind(verifier_hash)
        .bind(expires_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store verification token: {e}")))?;

        Ok(id)
    }

    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid> {
        // One uniform error for every failure mode so the endpoint never reveals
        // which condition hit.
        let invalid =
            || AppError::not_found("Verification link is invalid, expired, or already used");
        let now = Utc::now();

        let row = sqlx::query(
            r"
            SELECT user_id, token_hash, attempt_count
            FROM email_verification_tokens
            WHERE selector = $1
              AND used_at IS NULL
              AND expires_at > $2
            ",
        )
        .bind(selector)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to load verification token: {e}")))?;

        let Some(row) = row else {
            return Err(invalid());
        };
        let user_id: Uuid = row.get("user_id");
        let stored_hash: String = row.get("token_hash");
        // PG INTEGER maps to i32.
        let attempts: i32 = row.get("attempt_count");

        // Brute-force lockout: past the cap, invalidate the token outright.
        if i64::from(attempts) >= VERIFY_MAX_ATTEMPTS {
            sqlx::query("UPDATE email_verification_tokens SET used_at = $1 WHERE selector = $2")
                .bind(now)
                .bind(selector)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to lock verification token: {e}"))
                })?;
            return Err(invalid());
        }

        // Comparing SHA-256 hashes, not the secret. A wrong guess costs one
        // attempt and does NOT consume the token.
        if stored_hash != verifier_hash {
            sqlx::query(
                "UPDATE email_verification_tokens SET attempt_count = attempt_count + 1 WHERE selector = $1",
            )
            .bind(selector)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to record verification attempt: {e}")))?;
            return Err(invalid());
        }

        // Claim it exactly once: a concurrent request holding the same live row
        // affects 0 rows here and is rejected, preventing replay.
        let claimed = sqlx::query(
            "UPDATE email_verification_tokens SET used_at = $1 WHERE selector = $2 AND used_at IS NULL",
        )
        .bind(now)
        .bind(selector)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to consume verification token: {e}")))?;

        if claimed.rows_affected() == 0 {
            return Err(invalid());
        }

        Ok(user_id)
    }

    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as cnt
            FROM email_verification_tokens
            WHERE user_id = $1
              AND created_at >= $2
            ",
        )
        .bind(user_id)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to count recent verification tokens: {e}"))
        })?;

        Ok(row.get::<i64, _>("cnt"))
    }

    async fn mark_verified(&self, user_id: Uuid) -> AppResult<()> {
        // `IS NULL` keeps the first proof's timestamp — re-verifying must not
        // rewrite when the address was actually proven.
        sqlx::query(
            "UPDATE users SET email_verified_at = $1 WHERE id = $2 AND email_verified_at IS NULL",
        )
        .bind(Utc::now())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark email verified: {e}")))?;

        Ok(())
    }

    async fn is_verified(&self, user_id: Uuid) -> AppResult<bool> {
        let row = sqlx::query("SELECT email_verified_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to read verification state: {e}")))?;

        Ok(row
            .and_then(|r| r.get::<Option<DateTime<Utc>>, _>("email_verified_at"))
            .is_some())
    }
}
