// ABOUTME: Database operations for provider connections (unified connection tracking)
// ABOUTME: CRUD methods for the provider_connections table, the single source of truth for provider connectivity
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::ProviderConnectionRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::models::{ConnectionType, ProviderConnection};
use sqlx::Row;
use uuid::Uuid;

impl Database {
    /// Register a provider connection (upsert)
    ///
    /// Creates or updates a record in `provider_connections` for the given user/tenant/provider.
    /// Uses `ON CONFLICT` to update the `connection_type`, `connected_at`, and `metadata` if already exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn register_provider_connection_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> AppResult<()> {
        let id = Uuid::new_v4().to_string();
        let user_id_str = user_id.to_string();
        let now = Utc::now().to_rfc3339();
        let conn_type_str = connection_type.as_str();

        sqlx::query(
            r"
            INSERT INTO provider_connections (id, user_id, tenant_id, provider, connection_type, connected_at, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id, tenant_id, provider) DO UPDATE SET
                connection_type = excluded.connection_type,
                connected_at = excluded.connected_at,
                metadata = excluded.metadata
            ",
        )
        .bind(&id)
        .bind(&user_id_str)
        .bind(tenant_id)
        .bind(provider)
        .bind(conn_type_str)
        .bind(&now)
        .bind(metadata)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Remove a provider connection
    ///
    /// Deletes the `provider_connections` record for the given user/tenant/provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn remove_provider_connection_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        let user_id_str = user_id.to_string();

        sqlx::query(
            "DELETE FROM provider_connections WHERE user_id = ? AND tenant_id = ? AND provider = ?",
        )
        .bind(&user_id_str)
        .bind(tenant_id)
        .bind(provider)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Get all provider connections for a user
    ///
    /// Returns all connected providers across tenants (cross-tenant view) when `tenant_id` is None,
    /// or scoped to a specific tenant when `tenant_id` is provided.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_user_provider_connections_impl(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<ProviderConnection>> {
        let user_id_str = user_id.to_string();

        let rows = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, metadata
                FROM provider_connections
                WHERE user_id = ? AND tenant_id = ?
                ORDER BY connected_at DESC
                ",
            )
            .bind(&user_id_str)
            .bind(tid)
            .fetch_all(self.pool())
            .await?
        } else {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, metadata
                FROM provider_connections
                WHERE user_id = ?
                ORDER BY connected_at DESC
                ",
            )
            .bind(&user_id_str)
            .fetch_all(self.pool())
            .await?
        };

        let mut connections = Vec::with_capacity(rows.len());
        for row in rows {
            let conn_type_str: String = row.get("connection_type");
            let connected_at_str: String = row.get("connected_at");
            let connected_at = DateTime::parse_from_rfc3339(&connected_at_str)
                .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc));
            let last_used_at: Option<String> = row.try_get("last_used_at").ok();
            let last_used_at = last_used_at.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            });

            let user_id_from_db: String = row.get("user_id");
            let parsed_user_id = Uuid::parse_str(&user_id_from_db).unwrap_or_else(|_| Uuid::nil());

            connections.push(ProviderConnection {
                id: row.get("id"),
                user_id: parsed_user_id,
                tenant_id: row.get("tenant_id"),
                provider: row.get("provider"),
                connection_type: ConnectionType::from_str_value(&conn_type_str)
                    .unwrap_or(ConnectionType::Manual),
                connected_at,
                last_used_at,
                metadata: row.get("metadata"),
            });
        }

        Ok(connections)
    }

    /// Mark a provider connection as just-used.
    ///
    /// Updates `last_used_at = now()` for the matching row. Returns `Ok(())` when no
    /// row matches — touch-on-read is best-effort and absence is not an error.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn touch_provider_connection_last_used_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        let user_id_str = user_id.to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            UPDATE provider_connections
               SET last_used_at = ?
             WHERE user_id = ? AND tenant_id = ? AND provider = ?
            ",
        )
        .bind(&now)
        .bind(&user_id_str)
        .bind(tenant_id)
        .bind(provider)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Resolve the user's most-recently-used provider connection.
    ///
    /// Orders by `last_used_at DESC NULLS LAST, connected_at DESC` so a freshly-added
    /// connection without a touch yet sits behind any connection that has actually
    /// served data. Returns `None` when the user has no connections at all.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn resolve_most_recent_provider_connection_impl(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<ProviderConnection>> {
        let user_id_str = user_id.to_string();

        let row_opt = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, metadata
                  FROM provider_connections
                 WHERE user_id = ? AND tenant_id = ?
                 ORDER BY last_used_at DESC NULLS LAST, connected_at DESC
                 LIMIT 1
                ",
            )
            .bind(&user_id_str)
            .bind(tid)
            .fetch_optional(self.pool())
            .await?
        } else {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, metadata
                  FROM provider_connections
                 WHERE user_id = ?
                 ORDER BY last_used_at DESC NULLS LAST, connected_at DESC
                 LIMIT 1
                ",
            )
            .bind(&user_id_str)
            .fetch_optional(self.pool())
            .await?
        };

        let Some(row) = row_opt else {
            return Ok(None);
        };

        let conn_type_str: String = row.get("connection_type");
        let connected_at_str: String = row.get("connected_at");
        let connected_at = DateTime::parse_from_rfc3339(&connected_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc));
        let last_used_at: Option<String> = row.try_get("last_used_at").ok();
        let last_used_at = last_used_at.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });
        let user_id_from_db: String = row.get("user_id");
        let parsed_user_id = Uuid::parse_str(&user_id_from_db).unwrap_or_else(|_| Uuid::nil());

        Ok(Some(ProviderConnection {
            id: row.get("id"),
            user_id: parsed_user_id,
            tenant_id: row.get("tenant_id"),
            provider: row.get("provider"),
            connection_type: ConnectionType::from_str_value(&conn_type_str)
                .unwrap_or(ConnectionType::Manual),
            connected_at,
            last_used_at,
            metadata: row.get("metadata"),
        }))
    }

    /// Check if a specific provider is connected for a user
    ///
    /// Cross-tenant check: returns true if the provider is connected in any tenant.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn is_provider_connected_impl(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<bool> {
        let user_id_str = user_id.to_string();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM provider_connections WHERE user_id = ? AND provider = ?",
        )
        .bind(&user_id_str)
        .bind(provider)
        .fetch_one(self.pool())
        .await?;

        Ok(count > 0)
    }
}

#[async_trait]
impl ProviderConnectionRepository for Database {
    async fn register_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        connection_type: &ConnectionType,
        metadata: Option<&str>,
    ) -> AppResult<()> {
        Self::register_provider_connection_impl(
            self,
            user_id,
            tenant_id,
            provider,
            connection_type,
            metadata,
        )
        .await
    }
    async fn remove_connection(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        Self::remove_provider_connection_impl(self, user_id, tenant_id, provider).await
    }
    async fn get_for_user(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<ProviderConnection>> {
        Self::get_user_provider_connections_impl(self, user_id, tenant_id).await
    }
    async fn is_connected(&self, user_id: Uuid, provider: &str) -> AppResult<bool> {
        Self::is_provider_connected_impl(self, user_id, provider).await
    }
    async fn touch_last_used(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        Self::touch_provider_connection_last_used_impl(self, user_id, tenant_id, provider).await
    }
    async fn resolve_most_recent(
        &self,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<ProviderConnection>> {
        Self::resolve_most_recent_provider_connection_impl(self, user_id, tenant_id).await
    }
}
