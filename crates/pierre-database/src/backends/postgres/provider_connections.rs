// ABOUTME: PostgreSQL-backed ProviderConnectionRepository, emitted from the shared implementation in repositories/provider_connections.rs
// ABOUTME: user_id is TEXT and every timestamp TIMESTAMPTZ here, so the shared statements need no engine argument
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    ConnectionType, ProviderAccountRole, ProviderConnection, ReauthMark, TenantId, UserOAuthToken,
    API_KEY_TOKEN_TYPE,
};
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::clear_lock::{self, ProviderClearLock};
use crate::repositories::provider_connections::{
    connection_from_row, impl_provider_connection_repository, reauth_mark,
    CLAIM_REAUTH_NOTIFICATION_SQL, CLAIM_REAUTH_RETRY_SQL, CLAIM_SYNC_FAILURE_NOTIFICATION_SQL,
    CONNECTION_STATUS_SQL, COUNT_SERVER_LEVEL_GRANTS_SQL, GET_FOR_USER_IN_TENANT_SQL,
    GET_FOR_USER_SQL, GET_PROVIDER_GRANTS_SQL, LIST_ACTIVE_CONNECTION_USERS_SQL,
    LIST_REAUTH_RETRY_DUE_SQL, MARK_ACTIVE_IF_FLAGGED_FOR_SQL, MARK_ACTIVE_IF_NEEDS_REAUTH_SQL,
    MARK_ACTIVE_SQL, MARK_ACTIVE_UNLESS_FLAGGED_FOR_SQL, MARK_NEEDS_REAUTH_IF_TOKEN_CURRENT_SQL,
    MARK_NEEDS_REAUTH_SQL, RANK_FOR_ELECTION_IN_TENANT_SQL, RANK_FOR_ELECTION_SQL,
    REARM_SYNC_FAILURE_NOTIFICATION_SQL, REGISTER_CONNECTION_SQL, REMOVE_CONNECTION_SQL,
    REMOVE_DELEGATED_CONNECTION_SQL, SET_ACCOUNT_ROLE_SQL, SET_PROVIDER_GRANTS_SQL,
    TOUCH_LAST_USED_SQL,
};
use crate::repositories::ProviderConnectionRepository;

impl_provider_connection_repository!(PostgresDatabase);

/// Take the transaction-scoped advisory lock of a clear key.
///
/// The key is hashed by the server (`hashtextextended`), so every instance
/// and every build derives the same lock id from it.
const PROVIDER_CLEAR_ADVISORY_LOCK_SQL: &str =
    "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))";

impl PostgresDatabase {
    /// The provider clear lock on PostgreSQL, shared by every instance.
    ///
    /// This instance's waiters queue in process, then the holder takes the
    /// advisory lock on a transaction left open until the lock drops.
    async fn hold_provider_clear(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<ProviderClearLock> {
        let key = clear_lock::clear_key(user_id, tenant_id, provider);
        let local = clear_lock::hold_local(&key).await;
        let mut remote = self.pool().begin().await.map_err(|e| {
            AppError::database(format!("Failed to open the provider clear lock: {e}"))
        })?;
        sqlx::query(PROVIDER_CLEAR_ADVISORY_LOCK_SQL)
            .bind(&key)
            .execute(&mut *remote)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to take the provider clear lock: {e}"))
            })?;
        Ok(ProviderClearLock::with_remote(local, remote))
    }
}
