// ABOUTME: Decodes one row of each OAuth 2.0 server table, the same way on both backends
// ABOUTME: Ids stored as hyphenated text become uuids, and list columns stored as JSON text become lists
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    DeviceAuthorization, OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State,
    OAuthClientGrant,
};
use secrecy::SecretString;
use uuid::Uuid;

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
        resource: row
            .try_get("resource")
            .map_err(|e| column_error("resource", e))?,
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
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(OAuth2RefreshToken {
        token: SecretString::from(
            row.try_get::<String, _>("token")
                .map_err(|e| column_error("token", e))?,
        ),
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
        family_id: row
            .try_get("family_id")
            .map_err(|e| column_error("family_id", e))?,
        resource: row
            .try_get("resource")
            .map_err(|e| column_error("resource", e))?,
        family_created_at: row
            .try_get("family_created_at")
            .map_err(|e| column_error("family_created_at", e))?,
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
