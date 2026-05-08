// ABOUTME: SQLite implementation of messaging gateway repository operations
// ABOUTME: Channel configs, sessions, messages with idempotency, delivery receipts, and outbound queue
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::repositories::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, InsertMessageParams,
    MessagingRepository, UpsertChannelConfigParams,
};
use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::Row;

use super::Database;

// ════════════════════════════════════════════════════════════════
// Implementation methods (called by trait impl below)
// ════════════════════════════════════════════════════════════════

impl Database {
    // ── Channel Configs ──

    /// Upsert a channel configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn upsert_channel_config_impl(
        &self,
        params: &UpsertChannelConfigParams<'_>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let active_int: i32 = i32::from(params.is_active);

        sqlx::query(
            r"
            INSERT INTO messaging_channel_configs
                (id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                 verify_token, account_id, phone_number, bot_token, is_active,
                 created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(tenant_id, channel_type) DO UPDATE SET
                api_key = excluded.api_key,
                api_secret = excluded.api_secret,
                webhook_secret = excluded.webhook_secret,
                verify_token = excluded.verify_token,
                account_id = excluded.account_id,
                phone_number = excluded.phone_number,
                bot_token = excluded.bot_token,
                is_active = excluded.is_active,
                updated_at = excluded.updated_at
            ",
        )
        .bind(params.id)
        .bind(params.tenant_id)
        .bind(params.channel_type)
        .bind(params.api_key)
        .bind(params.api_secret)
        .bind(params.webhook_secret)
        .bind(params.verify_token)
        .bind(params.account_id)
        .bind(params.phone_number)
        .bind(params.bot_token)
        .bind(active_int)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert channel config: {e}")))?;

        Ok(())
    }

    /// Get a channel configuration by tenant and type
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_channel_config_impl(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<Option<Value>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                   verify_token, account_id, phone_number, bot_token, is_active,
                   created_at, updated_at
            FROM messaging_channel_configs
            WHERE tenant_id = ? AND channel_type = ?
            ",
        )
        .bind(tenant_id)
        .bind(channel_type)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get channel config: {e}")))?;

