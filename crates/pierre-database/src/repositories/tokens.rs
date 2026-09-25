// ABOUTME: Shared statements, row decoders and body for the OAuth 2.0 server — clients, codes, refresh tokens,
// ABOUTME: CSRF states, client grants and RFC 8628 device codes; one SQL text per operation, emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The OAuth 2.0 server's persistence, written once.
//!
//! Every table here declares its id columns as `TEXT` on both backends —
//! `oauth2_auth_codes.user_id`, `oauth2_refresh_tokens.user_id` and
//! `oauth2_states.user_id` included — so a uuid binds as its hyphenated text
//! and is parsed on the way back on both drivers, and no uuid codec is
//! needed. Timestamps bind as [`DateTime<Utc>`] on both (`TIMESTAMPTZ` on
//! Postgres, RFC 3339 text on `SQLite`, which orders correctly for the
//! `expires_at > now` filters because every value shares one offset and
//! width); `device_authorization` keeps epoch seconds in a `BIGINT`/`INTEGER`
//! column and reads as `i64` on both. Booleans are spelled `TRUE`/`FALSE`,
//! which both engines accept (`SQLite` stores them as 1/0).
//!
//! The refresh token value is never stored: `store_refresh_token` keeps its
//! HMAC-SHA256 under the blind-index key through
//! [`HasEncryption::hash_token_for_storage`], and every lookup hashes the
//! presented token the same way. The consume paths are single statements
//! (`UPDATE … WHERE … RETURNING`), so an authorization code is exchanged at
//! most once even under concurrent requests (RFC 6749 §4.1.2, §10.5), a
//! refresh token is rotated at most once, and a CSRF state is redeemed at
//! most once (RFC 6749 §10.12).
//!
//! Client registrations are reclaimed, not only gated by their expiry.
//! `store_refresh_token` stamps the client's `last_authorized_at` in the same
//! transaction as the token, which is what separates a client a user connected
//! from a registration nobody finished; `store_client_within_ceiling` holds the
//! unfinished ones to a ceiling, and `delete_stale_clients` deletes them once
//! abandoned, along with every registration past its expiry grace.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

use std::fmt::Display;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    DeviceAuthorization, OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State,
    OAuthClientGrant,
};
use uuid::Uuid;

/// Register an RFC 7591 client unless `$12` registrations are already
/// pending — carrying an expiry and never issued a refresh token. The count
/// and the insert are one statement, so no second round trip separates the
/// check from the write. The list columns hold JSON arrays as text.
pub(crate) const STORE_OAUTH2_CLIENT_WITHIN_CEILING_SQL: &str = r"
            INSERT INTO oauth2_clients (id, client_id, client_secret_hash, redirect_uris, grant_types, response_types, client_name, client_uri, scope, created_at, expires_at)
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
            WHERE (
                SELECT COUNT(*) FROM oauth2_clients
                WHERE expires_at IS NOT NULL AND last_authorized_at IS NULL
            ) < $12
            ";

/// Registrations whose expiry passed before `$1`, the grace cutoff. A row
/// without an expiry was not written by dynamic registration and never matches.
pub(crate) const DELETE_EXPIRED_OAUTH2_CLIENTS_SQL: &str = r"
            DELETE FROM oauth2_clients
            WHERE expires_at IS NOT NULL AND expires_at < $1
            ";

/// Pending registrations created before `$1`: registered, and no refresh
/// token ever issued through them, so no user finished authorizing them.
pub(crate) const DELETE_ABANDONED_OAUTH2_CLIENTS_SQL: &str = r"
            DELETE FROM oauth2_clients
            WHERE expires_at IS NOT NULL AND last_authorized_at IS NULL AND created_at < $1
            ";

