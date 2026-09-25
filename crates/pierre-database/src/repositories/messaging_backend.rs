// ABOUTME: The one MessagingRepository implementation, emitted per backend by impl_messaging_repository! with that backend's id codec
// ABOUTME: Row-to-JSON projections and the trait body over the statements in repositories/messaging.rs; the shells are one line each
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

/// The error a column that will not decode surfaces as, named after the
/// column so a corrupt row is locatable from the message.
pub fn messaging_column_error(column: &str, e: &sqlx::Error) -> AppError {
    AppError::database(format!("messaging column {column}: {e}"))
}

/// Parse the RFC 3339 instant a caller hands the repository as text.
///
/// # Errors
/// Returns an invalid-input error when the text is not an RFC 3339 instant.
pub fn instant_from_rfc3339(text: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::invalid_input(format!("Invalid RFC 3339 instant '{text}': {e}")))
}

/// Emit the whole [`MessagingRepository`](super::messaging::MessagingRepository)
/// implementation for one backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, its driver's row type, and its uuid codec from
/// [`super::uuid_columns`] — how that backend binds a user id the caller
/// holds as text against a `uuid`-typed column and reads one back. The
/// JSON projections are emitted inside the macro because those id reads are
/// the one thing in them that differs per driver; sqlx resolves the driver
/// from `self.pool()` per expansion.
///
/// Every timestamp column is read as `DateTime<Utc>` and rendered with
/// `to_rfc3339()`: on Postgres that is the `TIMESTAMPTZ` value, on `SQLite`
/// the RFC 3339 text these tables hold, rendered again as the same text.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_messaging_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// A NOT NULL text column via `try_get` only: `Row::get` is
        /// `try_get().unwrap()`, and an unwind here takes the whole
        /// container down with every in-flight request on it.
        fn text_column(row: &$row, name: &str) -> AppResult<String> {
            row.try_get(name).map_err(|e| messaging_column_error(name, &e))
        }

        /// A nullable text column.
        fn text_column_opt(row: &$row, name: &str) -> AppResult<Option<String>> {
            row.try_get(name).map_err(|e| messaging_column_error(name, &e))
        }

        /// A NOT NULL timestamp column, in the RFC 3339 text the wire carries.
        fn stamp_column(row: &$row, name: &str) -> AppResult<String> {
            let at: DateTime<Utc> = row
                .try_get(name)
                .map_err(|e| messaging_column_error(name, &e))?;
            Ok(at.to_rfc3339())
        }

        /// A nullable timestamp column, in the RFC 3339 text the wire carries.
        fn stamp_column_opt(row: &$row, name: &str) -> AppResult<Option<String>> {
            let at: Option<DateTime<Utc>> = row
                .try_get(name)
                .map_err(|e| messaging_column_error(name, &e))?;
            Ok(at.map(|at| at.to_rfc3339()))
        }

        /// The full channel config, secrets included.
        fn channel_config_json(row: &$row) -> AppResult<Value> {
            let is_active: bool = row
                .try_get("is_active")
                .map_err(|e| messaging_column_error("is_active", &e))?;
            Ok(serde_json::json!({
                "id": text_column(row, "id")?,
                "tenant_id": text_column(row, "tenant_id")?,
                "channel_type": text_column(row, "channel_type")?,
                "api_key": text_column_opt(row, "api_key")?,
                "api_secret": text_column_opt(row, "api_secret")?,
                "webhook_secret": text_column_opt(row, "webhook_secret")?,
                "verify_token": text_column_opt(row, "verify_token")?,
                "account_id": text_column_opt(row, "account_id")?,
                "phone_number": text_column_opt(row, "phone_number")?,
                "bot_token": text_column_opt(row, "bot_token")?,
                "is_active": is_active,
                "created_at": stamp_column(row, "created_at")?,
                "updated_at": stamp_column(row, "updated_at")?,
            }))
        }

        /// A channel config as the tenant's listing shows it: no secrets.
        fn channel_config_summary_json(row: &$row) -> AppResult<Value> {
            let is_active: bool = row
                .try_get("is_active")
                .map_err(|e| messaging_column_error("is_active", &e))?;
            Ok(serde_json::json!({
                "id": text_column(row, "id")?,
                "tenant_id": text_column(row, "tenant_id")?,
                "channel_type": text_column(row, "channel_type")?,
                "is_active": is_active,
                "created_at": stamp_column(row, "created_at")?,
                "updated_at": stamp_column(row, "updated_at")?,
            }))
        }

        /// A messaging session. `user_id` and `tenant_id` are uuid columns
        /// on Postgres, surfaced as text for the wire.
        fn session_json(row: &$row) -> AppResult<Value> {
            Ok(serde_json::json!({
                "id": text_column(row, "id")?,
                "user_id": $ids::read_text(row, "user_id")?,
                "tenant_id": $ids::read_text(row, "tenant_id")?,
                "channel_type": text_column(row, "channel_type")?,
                "channel_user_id": text_column(row, "channel_user_id")?,
                "channel_conversation_id": text_column_opt(row, "channel_conversation_id")?,
                "pierre_conversation_id": text_column_opt(row, "pierre_conversation_id")?,
                "last_message_at": stamp_column(row, "last_message_at")?,
                "created_at": stamp_column(row, "created_at")?,
            }))
        }

        /// A message log row, without its raw webhook payload.
        fn message_json(row: &$row) -> AppResult<Value> {
            Ok(serde_json::json!({
                "id": text_column(row, "id")?,
                "tenant_id": text_column(row, "tenant_id")?,
                "session_id": text_column(row, "session_id")?,
                "direction": text_column(row, "direction")?,
                "channel_type": text_column(row, "channel_type")?,
                "channel_message_id": text_column(row, "channel_message_id")?,
                "sender_id": text_column(row, "sender_id")?,
                "content_type": text_column(row, "content_type")?,
                "content_body": text_column_opt(row, "content_body")?,
                "correlation_id": text_column(row, "correlation_id")?,
                "created_at": stamp_column(row, "created_at")?,
            }))
        }

        /// An outbound queue entry. `user_id` is a uuid column on Postgres,
        /// nullable on both.
        fn outbound_json(row: &$row) -> AppResult<Value> {
            let attempt_count: i32 = row
                .try_get("attempt_count")
                .map_err(|e| messaging_column_error("attempt_count", &e))?;
            Ok(serde_json::json!({
                "id": text_column(row, "id")?,
                "message_id": text_column(row, "message_id")?,
                "tenant_id": text_column(row, "tenant_id")?,
                "user_id": $ids::read_text_opt(row, "user_id")?,
                "channel_type": text_column(row, "channel_type")?,
                "payload": text_column(row, "payload")?,
                "status": text_column(row, "status")?,
                "attempt_count": attempt_count,
                "next_retry_at": stamp_column_opt(row, "next_retry_at")?,
                "created_at": stamp_column(row, "created_at")?,
                "updated_at": stamp_column(row, "updated_at")?,
            }))
        }

        /// A channel link as the identity lookup returns it.
        fn channel_link_json(row: &$row) -> AppResult<Value> {
            Ok(serde_json::json!({
                "id": text_column(row, "id")?,
                "tenant_id": $ids::read_text(row, "tenant_id")?,
                "user_id": $ids::read_text(row, "user_id")?,
                "channel_type": text_column(row, "channel_type")?,
                "channel_user_id": text_column(row, "channel_user_id")?,
                "display_name": text_column_opt(row, "display_name")?,
                "linked_at": stamp_column(row, "linked_at")?,
            }))
        }

        /// A channel link as the user's listing returns it: the identity
        /// projection plus the link's own locale. Callers read
        /// `link["locale"]` to address proactive messages; without it every
        /// one went out in the default locale.
        fn user_channel_link_json(row: &$row) -> AppResult<Value> {
            let mut link = channel_link_json(row)?;
            if let Value::Object(map) = &mut link {
                map.insert(
                    "locale".to_owned(),
                    text_column_opt(row, "locale")?.map_or(Value::Null, Value::String),
                );
            }
            Ok(link)
        }

        /// A live in-chat OTP link state. `user_id` is NULL until the flow
        /// completes; the optional columns arrived by later migrations.
        fn otp_link_state_json(row: &$row) -> AppResult<Value> {
            let otp_attempts: i32 = row
                .try_get("otp_attempts")
                .map_err(|e| messaging_column_error("otp_attempts", &e))?;
            Ok(serde_json::json!({
                "id": text_column(row, "id")?,
                "tenant_id": $ids::read_text(row, "tenant_id")?,
                "user_id": $ids::read_text_opt(row, "user_id")?,
                "channel_type": text_column(row, "channel_type")?,
                "code": text_column(row, "code")?,
                "method": text_column(row, "method")?,
                "channel_user_id": text_column_opt(row, "channel_user_id")?,
                "sender_name": text_column_opt(row, "sender_name")?,
                "otp_step": text_column_opt(row, "otp_step")?,
                "email": text_column_opt(row, "email")?,
                "otp_hash": text_column_opt(row, "otp_hash")?,
                "otp_attempts": otp_attempts,
                "expires_at": stamp_column(row, "expires_at")?,
                "created_at": stamp_column(row, "created_at")?,
            }))
        }

        #[async_trait::async_trait]
        impl MessagingRepository for $ty {
            // ── Channel Configs ──

            async fn upsert_channel_config(
                &self,
                params: &UpsertChannelConfigParams<'_>,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_CHANNEL_CONFIG_SQL)
                    .bind(params.id)
                    .bind(params.tenant_id.to_string())
                    .bind(params.channel_type)
                    .bind(params.api_key)
                    .bind(params.api_secret)
                    .bind(params.webhook_secret)
                    .bind(params.verify_token)
                    .bind(params.account_id)
                    .bind(params.phone_number)
                    .bind(params.bot_token)
                    .bind(params.is_active)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert channel config: {e}"))
                    })?;
                Ok(())
            }

            async fn get_channel_config(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
            ) -> AppResult<Option<Value>> {
                let row = sqlx::query(GET_CHANNEL_CONFIG_SQL)
                    .bind(tenant_id.to_string())
                    .bind(channel_type)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get channel config: {e}")))?;
                row.as_ref().map(channel_config_json).transpose()
            }

            async fn list_channel_configs(&self, tenant_id: TenantId) -> AppResult<Vec<Value>> {
                let rows = sqlx::query(LIST_CHANNEL_CONFIGS_SQL)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list channel configs: {e}"))
                    })?;
                rows.iter().map(channel_config_summary_json).collect()
            }

            async fn get_configs_by_channel_type(&self, channel_type: &str) -> AppResult<Vec<Value>> {
                let rows = sqlx::query(CONFIGS_BY_CHANNEL_TYPE_SQL)
                    .bind(channel_type)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get configs by channel type: {e}"))
                    })?;
                rows.iter().map(channel_config_json).collect()
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
                sqlx::query_scalar(CHANNEL_IDENTITY_CLAIMED_SQL)
                    .bind(channel_type)
                    .bind(tenant_id.to_string())
                    .bind(phone_number)
                    .bind(account_id)
                    .bind(bot_token)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to check channel identity ownership: {e}"
                        ))
                    })
            }

            async fn delete_channel_config(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_CHANNEL_CONFIG_SQL)
                    .bind(tenant_id.to_string())
                    .bind(channel_type)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete channel config: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            // ── Sessions ──

            async fn create_session(&self, params: &CreateSessionParams<'_>) -> AppResult<()> {
                sqlx::query(CREATE_SESSION_SQL)
                    .bind(params.id)
                    .bind($ids::bind_text(params.user_id)?)
                    .bind(params.tenant_id)
                    .bind(params.channel_type)
                    .bind(params.channel_user_id)
                    .bind(params.channel_conversation_id)
                    .bind(params.pierre_conversation_id)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create messaging session: {e}"))
                    })?;
                Ok(())
            }

            async fn get_session_by_channel_identity(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                channel_user_id: &str,
                channel_conversation_id: Option<&str>,
            ) -> AppResult<Option<Value>> {
                let row = sqlx::query(SESSION_BY_CHANNEL_IDENTITY_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .bind(channel_conversation_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get session by identity: {e}"))
                    })?;
                row.as_ref().map(session_json).transpose()
            }

            async fn get_session_by_pierre_conversation_id(
                &self,
                tenant_id: TenantId,
                pierre_conversation_id: &str,
            ) -> AppResult<Option<Value>> {
                let row = sqlx::query(SESSION_BY_CONVERSATION_SQL)
                    .bind(tenant_id)
                    .bind(pierre_conversation_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get session by conversation: {e}"))
                    })?;
                row.as_ref().map(session_json).transpose()
            }

            async fn touch_session(&self, session_id: &str) -> AppResult<()> {
                sqlx::query(TOUCH_SESSION_SQL)
                    .bind(Utc::now())
                    .bind(session_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to touch session: {e}")))?;
                Ok(())
            }

            async fn set_session_conversation(
                &self,
                session_id: &str,
                pierre_conversation_id: &str,
            ) -> AppResult<()> {
                sqlx::query(SET_SESSION_CONVERSATION_SQL)
                    .bind(pierre_conversation_id)
                    .bind(session_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update session conversation: {e}"))
                    })?;
                Ok(())
            }

            // ── Messages ──

            async fn insert_message(&self, params: &InsertMessageParams<'_>) -> AppResult<bool> {
                let result = sqlx::query(INSERT_MESSAGE_SQL)
                    .bind(params.id)
                    .bind(params.tenant_id.to_string())
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
                    .bind(Utc::now())
                    .execute(self.pool())
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
                reactions::find_reaction_feedback_target(
                    self.pool(),
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
                let rows = sqlx::query(SESSION_MESSAGES_SQL)
                    .bind(session_id)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get session messages: {e}"))
                    })?;
                rows.iter().map(message_json).collect()
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
                sqlx::query(INSERT_DELIVERY_RECEIPT_SQL)
                    .bind(id)
                    .bind(tenant_id.to_string())
                    .bind(message_id)
                    .bind(channel_message_id)
                    .bind(status)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert delivery receipt: {e}"))
                    })?;
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
                sqlx::query(ENQUEUE_OUTBOUND_SQL)
                    .bind(id)
                    .bind(message_id)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind_text_opt(user_id)?)
                    .bind(channel_type)
                    .bind(payload)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to enqueue outbound message: {e}"))
                    })?;
                Ok(())
            }

            async fn get_pending_outbound(
                &self,
                tenant_id: TenantId,
                limit: i64,
            ) -> AppResult<Vec<Value>> {
                let rows = sqlx::query(PENDING_OUTBOUND_SQL)
                    .bind(tenant_id.to_string())
                    .bind(Utc::now())
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get pending outbound: {e}"))
                    })?;
                rows.iter().map(outbound_json).collect()
            }

            async fn get_all_pending_outbound(&self, limit: i64) -> AppResult<Vec<Value>> {
                let rows = sqlx::query(ALL_PENDING_OUTBOUND_SQL)
                    .bind(Utc::now())
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get all pending outbound: {e}"))
                    })?;
                rows.iter().map(outbound_json).collect()
            }

            async fn update_outbound_status(
                &self,
                id: &str,
                status: &str,
                attempt_count: i32,
                next_retry_at: Option<&str>,
            ) -> AppResult<()> {
                let next_retry = next_retry_at.map(instant_from_rfc3339).transpose()?;
                sqlx::query(UPDATE_OUTBOUND_STATUS_SQL)
                    .bind(status)
                    .bind(attempt_count)
                    .bind(next_retry)
                    .bind(Utc::now())
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update outbound status: {e}"))
                    })?;
                Ok(())
            }

            // ── Channel Linking ──

            async fn create_link_state(&self, params: &CreateLinkStateParams<'_>) -> AppResult<()> {
                link_states::create_link_state(self.pool(), params).await
            }

            async fn consume_link_state(&self, code: &str, tenant_id: TenantId) -> AppResult<Value> {
                link_states::consume_link_state(self.pool(), code, tenant_id).await
            }

            async fn get_link_state(&self, code: &str) -> AppResult<Option<Value>> {
                link_states::get_link_state(self.pool(), code).await
            }

            async fn complete_link_state(&self, code: &str, user_id: &str) -> AppResult<Value> {
                link_states::complete_link_state(self.pool(), code, user_id).await
            }

            async fn create_channel_link(
                &self,
                params: &CreateChannelLinkParams<'_>,
            ) -> AppResult<()> {
                sqlx::query(CREATE_CHANNEL_LINK_SQL)
                    .bind(params.id)
                    .bind(params.tenant_id)
                    .bind($ids::bind_text(params.user_id)?)
                    .bind(params.channel_type)
                    .bind(params.channel_user_id)
                    .bind(params.display_name)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        // The identity is already linked under this tenant:
                        // the unique constraint on (tenant_id, channel_type,
                        // channel_user_id), reported by the driver as such.
                        if e.as_database_error()
                            .is_some_and(|db| db.is_unique_violation())
                        {
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
                let row = sqlx::query(GET_CHANNEL_LINK_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get channel link: {e}")))?;
                row.as_ref().map(channel_link_json).transpose()
            }

            async fn get_channel_link_tenant(
                &self,
                channel_type: &str,
                channel_user_id: &str,
            ) -> AppResult<Option<TenantId>> {
                let row = sqlx::query(CHANNEL_LINK_TENANT_SQL)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to resolve channel link tenant: {e}"))
                    })?;
                row.as_ref()
                    .map(|r| $ids::read(r, "tenant_id").map(TenantId::from_uuid))
                    .transpose()
            }

            async fn list_user_channel_links(
                &self,
                tenant_id: TenantId,
                user_id: &str,
            ) -> AppResult<Vec<Value>> {
                let rows = sqlx::query(LIST_USER_CHANNEL_LINKS_SQL)
                    .bind(tenant_id)
                    .bind($ids::bind_text(user_id)?)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list user channel links: {e}"))
                    })?;
                rows.iter().map(user_channel_link_json).collect()
            }

            async fn delete_channel_link(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                channel_type: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_CHANNEL_LINK_SQL)
                    .bind(tenant_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(channel_type)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete channel link: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn get_channel_link_locale(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                channel_user_id: &str,
            ) -> AppResult<Option<String>> {
                let locale: Option<Option<String>> = sqlx::query_scalar(CHANNEL_LINK_LOCALE_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get channel link locale: {e}"))
                    })?;
                Ok(locale.flatten())
            }

            async fn set_channel_link_locale(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                channel_type: &str,
                locale: Option<&str>,
            ) -> AppResult<()> {
                let result = sqlx::query(SET_CHANNEL_LINK_LOCALE_SQL)
                    .bind(locale)
                    .bind(tenant_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(channel_type)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set channel link locale: {e}"))
                    })?;
                (result.rows_affected() > 0).ok_or_else(|| {
                    AppError::not_found(format!(
                        "Channel link for user {user_id} on {channel_type}"
                    ))
                })
            }

            async fn agent_proposal_sent(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                channel_user_id: &str,
            ) -> AppResult<bool> {
                let sent: Option<bool> = sqlx::query_scalar(AGENT_PROPOSAL_SENT_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read agent_proposal_sent_at: {e}"))
                    })?;
                Ok(sent.unwrap_or(false))
            }

            async fn mark_agent_proposal_sent(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                channel_user_id: &str,
                proposed_agent_ids: &[String],
            ) -> AppResult<()> {
                let ids_json = serde_json::to_string(proposed_agent_ids)?;
                sqlx::query(MARK_AGENT_PROPOSAL_SENT_SQL)
                    .bind(Utc::now())
                    .bind(ids_json)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark agent proposal sent: {e}"))
                    })?;
                Ok(())
            }

            async fn proposed_agent_ids(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                channel_user_id: &str,
            ) -> AppResult<Vec<String>> {
                let stored: Option<Option<String>> = sqlx::query_scalar(PROPOSED_AGENT_IDS_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read proposed agent ids: {e}"))
                    })?;
                // A malformed value degrades to "no offer" rather than
                // erroring: the worst outcome is that a numeric reply reaches
                // the model as ordinary text, which is exactly the old
                // behaviour.
                Ok(stored
                    .flatten()
                    .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
                    .unwrap_or_default())
            }

            async fn logout_channel_sender(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                sender_id: &str,
            ) -> AppResult<()> {
                sqlx::query(LOGOUT_DELETE_LINK_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(sender_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete channel link: {e}"))
                    })?;
                sqlx::query(LOGOUT_INVALIDATE_STATES_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(sender_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to invalidate OTP states: {e}"))
                    })?;
                Ok(())
            }

            // ── In-Chat OTP Linking ──

            async fn get_active_otp_link_state(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                channel_user_id: &str,
            ) -> AppResult<Option<Value>> {
                let row = sqlx::query(ACTIVE_OTP_LINK_STATE_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .bind(Utc::now())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get active OTP link state: {e}"))
                    })?;
                row.as_ref().map(otp_link_state_json).transpose()
            }

            async fn set_otp_on_link_state(
                &self,
                id: &str,
                email: &str,
                otp_hash: &str,
            ) -> AppResult<()> {
                sqlx::query(SET_OTP_ON_LINK_STATE_SQL)
                    .bind(email)
                    .bind(otp_hash)
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set OTP on link state: {e}"))
                    })?;
                Ok(())
            }

            async fn set_signup_pending_on_link_state(
                &self,
                id: &str,
                email: &str,
            ) -> AppResult<()> {
                sqlx::query(SET_SIGNUP_PENDING_SQL)
                    .bind(email)
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to park link state on signup: {e}"))
                    })?;
                Ok(())
            }

            async fn increment_otp_attempts(&self, id: &str) -> AppResult<i32> {
                sqlx::query_scalar(INCREMENT_OTP_ATTEMPTS_SQL)
                    .bind(id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to increment OTP attempts: {e}"))
                    })
            }

            async fn invalidate_otp_link_states(
                &self,
                tenant_id: TenantId,
                channel_type: &str,
                channel_user_id: &str,
            ) -> AppResult<()> {
                sqlx::query(INVALIDATE_OTP_LINK_STATES_SQL)
                    .bind(tenant_id)
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to invalidate OTP link states: {e}"))
                    })?;
                Ok(())
            }

            // ── Backfill Push Dedup ──

            async fn claim_backfill_push(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                provider: &str,
                after_ts: i64,
            ) -> AppResult<bool> {
                let result = sqlx::query(CLAIM_BACKFILL_PUSH_SQL)
                    .bind(tenant_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(provider)
                    .bind(after_ts)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim backfill push: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }
        }
    };
}
pub(crate) use impl_messaging_repository;
