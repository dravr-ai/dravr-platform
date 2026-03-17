// ABOUTME: Messaging repository dispatch for the database factory
// ABOUTME: Delegates MessagingRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::repositories::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, InsertMessageParams,
    MessagingRepository, UpsertChannelConfigParams,
};
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use serde_json::Value;

#[async_trait]
impl MessagingRepository for Database {
    async fn upsert_channel_config(&self, params: &UpsertChannelConfigParams<'_>) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.upsert_channel_config_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.upsert_channel_config(params).await,
        }
    }

    async fn get_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<Option<Value>> {
        match self {
            Self::SQLite(db) => db.get_channel_config_impl(tenant_id, channel_type).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_channel_config(tenant_id, channel_type).await,
        }
    }

    async fn list_channel_configs(&self, tenant_id: TenantId) -> AppResult<Vec<Value>> {
        match self {
            Self::SQLite(db) => db.list_channel_configs_impl(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_channel_configs(tenant_id).await,
        }
    }

    async fn get_configs_by_channel_type(&self, channel_type: &str) -> AppResult<Vec<Value>> {
        match self {
            Self::SQLite(db) => db.get_configs_by_channel_type_impl(channel_type).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_configs_by_channel_type(channel_type).await,
        }
    }

    async fn delete_channel_config(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.delete_channel_config_impl(tenant_id, channel_type).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_channel_config(tenant_id, channel_type).await,
        }
    }

    async fn create_session(&self, params: &CreateSessionParams<'_>) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.create_session_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_session(params).await,
        }
    }

    async fn get_session_by_channel_identity(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        match self {
            Self::SQLite(db) => {
                db.get_session_by_channel_identity_impl(tenant_id, channel_type, channel_user_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_session_by_channel_identity(tenant_id, channel_type, channel_user_id)
                    .await
            }
        }
    }

    async fn touch_session(&self, session_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.touch_session_impl(session_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.touch_session(session_id).await,
        }
    }

    async fn insert_message(&self, params: &InsertMessageParams<'_>) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.insert_message_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.insert_message(params).await,
        }
    }

    async fn get_session_messages(
        &self,
        session_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Value>> {
        match self {
            Self::SQLite(db) => {
                db.get_session_messages_impl(session_id, tenant_id, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_session_messages(session_id, tenant_id, limit, offset)
                    .await
            }
        }
    }

    async fn insert_delivery_receipt(
        &self,
        id: &str,
        tenant_id: TenantId,
        message_id: &str,
        channel_message_id: Option<&str>,
        status: &str,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.insert_delivery_receipt_impl(
                    id,
                    tenant_id,
                    message_id,
                    channel_message_id,
                    status,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.insert_delivery_receipt(id, tenant_id, message_id, channel_message_id, status)
                    .await
            }
        }
    }

    async fn enqueue_outbound(
        &self,
        id: &str,
        message_id: &str,
        tenant_id: TenantId,
        channel_type: &str,
        payload: &str,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.enqueue_outbound_impl(id, message_id, tenant_id, channel_type, payload)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.enqueue_outbound(id, message_id, tenant_id, channel_type, payload)
                    .await
            }
        }
    }

    async fn get_pending_outbound(&self, tenant_id: TenantId, limit: i64) -> AppResult<Vec<Value>> {
        match self {
            Self::SQLite(db) => db.get_pending_outbound_impl(tenant_id, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_pending_outbound(tenant_id, limit).await,
        }
    }

    async fn get_all_pending_outbound(&self, limit: i64) -> AppResult<Vec<Value>> {
        match self {
            Self::SQLite(db) => db.get_all_pending_outbound_impl(limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_all_pending_outbound(limit).await,
        }
    }

    async fn update_outbound_status(
        &self,
        id: &str,
        status: &str,
        attempt_count: i32,
        next_retry_at: Option<&str>,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.update_outbound_status_impl(id, status, attempt_count, next_retry_at)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_outbound_status(id, status, attempt_count, next_retry_at)
                    .await
            }
        }
    }

    async fn create_link_state(&self, params: &CreateLinkStateParams<'_>) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.create_link_state_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_link_state(params).await,
        }
    }

    async fn consume_link_state(&self, code: &str, tenant_id: TenantId) -> AppResult<Value> {
        match self {
            Self::SQLite(db) => db.consume_link_state_impl(code, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.consume_link_state(code, tenant_id).await,
        }
    }

    async fn get_link_state(&self, code: &str) -> AppResult<Option<Value>> {
        match self {
            Self::SQLite(db) => db.get_link_state_impl(code).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_link_state(code).await,
        }
    }

    async fn complete_link_state(&self, code: &str, user_id: &str) -> AppResult<Value> {
        match self {
            Self::SQLite(db) => db.complete_link_state_impl(code, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.complete_link_state(code, user_id).await,
        }
    }

    async fn create_channel_link(&self, params: &CreateChannelLinkParams<'_>) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.create_channel_link_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_channel_link(params).await,
        }
    }

    async fn get_channel_link(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        match self {
            Self::SQLite(db) => {
                db.get_channel_link_impl(tenant_id, channel_type, channel_user_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_channel_link(tenant_id, channel_type, channel_user_id)
                    .await
            }
        }
    }

    async fn list_user_channel_links(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<Value>> {
        match self {
            Self::SQLite(db) => db.list_user_channel_links_impl(tenant_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_user_channel_links(tenant_id, user_id).await,
        }
    }

    async fn delete_channel_link(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        channel_type: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.delete_channel_link_impl(tenant_id, user_id, channel_type)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.delete_channel_link(tenant_id, user_id, channel_type)
                    .await
            }
        }
    }

    // ── In-Chat OTP Linking ──

    async fn get_active_otp_link_state(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<Option<Value>> {
        match self {
            Self::SQLite(db) => {
                db.get_active_otp_link_state_impl(tenant_id, channel_type, channel_user_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_active_otp_link_state(tenant_id, channel_type, channel_user_id)
                    .await
            }
        }
    }

    async fn set_otp_on_link_state(&self, id: &str, email: &str, otp_hash: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.set_otp_on_link_state_impl(id, email, otp_hash).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.set_otp_on_link_state(id, email, otp_hash).await,
        }
    }

    async fn increment_otp_attempts(&self, id: &str) -> AppResult<i32> {
        match self {
            Self::SQLite(db) => db.increment_otp_attempts_impl(id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.increment_otp_attempts(id).await,
        }
    }

    async fn invalidate_otp_link_states(
        &self,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.invalidate_otp_link_states_impl(tenant_id, channel_type, channel_user_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.invalidate_otp_link_states(tenant_id, channel_type, channel_user_id)
                    .await
            }
        }
    }
}
