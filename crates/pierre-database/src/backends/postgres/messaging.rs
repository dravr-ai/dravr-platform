// ABOUTME: PostgreSQL implementation of messaging gateway repository operations
// ABOUTME: Channel configs, sessions, messages, delivery receipts, outbound queue, and channel linking
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::messaging_link_states as link_states;
use super::messaging_reactions::find_reaction_feedback_target;
use super::PostgresDatabase;
use crate::repositories::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, InsertMessageParams,
    MessagingRepository, ReactionFeedbackTarget, UpsertChannelConfigParams,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
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
            ORDER BY created_at, id
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

    async fn channel_identity_claimed_by_other_tenant(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        phone_number: Option<&str>,
        account_id: Option<&str>,
        bot_token: Option<&str>,
    ) -> AppResult<bool> {
        // No identity to collide on — nothing to claim.
        if phone_number.is_none() && account_id.is_none() && bot_token.is_none() {
            return Ok(false);
        }

        let exists: bool = sqlx::query_scalar(
            r"
            SELECT EXISTS(
                SELECT 1 FROM messaging_channel_configs
                WHERE channel_type = $1
                  AND is_active = TRUE
                  AND tenant_id <> $2
                  AND (
                      ($3::text IS NOT NULL AND phone_number = $3)
                   OR ($4::text IS NOT NULL AND account_id = $4)
                   OR ($5::text IS NOT NULL AND bot_token = $5)
                  )
            )
            ",
        )
        .bind(channel_type)
        .bind(tenant_id.to_string())
        .bind(phone_number)
        .bind(account_id)
        .bind(bot_token)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to check channel identity ownership: {e}"))
        })?;

        Ok(exists)
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

        // Casts: migration 20260417000001 converted user_id and tenant_id to
        // UUID. The bind sites still send text, so the SQL casts ($2::uuid,
        // $3::uuid) bridge the wire types without touching call-site plumbing.
        sqlx::query(
            r"
            INSERT INTO messaging_sessions
                (id, user_id, tenant_id, channel_type, channel_user_id,
                 channel_conversation_id, pierre_conversation_id, last_message_at, created_at)
            VALUES ($1, $2::uuid, $3::uuid, $4, $5, $6, $7, $8, $8)
            ",
        )
        .bind(params.id)
        .bind(params.user_id)
        .bind(params.tenant_id.to_string())
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
        channel_conversation_id: Option<&str>,
    ) -> AppResult<Option<Value>> {
        // Casts: migration 20260417000001 converted messaging_sessions.tenant_id
        // and user_id to UUID. SQL casts keep the Rust String/TenantId bind
        // sites stable while letting Postgres compare against UUID columns.
        // The COALESCE expression mirrors the unique-index expression in
        // migration 20260505000001_messaging_sessions_per_chat.
        let row = sqlx::query(
            r"
            SELECT id,
                   user_id::text   AS user_id,
                   tenant_id::text AS tenant_id,
                   channel_type, channel_user_id,
                   channel_conversation_id, pierre_conversation_id,
                   last_message_at, created_at
            FROM messaging_sessions
            WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3
              AND COALESCE(channel_conversation_id, '') = COALESCE($4, '')
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .bind(channel_conversation_id)
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

    async fn get_session_by_pierre_conversation_id(
        &self,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
    ) -> AppResult<Option<Value>> {
        // Casts mirror get_session_by_channel_identity: migration
        // 20260417000001 converted tenant_id/user_id to UUID, so the bound
        // TenantId string is cast to compare against the UUID column.
        let row = sqlx::query(
            r"
            SELECT id,
                   user_id::text   AS user_id,
                   tenant_id::text AS tenant_id,
                   channel_type, channel_user_id,
                   channel_conversation_id, pierre_conversation_id,
                   last_message_at, created_at
            FROM messaging_sessions
            WHERE tenant_id = $1::uuid AND pierre_conversation_id = $2
            LIMIT 1
            ",
        )
        .bind(tenant_id.to_string())
        .bind(pierre_conversation_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get session by conversation: {e}")))?;

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

    async fn set_session_conversation(
        &self,
        session_id: &str,
        pierre_conversation_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE messaging_sessions
            SET pierre_conversation_id = $1
            WHERE id = $2
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

    async fn insert_message(&self, params: &InsertMessageParams<'_>) -> AppResult<bool> {
        let now = Utc::now();

        let result = sqlx::query(
            r"
            INSERT INTO messaging_messages
                (id, tenant_id, session_id, direction, channel_type, channel_message_id,
                 sender_id, content_type, content_body, correlation_id, raw_payload,
                 chat_message_id, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
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
        .bind(params.chat_message_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert message: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn find_reaction_feedback_target(
        &self,
        channel_type: &str,
        channel_message_id: &str,
        channel_conversation_id: Option<&str>,
    ) -> AppResult<Option<ReactionFeedbackTarget>> {
        find_reaction_feedback_target(
            &self.pool,
            channel_type,
            channel_message_id,
            channel_conversation_id,
        )
        .await
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
        user_id: Option<&str>,
        channel_type: &str,
        payload: &str,
    ) -> AppResult<()> {
        let now = Utc::now();
        let user_uuid = user_id.and_then(|s| uuid::Uuid::parse_str(s).ok());

        sqlx::query(
            r"
            INSERT INTO messaging_outbound_queue
                (id, message_id, tenant_id, user_id, channel_type, payload, status,
                 attempt_count, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, 'pending', 0, $7, $7)
            ",
        )
        .bind(id)
        .bind(message_id)
        .bind(tenant_id.to_string())
        .bind(user_uuid)
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
            SELECT id, message_id, tenant_id, user_id, channel_type, payload, status,
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
            SELECT id, message_id, tenant_id, user_id, channel_type, payload, status,
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
        link_states::create_link_state(&self.pool, params).await
    }

    async fn consume_link_state(&self, code: &str, tenant_id: TenantId) -> AppResult<Value> {
        link_states::consume_link_state(&self.pool, code, tenant_id).await
    }

    async fn get_link_state(&self, code: &str) -> AppResult<Option<Value>> {
        link_states::get_link_state(&self.pool, code).await
    }

    async fn complete_link_state(&self, code: &str, user_id: &str) -> AppResult<Value> {
        link_states::complete_link_state(&self.pool, code, user_id).await
    }

    async fn create_channel_link(&self, params: &CreateChannelLinkParams<'_>) -> AppResult<()> {
        let now = Utc::now();

        // Casts: migration 20260417000001 converted tenant_id and user_id to
        // UUID. SQL casts ($2::uuid, $3::uuid) bridge the wire types so the
        // call sites can keep binding via TenantId/String.
        sqlx::query(
            r"
            INSERT INTO messaging_channel_links
                (id, tenant_id, user_id, channel_type, channel_user_id, display_name, linked_at)
            VALUES ($1, $2::uuid, $3::uuid, $4, $5, $6, $7)
            ",
        )
        .bind(params.id)
        .bind(params.tenant_id.to_string())
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
            SELECT id,
                   tenant_id::text AS tenant_id,
                   user_id::text   AS user_id,
                   channel_type, channel_user_id, display_name, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3
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

    async fn get_channel_link_tenant(
        &self,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<TenantId>> {
        // Cross-tenant lookup justified like get_configs_by_channel_type: the
        // backfill-completion push runs under the user's own tenant but loads
        // the channel config from the BOT/channel-owner tenant, which the link
        // (channel identity -> owner) is the authoritative source for. LIMIT 1
        // by linked_at so a channel identity bound under more than one bot
        // tenant resolves deterministically to the earliest binding.
        let row = sqlx::query(
            r"
            SELECT tenant_id::text AS tenant_id
            FROM messaging_channel_links
            WHERE channel_type = $1 AND channel_user_id = $2
            ORDER BY linked_at
            LIMIT 1
            ",
        )
        .bind(channel_type)
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to resolve channel link tenant: {e}")))?;

        match row {
            Some(r) => {
                let raw: String = r.get("tenant_id");
                Ok(Some(TenantId::from_uuid(parse_uuid(&raw)?)))
            }
            None => Ok(None),
        }
    }

    async fn list_user_channel_links(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT id,
                   tenant_id::text AS tenant_id,
                   user_id::text   AS user_id,
                   channel_type, channel_user_id, display_name, locale, linked_at
            FROM messaging_channel_links
            WHERE tenant_id = $1::uuid AND user_id = $2::uuid
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
                    // The link's own locale — see the SQLite backend for why
                    // its absence was invisible.
                    "locale": r.get::<Option<String>, _>("locale"),
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
            WHERE tenant_id = $1::uuid AND user_id = $2::uuid AND channel_type = $3
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
            WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3
            ",
        )
        .bind(tenant_id.to_string())
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
               SET locale = $1
             WHERE tenant_id = $2::uuid AND user_id = $3::uuid AND channel_type = $4
            ",
        )
        .bind(locale)
        .bind(tenant_id.to_string())
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

    async fn coach_proposal_sent(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<bool> {
        let sent: Option<bool> = sqlx::query_scalar(
            r"
            SELECT coach_proposal_sent_at IS NOT NULL
            FROM messaging_channel_links
            WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to read coach_proposal_sent_at: {e}")))?;

        Ok(sent.unwrap_or(false))
    }

    async fn proposed_coach_ids(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Vec<String>> {
        let row = sqlx::query(
            r"
            SELECT proposed_coach_ids FROM messaging_channel_links
             WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to read proposed coaches: {e}")))?;

        // A malformed value degrades to "no offer": a numeric reply then reaches
        // the model as ordinary text, which is the old behaviour.
        Ok(row
            .and_then(|r| r.get::<Option<String>, _>("proposed_coach_ids"))
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .unwrap_or_default())
    }

    async fn mark_coach_proposal_sent(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
        proposed_coach_ids: &[String],
    ) -> AppResult<()> {
        let ids_json =
            serde_json::to_string(proposed_coach_ids).unwrap_or_else(|_| "[]".to_owned());
        sqlx::query(
            r"
            UPDATE messaging_channel_links
               SET coach_proposal_sent_at = now(),
                   proposed_coach_ids = $4
             WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .bind(ids_json)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark coach_proposal_sent: {e}")))?;

        Ok(())
    }

    async fn logout_channel_sender(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        sender_id: &str,
    ) -> AppResult<()> {
        let tid = tenant_id.to_string();

        // messaging_sessions and messaging_messages are intentionally retained for
        // support and audit. The channel_link DELETE below is what unbinds the
        // sender — resolve_linked_session checks the link before resuming a
        // session, so post-logout messages route to the unlinked-prompt path
        // instead of leaking to the previously linked Pierre user.
        sqlx::query(
            "DELETE FROM messaging_channel_links WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3",
        )
        .bind(&tid)
        .bind(channel_type)
        .bind(sender_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete channel link: {e}")))?;

        sqlx::query(
            "UPDATE messaging_link_states SET used = TRUE WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3 AND used = FALSE",
        )
        .bind(&tid)
        .bind(channel_type)
        .bind(sender_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to invalidate OTP states: {e}")))?;

        Ok(())
    }

    // ── In-Chat OTP Linking ──

    async fn get_active_otp_link_state(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        let row = sqlx::query(
            r"
            SELECT id,
                   tenant_id::text AS tenant_id,
                   user_id::text   AS user_id,
                   channel_type, code, method, used,
                   channel_user_id, sender_name, otp_step, email, otp_hash,
                   otp_attempts, expires_at, created_at
            FROM messaging_link_states
            WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3
              AND otp_step IS NOT NULL AND used = FALSE AND expires_at > CURRENT_TIMESTAMP
            ORDER BY created_at DESC
            LIMIT 1
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get active OTP link state: {e}")))?;

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
                "otp_step": r.try_get::<Option<String>, _>("otp_step").ok().flatten(),
                "email": r.try_get::<Option<String>, _>("email").ok().flatten(),
                "otp_hash": r.try_get::<Option<String>, _>("otp_hash").ok().flatten(),
                "otp_attempts": r.get::<i32, _>("otp_attempts"),
                "expires_at": expires_at.to_rfc3339(),
                "created_at": created_at.to_rfc3339(),
            })
        }))
    }

    async fn set_signup_pending_on_link_state(&self, id: &str, email: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE messaging_link_states
            SET email = $1, otp_hash = NULL, otp_step = 'awaiting_signup', otp_attempts = 0
            WHERE id = $2
            ",
        )
        .bind(email)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to park link state on signup: {e}")))?;
        Ok(())
    }

    async fn set_otp_on_link_state(&self, id: &str, email: &str, otp_hash: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE messaging_link_states
            SET email = $1, otp_hash = $2, otp_step = 'awaiting_otp', otp_attempts = 0
            WHERE id = $3
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

    async fn increment_otp_attempts(&self, id: &str) -> AppResult<i32> {
        sqlx::query(
            r"
            UPDATE messaging_link_states
            SET otp_attempts = otp_attempts + 1
            WHERE id = $1
            ",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to increment OTP attempts: {e}")))?;

        let row = sqlx::query(r"SELECT otp_attempts FROM messaging_link_states WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to read OTP attempts: {e}")))?;

        Ok(row.get::<i32, _>("otp_attempts"))
    }

    async fn invalidate_otp_link_states(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE messaging_link_states
            SET used = TRUE
            WHERE tenant_id = $1::uuid AND channel_type = $2 AND channel_user_id = $3
              AND otp_step IS NOT NULL AND used = FALSE
            ",
        )
        .bind(tenant_id.to_string())
        .bind(channel_type)
        .bind(channel_user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to invalidate OTP link states: {e}")))?;

        Ok(())
    }

    async fn claim_backfill_push(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        provider: &str,
        after_ts: i64,
    ) -> AppResult<bool> {
        // tenant_id/user_id are UUID columns (cf. messaging_sessions): bind the
        // string form and cast. ON CONFLICT DO NOTHING claims the window once;
        // rows_affected() == 1 means THIS caller won the claim and must send.
        let result = sqlx::query(
            r"
            INSERT INTO backfill_push_log
                (tenant_id, user_id, provider, after_ts)
            VALUES ($1::uuid, $2::uuid, $3, $4)
            ON CONFLICT (tenant_id, user_id, provider, after_ts) DO NOTHING
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(provider)
        .bind(after_ts)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to claim backfill push: {e}")))?;

        Ok(result.rows_affected() == 1)
    }
}

/// Convert an outbound queue row to JSON value
fn row_to_outbound_json(r: &PgRow) -> Value {
    let next_retry_at: Option<DateTime<Utc>> = r.get("next_retry_at");
    let created_at: DateTime<Utc> = r.get("created_at");
    let updated_at: DateTime<Utc> = r.get("updated_at");
    let user_id: Option<uuid::Uuid> = r.try_get("user_id").ok();

    serde_json::json!({
        "id": r.get::<String, _>("id"),
        "message_id": r.get::<String, _>("message_id"),
        "tenant_id": r.get::<String, _>("tenant_id"),
        "user_id": user_id.map(|u| u.to_string()),
        "channel_type": r.get::<String, _>("channel_type"),
        "payload": r.get::<String, _>("payload"),
        "status": r.get::<String, _>("status"),
        "attempt_count": r.get::<i32, _>("attempt_count"),
        "next_retry_at": next_retry_at.map(|dt| dt.to_rfc3339()),
        "created_at": created_at.to_rfc3339(),
        "updated_at": updated_at.to_rfc3339(),
    })
}
