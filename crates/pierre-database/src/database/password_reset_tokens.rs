// ABOUTME: Database operations for password reset tokens
// ABOUTME: CRUD methods for one-time password reset tokens issued by admins
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::PasswordResetRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

/// Duration before a password reset token expires (1 hour)
const RESET_TOKEN_TTL_HOURS: i64 = 1;

/// Max wrong verifier guesses against one reset token before it self-invalidates
/// (per-token brute-force lockout). Shared by both backend consume paths.
pub(crate) const RESET_MAX_VERIFY_ATTEMPTS: i64 = 5;

impl Database {
    /// Store a password reset token
    ///
    /// The `token_hash` should be a SHA-256 hash of the raw token — the raw token
    /// is returned to the admin and never stored in the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn store_password_reset_token_impl(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(RESET_TOKEN_TTL_HOURS);

        sqlx::query(
            r"
            INSERT INTO password_reset_tokens (id, user_id, selector, token_hash, expires_at, created_by, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(selector)
        .bind(verifier_hash)
        .bind(expires_at.to_rfc3339())
        .bind(created_by)
        .bind(now.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to store password reset token: {e}")))?;

        Ok(id)
    }

    /// Consume a password reset token by its hash
    ///
    /// Returns the `user_id` if the token is valid: exists, not expired, and not yet used.
    /// Marks the token as used atomically to prevent replay.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` if the token doesn't exist or is already used/expired.
    pub async fn consume_password_reset_token_impl(
        &self,
        selector: &str,
        verifier_hash: &str,
    ) -> AppResult<Uuid> {
        // Uniform error for every failure mode (missing selector, expired, wrong
        // verifier, locked out) so the endpoint never reveals which condition hit.
        let invalid =
            || AppError::not_found("Password reset token is invalid, expired, or already used");
        let now = Utc::now().to_rfc3339();

        // Look up the single live token for this selector — no global token_hash scan.
        let row = sqlx::query(
            r"
            SELECT user_id, token_hash, attempt_count
            FROM password_reset_tokens
            WHERE selector = ?1
              AND used_at IS NULL
              AND expires_at > ?2
            ",
        )
        .bind(selector)
        .bind(&now)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to load reset token: {e}")))?;

        let Some(row) = row else {
            return Err(invalid());
        };
        let user_id_str: String = row.get("user_id");
        let stored_hash: String = row.get("token_hash");
        let attempts: i64 = row.get("attempt_count");

        // Brute-force lockout: past the attempt cap, invalidate the token outright.
        if attempts >= RESET_MAX_VERIFY_ATTEMPTS {
            sqlx::query("UPDATE password_reset_tokens SET used_at = ?1 WHERE selector = ?2")
                .bind(&now)
                .bind(selector)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to lock reset token: {e}")))?;
            return Err(invalid());
        }

        // Compare SHA-256 hashes (not the secret itself): a timing side-channel on the
        // hash cannot help forge the verifier without a preimage, so a plain compare is
        // safe here. A wrong guess costs one attempt and does NOT consume the token.
        if stored_hash != verifier_hash {
            sqlx::query(
                "UPDATE password_reset_tokens SET attempt_count = attempt_count + 1 WHERE selector = ?1",
            )
            .bind(selector)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to record reset attempt: {e}")))?;
            return Err(invalid());
        }

        // Success: atomically claim the token. The `used_at IS NULL` guard makes this
        // exactly-once — only the request that flips NULL->now affects a row; a concurrent
        // request that read the same live row loses the race here (0 rows) and is rejected,
        // preventing token replay.
        let claimed = sqlx::query(
            "UPDATE password_reset_tokens SET used_at = ?1 WHERE selector = ?2 AND used_at IS NULL",
        )
        .bind(&now)
        .bind(selector)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to consume reset token: {e}")))?;

        if claimed.rows_affected() == 0 {
            return Err(invalid());
        }

        Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::internal(format!("Invalid user_id in reset token: {e}")))
    }

    /// Invalidate all unused reset tokens for a user
    ///
    /// Called after a successful password change to prevent old tokens from being used.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn invalidate_user_reset_tokens_impl(&self, user_id: Uuid) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            UPDATE password_reset_tokens
            SET used_at = ?1
            WHERE user_id = ?2
              AND used_at IS NULL
            ",
        )
        .bind(&now)
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to invalidate reset tokens: {e}")))?;

        Ok(())
    }
    /// Store a password reset token with a custom TTL
    ///
    /// Similar to `store_password_reset_token_impl` but allows specifying the
    /// expiry duration in minutes instead of using the default 1-hour TTL.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn store_password_reset_token_with_ttl_impl(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::minutes(ttl_minutes);

        sqlx::query(
            r"
            INSERT INTO password_reset_tokens (id, user_id, selector, token_hash, expires_at, created_by, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(selector)
        .bind(verifier_hash)
        .bind(expires_at.to_rfc3339())
        .bind(created_by)
        .bind(now.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to store password reset token: {e}")))?;

        Ok(id)
    }

    /// Count password reset tokens created for a user since a given timestamp
    ///
    /// Used for rate limiting to prevent abuse of the self-service reset flow.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn count_recent_password_reset_tokens_impl(
        &self,
        user_id: Uuid,
        since: DateTime<Utc>,
    ) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as cnt
            FROM password_reset_tokens
            WHERE user_id = ?1
              AND created_at >= ?2
            ",
        )
        .bind(user_id.to_string())
        .bind(since.to_rfc3339())
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to count recent reset tokens: {e}")))?;

        Ok(row.get::<i64, _>("cnt"))
    }
}

#[async_trait]
impl PasswordResetRepository for Database {
    async fn store_token(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid> {
        Self::store_password_reset_token_impl(self, user_id, selector, verifier_hash, created_by)
            .await
    }

    async fn store_token_with_ttl(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid> {
        Self::store_password_reset_token_with_ttl_impl(
            self,
            user_id,
            selector,
            verifier_hash,
            created_by,
            ttl_minutes,
        )
        .await
    }

    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid> {
        Self::consume_password_reset_token_impl(self, selector, verifier_hash).await
    }

    async fn invalidate_tokens(&self, user_id: Uuid) -> AppResult<()> {
        Self::invalidate_user_reset_tokens_impl(self, user_id).await
    }

    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64> {
        Self::count_recent_password_reset_tokens_impl(self, user_id, since).await
    }
}
