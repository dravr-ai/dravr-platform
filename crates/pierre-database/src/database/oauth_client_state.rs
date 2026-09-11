// ABOUTME: SQLite persistence for OAuth client states — the CSRF `state` and PKCE verifier
// ABOUTME: minted at authorization start, consumed once at callback, and reaped once expired
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! `SQLite` OAuth client-state persistence.
//!
//! Owns the `oauth_client_states` table: the one-shot CSRF `state` value and the
//! PKCE code verifier that a provider authorization round trip is pinned to.
//!
//! It is its own module because its three operations form a closed lifecycle —
//! store, consume-exactly-once, reap-when-expired — that shares no row mapper or
//! query with the rest of [`Database`]. The inherent methods here back the
//! `OAuthClientStateRepository` impl in [`super::tokens`].

use super::Database;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthClientState;
use sqlx::Row;
use std::collections::BTreeMap;

impl Database {
    /// Store OAuth client-side state for CSRF protection (internal implementation)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn store_oauth_client_state_impl(&self, state: &OAuthClientState) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO oauth_client_states (state, provider, user_id, tenant_id, redirect_uri, scope, pkce_code_verifier, created_at, expires_at, used, oauth_app_client_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ",
        )
        .bind(&state.state)
        .bind(&state.provider)
        .bind(state.user_id)
        .bind(&state.tenant_id)
        .bind(&state.redirect_uri)
        .bind(&state.scope)
        .bind(&state.pkce_code_verifier)
        .bind(state.created_at)
        .bind(state.expires_at)
        .bind(state.used)
        .bind(&state.oauth_app_client_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store OAuth client state: {e}")))?;

        Ok(())
    }

    /// Atomically consume OAuth client state (internal implementation)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn consume_oauth_client_state_impl(
        &self,
        state_value: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<OAuthClientState>> {
        let row = sqlx::query(
            r"
            UPDATE oauth_client_states
            SET used = 1
            WHERE state = ?1
              AND provider = ?2
              AND used = 0
              AND expires_at > ?3
            RETURNING state, provider, user_id, tenant_id, redirect_uri, scope, pkce_code_verifier, created_at, expires_at, used, oauth_app_client_id
            ",
        )
        .bind(state_value)
        .bind(provider)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to consume OAuth client state: {e}")))?;

        if let Some(row) = row {
            Ok(Some(OAuthClientState {
                state: row
                    .try_get("state")
                    .map_err(|e| AppError::database(format!("Failed to get state: {e}")))?,
                provider: row
                    .try_get("provider")
                    .map_err(|e| AppError::database(format!("Failed to get provider: {e}")))?,
                user_id: row
                    .try_get("user_id")
                    .map_err(|e| AppError::database(format!("Failed to get user_id: {e}")))?,
                tenant_id: row
                    .try_get("tenant_id")
                    .map_err(|e| AppError::database(format!("Failed to get tenant_id: {e}")))?,
                redirect_uri: row
                    .try_get("redirect_uri")
                    .map_err(|e| AppError::database(format!("Failed to get redirect_uri: {e}")))?,
                scope: row
                    .try_get("scope")
                    .map_err(|e| AppError::database(format!("Failed to get scope: {e}")))?,
                pkce_code_verifier: row.try_get("pkce_code_verifier").map_err(|e| {
                    AppError::database(format!("Failed to get pkce_code_verifier: {e}"))
                })?,
                oauth_app_client_id: row.try_get("oauth_app_client_id").ok(),
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| AppError::database(format!("Failed to get created_at: {e}")))?,
                expires_at: row
                    .try_get("expires_at")
                    .map_err(|e| AppError::database(format!("Failed to get expires_at: {e}")))?,
                used: row
                    .try_get("used")
                    .map_err(|e| AppError::database(format!("Failed to get used: {e}")))?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Delete expired-but-unconsumed OAuth client states (internal implementation)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn reap_expired_oauth_client_states_impl(
        &self,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<(String, u64)>> {
        let rows = sqlx::query(
            r"
            DELETE FROM oauth_client_states
            WHERE expires_at < ?1
              AND used = 0
            RETURNING provider
            ",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to reap expired OAuth client states: {e}"))
        })?;

        let mut per_provider: BTreeMap<String, u64> = BTreeMap::new();
        for row in &rows {
            let provider: String = row
                .try_get("provider")
                .map_err(|e| AppError::database(format!("Failed to get provider: {e}")))?;
            *per_provider.entry(provider).or_default() += 1;
        }
        Ok(per_provider.into_iter().collect())
    }
}