/// Consent grants naming a client that no longer exists. `oauth_client_grants`
/// has no foreign key to `oauth2_clients`, so deleting a client cascades to its
/// codes, refresh tokens and states but not to these.
pub(crate) const DELETE_ORPHANED_CLIENT_GRANTS_SQL: &str = r"
            DELETE FROM oauth_client_grants
            WHERE NOT EXISTS (
                SELECT 1 FROM oauth2_clients c WHERE c.client_id = oauth_client_grants.client_id
            )
            ";

/// Record that a refresh token was issued through client `$1` at `$2`. Only a
/// user-bound grant issues one, so this is what marks a registration authorized.
pub(crate) const STAMP_OAUTH2_CLIENT_AUTHORIZED_SQL: &str = r"
            UPDATE oauth2_clients SET last_authorized_at = $2 WHERE client_id = $1
            ";

/// One client by its public `client_id`.
pub(crate) const GET_OAUTH2_CLIENT_SQL: &str = r"
            SELECT id, client_id, client_secret_hash, redirect_uris, grant_types, response_types, client_name, client_uri, scope, created_at, expires_at
            FROM oauth2_clients
            WHERE client_id = $1
            ";

/// Mint an authorization code with its PKCE challenge.
pub(crate) const STORE_OAUTH2_AUTH_CODE_SQL: &str = r"
            INSERT INTO oauth2_auth_codes (code, client_id, user_id, tenant_id, redirect_uri, scope, code_challenge, code_challenge_method, expires_at, used, state)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ";

/// Exchange the code exactly once: it must belong to the client, match the
/// redirect it was issued for, be unused and unexpired at `$4`. A second
/// exchange, even a concurrent one, matches zero rows.
pub(crate) const CONSUME_OAUTH2_AUTH_CODE_SQL: &str = r"
            UPDATE oauth2_auth_codes
            SET used = TRUE
            WHERE code = $1
              AND client_id = $2
              AND redirect_uri = $3
              AND used = FALSE
              AND expires_at > $4
            RETURNING code, client_id, user_id, tenant_id, redirect_uri, scope, expires_at, used, state, code_challenge, code_challenge_method
            ";

/// Store a refresh token under its HMAC.
pub(crate) const STORE_OAUTH2_REFRESH_TOKEN_SQL: &str = r"
            INSERT INTO oauth2_refresh_tokens (token, client_id, user_id, tenant_id, scope, created_at, expires_at, revoked)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ";

/// A refresh token by its HMAC, whatever its state.
pub(crate) const GET_OAUTH2_REFRESH_TOKEN_SQL: &str = r"
            SELECT token, client_id, user_id, tenant_id, scope, created_at, expires_at, revoked
            FROM oauth2_refresh_tokens
            WHERE token = $1
            ";

/// Rotate a refresh token exactly once: live, unexpired at `$3` and owned by
/// the client presenting it.
pub(crate) const CONSUME_OAUTH2_REFRESH_TOKEN_SQL: &str = r"
            UPDATE oauth2_refresh_tokens
            SET revoked = TRUE
            WHERE token = $1
              AND client_id = $2
              AND revoked = FALSE
              AND expires_at > $3
            RETURNING token, client_id, user_id, tenant_id, scope, created_at, expires_at, revoked
            ";

/// Mint a CSRF state for one authorization round trip.
pub(crate) const STORE_OAUTH2_STATE_SQL: &str = r"
            INSERT INTO oauth2_states (state, client_id, user_id, tenant_id, redirect_uri, scope, code_challenge, code_challenge_method, created_at, expires_at, used)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ";

/// Redeem a state exactly once, for the client it was minted for.
pub(crate) const CONSUME_OAUTH2_STATE_SQL: &str = r"
            UPDATE oauth2_states
            SET used = TRUE
            WHERE state = $1
              AND client_id = $2
              AND used = FALSE
              AND expires_at > $3
            RETURNING state, client_id, user_id, tenant_id, redirect_uri, scope, code_challenge, code_challenge_method, created_at, expires_at, used
            ";

