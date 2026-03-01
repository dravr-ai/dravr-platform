// ABOUTME: SQLite database operations for messaging connections and channel bindings
// ABOUTME: Implements MessagingRepository for bidirectional chat bridging with tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::repositories::MessagingRepository;
use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::messaging::{
    ChannelBindingRecord, CreateChannelBindingParams, CreateMessagingConnectionParams,
    MessagingConnectionRecord,
};
use pierre_core::models::TenantId;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use super::Database;

#[async_trait]
impl MessagingRepository for Database {
    async fn create_messaging_connection(
        &self,
        params: &CreateMessagingConnectionParams<'_>,
    ) -> AppResult<MessagingConnectionRecord> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let aad_context = messaging_aad_context(params.tenant_id, params.provider, params.team_id);
        let encrypted_bot_token = self.encrypt_data_with_aad(params.bot_token, &aad_context)?;
        let encrypted_signing_secret =
            self.encrypt_data_with_aad(params.signing_secret, &aad_context)?;

        sqlx::query(
            r"
            INSERT INTO messaging_connections
                (id, tenant_id, provider, team_id, team_name, bot_token, signing_secret, created_by, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id)
        .bind(params.provider)
        .bind(params.team_id)
        .bind(params.team_name)
        .bind(&encrypted_bot_token)
        .bind(&encrypted_signing_secret)
        .bind(params.created_by)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create messaging connection: {e}")))?;

        // Return decrypted values so callers work transparently
        Ok(MessagingConnectionRecord {
            id,
            tenant_id: params.tenant_id.to_owned(),
            provider: params.provider.to_owned(),
            team_id: params.team_id.to_owned(),
            team_name: params.team_name.map(ToOwned::to_owned),
            bot_token: params.bot_token.to_owned(),
            signing_secret: params.signing_secret.to_owned(),
            created_by: params.created_by.map(ToOwned::to_owned),
            created_at: now.clone(),
            updated_at: now,
        })
    }

    async fn get_messaging_connection(
        &self,
        id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<MessagingConnectionRecord>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, provider, team_id, team_name, bot_token, signing_secret,
                   created_by, created_at, updated_at
            FROM messaging_connections
            WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get messaging connection: {e}")))?;

        row.map(|r| self.decrypt_messaging_row(&r)).transpose()
    }

    async fn get_messaging_connection_by_team(
        &self,
        provider: &str,
        team_id: &str,
    ) -> AppResult<Option<MessagingConnectionRecord>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, provider, team_id, team_name, bot_token, signing_secret,
                   created_by, created_at, updated_at
            FROM messaging_connections
            WHERE provider = $1 AND team_id = $2
            ",
        )
        .bind(provider)
        .bind(team_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get messaging connection by team: {e}"))
        })?;

        row.map(|r| self.decrypt_messaging_row(&r)).transpose()
    }

    async fn list_messaging_connections(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessagingConnectionRecord>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, provider, team_id, team_name, bot_token, signing_secret,
                   created_by, created_at, updated_at
            FROM messaging_connections
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(tenant_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list messaging connections: {e}")))?;

        rows.into_iter()
            .map(|r| self.decrypt_messaging_row(&r))
            .collect()
    }

    async fn delete_messaging_connection(&self, id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM messaging_connections
            WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to delete messaging connection: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn create_channel_binding(
        &self,
        params: &CreateChannelBindingParams<'_>,
    ) -> AppResult<ChannelBindingRecord> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO channel_bindings
                (id, messaging_connection_id, tenant_id, channel_id, channel_name,
                 conversation_id, user_id, active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8, $8)
            ",
        )
        .bind(&id)
        .bind(params.messaging_connection_id)
        .bind(params.tenant_id)
        .bind(params.channel_id)
        .bind(params.channel_name)
        .bind(params.conversation_id)
        .bind(params.user_id)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create channel binding: {e}")))?;

        Ok(ChannelBindingRecord {
            id,
            messaging_connection_id: params.messaging_connection_id.to_owned(),
            tenant_id: params.tenant_id.to_owned(),
            channel_id: params.channel_id.to_owned(),
            channel_name: params.channel_name.map(ToOwned::to_owned),
            conversation_id: params.conversation_id.to_owned(),
            user_id: params.user_id.to_owned(),
            active: true,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    async fn get_channel_binding_by_channel(
        &self,
        connection_id: &str,
        channel_id: &str,
    ) -> AppResult<Option<ChannelBindingRecord>> {
        let row = sqlx::query(
            r"
            SELECT id, messaging_connection_id, tenant_id, channel_id, channel_name,
                   conversation_id, user_id, active, created_at, updated_at
            FROM channel_bindings
            WHERE messaging_connection_id = $1 AND channel_id = $2 AND active = 1
            ",
        )
        .bind(connection_id)
        .bind(channel_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get channel binding: {e}")))?;

        Ok(row.map(|r| ChannelBindingRecord {
            id: r.get("id"),
            messaging_connection_id: r.get("messaging_connection_id"),
            tenant_id: r.get("tenant_id"),
            channel_id: r.get("channel_id"),
            channel_name: r.get("channel_name"),
            conversation_id: r.get("conversation_id"),
            user_id: r.get("user_id"),
            active: r.get::<i32, _>("active") == 1,
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    async fn list_channel_bindings(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ChannelBindingRecord>> {
        let rows = sqlx::query(
            r"
            SELECT id, messaging_connection_id, tenant_id, channel_id, channel_name,
                   conversation_id, user_id, active, created_at, updated_at
            FROM channel_bindings
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(tenant_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list channel bindings: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| ChannelBindingRecord {
                id: r.get("id"),
                messaging_connection_id: r.get("messaging_connection_id"),
                tenant_id: r.get("tenant_id"),
                channel_id: r.get("channel_id"),
                channel_name: r.get("channel_name"),
                conversation_id: r.get("conversation_id"),
                user_id: r.get("user_id"),
                active: r.get::<i32, _>("active") == 1,
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    async fn delete_channel_binding(&self, id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM channel_bindings
            WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to delete channel binding: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}

/// Build the AAD context string for messaging credential encryption.
/// Format: `{tenant_id}|{provider}|{team_id}|messaging_connections`
fn messaging_aad_context(tenant_id: &str, provider: &str, team_id: &str) -> String {
    format!("{tenant_id}|{provider}|{team_id}|messaging_connections")
}

impl Database {
    /// Decrypt `bot_token` and `signing_secret` from a `messaging_connections` row
    fn decrypt_messaging_row(&self, row: &SqliteRow) -> AppResult<MessagingConnectionRecord> {
        let tenant_id: String = row.get("tenant_id");
        let provider: String = row.get("provider");
        let team_id: String = row.get("team_id");

        let aad_context = messaging_aad_context(&tenant_id, &provider, &team_id);

        let encrypted_bot_token: String = row.get("bot_token");
        let bot_token = self.decrypt_data_with_aad(&encrypted_bot_token, &aad_context)?;

        let encrypted_signing_secret: String = row.get("signing_secret");
        let signing_secret = self.decrypt_data_with_aad(&encrypted_signing_secret, &aad_context)?;

        Ok(MessagingConnectionRecord {
            id: row.get("id"),
            tenant_id,
            provider,
            team_id,
            team_name: row.get("team_name"),
            bot_token,
            signing_secret,
            created_by: row.get("created_by"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
    }
}
