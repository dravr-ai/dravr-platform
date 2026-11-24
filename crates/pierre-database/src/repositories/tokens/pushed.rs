// ABOUTME: The RFC 9126 pushed authorization request store, one SQL text per operation for both backends
// ABOUTME: Rows are keyed by the request_uri's SHA-256 and the client that pushed them; a request is spent once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pushed authorization requests (RFC 9126).
//!
//! `oauth2_pushed_requests` is an authorization server protocol table written
//! before any user is known, like `oauth2_clients`: it has no tenant or user
//! column, and every statement names both `request_uri_hash` and `client_id`,
//! so one client can never read, extend or spend another's request.

use std::fmt::Display;

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuth2PushedRequest;

/// Store one pushed request.
pub(crate) const STORE_PUSHED_REQUEST_SQL: &str = r"
            INSERT INTO oauth2_pushed_requests
                (id, request_uri_hash, client_id, parameters, expires_at, used_at, created_at)
            VALUES ($1, $2, $3, $4, $5, NULL, $6)
            ";

/// The unused, unexpired request `$1` names, when client `$2` pushed it.
pub(crate) const FIND_PUSHED_REQUEST_SQL: &str = r"
            SELECT id, request_uri_hash, client_id, parameters, expires_at, used_at, created_at
            FROM oauth2_pushed_requests
            WHERE request_uri_hash = $1 AND client_id = $2
              AND used_at IS NULL AND expires_at > $3
            ";

/// Move an unused request's expiry forward to `$3`, never back.
pub(crate) const EXTEND_PUSHED_REQUEST_SQL: &str = r"
            UPDATE oauth2_pushed_requests SET expires_at = $3
            WHERE request_uri_hash = $1 AND client_id = $2
              AND used_at IS NULL AND expires_at < $3
            ";

/// Spend the request: it mints one code, once (RFC 9126 §4).
pub(crate) const CONSUME_PUSHED_REQUEST_SQL: &str = r"
            UPDATE oauth2_pushed_requests SET used_at = $3
            WHERE request_uri_hash = $1 AND client_id = $2
              AND used_at IS NULL AND expires_at > $3
            ";

/// Every pushed request past its expiry at `$1`, used or not: nothing reads
/// one again.
pub(crate) const DELETE_EXPIRED_PUSHED_REQUESTS_SQL: &str =
    "DELETE FROM oauth2_pushed_requests WHERE expires_at <= $1";

fn column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to get {col}: {e}"))
}

/// Decode one `oauth2_pushed_requests` row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn pushed_request_from_row<R>(row: &R) -> AppResult<OAuth2PushedRequest>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Timestamp: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<Timestamp>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(OAuth2PushedRequest {
        id: row.try_get("id").map_err(|e| column_error("id", e))?,
        request_uri_hash: row
            .try_get("request_uri_hash")
            .map_err(|e| column_error("request_uri_hash", e))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column_error("client_id", e))?,
        parameters: row
            .try_get("parameters")
            .map_err(|e| column_error("parameters", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column_error("expires_at", e))?,
        used_at: row
            .try_get("used_at")
            .map_err(|e| column_error("used_at", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column_error("created_at", e))?,
    })
}

/// Emit the pushed request store for one backend type, which the
/// `OAuth2ServerRepository` implementation delegates to.
macro_rules! impl_pushed_request_store {
    ($ty:ty) => {
        impl $ty {
            /// `store_pushed_request`.
            async fn store_pushed_row(&self, request: &OAuth2PushedRequest) -> AppResult<()> {
                sqlx::query($crate::repositories::tokens::pushed::STORE_PUSHED_REQUEST_SQL)
                    .bind(&request.id)
                    .bind(&request.request_uri_hash)
                    .bind(&request.client_id)
                    .bind(&request.parameters)
                    .bind(request.expires_at)
                    .bind(request.created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store pushed request: {e}"))
                    })?;
                Ok(())
            }

            /// `find_pushed_request`.
            async fn find_pushed_row(
                &self,
                request_uri_hash: &str,
                client_id: &str,
                now: Timestamp,
            ) -> AppResult<Option<OAuth2PushedRequest>> {
                let row =
                    sqlx::query($crate::repositories::tokens::pushed::FIND_PUSHED_REQUEST_SQL)
                        .bind(request_uri_hash)
                        .bind(client_id)
                        .bind(now)
                        .fetch_optional(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to read pushed request: {e}"))
                        })?;
                row.map(|row| $crate::repositories::tokens::pushed::pushed_request_from_row(&row))
                    .transpose()
            }

            /// `extend_pushed_request`.
            async fn extend_pushed_row(
                &self,
                request_uri_hash: &str,
                client_id: &str,
                expires_at: Timestamp,
            ) -> AppResult<()> {
                sqlx::query($crate::repositories::tokens::pushed::EXTEND_PUSHED_REQUEST_SQL)
                    .bind(request_uri_hash)
                    .bind(client_id)
                    .bind(expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to extend pushed request: {e}"))
                    })?;
                Ok(())
            }

            /// `consume_pushed_request`.
            async fn consume_pushed_row(
                &self,
                request_uri_hash: &str,
                client_id: &str,
                now: Timestamp,
            ) -> AppResult<bool> {
                let spent =
                    sqlx::query($crate::repositories::tokens::pushed::CONSUME_PUSHED_REQUEST_SQL)
                        .bind(request_uri_hash)
                        .bind(client_id)
                        .bind(now)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to spend pushed request: {e}"))
                        })?
                        .rows_affected();
                Ok(spent > 0)
            }
        }
    };
}

pub(crate) use impl_pushed_request_store;