/// Record a consent. The active partial-unique index makes re-consent a
/// no-op: the conflict target names that index, which both engines resolve.
pub(crate) const STORE_CLIENT_GRANT_SQL: &str = r"
            INSERT INTO oauth_client_grants
                (id, user_id, tenant_id, client_id, scope, granted_at, revoked_at)
            VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP, NULL)
            ON CONFLICT (user_id, tenant_id, client_id, scope) WHERE revoked_at IS NULL DO NOTHING
            ";

/// The live grant for one `(user, tenant, client, scope)` tuple.
pub(crate) const FIND_ACTIVE_CLIENT_GRANT_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND scope = $4
              AND revoked_at IS NULL
            LIMIT 1
            ";

/// A user's live grants within a tenant, newest first.
pub(crate) const LIST_CLIENT_GRANTS_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND revoked_at IS NULL
            ORDER BY granted_at DESC
            ";

/// Soft-delete a grant; `user_id` + `tenant_id` in the filter is the
/// ownership check.
pub(crate) const REVOKE_CLIENT_GRANT_SQL: &str = r"
            UPDATE oauth_client_grants
            SET revoked_at = CURRENT_TIMESTAMP
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3 AND revoked_at IS NULL
            ";

/// Open an RFC 8628 device authorization, pending until an operator acts.
pub(crate) const CREATE_DEVICE_AUTHORIZATION_SQL: &str = r"
            INSERT INTO device_authorization
                (device_code_hash, user_code, status, approved_by, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ";

/// A device authorization by the hash of its device code (the polling side).
pub(crate) const GET_DEVICE_AUTHORIZATION_BY_CODE_HASH_SQL: &str = r"
            SELECT device_code_hash, user_code, status, approved_by, created_at, expires_at
            FROM device_authorization
            WHERE device_code_hash = $1
            ";

/// A device authorization by its user code (the operator side).
pub(crate) const GET_DEVICE_AUTHORIZATION_BY_USER_CODE_SQL: &str = r"
            SELECT device_code_hash, user_code, status, approved_by, created_at, expires_at
            FROM device_authorization
            WHERE user_code = $1
            ";

/// Approve a still-pending authorization; an already-decided one matches nothing.
pub(crate) const APPROVE_DEVICE_AUTHORIZATION_SQL: &str = r"
            UPDATE device_authorization
            SET status = 'approved', approved_by = $2
            WHERE user_code = $1 AND status = 'pending'
            ";

/// Deny a still-pending authorization.
pub(crate) const DENY_DEVICE_AUTHORIZATION_SQL: &str = r"
            UPDATE device_authorization
            SET status = 'denied'
            WHERE user_code = $1 AND status = 'pending'
            ";

/// Consume a device authorization; the token endpoint mints only when a row
/// went, so a duplicate poll can never mint twice.
pub(crate) const DELETE_DEVICE_AUTHORIZATION_SQL: &str =
    "DELETE FROM device_authorization WHERE device_code_hash = $1";

fn column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to get {col}: {e}"))
}

/// Read a uuid the table stores as hyphenated text.
fn text_uuid<R>(row: &R, col: &str) -> AppResult<Uuid>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let raw: String = row.try_get(col).map_err(|e| column_error(col, e))?;
    Uuid::parse_str(&raw).map_err(|e| AppError::database(format!("Failed to parse {col}: {e}")))
}

/// Read a JSON-array-of-strings column.
fn json_list<R>(row: &R, col: &str) -> AppResult<Vec<String>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let raw: String = row.try_get(col).map_err(|e| column_error(col, e))?;
    serde_json::from_str(&raw)
        .map_err(|e| AppError::database(format!("Failed to parse {col}: {e}")))
}

