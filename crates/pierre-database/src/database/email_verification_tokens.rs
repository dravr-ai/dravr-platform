// ABOUTME: SQLite operations for email-verification tokens — issue, consume, rate-limit, stamp verified
// ABOUTME: Mirrors password_reset_tokens' selector/verifier mechanism against a separate token space

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::EmailVerificationRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

/// Max wrong verifier guesses against one verification token before it
/// self-invalidates. Same posture as the reset flow's lockout (CWE-307).
pub(crate) const VERIFY_MAX_ATTEMPTS: i64 = 5;

#[async_trait]
impl EmailVerificationRepository for Database {
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

        sqlx::query(
            r"
            INSERT INTO email_verification_tokens (id, user_id, selector, token_hash, expires_at, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(selector)
        .bind(verifier_hash)
        .bind(expires_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to store verification token: {e}")))?;

        Ok(id)
    }

    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid> {
        // One uniform error for every failure mode (unknown selector, expired,
        // wrong verifier, locked out) so the endpoint never reveals which hit.
        let invalid =
            || AppError::not_found("Verification link is invalid, expired, or already used");
        let now = Utc::now().to_rfc3339();

        let row = sqlx::query(
            r"
            SELECT user_id, token_hash, attempt_count
            FROM email_verification_tokens
            WHERE selector = ?1
              AND used_at IS NULL
              AND expires_at > ?2
            ",
        )
        .bind(selector)
        .bind(&now)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to load verification token: {e}")))?;

        let Some(row) = row else {
            return Err(invalid());
        };
        let user_id_str: String = row.get("user_id");
        let stored_hash: String = row.get("token_hash");
        let attempts: i64 = row.get("attempt_count");

        // Brute-force lockout: past the cap, invalidate the token outright.
        if attempts >= VERIFY_MAX_ATTEMPTS {
            sqlx::query("UPDATE email_verification_tokens SET used_at = ?1 WHERE selector = ?2")
                .bind(&now)
                .bind(selector)
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to lock verification token: {e}"))
                })?;
            return Err(invalid());
        }

        // Comparing SHA-256 hashes, not the secret: a timing side-channel on the
        // hash cannot forge the verifier without a preimage. A wrong guess costs
        // one attempt and does NOT consume the token.
        if stored_hash != verifier_hash {
            sqlx::query(
                "UPDATE email_verification_tokens SET attempt_count = attempt_count + 1 WHERE selector = ?1",
            )
            .bind(selector)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to record verification attempt: {e}")))?;
            return Err(invalid());
        }

        // Claim it exactly once: only the request that flips NULL->now affects a
        // row, so a concurrent request holding the same live row loses here.
        let claimed = sqlx::query(
            "UPDATE email_verification_tokens SET used_at = ?1 WHERE selector = ?2 AND used_at IS NULL",
        )
        .bind(&now)
        .bind(selector)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to consume verification token: {e}")))?;

        if claimed.rows_affected() == 0 {
            return Err(invalid());
        }

        Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::internal(format!("Invalid user_id in verification token: {e}")))
    }

    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as cnt
            FROM email_verification_tokens
            WHERE user_id = ?1
              AND created_at >= ?2
            ",
        )
        .bind(user_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_one(self.pool())
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to count recent verification tokens: {e}"))
        })?;

        Ok(row.get::<i64, _>("cnt"))
    }

    async fn mark_verified(&self, user_id: Uuid) -> AppResult<()> {
        // `IS NULL` keeps the first proof's timestamp — re-verifying later (a
        // second link from the same inbox) must not rewrite when it happened.
        sqlx::query(
            "UPDATE users SET email_verified_at = ?1 WHERE id = ?2 AND email_verified_at IS NULL",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to mark email verified: {e}")))?;

        Ok(())
    }

    async fn is_verified(&self, user_id: Uuid) -> AppResult<bool> {
        let row = sqlx::query("SELECT email_verified_at FROM users WHERE id = ?1")
            .bind(user_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to read verification state: {e}")))?;

        Ok(row
            .and_then(|r| r.get::<Option<String>, _>("email_verified_at"))
            .is_some())
    }
}
