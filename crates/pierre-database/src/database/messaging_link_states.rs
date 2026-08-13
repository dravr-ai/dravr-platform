// ABOUTME: SQLite link-state lifecycle — the short-lived codes that bind a channel identity to a user
// ABOUTME: Split from messaging.rs: creation, consumption, lookup and webhook-initiated completion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A link state is the one-time code that turns "someone messaged us from this
//! Telegram/Slack id" into "this id belongs to that Dravr account".
//!
//! It has its own lifecycle — minted with a TTL, consumed exactly once, or
//! completed by a webhook that learned the user id after the fact — and that
//! lifecycle is what this module holds. Splitting it out of `messaging.rs`
//! leaves that file to channel configs, sessions, messages and the outbound
//! queue, which are unrelated concerns that merely shared a table prefix.
//!
//! These are inherent methods on [`Database`], so the split is a second `impl`
//! block rather than a new type; the `MessagingRepository` trait impl in
//! `messaging.rs` still delegates to them unchanged.

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::Row;

use crate::repositories::CreateLinkStateParams;

use super::Database;

impl Database {
    /// Create a pending link state with a verification code
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails
    pub async fn create_link_state_impl(
        &self,
        params: &CreateLinkStateParams<'_>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO messaging_link_states
                (id, tenant_id, user_id, channel_type, code, method, used,
                 channel_user_id, sender_name, expires_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?)
            ",
        )
        .bind(params.id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.channel_type)
        .bind(params.code)
        .bind(params.method)
        .bind(params.channel_user_id)
        .bind(params.sender_name)
        .bind(params.expires_at)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create link state: {e}")))?;

