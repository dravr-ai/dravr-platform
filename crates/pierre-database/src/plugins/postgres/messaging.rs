// ABOUTME: PostgreSQL implementation of messaging gateway repository operations
// ABOUTME: Channel configs, sessions, messages, delivery receipts, outbound queue, and channel linking
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::PostgresDatabase;
use crate::repositories::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, InsertMessageParams,
    MessagingRepository, UpsertChannelConfigParams,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::Row;

#[async_trait]
impl MessagingRepository for PostgresDatabase {
    // ── Channel Configs ──

    async fn upsert_channel_config(&self, params: &UpsertChannelConfigParams<'_>) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO messaging_channel_configs
                (id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                 verify_token, account_id, phone_number, bot_token, is_active,
                 created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)
            ON CONFLICT(tenant_id, channel_type) DO UPDATE SET
                api_key = EXCLUDED.api_key,
                api_secret = EXCLUDED.api_secret,
                webhook_secret = EXCLUDED.webhook_secret,
                verify_token = EXCLUDED.verify_token,
                account_id = EXCLUDED.account_id,
                phone_number = EXCLUDED.phone_number,
                bot_token = EXCLUDED.bot_token,
                is_active = EXCLUDED.is_active,
                updated_at = EXCLUDED.updated_at
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
        .bind(params.is_active)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert channel config: {e}")))?;

        Ok(())
    }