        Ok(row.map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "tenant_id": r.get::<String, _>("tenant_id"),
                "channel_type": r.get::<String, _>("channel_type"),
                "api_key": r.get::<Option<String>, _>("api_key"),
                "api_secret": r.get::<Option<String>, _>("api_secret"),
                "webhook_secret": r.get::<Option<String>, _>("webhook_secret"),
                "verify_token": r.get::<Option<String>, _>("verify_token"),
                "account_id": r.get::<Option<String>, _>("account_id"),
                "phone_number": r.get::<Option<String>, _>("phone_number"),
                "bot_token": r.get::<Option<String>, _>("bot_token"),
                "is_active": r.get::<i32, _>("is_active") == 1,
                "created_at": r.get::<String, _>("created_at"),
                "updated_at": r.get::<String, _>("updated_at"),
            })
        }))
    }

    /// Get all active configs for a channel type across all tenants.
    ///
    /// Cross-tenant query justified for webhook authentication: the inbound webhook
    /// carries no Pierre auth token, so we must try each tenant's signing secret
    /// to identify the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_configs_by_channel_type_impl(
        &self,
        channel_type: &str,
    ) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                   verify_token, account_id, phone_number, bot_token, is_active,
                   created_at, updated_at
            FROM messaging_channel_configs
            WHERE channel_type = ? AND is_active = 1
            ",
        )
        .bind(channel_type)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get configs by channel type: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "tenant_id": r.get::<String, _>("tenant_id"),
                    "channel_type": r.get::<String, _>("channel_type"),
                    "api_key": r.get::<Option<String>, _>("api_key"),
                    "api_secret": r.get::<Option<String>, _>("api_secret"),
                    "webhook_secret": r.get::<Option<String>, _>("webhook_secret"),
                    "verify_token": r.get::<Option<String>, _>("verify_token"),
                    "account_id": r.get::<Option<String>, _>("account_id"),
                    "phone_number": r.get::<Option<String>, _>("phone_number"),
                    "bot_token": r.get::<Option<String>, _>("bot_token"),
                    "is_active": r.get::<i32, _>("is_active") == 1,
                    "created_at": r.get::<String, _>("created_at"),
                    "updated_at": r.get::<String, _>("updated_at"),
                })
            })
            .collect())
    }

    /// List all channel configurations for a tenant
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn list_channel_configs_impl(&self, tenant_id: TenantId) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                   account_id, phone_number, bot_token, is_active, created_at, updated_at
            FROM messaging_channel_configs
            WHERE tenant_id = ?
            ORDER BY channel_type
            ",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list channel configs: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "tenant_id": r.get::<String, _>("tenant_id"),
                    "channel_type": r.get::<String, _>("channel_type"),
                    "is_active": r.get::<i32, _>("is_active") == 1,
                    "created_at": r.get::<String, _>("created_at"),
                    "updated_at": r.get::<String, _>("updated_at"),
                })
            })
            .collect())
    }

    /// Delete a channel configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn delete_channel_config_impl(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM messaging_channel_configs
            WHERE tenant_id = ? AND channel_type = ?
            ",
        )
        .bind(tenant_id)
        .bind(channel_type)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete channel config: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    // ── Sessions ──

    /// Create a messaging session
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails
    pub async fn create_session_impl(&self, params: &CreateSessionParams<'_>) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO messaging_sessions
                (id, user_id, tenant_id, channel_type, channel_user_id,
                 channel_conversation_id, pierre_conversation_id, last_message_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(params.id)
        .bind(params.user_id)
        .bind(params.tenant_id)
        .bind(params.channel_type)
        .bind(params.channel_user_id)
        .bind(params.channel_conversation_id)
        .bind(params.pierre_conversation_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create messaging session: {e}")))?;

        Ok(())
    }

    /// Look up a session by channel identity (per-chat).
    ///
    /// `channel_conversation_id` keys group/DM split: a Telegram user has one
    /// session per chat (DM vs each group). NULL is treated as the empty
    /// sentinel to match the unique-index expression
    /// `COALESCE(channel_conversation_id, '')`.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_session_by_channel_identity_impl(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
        channel_conversation_id: Option<&str>,
    ) -> AppResult<Option<Value>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, channel_type, channel_user_id,
                   channel_conversation_id, pierre_conversation_id, last_message_at, created_at
            FROM messaging_sessions
            WHERE tenant_id = ? AND channel_type = ? AND channel_user_id = ?
              AND COALESCE(channel_conversation_id, '') = COALESCE(?, '')
            ",
        )
        .bind(tenant_id)
        .bind(channel_type)
        .bind(channel_user_id)
        .bind(channel_conversation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get session by identity: {e}")))?;

        Ok(row.map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "user_id": r.get::<String, _>("user_id"),
                "tenant_id": r.get::<String, _>("tenant_id"),
                "channel_type": r.get::<String, _>("channel_type"),
                "channel_user_id": r.get::<String, _>("channel_user_id"),
                "channel_conversation_id": r.get::<Option<String>, _>("channel_conversation_id"),
                "pierre_conversation_id": r.get::<Option<String>, _>("pierre_conversation_id"),
                "last_message_at": r.get::<String, _>("last_message_at"),
                "created_at": r.get::<String, _>("created_at"),
            })
        }))
    }

    /// Touch a session's last message timestamp
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails
    pub async fn touch_session_impl(&self, session_id: &str) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            UPDATE messaging_sessions SET last_message_at = ? WHERE id = ?
            ",
        )
        .bind(&now)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to touch session: {e}")))?;

        Ok(())
    }

    /// Repoint a session at a fresh Pierre conversation.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails
    pub async fn set_session_conversation_impl(
        &self,
        session_id: &str,
        pierre_conversation_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE messaging_sessions
            SET pierre_conversation_id = ?
            WHERE id = ?
            ",
        )
        .bind(pierre_conversation_id)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update session conversation: {e}")))?;

        Ok(())
    }

    // ── Messages ──

    /// Insert a message with idempotency (INSERT OR IGNORE on `channel_message_id`)
    ///
    /// Returns `true` if inserted, `false` if duplicate (already processed).
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails
    pub async fn insert_message_impl(&self, params: &InsertMessageParams<'_>) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            r"
            INSERT OR IGNORE INTO messaging_messages
                (id, tenant_id, session_id, direction, channel_type, channel_message_id,
                 sender_id, content_type, content_body, correlation_id, raw_payload, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(params.id)
        .bind(params.tenant_id)
        .bind(params.session_id)
        .bind(params.direction)
        .bind(params.channel_type)
        .bind(params.channel_message_id)
        .bind(params.sender_id)
        .bind(params.content_type)
        .bind(params.content_body)
        .bind(params.correlation_id)
        .bind(params.raw_payload)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert message: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Get messages for a session
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_session_messages_impl(
        &self,
        session_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, session_id, direction, channel_type, channel_message_id,
                   sender_id, content_type, content_body, correlation_id, raw_payload, created_at
            FROM messaging_messages
            WHERE session_id = ? AND tenant_id = ?
            ORDER BY created_at ASC
            LIMIT ? OFFSET ?
            ",
        )
        .bind(session_id)
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get session messages: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "tenant_id": r.get::<String, _>("tenant_id"),
                    "session_id": r.get::<String, _>("session_id"),
                    "direction": r.get::<String, _>("direction"),
                    "channel_type": r.get::<String, _>("channel_type"),
                    "channel_message_id": r.get::<String, _>("channel_message_id"),
                    "sender_id": r.get::<String, _>("sender_id"),
                    "content_type": r.get::<String, _>("content_type"),
                    "content_body": r.get::<Option<String>, _>("content_body"),
                    "correlation_id": r.get::<String, _>("correlation_id"),
                    "created_at": r.get::<String, _>("created_at"),
                })
            })
            .collect())
    }

    // ── Delivery Receipts ──

    /// Insert a delivery receipt
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails
    pub async fn insert_delivery_receipt_impl(
        &self,
        id: &str,
        tenant_id: TenantId,
        message_id: &str,
        channel_message_id: Option<&str>,
        status: &str,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO messaging_delivery_receipts
                (id, tenant_id, message_id, channel_message_id, status, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(id)
        .bind(tenant_id)
        .bind(message_id)
        .bind(channel_message_id)
        .bind(status)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert delivery receipt: {e}")))?;

        Ok(())
    }

    // ── Outbound Queue ──

    /// Enqueue an outbound message
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails
    pub async fn enqueue_outbound_impl(
        &self,
        id: &str,
        message_id: &str,
        tenant_id: TenantId,
        user_id: Option<&str>,
        channel_type: &str,
        payload: &str,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO messaging_outbound_queue
                (id, message_id, tenant_id, user_id, channel_type, payload, status,
                 attempt_count, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, 'pending', 0, ?, ?)
            ",
        )
        .bind(id)
        .bind(message_id)
        .bind(tenant_id)
        .bind(user_id)
        .bind(channel_type)
        .bind(payload)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to enqueue outbound message: {e}")))?;

        Ok(())
    }

    /// Get pending or retryable outbound messages
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_pending_outbound_impl(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<Value>> {
        let now = Utc::now().to_rfc3339();

        let rows = sqlx::query(
            r"
            SELECT id, message_id, tenant_id, user_id, channel_type, payload, status,
                   attempt_count, next_retry_at, created_at, updated_at
            FROM messaging_outbound_queue
            WHERE tenant_id = ?
              AND (status = 'pending' OR (status LIKE 'retrying:%' AND next_retry_at <= ?))
            ORDER BY next_retry_at ASC, created_at ASC
            LIMIT ?
            ",
        )
        .bind(tenant_id)
        .bind(&now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get pending outbound: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "message_id": r.get::<String, _>("message_id"),
                    "tenant_id": r.get::<String, _>("tenant_id"),
                    "user_id": r.get::<Option<String>, _>("user_id"),
                    "channel_type": r.get::<String, _>("channel_type"),
                    "payload": r.get::<String, _>("payload"),
                    "status": r.get::<String, _>("status"),
                    "attempt_count": r.get::<i32, _>("attempt_count"),
                    "next_retry_at": r.get::<Option<String>, _>("next_retry_at"),
                    "created_at": r.get::<String, _>("created_at"),
                    "updated_at": r.get::<String, _>("updated_at"),
                })
            })
            .collect())
    }

    /// Get pending/retryable outbound entries across all tenants.
    ///
    /// Cross-tenant query justified for the background retry worker: it must process
    /// outbound messages for all tenants without knowing tenant IDs in advance.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_all_pending_outbound_impl(&self, limit: i64) -> AppResult<Vec<Value>> {
        let now = Utc::now().to_rfc3339();

        let rows = sqlx::query(
            r"
            SELECT id, message_id, tenant_id, user_id, channel_type, payload, status,
                   attempt_count, next_retry_at, created_at, updated_at
            FROM messaging_outbound_queue
            WHERE status = 'pending' OR (status LIKE 'retrying:%' AND next_retry_at <= ?)
            ORDER BY next_retry_at ASC, created_at ASC
            LIMIT ?
            ",
        )
        .bind(&now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get all pending outbound: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "message_id": r.get::<String, _>("message_id"),
                    "tenant_id": r.get::<String, _>("tenant_id"),
                    "user_id": r.get::<Option<String>, _>("user_id"),
                    "channel_type": r.get::<String, _>("channel_type"),
                    "payload": r.get::<String, _>("payload"),
                    "status": r.get::<String, _>("status"),
                    "attempt_count": r.get::<i32, _>("attempt_count"),
                    "next_retry_at": r.get::<Option<String>, _>("next_retry_at"),
                    "created_at": r.get::<String, _>("created_at"),
                    "updated_at": r.get::<String, _>("updated_at"),
                })
            })
            .collect())
    }

    /// Update outbound queue entry status after a send attempt
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails
    pub async fn update_outbound_status_impl(
        &self,
        id: &str,
        status: &str,
        attempt_count: i32,
        next_retry_at: Option<&str>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            UPDATE messaging_outbound_queue
            SET status = ?, attempt_count = ?, next_retry_at = ?, updated_at = ?
            WHERE id = ?
            ",
        )
        .bind(status)
        .bind(attempt_count)
        .bind(next_retry_at)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update outbound status: {e}")))?;

        Ok(())
    }

    // ── Channel Linking ──

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

    /// Create a permanent channel link
    ///
    /// # Errors
    ///
    /// Returns an error if the channel identity is already linked (UNIQUE constraint)
    pub async fn create_channel_link_impl(
        &self,
        params: &CreateChannelLinkParams<'_>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO messaging_channel_links
                (id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(params.id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.channel_type)
        .bind(params.channel_user_id)
        .bind(params.display_name)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint") {
                use pierre_core::errors::messaging::MessagingError;
                MessagingError::ChannelAlreadyLinked {
                    channel: params.channel_type.to_owned(),
                    channel_user_id: params.channel_user_id.to_owned(),
                }
                .into()
            } else {
                AppError::database(format!("Failed to create channel link: {e}"))
            }
        })?;

        Ok(())
    }

    /// Look up a channel link by channel identity
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_channel_link_impl(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = ? AND channel_type = ? AND channel_user_id = ?
            ",
        )
        .bind(tenant_id)
        .bind(channel_type)
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get channel link: {e}")))?;

        Ok(row.map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "tenant_id": r.get::<String, _>("tenant_id"),
                "user_id": r.get::<String, _>("user_id"),
                "channel_type": r.get::<String, _>("channel_type"),
                "channel_user_id": r.get::<String, _>("channel_user_id"),
                "display_name": r.get::<Option<String>, _>("display_name"),
                "linked_at": r.get::<String, _>("linked_at"),
            })
        }))
    }

    /// List all channel links for a user
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn list_user_channel_links_impl(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = ? AND user_id = ?
            ORDER BY linked_at
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list user channel links: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "tenant_id": r.get::<String, _>("tenant_id"),
                    "user_id": r.get::<String, _>("user_id"),
                    "channel_type": r.get::<String, _>("channel_type"),
                    "channel_user_id": r.get::<String, _>("channel_user_id"),
                    "display_name": r.get::<Option<String>, _>("display_name"),
                    "linked_at": r.get::<String, _>("linked_at"),
                })
            })
            .collect())
    }

    // ── In-Chat OTP Linking ──

    /// Look up an active in-chat OTP linking flow by channel identity
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_active_otp_link_state_impl(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        let now = Utc::now().to_rfc3339();

        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, code, method, used,
                   channel_user_id, sender_name, otp_step, email, otp_hash,
                   otp_attempts, expires_at, created_at
            FROM messaging_link_states
            WHERE tenant_id = ? AND channel_type = ? AND channel_user_id = ?
              AND otp_step IS NOT NULL AND used = 0 AND expires_at > ?
            ORDER BY created_at DESC
            LIMIT 1
            ",
        )
        .bind(tenant_id)
        .bind(channel_type)
        .bind(channel_user_id)
        .bind(&now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get active OTP link state: {e}")))?;

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
                "otp_step": r.try_get::<Option<String>, _>("otp_step").ok().flatten(),
                "email": r.try_get::<Option<String>, _>("email").ok().flatten(),
                "otp_hash": r.try_get::<Option<String>, _>("otp_hash").ok().flatten(),
                "otp_attempts": r.get::<i32, _>("otp_attempts"),
                "expires_at": r.get::<String, _>("expires_at"),
                "created_at": r.get::<String, _>("created_at"),
            })
        }))
    }

    /// Advance the OTP flow: set email and OTP hash, transition to `awaiting_otp`
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails
    pub async fn set_otp_on_link_state_impl(
        &self,
        id: &str,
        email: &str,
        otp_hash: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE messaging_link_states
            SET email = ?, otp_hash = ?, otp_step = 'awaiting_otp', otp_attempts = 0
            WHERE id = ?
            ",
        )
        .bind(email)
        .bind(otp_hash)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set OTP on link state: {e}")))?;

        Ok(())
    }

    /// Increment OTP attempt counter and return the new count
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn increment_otp_attempts_impl(&self, id: &str) -> AppResult<i32> {
        sqlx::query(
            r"
            UPDATE messaging_link_states
            SET otp_attempts = otp_attempts + 1
            WHERE id = ?
            ",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to increment OTP attempts: {e}")))?;

        let row = sqlx::query(r"SELECT otp_attempts FROM messaging_link_states WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to read OTP attempts: {e}")))?;

        Ok(row.get::<i32, _>("otp_attempts"))
    }

    /// Invalidate any active OTP link states for a sender
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails
    pub async fn invalidate_otp_link_states_impl(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE messaging_link_states
            SET used = 1
            WHERE tenant_id = ? AND channel_type = ? AND channel_user_id = ?
              AND otp_step IS NOT NULL AND used = 0
            ",
        )
        .bind(tenant_id)
        .bind(channel_type)
        .bind(channel_user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to invalidate OTP link states: {e}")))?;

        Ok(())
    }

    /// Delete a channel link (unlink)
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn delete_channel_link_impl(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM messaging_channel_links
            WHERE tenant_id = ? AND user_id = ? AND channel_type = ?
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(channel_type)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete channel link: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Logout a channel sender: delete the channel link and invalidate OTP
    /// states. `messaging_sessions` and `messaging_messages` are intentionally
    /// retained for support and audit — `resolve_linked_session` gates session
    /// resumption on the channel link, so the absence of the link is enough to
    /// unbind the sender from the previously linked Pierre user.
    ///
    /// # Errors
    ///
    /// Returns `AppError` if any database deletion query fails.
    pub async fn logout_channel_sender_impl(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        sender_id: &str,
    ) -> AppResult<()> {
        // Delete channel link (uses channel_user_id, not user_id)
        sqlx::query(
            r"
            DELETE FROM messaging_channel_links
            WHERE tenant_id = ? AND channel_type = ? AND channel_user_id = ?
            ",
        )
        .bind(tenant_id)
        .bind(channel_type)
        .bind(sender_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete channel link: {e}")))?;

        // Invalidate OTP states
        sqlx::query(
            r"
            UPDATE messaging_link_states
            SET used = 1
            WHERE tenant_id = ? AND channel_type = ? AND channel_user_id = ? AND used = 0
            ",
        )
        .bind(tenant_id)
        .bind(channel_type)
        .bind(sender_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to invalidate OTP states: {e}")))?;

        Ok(())
    }
}

// ════════════════════════════════════════════════════════════════
// Trait implementation delegating to _impl methods
// ════════════════════════════════════════════════════════════════

#[async_trait]
impl MessagingRepository for Database {
    async fn upsert_channel_config(&self, params: &UpsertChannelConfigParams<'_>) -> AppResult<()> {
        self.upsert_channel_config_impl(params).await
    }

    async fn get_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<Option<Value>> {
        self.get_channel_config_impl(tenant_id, channel_type).await
    }

    async fn list_channel_configs(&self, tenant_id: TenantId) -> AppResult<Vec<Value>> {
        self.list_channel_configs_impl(tenant_id).await
    }

    async fn get_configs_by_channel_type(&self, channel_type: &str) -> AppResult<Vec<Value>> {
        self.get_configs_by_channel_type_impl(channel_type).await
    }

    async fn delete_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool> {
        self.delete_channel_config_impl(tenant_id, channel_type)
            .await
    }

    async fn create_session(&self, params: &CreateSessionParams<'_>) -> AppResult<()> {
        self.create_session_impl(params).await
    }

    async fn get_session_by_channel_identity(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
        channel_conversation_id: Option<&str>,
    ) -> AppResult<Option<Value>> {
        self.get_session_by_channel_identity_impl(
            tenant_id,
            channel_type,
            channel_user_id,
            channel_conversation_id,
        )
        .await
    }

    async fn touch_session(&self, session_id: &str) -> AppResult<()> {
        self.touch_session_impl(session_id).await
    }

    async fn set_session_conversation(
        &self,
        session_id: &str,
        pierre_conversation_id: &str,
    ) -> AppResult<()> {
        self.set_session_conversation_impl(session_id, pierre_conversation_id)
            .await
    }

    async fn insert_message(&self, params: &InsertMessageParams<'_>) -> AppResult<bool> {
        self.insert_message_impl(params).await
    }

    async fn get_session_messages(
        &self,
        session_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Value>> {
        self.get_session_messages_impl(session_id, tenant_id, limit, offset)
            .await
    }

    async fn insert_delivery_receipt(
        &self,
        id: &str,
        tenant_id: TenantId,
        message_id: &str,
        channel_message_id: Option<&str>,
        status: &str,
    ) -> AppResult<()> {
        self.insert_delivery_receipt_impl(id, tenant_id, message_id, channel_message_id, status)
            .await
    }

    async fn enqueue_outbound(
        &self,
        id: &str,
        message_id: &str,
        tenant_id: TenantId,
        user_id: Option<&str>,
        channel_type: &str,
        payload: &str,
    ) -> AppResult<()> {
        self.enqueue_outbound_impl(id, message_id, tenant_id, user_id, channel_type, payload)
            .await
    }

    async fn get_pending_outbound(&self, tenant_id: TenantId, limit: i64) -> AppResult<Vec<Value>> {
        self.get_pending_outbound_impl(tenant_id, limit).await
    }

    async fn get_all_pending_outbound(&self, limit: i64) -> AppResult<Vec<Value>> {
        self.get_all_pending_outbound_impl(limit).await
    }

    async fn update_outbound_status(
        &self,
        id: &str,
        status: &str,
        attempt_count: i32,
        next_retry_at: Option<&str>,
    ) -> AppResult<()> {
        self.update_outbound_status_impl(id, status, attempt_count, next_retry_at)
            .await
    }

    async fn create_link_state(&self, params: &CreateLinkStateParams<'_>) -> AppResult<()> {
        self.create_link_state_impl(params).await
    }

    async fn consume_link_state(&self, code: &str, tenant_id: TenantId) -> AppResult<Value> {
        self.consume_link_state_impl(code, tenant_id).await
    }

    async fn get_link_state(&self, code: &str) -> AppResult<Option<Value>> {
        self.get_link_state_impl(code).await
    }

    async fn complete_link_state(&self, code: &str, user_id: &str) -> AppResult<Value> {
        self.complete_link_state_impl(code, user_id).await
    }

    async fn create_channel_link(&self, params: &CreateChannelLinkParams<'_>) -> AppResult<()> {
        self.create_channel_link_impl(params).await
    }

    async fn get_channel_link(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        self.get_channel_link_impl(tenant_id, channel_type, channel_user_id)
            .await
    }

    async fn list_user_channel_links(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<Value>> {
        self.list_user_channel_links_impl(tenant_id, user_id).await
    }

    async fn delete_channel_link(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
    ) -> AppResult<bool> {
        self.delete_channel_link_impl(tenant_id, user_id, channel_type)
            .await
    }

    async fn get_channel_link_locale(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<String>> {
        let locale: Option<Option<String>> = sqlx::query_scalar(
            r"
            SELECT locale
            FROM messaging_channel_links
            WHERE tenant_id = ?1 AND channel_type = ?2 AND channel_user_id = ?3
            ",
        )
        .bind(tenant_id.0.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get channel link locale: {e}")))?;

        Ok(locale.flatten())
    }

    async fn set_channel_link_locale(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
        locale: Option<&str>,
    ) -> AppResult<()> {
        let result = sqlx::query(
            r"
            UPDATE messaging_channel_links
               SET locale = ?1
             WHERE tenant_id = ?2 AND user_id = ?3 AND channel_type = ?4
            ",
        )
        .bind(locale)
        .bind(tenant_id.0.to_string())
        .bind(user_id)
        .bind(channel_type)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set channel link locale: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!(
                "Channel link for user {user_id} on {channel_type}"
            )));
        }

        Ok(())
    }

    async fn logout_channel_sender(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        sender_id: &str,
    ) -> AppResult<()> {
        self.logout_channel_sender_impl(tenant_id, channel_type, sender_id)
            .await
    }

    // ── In-Chat OTP Linking ──

    async fn get_active_otp_link_state(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        self.get_active_otp_link_state_impl(tenant_id, channel_type, channel_user_id)
            .await
    }

    async fn set_otp_on_link_state(&self, id: &str, email: &str, otp_hash: &str) -> AppResult<()> {
        self.set_otp_on_link_state_impl(id, email, otp_hash).await
    }

    async fn increment_otp_attempts(&self, id: &str) -> AppResult<i32> {
        self.increment_otp_attempts_impl(id).await
    }

    async fn invalidate_otp_link_states(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<()> {
        self.invalidate_otp_link_states_impl(tenant_id, channel_type, channel_user_id)
            .await
    }
}