        Ok(())
    }

    /// Atomically consume a link state by verification code
    ///
    /// Uses `UPDATE` with `used = 0` guard and expiry check, then verifies `rows_affected`
    /// to ensure one-time use. Returns the consumed state's full data.
    ///
    /// # Errors
    ///
    /// Returns `MessagingError::LinkCodeExpired` if the code has expired or does not exist,
    /// or `MessagingError::LinkCodeAlreadyUsed` if the code was already consumed.
    pub async fn consume_link_state_impl(
        &self,
        code: &str,
        tenant_id: TenantId,
    ) -> AppResult<Value> {
        use pierre_core::errors::messaging::MessagingError;

        let now = Utc::now().to_rfc3339();
        let tenant_id_str = tenant_id.to_string();

        // Check if the code exists for this tenant (before attempting consumption)
        let existing = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, code, method, used,
                   channel_user_id, sender_name, expires_at, created_at
            FROM messaging_link_states
            WHERE code = ? AND tenant_id = ?
            ",
        )
        .bind(code)
        .bind(&tenant_id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;

        let Some(row) = existing else {
            return Err(MessagingError::LinkCodeExpired.into());
        };

        let used: i32 = row.get("used");
        if used != 0 {
            return Err(MessagingError::LinkCodeAlreadyUsed.into());
        }

        let expires_at: String = row.get("expires_at");
        if expires_at < now {
            return Err(MessagingError::LinkCodeExpired.into());
        }

        // Atomic consumption: UPDATE with guards (including tenant_id)
        let result = sqlx::query(
            r"
            UPDATE messaging_link_states
            SET used = 1
            WHERE code = ? AND tenant_id = ? AND used = 0 AND expires_at > ?
            ",
        )
        .bind(code)
        .bind(&tenant_id_str)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to consume link state: {e}")))?;

        if result.rows_affected() == 0 {
            // Race condition: another request consumed it between our SELECT and UPDATE
            return Err(MessagingError::LinkCodeAlreadyUsed.into());
        }

        Ok(serde_json::json!({
            "id": row.get::<String, _>("id"),
            "tenant_id": row.get::<String, _>("tenant_id"),
            "user_id": row.try_get::<Option<String>, _>("user_id").ok().flatten(),
            "channel_type": row.get::<String, _>("channel_type"),
            "code": row.get::<String, _>("code"),
            "method": row.get::<String, _>("method"),
            "channel_user_id": row.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
            "sender_name": row.try_get::<Option<String>, _>("sender_name").ok().flatten(),
            "expires_at": row.get::<String, _>("expires_at"),
            "created_at": row.get::<String, _>("created_at"),
        }))
    }

    /// Read-only lookup of a link state by code for rendering the login page
    ///
    /// Returns the link state data if the code exists, is not expired, and has not been used.
    /// Does NOT consume the code.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_link_state_impl(&self, code: &str) -> AppResult<Option<Value>> {
        let now = Utc::now().to_rfc3339();

        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, code, method, used,
                   channel_user_id, sender_name, expires_at, created_at
            FROM messaging_link_states
            WHERE code = ? AND used = 0 AND expires_at > ?
            ",
        )
        .bind(code)
        .bind(&now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;

        Ok(row.map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "tenant_id": r.get::<String, _>("tenant_id"),
                "user_id": r.try_get::<Option<String>, _>("user_id").ok().flatten(),
                "channel_type": r.get::<String, _>("channel_type"),
                "code": r.get::<String, _>("code"),
                "method": r.get::<String, _>("method"),
                "channel_user_id": r.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
                "sender_name": r.try_get::<Option<String>, _>("sender_name").ok().flatten(),
                "expires_at": r.get::<String, _>("expires_at"),
                "created_at": r.get::<String, _>("created_at"),
            })
        }))
    }

    /// Atomically complete a webhook-initiated link state by setting its `user_id`
    ///
    /// Only succeeds if the code exists, is not expired, is not used, and has no `user_id` set.
    /// On success, marks the code as used and returns the link state data.
    ///
    /// # Errors
    ///
    /// Returns `MessagingError::LinkCodeExpired` if the code has expired or does not exist,
    /// `MessagingError::LinkCodeAlreadyUsed` if the code was already consumed, or
    /// `MessagingError::LinkCodeNotCompletable` if the code already has a `user_id` set.
    pub async fn complete_link_state_impl(&self, code: &str, user_id: &str) -> AppResult<Value> {
        use pierre_core::errors::messaging::MessagingError;

        let now = Utc::now().to_rfc3339();

        // Check if the code exists at all
        let existing = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, code, method, used,
                   channel_user_id, sender_name, expires_at, created_at
            FROM messaging_link_states
            WHERE code = ?
            ",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;

        let Some(row) = existing else {
            return Err(MessagingError::LinkCodeExpired.into());
        };

        let used: i32 = row.get("used");
        if used != 0 {
            return Err(MessagingError::LinkCodeAlreadyUsed.into());
        }

        let expires_at: String = row.get("expires_at");
        if expires_at < now {
            return Err(MessagingError::LinkCodeExpired.into());
        }

        // Check that user_id is not already set (webhook-initiated codes only)
        let existing_user_id: Option<String> =
            row.try_get::<Option<String>, _>("user_id").ok().flatten();
        if existing_user_id.is_some() {
            return Err(MessagingError::LinkCodeNotCompletable {
                code: code.to_owned(),
                reason: "Link code already has a user_id set".to_owned(),
            }
            .into());
        }

        // Atomic completion: set user_id and mark used in one UPDATE
        let result = sqlx::query(
            r"
            UPDATE messaging_link_states
            SET user_id = ?, used = 1
            WHERE code = ? AND used = 0 AND user_id IS NULL AND expires_at > ?
            ",
        )
        .bind(user_id)
        .bind(code)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to complete link state: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(MessagingError::LinkCodeAlreadyUsed.into());
        }

        Ok(serde_json::json!({
            "id": row.get::<String, _>("id"),
            "tenant_id": row.get::<String, _>("tenant_id"),
            "user_id": user_id,
            "channel_type": row.get::<String, _>("channel_type"),
            "code": row.get::<String, _>("code"),
            "method": row.get::<String, _>("method"),
            "channel_user_id": row.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
            "sender_name": row.try_get::<Option<String>, _>("sender_name").ok().flatten(),
            "expires_at": row.get::<String, _>("expires_at"),
            "created_at": row.get::<String, _>("created_at"),
        }))
    }
}