    async fn get_channel_config(
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
            WHERE tenant_id = $1 AND channel_type = $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get channel config: {e}")))?;

        Ok(row.map(|r| {
            let created_at: DateTime<Utc> = r.get("created_at");
            let updated_at: DateTime<Utc> = r.get("updated_at");
            let is_active: bool = r.get("is_active");

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
                "is_active": is_active,
                "created_at": created_at.to_rfc3339(),
                "updated_at": updated_at.to_rfc3339(),
            })
        }))
    }

    async fn list_channel_configs(&self, tenant_id: TenantId) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                   account_id, phone_number, bot_token, is_active, created_at, updated_at
            FROM messaging_channel_configs
            WHERE tenant_id = $1
            ORDER BY channel_type
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list channel configs: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                let created_at: DateTime<Utc> = r.get("created_at");
                let updated_at: DateTime<Utc> = r.get("updated_at");
                let is_active: bool = r.get("is_active");

                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "tenant_id": r.get::<String, _>("tenant_id"),
                    "channel_type": r.get::<String, _>("channel_type"),
                    "is_active": is_active,
                    "created_at": created_at.to_rfc3339(),
                    "updated_at": updated_at.to_rfc3339(),
                })
            })
            .collect())
    }

    /// Cross-tenant query justified for webhook authentication: the inbound webhook
    /// carries no Pierre auth token, so we must try each tenant's signing secret
    /// to identify the caller.
    async fn get_configs_by_channel_type(&self, channel_type: &str) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, channel_type, api_key, api_secret, webhook_secret,
                   verify_token, account_id, phone_number, bot_token, is_active,
                   created_at, updated_at
            FROM messaging_channel_configs
            WHERE channel_type = $1 AND is_active = TRUE
            ",
        )
        .bind(channel_type)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get configs by channel type: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                let created_at: DateTime<Utc> = r.get("created_at");
                let updated_at: DateTime<Utc> = r.get("updated_at");
                let is_active: bool = r.get("is_active");

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
                    "is_active": is_active,
                    "created_at": created_at.to_rfc3339(),
                    "updated_at": updated_at.to_rfc3339(),
                })
            })
            .collect())
    }

    async fn delete_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM messaging_channel_configs
            WHERE tenant_id = $1 AND channel_type = $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete channel config: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    // ── Sessions ──

    async fn create_session(&self, params: &CreateSessionParams<'_>) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO messaging_sessions
                (id, user_id, tenant_id, channel_type, channel_user_id,
                 channel_conversation_id, pierre_conversation_id, last_message_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
            ",
        )
        .bind(params.id)
        .bind(params.user_id)
        .bind(params.tenant_id)
        .bind(params.channel_type)
        .bind(params.channel_user_id)
        .bind(params.channel_conversation_id)
        .bind(params.pierre_conversation_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create messaging session: {e}")))?;

        Ok(())
    }

    async fn get_session_by_channel_identity(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, channel_type, channel_user_id,
                   channel_conversation_id, pierre_conversation_id, last_message_at, created_at
            FROM messaging_sessions
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get session by identity: {e}")))?;

        Ok(row.map(|r| {
            let last_message_at: DateTime<Utc> = r.get("last_message_at");
            let created_at: DateTime<Utc> = r.get("created_at");

            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "user_id": r.get::<String, _>("user_id"),
                "tenant_id": r.get::<String, _>("tenant_id"),
                "channel_type": r.get::<String, _>("channel_type"),
                "channel_user_id": r.get::<String, _>("channel_user_id"),
                "channel_conversation_id": r.get::<Option<String>, _>("channel_conversation_id"),
                "pierre_conversation_id": r.get::<Option<String>, _>("pierre_conversation_id"),
                "last_message_at": last_message_at.to_rfc3339(),
                "created_at": created_at.to_rfc3339(),
            })
        }))
    }

    async fn touch_session(&self, session_id: &str) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            UPDATE messaging_sessions SET last_message_at = $1 WHERE id = $2
            ",
        )
        .bind(now)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to touch session: {e}")))?;

        Ok(())
    }

    // ── Messages ──

    async fn insert_message(&self, params: &InsertMessageParams<'_>) -> AppResult<bool> {
        let now = Utc::now();

        let result = sqlx::query(
            r"
            INSERT INTO messaging_messages
                (id, tenant_id, session_id, direction, channel_type, channel_message_id,
                 sender_id, content_type, content_body, correlation_id, raw_payload, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (tenant_id, channel_message_id) DO NOTHING
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
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert message: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_session_messages(
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
            WHERE session_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            LIMIT $3 OFFSET $4
            ",
        )
        .bind(session_id)
        .bind(tenant_id.to_string())
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get session messages: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                let created_at: DateTime<Utc> = r.get("created_at");

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
                    "created_at": created_at.to_rfc3339(),
                })
            })
            .collect())
    }

    // ── Delivery Receipts ──

    async fn insert_delivery_receipt(
        &self,
        id: &str,
        tenant_id: TenantId,
        message_id: &str,
        channel_message_id: Option<&str>,
        status: &str,
    ) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO messaging_delivery_receipts
                (id, tenant_id, message_id, channel_message_id, status, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(id)
        .bind(tenant_id.to_string())
        .bind(message_id)
        .bind(channel_message_id)
        .bind(status)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert delivery receipt: {e}")))?;

        Ok(())
    }

    // ── Outbound Queue ──

    async fn enqueue_outbound(
        &self,
        id: &str,
        message_id: &str,
        tenant_id: TenantId,
        channel_type: &str,
        payload: &str,
    ) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO messaging_outbound_queue
                (id, message_id, tenant_id, channel_type, payload, status,
                 attempt_count, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, 'pending', 0, $6, $6)
            ",
        )
        .bind(id)
        .bind(message_id)
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(payload)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to enqueue outbound message: {e}")))?;

        Ok(())
    }

    async fn get_pending_outbound(&self, tenant_id: TenantId, limit: i64) -> AppResult<Vec<Value>> {
        let now = Utc::now();

        let rows = sqlx::query(
            r"
            SELECT id, message_id, tenant_id, channel_type, payload, status,
                   attempt_count, next_retry_at, created_at, updated_at
            FROM messaging_outbound_queue
            WHERE tenant_id = $1
              AND (status = 'pending' OR (status LIKE 'retrying:%' AND next_retry_at <= $2))
            ORDER BY next_retry_at ASC NULLS FIRST, created_at ASC
            LIMIT $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get pending outbound: {e}")))?;

        Ok(rows.iter().map(row_to_outbound_json).collect())
    }

    /// Cross-tenant query justified for the background retry worker: it must process
    /// outbound messages for all tenants without knowing tenant IDs in advance.
    async fn get_all_pending_outbound(&self, limit: i64) -> AppResult<Vec<Value>> {
        let now = Utc::now();

        let rows = sqlx::query(
            r"
            SELECT id, message_id, tenant_id, channel_type, payload, status,
                   attempt_count, next_retry_at, created_at, updated_at
            FROM messaging_outbound_queue
            WHERE status = 'pending' OR (status LIKE 'retrying:%' AND next_retry_at <= $1)
            ORDER BY next_retry_at ASC NULLS FIRST, created_at ASC
            LIMIT $2
            ",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get all pending outbound: {e}")))?;

        Ok(rows.iter().map(row_to_outbound_json).collect())
    }

    async fn update_outbound_status(
        &self,
        id: &str,
        status: &str,
        attempt_count: i32,
        next_retry_at: Option<&str>,
    ) -> AppResult<()> {
        let now = Utc::now();

        // Parse the next_retry_at string into a DateTime if provided
        let next_retry: Option<DateTime<Utc>> = next_retry_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });

        sqlx::query(
            r"
            UPDATE messaging_outbound_queue
            SET status = $1, attempt_count = $2, next_retry_at = $3, updated_at = $4
            WHERE id = $5
            ",
        )
        .bind(status)
        .bind(attempt_count)
        .bind(next_retry)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update outbound status: {e}")))?;

        Ok(())
    }

    // ── Channel Linking ──

    async fn create_link_state(&self, params: &CreateLinkStateParams<'_>) -> AppResult<()> {
        let now = Utc::now();

        // Parse the expires_at string into a DateTime for PostgreSQL TIMESTAMPTZ
        let expires_at: DateTime<Utc> = chrono::DateTime::parse_from_rfc3339(params.expires_at)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| AppError::invalid_input(format!("Invalid expires_at timestamp: {e}")))?;

        sqlx::query(
            r"
            INSERT INTO messaging_link_states
                (id, tenant_id, user_id, channel_type, code, method, used,
                 channel_user_id, sender_name, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, FALSE, $7, $8, $9, $10)
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
        .bind(expires_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create link state: {e}")))?;

        Ok(())
    }

    /// Atomically consume a link state by verification code.
    ///
    /// Uses SELECT then atomic `UPDATE` with `used = FALSE` guard and expiry check,
    /// verifying `rows_affected` to ensure one-time use.
    async fn consume_link_state(&self, code: &str, tenant_id: TenantId) -> AppResult<Value> {
        use pierre_core::errors::messaging::MessagingError;

        let now = Utc::now();
        let tenant_id_str = tenant_id.to_string();

        // Check if the code exists for this tenant (before attempting consumption)
        let existing = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, code, method, used,
                   channel_user_id, sender_name, expires_at, created_at
            FROM messaging_link_states
            WHERE code = $1 AND tenant_id = $2
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

        let used: bool = row.get("used");
        if used {
            return Err(MessagingError::LinkCodeAlreadyUsed.into());
        }

        let expires_at: DateTime<Utc> = row.get("expires_at");
        if expires_at < now {
            return Err(MessagingError::LinkCodeExpired.into());
        }

        // Atomic consumption: UPDATE with guards (including tenant_id)
        let result = sqlx::query(
            r"
            UPDATE messaging_link_states
            SET used = TRUE
            WHERE code = $1 AND tenant_id = $2 AND used = FALSE AND expires_at > $3
            ",
        )
        .bind(code)
        .bind(&tenant_id_str)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to consume link state: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(MessagingError::LinkCodeAlreadyUsed.into());
        }

        let created_at: DateTime<Utc> = row.get("created_at");

        Ok(serde_json::json!({
            "id": row.get::<String, _>("id"),
            "tenant_id": row.get::<String, _>("tenant_id"),
            "user_id": row.try_get::<Option<String>, _>("user_id").ok().flatten(),
            "channel_type": row.get::<String, _>("channel_type"),
            "code": row.get::<String, _>("code"),
            "method": row.get::<String, _>("method"),
            "channel_user_id": row.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
            "sender_name": row.try_get::<Option<String>, _>("sender_name").ok().flatten(),
            "expires_at": expires_at.to_rfc3339(),
            "created_at": created_at.to_rfc3339(),
        }))
    }

    async fn get_link_state(&self, code: &str) -> AppResult<Option<Value>> {
        let now = Utc::now();

        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, code, method, used,
                   channel_user_id, sender_name, expires_at, created_at
            FROM messaging_link_states
            WHERE code = $1 AND used = FALSE AND expires_at > $2
            ",
        )
        .bind(code)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;

        Ok(row.map(|r| {
            let expires_at: DateTime<Utc> = r.get("expires_at");
            let created_at: DateTime<Utc> = r.get("created_at");
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "tenant_id": r.get::<String, _>("tenant_id"),
                "user_id": r.try_get::<Option<String>, _>("user_id").ok().flatten(),
                "channel_type": r.get::<String, _>("channel_type"),
                "code": r.get::<String, _>("code"),
                "method": r.get::<String, _>("method"),
                "channel_user_id": r.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
                "sender_name": r.try_get::<Option<String>, _>("sender_name").ok().flatten(),
                "expires_at": expires_at.to_rfc3339(),
                "created_at": created_at.to_rfc3339(),
            })
        }))
    }

    async fn complete_link_state(&self, code: &str, user_id: &str) -> AppResult<Value> {
        use pierre_core::errors::messaging::MessagingError;

        let now = Utc::now();

        let existing = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, code, method, used,
                   channel_user_id, sender_name, expires_at, created_at
            FROM messaging_link_states
            WHERE code = $1
            ",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;

        let Some(row) = existing else {
            return Err(MessagingError::LinkCodeExpired.into());
        };

        let used: bool = row.get("used");
        if used {
            return Err(MessagingError::LinkCodeAlreadyUsed.into());
        }

        let expires_at: DateTime<Utc> = row.get("expires_at");
        if expires_at < now {
            return Err(MessagingError::LinkCodeExpired.into());
        }

        let existing_user_id: Option<String> =
            row.try_get::<Option<String>, _>("user_id").ok().flatten();
        if existing_user_id.is_some() {
            return Err(MessagingError::LinkCodeNotCompletable {
                code: code.to_owned(),
                reason: "Link code already has a user_id set".to_owned(),
            }
            .into());
        }

        let result = sqlx::query(
            r"
            UPDATE messaging_link_states
            SET user_id = $1, used = TRUE
            WHERE code = $2 AND used = FALSE AND user_id IS NULL AND expires_at > $3
            ",
        )
        .bind(user_id)
        .bind(code)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to complete link state: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(MessagingError::LinkCodeAlreadyUsed.into());
        }

        let created_at: DateTime<Utc> = row.get("created_at");

        Ok(serde_json::json!({
            "id": row.get::<String, _>("id"),
            "tenant_id": row.get::<String, _>("tenant_id"),
            "user_id": user_id,
            "channel_type": row.get::<String, _>("channel_type"),
            "code": row.get::<String, _>("code"),
            "method": row.get::<String, _>("method"),
            "channel_user_id": row.try_get::<Option<String>, _>("channel_user_id").ok().flatten(),
            "sender_name": row.try_get::<Option<String>, _>("sender_name").ok().flatten(),
            "expires_at": expires_at.to_rfc3339(),
            "created_at": created_at.to_rfc3339(),
        }))
    }

    async fn create_channel_link(&self, params: &CreateChannelLinkParams<'_>) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO messaging_channel_links
                (id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
        )
        .bind(params.id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.channel_type)
        .bind(params.channel_user_id)
        .bind(params.display_name)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("unique constraint")
                || e.to_string().contains("duplicate key")
            {
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

    async fn get_channel_link(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND channel_type = $2 AND channel_user_id = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get channel link: {e}")))?;

        Ok(row.map(|r| {
            let linked_at: DateTime<Utc> = r.get("linked_at");

            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "tenant_id": r.get::<String, _>("tenant_id"),
                "user_id": r.get::<String, _>("user_id"),
                "channel_type": r.get::<String, _>("channel_type"),
                "channel_user_id": r.get::<String, _>("channel_user_id"),
                "display_name": r.get::<Option<String>, _>("display_name"),
                "linked_at": linked_at.to_rfc3339(),
            })
        }))
    }

    async fn list_user_channel_links(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY linked_at
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list user channel links: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| {
                let linked_at: DateTime<Utc> = r.get("linked_at");

                serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "tenant_id": r.get::<String, _>("tenant_id"),
                    "user_id": r.get::<String, _>("user_id"),
                    "channel_type": r.get::<String, _>("channel_type"),
                    "channel_user_id": r.get::<String, _>("channel_user_id"),
                    "display_name": r.get::<Option<String>, _>("display_name"),
                    "linked_at": linked_at.to_rfc3339(),
                })
            })
            .collect())
    }

    async fn delete_channel_link(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM messaging_channel_links
            WHERE tenant_id = $1 AND user_id = $2 AND channel_type = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(channel_type)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete channel link: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}

/// Convert an outbound queue row to JSON value
fn row_to_outbound_json(r: &PgRow) -> Value {
    let next_retry_at: Option<DateTime<Utc>> = r.get("next_retry_at");
    let created_at: DateTime<Utc> = r.get("created_at");
    let updated_at: DateTime<Utc> = r.get("updated_at");

    serde_json::json!({
        "id": r.get::<String, _>("id"),
        "message_id": r.get::<String, _>("message_id"),
        "tenant_id": r.get::<String, _>("tenant_id"),
        "channel_type": r.get::<String, _>("channel_type"),
        "payload": r.get::<String, _>("payload"),
        "status": r.get::<String, _>("status"),
        "attempt_count": r.get::<i32, _>("attempt_count"),
        "next_retry_at": next_retry_at.map(|dt| dt.to_rfc3339()),
        "created_at": created_at.to_rfc3339(),
        "updated_at": updated_at.to_rfc3339(),
    })
}
