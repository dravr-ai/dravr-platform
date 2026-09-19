// ABOUTME: SQLite link-state lifecycle, emitted from the shared body in repositories/messaging_link_states.rs
// ABOUTME: Every id column is TEXT here, so the shared body expands with no uuid cast and no text cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Link states for the `SQLite` backend.
//!
//! The statements, the guards and the row decode live in
//! [`crate::repositories::messaging_link_states`]; this file expands them
//! over the `SQLite` pool. The `MessagingRepository` impl in `messaging.rs`
//! reaches them through the inherent `*_impl` methods below, which carry no
//! statement of their own.

use chrono::Utc;
use pierre_core::errors::messaging::MessagingError;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use sqlx::{Pool, Sqlite};

use super::Database;
use crate::repositories::messaging_link_states::{
    complete_link_state_sql, create_link_state_sql, impl_link_state_functions,
    link_state_by_code_sql, link_state_columns, link_state_for_tenant_sql, link_state_from_row,
    live_link_state_sql, parse_expires_at, LinkStateRow, CONSUME_LINK_STATE_SQL,
};
use crate::repositories::CreateLinkStateParams;

impl_link_state_functions!(Sqlite, "", "");

impl Database {
    /// Create a pending link state with a verification code.
    ///
    /// # Errors
    /// Returns an invalid-input error when `expires_at` is not RFC 3339, or a
    /// database error when the insert fails.
    pub async fn create_link_state_impl(
        &self,
        params: &CreateLinkStateParams<'_>,
    ) -> AppResult<()> {
        create_link_state(&self.pool, params).await
    }

    /// Atomically consume a link state by verification code.
    ///
    /// # Errors
    /// Returns `MessagingError::LinkCodeExpired` when the code has expired or
    /// does not exist, or `MessagingError::LinkCodeAlreadyUsed` when it was
    /// already consumed.
    pub async fn consume_link_state_impl(
        &self,
        code: &str,
        tenant_id: TenantId,
    ) -> AppResult<Value> {
        consume_link_state(&self.pool, code, tenant_id).await
    }

    /// Read-only lookup of a live link state by code; never consumes it.
    ///
    /// # Errors
    /// Returns a database error when the query fails.
    pub async fn get_link_state_impl(&self, code: &str) -> AppResult<Option<Value>> {
        get_link_state(&self.pool, code).await
    }

    /// Atomically complete a webhook-initiated link state by setting its
    /// `user_id`, which also consumes it.
    ///
    /// # Errors
    /// Returns `MessagingError::LinkCodeExpired` when the code has expired or
    /// does not exist, `MessagingError::LinkCodeAlreadyUsed` when it was
    /// already consumed, or `MessagingError::LinkCodeNotCompletable` when it
    /// already has a `user_id`.
    pub async fn complete_link_state_impl(&self, code: &str, user_id: &str) -> AppResult<Value> {
        complete_link_state(&self.pool, code, user_id).await
    }
}