/// Decode one `oauth2_clients` row. `try_get` throughout, never `Row::get`,
/// so a corrupt row surfaces as a recoverable error rather than a panic.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn oauth2_client_from_row<R>(row: &R) -> AppResult<OAuth2Client>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(OAuth2Client {
        id: row.try_get("id").map_err(|e| column_error("id", e))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column_error("client_id", e))?,
        client_secret_hash: row
            .try_get("client_secret_hash")
            .map_err(|e| column_error("client_secret_hash", e))?,
        redirect_uris: json_list(row, "redirect_uris")?,
        grant_types: json_list(row, "grant_types")?,
        response_types: json_list(row, "response_types")?,
        client_name: row
            .try_get("client_name")
            .map_err(|e| column_error("client_name", e))?,
        client_uri: row
            .try_get("client_uri")
            .map_err(|e| column_error("client_uri", e))?,
        scope: row.try_get("scope").map_err(|e| column_error("scope", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column_error("created_at", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column_error("expires_at", e))?,
    })
}

/// Decode one `oauth2_auth_codes` row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn oauth2_auth_code_from_row<R>(row: &R) -> AppResult<OAuth2AuthCode>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(OAuth2AuthCode {
        code: row.try_get("code").map_err(|e| column_error("code", e))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column_error("client_id", e))?,
        user_id: text_uuid(row, "user_id")?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column_error("tenant_id", e))?,
        redirect_uri: row
            .try_get("redirect_uri")
            .map_err(|e| column_error("redirect_uri", e))?,
        scope: row.try_get("scope").map_err(|e| column_error("scope", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column_error("expires_at", e))?,
        used: row.try_get("used").map_err(|e| column_error("used", e))?,
        state: row.try_get("state").map_err(|e| column_error("state", e))?,
        code_challenge: row
            .try_get("code_challenge")
            .map_err(|e| column_error("code_challenge", e))?,
        code_challenge_method: row
            .try_get("code_challenge_method")
            .map_err(|e| column_error("code_challenge_method", e))?,
    })
}

/// Decode one `oauth2_refresh_tokens` row. `token` is the stored HMAC, not
/// the credential.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn oauth2_refresh_token_from_row<R>(row: &R) -> AppResult<OAuth2RefreshToken>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(OAuth2RefreshToken {
        token: row.try_get("token").map_err(|e| column_error("token", e))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column_error("client_id", e))?,
        user_id: text_uuid(row, "user_id")?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column_error("tenant_id", e))?,
        scope: row.try_get("scope").map_err(|e| column_error("scope", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column_error("created_at", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column_error("expires_at", e))?,
        revoked: row
            .try_get("revoked")
            .map_err(|e| column_error("revoked", e))?,
    })
}

/// Decode one `oauth2_states` row. `user_id` is nullable: a state minted
/// before the user is known carries none.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn oauth2_state_from_row<R>(row: &R) -> AppResult<OAuth2State>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let user_id: Option<String> = row
        .try_get("user_id")
        .map_err(|e| column_error("user_id", e))?;
    let user_id = user_id
        .map(|raw| Uuid::parse_str(&raw))
        .transpose()
        .map_err(|e| AppError::database(format!("Failed to parse user_id: {e}")))?;
    Ok(OAuth2State {
        state: row.try_get("state").map_err(|e| column_error("state", e))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column_error("client_id", e))?,
        user_id,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column_error("tenant_id", e))?,
        redirect_uri: row
            .try_get("redirect_uri")
            .map_err(|e| column_error("redirect_uri", e))?,
        scope: row.try_get("scope").map_err(|e| column_error("scope", e))?,
        code_challenge: row
            .try_get("code_challenge")
            .map_err(|e| column_error("code_challenge", e))?,
        code_challenge_method: row
            .try_get("code_challenge_method")
            .map_err(|e| column_error("code_challenge_method", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column_error("created_at", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column_error("expires_at", e))?,
        used: row.try_get("used").map_err(|e| column_error("used", e))?,
    })
}

/// Decode one `oauth_client_grants` row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn client_grant_from_row<R>(row: &R) -> AppResult<OAuthClientGrant>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(OAuthClientGrant {
        id: row.try_get("id").map_err(|e| column_error("id", e))?,
        user_id: row
            .try_get("user_id")
            .map_err(|e| column_error("user_id", e))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column_error("tenant_id", e))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column_error("client_id", e))?,
        scope: row.try_get("scope").map_err(|e| column_error("scope", e))?,
        granted_at: row
            .try_get("granted_at")
            .map_err(|e| column_error("granted_at", e))?,
        revoked_at: row
            .try_get("revoked_at")
            .map_err(|e| column_error("revoked_at", e))?,
    })
}

