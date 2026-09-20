// ABOUTME: SQLite user_oauth_tokens.last_sync reads and writes, reached from the OAuthTokenRepository impl
// ABOUTME: The provider sync stamp is per user, tenant and provider; the token rows themselves are in user_oauth_tokens.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

impl Database {
    /// Get last sync timestamp for a provider from `user_oauth_tokens`
    ///
    /// Includes `tenant_id` in the query to prevent cross-tenant sync timestamp
    /// collisions in multi-tenant deployments.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<chrono::DateTime<chrono::Utc>>> {
        // Outer Option: row may not exist. Inner Option: last_sync column is
        // nullable (NULL until the first successful sync) — decoding it as a
        // bare DateTime errors with "unexpected null" on never-synced rows.
        let last_sync: Option<Option<chrono::DateTime<chrono::Utc>>> = sqlx::query_scalar(
            "SELECT last_sync FROM user_oauth_tokens WHERE user_id = $1 AND tenant_id = $2 AND provider = $3",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get provider last sync: {e}")))?;

        Ok(last_sync.flatten())
    }

    /// Update last sync timestamp for a provider in `user_oauth_tokens`
    ///
    /// Includes `tenant_id` in the query to prevent cross-tenant sync timestamp
    /// collisions in multi-tenant deployments.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn update_provider_last_sync(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        sync_time: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE user_oauth_tokens SET last_sync = $1 WHERE user_id = $2 AND tenant_id = $3 AND provider = $4",
        )
        .bind(sync_time)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(provider)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update provider last sync: {e}")))?;

        Ok(())
    }
}
