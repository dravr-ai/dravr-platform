// ABOUTME: Database operations for provider connections (unified connection tracking)
// ABOUTME: CRUD methods for the provider_connections table, the single source of truth for provider connectivity
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::column_decode::uuid_column;
use crate::database::Database;
use crate::repositories::ProviderConnectionRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::models::{ConnectionStatus, ConnectionType, ProviderConnection};
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
                metadata = excluded.metadata,
                status = 'active',
                status_changed_at = excluded.connected_at,
                last_error = NULL,
                notified_at = NULL
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
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata
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
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata
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
            let parsed_user_id = uuid_column("provider_connections.user_id", &user_id_from_db)?;

            connections.push(ProviderConnection {
                id: row.get("id"),
                user_id: parsed_user_id,
                tenant_id: row.get("tenant_id"),
                provider: row.get("provider"),
                connection_type: ConnectionType::from_str_value(&conn_type_str)
                    .unwrap_or(ConnectionType::Manual),
                connected_at,
                last_used_at,
                status: ConnectionStatus::from_str_value(&row.get::<String, _>("status")),
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

    /// Resolve the user's most-recently-used *usable* provider connection.
    ///
    /// Orders by health first (`status = 'active'` ahead of `needs_reauth`/`revoked`),
    /// then `last_used_at DESC NULLS LAST, connected_at DESC` so a freshly-added
    /// connection without a touch yet sits behind any connection that has actually
    /// served data. A flagged connection is elected only when the user has nothing
    /// else — the caller then surfaces the reconnect signal instead of a healthy
    /// sibling being shadowed by a dead one. Returns `None` when the user has no
    /// connections at all.
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
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata
                  FROM provider_connections
                 WHERE user_id = ? AND tenant_id = ?
                 ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END,
                          last_used_at DESC NULLS LAST,
                          connected_at DESC
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
                SELECT id, user_id, tenant_id, provider, connection_type, connected_at, last_used_at, status, metadata
                  FROM provider_connections
                 WHERE user_id = ?
                 ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END,
                          last_used_at DESC NULLS LAST,
                          connected_at DESC
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
        let parsed_user_id = uuid_column("provider_connections.user_id", &user_id_from_db)?;

        Ok(Some(ProviderConnection {
            id: row.get("id"),
            user_id: parsed_user_id,
            tenant_id: row.get("tenant_id"),
            provider: row.get("provider"),
            connection_type: ConnectionType::from_str_value(&conn_type_str)
                .unwrap_or(ConnectionType::Manual),
            connected_at,
            last_used_at,
            status: ConnectionStatus::from_str_value(&row.get::<String, _>("status")),
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

    /// Mark a provider connection as needing re-authentication after a non-recoverable
    /// token refresh failure.
    ///
    /// Transitions `status` to `needs_reauth`, stamps `status_changed_at`, and records the
    /// token-free error classification in `last_error`. Guarded on `status != 'needs_reauth'`
    /// so the transition timestamp (and the notify dedup it drives) reflects the *first*
    /// failure, not every retry. No-op when the (user, tenant, provider) row does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn mark_provider_connection_needs_reauth_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        error_code: Option<&str>,
    ) -> AppResult<()> {
        let user_id_str = user_id.to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            UPDATE provider_connections
               SET status = 'needs_reauth',
                   status_changed_at = ?,
                   last_error = ?
             WHERE user_id = ? AND tenant_id = ? AND provider = ?
               AND status != 'needs_reauth'
            ",
        )
        .bind(&now)
        .bind(error_code)
        .bind(&user_id_str)
        .bind(tenant_id)
        .bind(provider)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Re-arm a provider connection after a successful (re)connect or token refresh.
    ///
    /// Transitions `status` back to `active` and clears `last_error` and `notified_at`
    /// so a future disconnect notifies the user again. Guarded on `status != 'active'`
    /// so a normal refresh of an already-healthy connection is a no-op. No-op when the
    /// (user, tenant, provider) row does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn mark_provider_connection_active_impl(
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
               SET status = 'active',
                   status_changed_at = ?,
                   last_error = NULL,
                   notified_at = NULL
             WHERE user_id = ? AND tenant_id = ? AND provider = ?
               AND status != 'active'
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

    /// Atomically claim the one-time disconnect notification for a `needs_reauth`
    /// connection.
    ///
    /// Sets `notified_at` only when the connection is currently `needs_reauth` AND has not
    /// been notified yet, returning whether this call won the claim. Drives "notify the
    /// user exactly once per disconnect": the first caller after the transition gets `true`
    /// and sends the push; everyone else gets `false`. The marker is cleared by
    /// [`Self::mark_provider_connection_active_impl`] on reconnect, re-arming a future nudge.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn claim_provider_reauth_notification_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<bool> {
        let user_id_str = user_id.to_string();
        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            r"
            UPDATE provider_connections
               SET notified_at = ?
             WHERE user_id = ? AND tenant_id = ? AND provider = ?
               AND status = 'needs_reauth'
               AND notified_at IS NULL
            ",
        )
        .bind(&now)
        .bind(&user_id_str)
        .bind(tenant_id)
        .bind(provider)
        .execute(self.pool())
        .await?;

        Ok(result.rows_affected() > 0)
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
    async fn mark_needs_reauth(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        error_code: Option<&str>,
    ) -> AppResult<()> {
        Self::mark_provider_connection_needs_reauth_impl(
            self, user_id, tenant_id, provider, error_code,
        )
        .await
    }
    async fn mark_active(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        Self::mark_provider_connection_active_impl(self, user_id, tenant_id, provider).await
    }
    async fn claim_reauth_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<bool> {
        Self::claim_provider_reauth_notification_impl(self, user_id, tenant_id, provider).await
    }
}