/// Decode one `device_authorization` row; the timestamps are epoch seconds.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn device_authorization_from_row<R>(row: &R) -> AppResult<DeviceAuthorization>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(DeviceAuthorization {
        device_code_hash: row
            .try_get("device_code_hash")
            .map_err(|e| column_error("device_code_hash", e))?,
        user_code: row
            .try_get("user_code")
            .map_err(|e| column_error("user_code", e))?,
        status: row
            .try_get("status")
            .map_err(|e| column_error("status", e))?,
        approved_by: row
            .try_get("approved_by")
            .map_err(|e| column_error("approved_by", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column_error("created_at", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column_error("expires_at", e))?,
    })
}

/// Emit the whole [`OAuth2ServerRepository`](super::OAuth2ServerRepository)
/// implementation for one backend type.
///
/// No per-engine argument: every column involved has the same shape on both
/// backends (see the module doc). The body is written once here; each
/// backend's shell invokes it with its own type, and sqlx resolves the driver
/// from `self.pool()` per expansion. The body names its consts, helpers and
/// types unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_oauth2_server_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl OAuth2ServerRepository for $ty {
            async fn store_client_within_ceiling(
                &self,
                client: &OAuth2Client,
                ceiling: u64,
            ) -> AppResult<bool> {
                // COUNT(*) is a signed 64-bit value on both engines; a ceiling
                // past its range is no ceiling at all.
                let ceiling = i64::try_from(ceiling).unwrap_or(i64::MAX);
                let result = sqlx::query(STORE_OAUTH2_CLIENT_WITHIN_CEILING_SQL)
                    .bind(&client.id)
                    .bind(&client.client_id)
                    .bind(&client.client_secret_hash)
                    .bind(serde_json::to_string(&client.redirect_uris)?)
                    .bind(serde_json::to_string(&client.grant_types)?)
                    .bind(serde_json::to_string(&client.response_types)?)
                    .bind(&client.client_name)
                    .bind(&client.client_uri)
                    .bind(&client.scope)
                    .bind(client.created_at)
                    .bind(client.expires_at)
                    .bind(ceiling)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth2 client: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn delete_stale_clients(
                &self,
                expired_before: DateTime<Utc>,
                unauthorized_before: DateTime<Utc>,
            ) -> AppResult<OAuth2ClientSweep> {
                let sweep_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to sweep OAuth2 clients: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(sweep_error)?;

                let expired = sqlx::query(DELETE_EXPIRED_OAUTH2_CLIENTS_SQL)
                    .bind(expired_before)
                    .execute(&mut *tx)
                    .await
                    .map_err(sweep_error)?
                    .rows_affected();
                let abandoned = sqlx::query(DELETE_ABANDONED_OAUTH2_CLIENTS_SQL)
                    .bind(unauthorized_before)
                    .execute(&mut *tx)
                    .await
                    .map_err(sweep_error)?
                    .rows_affected();
                let orphaned_grants = sqlx::query(DELETE_ORPHANED_CLIENT_GRANTS_SQL)
                    .execute(&mut *tx)
                    .await
                    .map_err(sweep_error)?
                    .rows_affected();

                tx.commit().await.map_err(sweep_error)?;

                Ok(OAuth2ClientSweep {
                    expired,
                    abandoned,
                    orphaned_grants,
                })
            }

            async fn get_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>> {
                let row = sqlx::query(GET_OAUTH2_CLIENT_SQL)
                    .bind(client_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query OAuth2 client: {e}"))
                    })?;

                row.map(|row| oauth2_client_from_row(&row)).transpose()
            }

            async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
                sqlx::query(STORE_OAUTH2_AUTH_CODE_SQL)
                    .bind(&auth_code.code)
                    .bind(&auth_code.client_id)
                    .bind(auth_code.user_id.to_string())
                    .bind(&auth_code.tenant_id)
                    .bind(&auth_code.redirect_uri)
                    .bind(&auth_code.scope)
                    .bind(&auth_code.code_challenge)
                    .bind(&auth_code.code_challenge_method)
                    .bind(auth_code.expires_at)
                    .bind(auth_code.used)
                    .bind(&auth_code.state)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth2 auth code: {e}"))
                    })?;

                Ok(())
            }

            async fn store_refresh_token(
                &self,
                refresh_token: &OAuth2RefreshToken,
            ) -> AppResult<()> {
                let token_hash = HasEncryption::hash_token_for_storage(self, &refresh_token.token)?;
                let store_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to store OAuth2 refresh token: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(store_error)?;

                sqlx::query(STORE_OAUTH2_REFRESH_TOKEN_SQL)
                    .bind(&token_hash)
                    .bind(&refresh_token.client_id)
                    .bind(refresh_token.user_id.to_string())
                    .bind(&refresh_token.tenant_id)
                    .bind(&refresh_token.scope)
                    .bind(refresh_token.created_at)
                    .bind(refresh_token.expires_at)
                    .bind(refresh_token.revoked)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
                sqlx::query(STAMP_OAUTH2_CLIENT_AUTHORIZED_SQL)
                    .bind(&refresh_token.client_id)
                    .bind(refresh_token.created_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;

                tx.commit().await.map_err(store_error)
            }

            async fn consume_auth_code(
                &self,
                code: &str,
                client_id: &str,
                redirect_uri: &str,
                now: DateTime<Utc>,
            ) -> AppResult<Option<OAuth2AuthCode>> {
                let row = sqlx::query(CONSUME_OAUTH2_AUTH_CODE_SQL)
                    .bind(code)
                    .bind(client_id)
                    .bind(redirect_uri)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume OAuth2 auth code: {e}"))
                    })?;

                row.map(|row| oauth2_auth_code_from_row(&row)).transpose()
            }

            async fn consume_refresh_token(
                &self,
                token: &str,
                client_id: &str,
                now: DateTime<Utc>,
            ) -> AppResult<Option<OAuth2RefreshToken>> {
                let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

                let row = sqlx::query(CONSUME_OAUTH2_REFRESH_TOKEN_SQL)
                    .bind(&token_hash)
                    .bind(client_id)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume OAuth2 refresh token: {e}"))
                    })?;

                row.map(|row| oauth2_refresh_token_from_row(&row))
                    .transpose()
            }

            async fn get_refresh_token_by_value(
                &self,
                token: &str,
            ) -> AppResult<Option<OAuth2RefreshToken>> {
                let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

                let row = sqlx::query(GET_OAUTH2_REFRESH_TOKEN_SQL)
                    .bind(&token_hash)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query OAuth2 refresh token: {e}"))
                    })?;

                row.map(|row| oauth2_refresh_token_from_row(&row))
                    .transpose()
            }

            async fn store_state(&self, state: &OAuth2State) -> AppResult<()> {
                sqlx::query(STORE_OAUTH2_STATE_SQL)
                    .bind(&state.state)
                    .bind(&state.client_id)
                    .bind(state.user_id.map(|id| id.to_string()))
                    .bind(&state.tenant_id)
                    .bind(&state.redirect_uri)
                    .bind(&state.scope)
                    .bind(&state.code_challenge)
                    .bind(&state.code_challenge_method)
                    .bind(state.created_at)
                    .bind(state.expires_at)
                    .bind(state.used)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth2 state: {e}"))
                    })?;

                Ok(())
            }

            async fn consume_state(
                &self,
                state_value: &str,
                client_id: &str,
                now: DateTime<Utc>,
            ) -> AppResult<Option<OAuth2State>> {
                let row = sqlx::query(CONSUME_OAUTH2_STATE_SQL)
                    .bind(state_value)
                    .bind(client_id)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume OAuth2 state: {e}"))
                    })?;

                row.map(|row| oauth2_state_from_row(&row)).transpose()
            }

            async fn store_client_grant(&self, grant: &OAuthClientGrant) -> AppResult<()> {
                sqlx::query(STORE_CLIENT_GRANT_SQL)
                    .bind(&grant.id)
                    .bind(&grant.user_id)
                    .bind(&grant.tenant_id)
                    .bind(&grant.client_id)
                    .bind(&grant.scope)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth client grant: {e}"))
                    })?;

                Ok(())
            }

            async fn find_active_client_grant(
                &self,
                user_id: &str,
                tenant_id: &str,
                client_id: &str,
                scope: &str,
            ) -> AppResult<Option<OAuthClientGrant>> {
                let row = sqlx::query(FIND_ACTIVE_CLIENT_GRANT_SQL)
                    .bind(user_id)
                    .bind(tenant_id)
                    .bind(client_id)
                    .bind(scope)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query OAuth client grant: {e}"))
                    })?;

                row.map(|row| client_grant_from_row(&row)).transpose()
            }

            async fn list_client_grants(
                &self,
                user_id: &str,
                tenant_id: &str,
            ) -> AppResult<Vec<OAuthClientGrant>> {
                let rows = sqlx::query(LIST_CLIENT_GRANTS_SQL)
                    .bind(user_id)
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list OAuth client grants: {e}"))
                    })?;

                rows.iter().map(client_grant_from_row).collect()
            }

            async fn revoke_client_grant(
                &self,
                id: &str,
                user_id: &str,
                tenant_id: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(REVOKE_CLIENT_GRANT_SQL)
                    .bind(id)
                    .bind(user_id)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to revoke OAuth client grant: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn create_device_authorization(&self, da: &DeviceAuthorization) -> AppResult<()> {
                sqlx::query(CREATE_DEVICE_AUTHORIZATION_SQL)
                    .bind(&da.device_code_hash)
                    .bind(&da.user_code)
                    .bind(&da.status)
                    .bind(&da.approved_by)
                    .bind(da.created_at)
                    .bind(da.expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store device authorization: {e}"))
                    })?;
                Ok(())
            }

            async fn get_device_authorization_by_code_hash(
                &self,
                device_code_hash: &str,
            ) -> AppResult<Option<DeviceAuthorization>> {
                let row = sqlx::query(GET_DEVICE_AUTHORIZATION_BY_CODE_HASH_SQL)
                    .bind(device_code_hash)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to load device authorization: {e}"))
                    })?;
                row.map(|row| device_authorization_from_row(&row))
                    .transpose()
            }

            async fn get_device_authorization_by_user_code(
                &self,
                user_code: &str,
            ) -> AppResult<Option<DeviceAuthorization>> {
                let row = sqlx::query(GET_DEVICE_AUTHORIZATION_BY_USER_CODE_SQL)
                    .bind(user_code)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to load device authorization by user_code: {e}"
                        ))
                    })?;
                row.map(|row| device_authorization_from_row(&row))
                    .transpose()
            }

            async fn approve_device_authorization(
                &self,
                user_code: &str,
                approved_by: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(APPROVE_DEVICE_AUTHORIZATION_SQL)
                    .bind(user_code)
                    .bind(approved_by)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to approve device authorization: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn deny_device_authorization(&self, user_code: &str) -> AppResult<bool> {
                let result = sqlx::query(DENY_DEVICE_AUTHORIZATION_SQL)
                    .bind(user_code)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to deny device authorization: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn delete_device_authorization(&self, device_code_hash: &str) -> AppResult<bool> {
                let result = sqlx::query(DELETE_DEVICE_AUTHORIZATION_SQL)
                    .bind(device_code_hash)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete device authorization: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_oauth2_server_repository;
