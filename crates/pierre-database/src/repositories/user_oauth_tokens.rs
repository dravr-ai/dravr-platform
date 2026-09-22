// ABOUTME: Statements, row decoders and the one OAuthTokenRepository body for per-user, per-tenant provider
// ABOUTME: OAuth tokens, the Strava shared-app pool and BYO OAuth apps, emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider OAuth tokens, written once.
//!
//! Every access and refresh token is encrypted at rest with AES-256-GCM
//! through [`HasEncryption`], bound by AAD to its `(tenant, user, provider)`
//! so a ciphertext copied to another row will not decrypt; the pool app
//! secrets are bound to their `client_id` the same way. The body reaches
//! the cipher only through the trait, never a backend's own helper, so both
//! backends encrypt identically and a token is never logged.
//!
//! The two backends differ in one respect: `user_oauth_tokens.user_id` and
//! `user_oauth_app_credentials.user_id` are `uuid` columns on Postgres and
//! `TEXT` on `SQLite`, so the shell hands the body its
//! [`uuid_columns`](super::uuid_columns) codec. `tenant_id` is text on both
//! and binds as such. Timestamps bind as [`DateTime<Utc>`] on both
//! (`TIMESTAMPTZ` on Postgres, RFC 3339 text on `SQLite`); the pool table
//! keeps epoch seconds in a `BIGINT`/`INTEGER` column and reads as `i64`,
//! its `seat_cap` is `INTEGER` on both and reads as `i32`, and `enabled` is
//! `BOOLEAN`/`INTEGER` and binds and reads as `bool`. `TRUE` is the boolean
//! literal both engines accept.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them. A placeholder reused in one statement (`$1` twice, `$5` twice)
//! binds once on both: sqlx maps `$NNN` to the same argument on `SQLite`.

use std::fmt::Display;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    ConnectionStatus, StravaPoolApp, StravaSeatHolder, UserOAuthApp, UserOAuthToken,
};
use uuid::Uuid;

use crate::backends::shared::encryption::{decrypt_oauth_token, HasEncryption};

/// Store or replace the one token a user holds per tenant and provider. The
/// caller's `created_at`/`updated_at` are what get stored; on conflict
/// `created_at` keeps the original row's value.
pub(crate) const UPSERT_TOKEN_SQL: &str = r"
            INSERT INTO user_oauth_tokens (
                id, user_id, tenant_id, provider, access_token, refresh_token,
                token_type, expires_at, scope, created_at, updated_at, provider_user_id,
                oauth_app_client_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (user_id, tenant_id, provider)
            DO UPDATE SET
                id = EXCLUDED.id,
                access_token = EXCLUDED.access_token,
                refresh_token = EXCLUDED.refresh_token,
                token_type = EXCLUDED.token_type,
                expires_at = EXCLUDED.expires_at,
                scope = EXCLUDED.scope,
                provider_user_id = EXCLUDED.provider_user_id,
                oauth_app_client_id = EXCLUDED.oauth_app_client_id,
                updated_at = EXCLUDED.updated_at
            ";

/// Store a token only while the user holds none for its tenant and provider:
/// [`UPSERT_TOKEN_SQL`]'s columns and binds, with a conflict left untouched.
pub(crate) const INSERT_TOKEN_IF_ABSENT_SQL: &str = r"
            INSERT INTO user_oauth_tokens (
                id, user_id, tenant_id, provider, access_token, refresh_token,
                token_type, expires_at, scope, created_at, updated_at, provider_user_id,
                oauth_app_client_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (user_id, tenant_id, provider) DO NOTHING
            ";

/// Replace a token only while the stored row is still the one whose `id` is
/// `$13`: every store writes a fresh `id` (a refresh keeps the row's), so a
/// row another writer stored since no longer matches and nothing changes.
pub(crate) const REPLACE_TOKEN_IF_CURRENT_SQL: &str = r"
            UPDATE user_oauth_tokens SET
                id = $1, access_token = $2, refresh_token = $3, token_type = $4,
                expires_at = $5, scope = $6, updated_at = $7, provider_user_id = $8,
                oauth_app_client_id = $9
            WHERE user_id = $10 AND tenant_id = $11 AND provider = $12 AND id = $13
            ";

/// One read of the token columns. `$filter` is the `WHERE` clause; `$order`
/// closes the statement.
macro_rules! token_select_sql {
    ($filter:literal, $order:literal) => {
        concat!(
            "SELECT id, user_id, tenant_id, provider, access_token, refresh_token,
                    token_type, expires_at, scope, provider_user_id, created_at, updated_at, oauth_app_client_id
             FROM user_oauth_tokens
             WHERE ",
            $filter,
            $order
        )
    };
}

/// The token for one user, tenant and provider.
pub(crate) const GET_TOKEN_SQL: &str =
    token_select_sql!("user_id = $1 AND tenant_id = $2 AND provider = $3", "");

/// A user's tokens within one tenant, newest first.
pub(crate) const GET_TOKENS_IN_TENANT_SQL: &str = token_select_sql!(
    "user_id = $1 AND tenant_id = $2",
    " ORDER BY created_at DESC"
);

/// A user's tokens across every tenant, newest first: the cross-tenant view
/// the OAuth status checks and admin dashboards take.
pub(crate) const GET_TOKENS_SQL: &str =
    token_select_sql!("user_id = $1", " ORDER BY created_at DESC");

/// Every token a tenant holds for one provider, newest first.
pub(crate) const GET_TENANT_PROVIDER_TOKENS_SQL: &str = token_select_sql!(
    "tenant_id = $1 AND provider = $2",
    " ORDER BY created_at DESC"
);

/// The owner of a provider-side account id (a webhook's `owner_id`).
pub(crate) const FIND_USER_BY_PROVIDER_USER_ID_SQL: &str = r"
            SELECT user_id, tenant_id
            FROM user_oauth_tokens
            WHERE provider = $1 AND provider_user_id = $2
            LIMIT 1
            ";

/// The conditions under which a token `t`'s grant is still authorized at the
/// provider, appended to a `WHERE`.
///
/// A grant is live until its connection (same user, tenant and provider) is
/// `revoked`, or is `needs_reauth` for any reason but our own client
/// credentials. Every writer of `needs_reauth` records why in `last_error`: a
/// refresh the provider refused as a dead grant (`invalid_grant`, which
/// Strava's `RefreshToken` rejection is classified as, or `invalid_request`
/// for a consumed, rotated refresh token), a bare HTTP 401/403
/// (`unauthorized`, `forbidden`), a session a background sync found expired
/// (`session_expired`). None of those leaves a grant we can use. A
/// `needs_reauth` over our own client credentials (`invalid_client`,
/// `unauthorized_client`) is the one exception: the athlete's grant stays
/// authorized at the provider, which still counts them; so does a token with
/// no connection row. `provider_connections.user_id` is TEXT on both backends
/// while the token's is `uuid` on Postgres, so the token side is cast down to
/// text.
macro_rules! live_grant_filter_sql {
    () => {
        "
              AND NOT EXISTS (
                  SELECT 1 FROM provider_connections c
                  WHERE c.user_id = CAST(t.user_id AS TEXT)
                    AND c.tenant_id = t.tenant_id
                    AND c.provider = t.provider
                    AND (c.status = 'revoked'
                         OR (c.status = 'needs_reauth'
                             AND (c.last_error IS NULL
                                  OR c.last_error NOT IN ('invalid_client', 'unauthorized_client'))))
              )"
    };
}

/// The conditions under which a token `t` holds a shared-app seat, appended to
/// the `WHERE` of every seat count so the counts cannot disagree on what a seat
/// is. A seat is the athlete's grant at the provider, so a token holds one
/// while that grant is live ([`live_grant_filter_sql`]); a dead one goes back
/// to the pool. A user with a BYO app for the provider runs on their own
/// quota and holds none.
macro_rules! seat_holder_filter_sql {
    () => {
        concat!(
            "
              AND NOT EXISTS (
                  SELECT 1 FROM user_oauth_app_credentials a
                  WHERE a.user_id = t.user_id AND a.provider = t.provider
              )",
            live_grant_filter_sql!()
        )
    };
}

/// Strava seat holders grouped by the app that issued the token; the NULL
/// group is the env-default app and every pre-pool token. A non-NULL `$1`
/// leaves that user out of every group.
pub(crate) const COUNT_STRAVA_SEAT_USAGE_BY_APP_SQL: &str = concat!(
    "
            SELECT t.oauth_app_client_id AS app, COUNT(DISTINCT t.user_id) AS n
            FROM user_oauth_tokens t
            WHERE t.provider = 'strava'
              AND ($1 IS NULL OR t.user_id <> $1)",
    seat_holder_filter_sql!(),
    "
            GROUP BY t.oauth_app_client_id
            "
);

/// Every Strava token with its holder and when they were last active, the
/// issuing app, the matching connection's state and whether it holds a seat.
/// `counts_as_seat` is the seat filter itself, evaluated per row, so this
/// listing cannot disagree with the counts above about any one athlete. The
/// users join is an outer one: the counts include a token whose account row is
/// gone (`SQLite` carries no foreign key from the token to its user), so the
/// listing names it too, with no email and no last activity. Ordered by email, an absent one first, then user and tenant, so
/// both engines return the same order (their NULL ordering differs, so no
/// nullable column leads the sort bare).
pub(crate) const LIST_STRAVA_SEAT_HOLDERS_SQL: &str = concat!(
    "
            SELECT t.user_id, u.email, u.last_active, t.tenant_id, t.oauth_app_client_id,
                   c.status AS connection_status,
                   c.connected_at AS connection_connected_at,
                   t.created_at AS token_created_at,
                   CAST(CASE WHEN 1 = 1",
    seat_holder_filter_sql!(),
    "
                        THEN 1 ELSE 0 END AS BIGINT) AS counts_as_seat
            FROM user_oauth_tokens t
            LEFT JOIN users u ON u.id = t.user_id
            LEFT JOIN provider_connections c
              ON c.user_id = CAST(t.user_id AS TEXT)
             AND c.tenant_id = t.tenant_id
             AND c.provider = t.provider
            WHERE t.provider = 'strava'
            ORDER BY COALESCE(u.email, ''), CAST(t.user_id AS TEXT), t.tenant_id
            "
);

/// The `(tenant, provider)` of every token a user holds, read without
/// touching the encrypted token columns — so an undecryptable token still
/// shows up for the disconnect that would clear it.
pub(crate) const LIST_TOKEN_PROVIDERS_SQL: &str = r"
            SELECT tenant_id, provider
            FROM user_oauth_tokens
            WHERE user_id = $1
            ORDER BY tenant_id, provider
            ";

/// The app attribution of each of a user's Strava tokens, the tenant it is
/// stored in, whether it holds a seat and whether its grant is live, read
/// without touching the encrypted token columns: tokens holding a seat first,
/// then the one stored in tenant `$2`, then the most recently written. Strava
/// counts the athlete per app whatever our tenant, so a token in another
/// tenant still names the app they hold a seat on.
pub(crate) const LIST_STRAVA_TOKEN_APPS_SQL: &str = concat!(
    "
            SELECT t.tenant_id, t.oauth_app_client_id AS app,
                   CAST(CASE WHEN 1 = 1",
    seat_holder_filter_sql!(),
    "
                        THEN 1 ELSE 0 END AS BIGINT) AS holds_seat,
                   CAST(CASE WHEN 1 = 1",
    live_grant_filter_sql!(),
    "
                        THEN 1 ELSE 0 END AS BIGINT) AS grant_live
            FROM user_oauth_tokens t
            WHERE t.user_id = $1 AND t.provider = 'strava'
            ORDER BY holds_seat DESC,
                     CASE WHEN t.tenant_id = $2 THEN 0 ELSE 1 END,
                     t.updated_at DESC
            "
);

/// Every pool app, in registration order.
pub(crate) const LIST_STRAVA_POOL_APPS_SQL: &str = r"
            SELECT client_id, seat_cap, enabled, label, created_at, updated_at
            FROM strava_oauth_app_pool
            ORDER BY created_at
            ";

/// The pool apps a new connect may pick from.
pub(crate) const LIST_ENABLED_STRAVA_POOL_APPS_SQL: &str = r"
            SELECT client_id, seat_cap, enabled, label, created_at, updated_at
            FROM strava_oauth_app_pool
            WHERE enabled = TRUE
            ORDER BY created_at
            ";

/// A pool app's encrypted secret.
pub(crate) const GET_STRAVA_POOL_APP_SECRET_SQL: &str =
    "SELECT client_secret_encrypted FROM strava_oauth_app_pool WHERE client_id = $1";

/// Register or update a pool app; a new one starts enabled.
pub(crate) const UPSERT_STRAVA_POOL_APP_SQL: &str = r"
            INSERT INTO strava_oauth_app_pool (client_id, client_secret_encrypted, seat_cap, enabled, label, created_at, updated_at)
            VALUES ($1, $2, $3, TRUE, $4, $5, $5)
            ON CONFLICT (client_id) DO UPDATE SET
                client_secret_encrypted = EXCLUDED.client_secret_encrypted,
                seat_cap = EXCLUDED.seat_cap,
                label = EXCLUDED.label,
                updated_at = EXCLUDED.updated_at
            ";

/// Enable or disable a pool app.
pub(crate) const SET_STRAVA_POOL_APP_ENABLED_SQL: &str =
    "UPDATE strava_oauth_app_pool SET enabled = $1, updated_at = $2 WHERE client_id = $3";

/// Remove a pool app.
pub(crate) const DELETE_STRAVA_POOL_APP_SQL: &str =
    "DELETE FROM strava_oauth_app_pool WHERE client_id = $1";

/// Drop the one token a user holds per tenant and provider.
pub(crate) const DELETE_TOKEN_SQL: &str = r"
            DELETE FROM user_oauth_tokens
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ";

/// Drop every token a user holds within a tenant.
pub(crate) const DELETE_TOKENS_SQL: &str = r"
            DELETE FROM user_oauth_tokens
            WHERE user_id = $1 AND tenant_id = $2
            ";

/// Replace the credentials after a refresh: the path every expired token
/// takes, with the new pair encrypted under the same AAD as the old. The row
/// keeps its `id`, and is written only while it is still the one the refresh
/// read (`$8`): a reconnect that stored a new token meanwhile wrote a fresh
/// `id`, and the refresh of the grant it replaced must not land over it.
pub(crate) const REFRESH_TOKEN_SQL: &str = r"
            UPDATE user_oauth_tokens
            SET access_token = $4,
                refresh_token = $5,
                expires_at = $6,
                updated_at = $7
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND id = $8
            ";

/// Register or replace a user's own OAuth app for a provider.
pub(crate) const STORE_USER_OAUTH_APP_SQL: &str = r"
            INSERT INTO user_oauth_app_credentials (id, user_id, provider, client_id, client_secret, redirect_uri, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
            ON CONFLICT (user_id, provider) DO UPDATE SET
                client_id = EXCLUDED.client_id,
                client_secret = EXCLUDED.client_secret,
                redirect_uri = EXCLUDED.redirect_uri,
                updated_at = EXCLUDED.updated_at
            ";

/// A user's own OAuth app for one provider.
pub(crate) const GET_USER_OAUTH_APP_SQL: &str = r"
            SELECT id, user_id, provider, client_id, client_secret, redirect_uri, created_at, updated_at
            FROM user_oauth_app_credentials
            WHERE user_id = $1 AND provider = $2
            ";

/// Every OAuth app a user registered, by provider.
pub(crate) const LIST_USER_OAUTH_APPS_SQL: &str = r"
            SELECT id, user_id, provider, client_id, client_secret, redirect_uri, created_at, updated_at
            FROM user_oauth_app_credentials
            WHERE user_id = $1
            ORDER BY provider
            ";

/// Remove a user's own OAuth app for a provider.
pub(crate) const REMOVE_USER_OAUTH_APP_SQL: &str = r"
            DELETE FROM user_oauth_app_credentials
            WHERE user_id = $1 AND provider = $2
            ";

/// The provider's last background sync for one user, tenant and provider.
pub(crate) const GET_PROVIDER_LAST_SYNC_SQL: &str =
    "SELECT last_sync FROM user_oauth_tokens WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";

/// Stamp the provider's last background sync.
pub(crate) const UPDATE_PROVIDER_LAST_SYNC_SQL: &str =
    "UPDATE user_oauth_tokens SET last_sync = $1 WHERE user_id = $2 AND tenant_id = $3 AND provider = $4";

fn column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to get {col}: {e}"))
}

/// Decode one `user_oauth_tokens` row, decrypting both tokens under the
/// row's own `(tenant, user, provider)` AAD. `user_id` is read by the caller
/// through its backend's codec; every other column decodes the same way on
/// both drivers. `try_get` throughout, never `Row::get`.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or the decryption error when a token does not decrypt under its AAD
/// (tampered data, or a ciphertext moved between rows).
pub(crate) fn user_oauth_token_from_row<D, R>(
    db: &D,
    row: &R,
    user_id: Uuid,
) -> AppResult<UserOAuthToken>
where
    D: HasEncryption,
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let tenant_id: String = row
        .try_get("tenant_id")
        .map_err(|e| column_error("tenant_id", e))?;
    let provider: String = row
        .try_get("provider")
        .map_err(|e| column_error("provider", e))?;

    let encrypted_access_token: String = row
        .try_get("access_token")
        .map_err(|e| column_error("access_token", e))?;
    let access_token =
        decrypt_oauth_token(db, &encrypted_access_token, &tenant_id, user_id, &provider)?;

    let encrypted_refresh_token: Option<String> = row
        .try_get("refresh_token")
        .map_err(|e| column_error("refresh_token", e))?;
    let refresh_token = encrypted_refresh_token
        .as_deref()
        .map(|encrypted| decrypt_oauth_token(db, encrypted, &tenant_id, user_id, &provider))
        .transpose()?;

    Ok(UserOAuthToken {
        id: row.try_get("id").map_err(|e| column_error("id", e))?,
        user_id,
        tenant_id,
        provider,
        access_token,
        refresh_token,
        token_type: row
            .try_get("token_type")
            .map_err(|e| column_error("token_type", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column_error("expires_at", e))?,
        scope: row.try_get("scope").map_err(|e| column_error("scope", e))?,
        provider_user_id: row
            .try_get("provider_user_id")
            .map_err(|e| column_error("provider_user_id", e))?,
        oauth_app_client_id: row
            .try_get("oauth_app_client_id")
            .map_err(|e| column_error("oauth_app_client_id", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column_error("created_at", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column_error("updated_at", e))?,
    })
}

/// Decode one `strava_oauth_app_pool` row; the secret is never part of it.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn strava_pool_app_from_row<R>(row: &R) -> AppResult<StravaPoolApp>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let seat_cap: i32 = row
        .try_get("seat_cap")
        .map_err(|e| column_error("seat_cap", e))?;
    Ok(StravaPoolApp {
        client_id: row
            .try_get("client_id")
            .map_err(|e| column_error("client_id", e))?,
        seat_cap: u32::try_from(seat_cap).unwrap_or(0),
        enabled: row
            .try_get("enabled")
            .map_err(|e| column_error("enabled", e))?,
        label: row.try_get("label").map_err(|e| column_error("label", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column_error("created_at", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column_error("updated_at", e))?,
    })
}

/// Decode one [`LIST_STRAVA_SEAT_HOLDERS_SQL`] row. `user_id` is read by the
/// caller through its backend's codec.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn strava_seat_holder_from_row<R>(row: &R, user_id: Uuid) -> AppResult<StravaSeatHolder>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let status: Option<String> = row
        .try_get("connection_status")
        .map_err(|e| column_error("connection_status", e))?;
    let connection_connected_at: Option<DateTime<Utc>> = row
        .try_get("connection_connected_at")
        .map_err(|e| column_error("connection_connected_at", e))?;
    let token_created_at: DateTime<Utc> = row
        .try_get("token_created_at")
        .map_err(|e| column_error("token_created_at", e))?;
    let counts: i64 = row
        .try_get("counts_as_seat")
        .map_err(|e| column_error("counts_as_seat", e))?;
    Ok(StravaSeatHolder {
        user_id,
        email: row.try_get("email").map_err(|e| column_error("email", e))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column_error("tenant_id", e))?,
        oauth_app_client_id: row
            .try_get("oauth_app_client_id")
            .map_err(|e| column_error("oauth_app_client_id", e))?,
        connection_status: status.as_deref().map(ConnectionStatus::from_str_value),
        connected_at: connection_connected_at.unwrap_or(token_created_at),
        last_active: row
            .try_get("last_active")
            .map_err(|e| column_error("last_active", e))?,
        counts_as_seat: counts != 0,
    })
}

/// Decode one `user_oauth_app_credentials` row. `user_id` is read by the
/// caller through its backend's codec.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn user_oauth_app_from_row<R>(row: &R, user_id: Uuid) -> AppResult<UserOAuthApp>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(UserOAuthApp {
        id: row.try_get("id").map_err(|e| column_error("id", e))?,
        user_id,
        provider: row
            .try_get("provider")
            .map_err(|e| column_error("provider", e))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column_error("client_id", e))?,
        client_secret: row
            .try_get("client_secret")
            .map_err(|e| column_error("client_secret", e))?,
        redirect_uri: row
            .try_get("redirect_uri")
            .map_err(|e| column_error("redirect_uri", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column_error("created_at", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column_error("updated_at", e))?,
    })
}

/// The AAD a pool app's secret is bound to.
pub(crate) fn strava_pool_app_aad(client_id: &str) -> String {
    format!("strava_oauth_app_pool|{client_id}")
}

/// Emit the whole [`OAuthTokenRepository`](super::OAuthTokenRepository)
/// implementation for one backend type.
///
/// `$row` is that backend's sqlx row type and `$ids` its
/// [`uuid_columns`](super::uuid_columns) codec, which together spell how the
/// two `user_id` columns bind and read. The body is written once here; each
/// backend's shell invokes it with its own types, and sqlx resolves the
/// driver from `self.pool()` per expansion. The body names its consts,
/// helpers and types unqualified, so the invoking shell must `use` every
/// one of them.
macro_rules! impl_oauth_token_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl OAuthTokenRepository for $ty {
            async fn upsert_token(&self, token: &UserOAuthToken) -> AppResult<()> {
                let (access, refresh) = self.encrypted_token_columns(token)?;
                sqlx::query(UPSERT_TOKEN_SQL)
                    .bind(&token.id)
                    .bind($ids::bind(token.user_id))
                    .bind(&token.tenant_id)
                    .bind(&token.provider)
                    .bind(&access)
                    .bind(refresh.as_deref())
                    .bind(&token.token_type)
                    .bind(token.expires_at)
                    .bind(token.scope.as_deref().unwrap_or(""))
                    .bind(token.created_at)
                    .bind(token.updated_at)
                    .bind(token.provider_user_id.as_deref())
                    .bind(token.oauth_app_client_id.as_deref())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert user OAuth token: {e}"))
                    })?;

                Ok(())
            }

            async fn replace_token_if_current(
                &self,
                token: &UserOAuthToken,
                expected_id: Option<&str>,
            ) -> AppResult<bool> {
                let (access, refresh) = self.encrypted_token_columns(token)?;
                let result = match expected_id {
                    None => {
                        sqlx::query(INSERT_TOKEN_IF_ABSENT_SQL)
                            .bind(&token.id)
                            .bind($ids::bind(token.user_id))
                            .bind(&token.tenant_id)
                            .bind(&token.provider)
                            .bind(&access)
                            .bind(refresh.as_deref())
                            .bind(&token.token_type)
                            .bind(token.expires_at)
                            .bind(token.scope.as_deref().unwrap_or(""))
                            .bind(token.created_at)
                            .bind(token.updated_at)
                            .bind(token.provider_user_id.as_deref())
                            .bind(token.oauth_app_client_id.as_deref())
                            .execute(self.pool())
                            .await
                    }
                    Some(expected) => {
                        sqlx::query(REPLACE_TOKEN_IF_CURRENT_SQL)
                            .bind(&token.id)
                            .bind(&access)
                            .bind(refresh.as_deref())
                            .bind(&token.token_type)
                            .bind(token.expires_at)
                            .bind(token.scope.as_deref().unwrap_or(""))
                            .bind(token.updated_at)
                            .bind(token.provider_user_id.as_deref())
                            .bind(token.oauth_app_client_id.as_deref())
                            .bind($ids::bind(token.user_id))
                            .bind(&token.tenant_id)
                            .bind(&token.provider)
                            .bind(expected)
                            .execute(self.pool())
                            .await
                    }
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to store user OAuth token: {e}"))
                })?;

                Ok(result.rows_affected() > 0)
            }

            async fn get_token(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Option<UserOAuthToken>> {
                let row = sqlx::query(GET_TOKEN_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query user OAuth token: {e}"))
                    })?;

                row.map(|row| self.token_from_row(&row)).transpose()
            }

            async fn get_tokens(
                &self,
                user_id: Uuid,
                tenant_id: Option<TenantId>,
            ) -> AppResult<Vec<UserOAuthToken>> {
                let rows = match tenant_id {
                    Some(tid) => {
                        sqlx::query(GET_TOKENS_IN_TENANT_SQL)
                            .bind($ids::bind(user_id))
                            .bind(tid.to_string())
                            .fetch_all(self.pool())
                            .await
                    }
                    None => {
                        sqlx::query(GET_TOKENS_SQL)
                            .bind($ids::bind(user_id))
                            .fetch_all(self.pool())
                            .await
                    }
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to query user OAuth tokens: {e}"))
                })?;

                rows.iter().map(|row| self.token_from_row(row)).collect()
            }

            async fn get_tenant_provider_tokens(
                &self,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Vec<UserOAuthToken>> {
                let rows = sqlx::query(GET_TENANT_PROVIDER_TOKENS_SQL)
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query tenant provider tokens: {e}"))
                    })?;

                rows.iter().map(|row| self.token_from_row(row)).collect()
            }

            async fn find_user_by_provider_user_id(
                &self,
                provider: &str,
                provider_user_id: &str,
            ) -> AppResult<Option<(Uuid, String)>> {
                let row = sqlx::query(FIND_USER_BY_PROVIDER_USER_ID_SQL)
                    .bind(provider)
                    .bind(provider_user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to look up user by provider_user_id: {e}"
                        ))
                    })?;

                row.map(|row| {
                    let user_id = $ids::read(&row, "user_id")?;
                    let tenant_id: String = row
                        .try_get("tenant_id")
                        .map_err(|e| AppError::database(format!("Failed to get tenant_id: {e}")))?;
                    Ok((user_id, tenant_id))
                })
                .transpose()
            }

            async fn list_strava_pool_apps(
                &self,
                only_enabled: bool,
            ) -> AppResult<Vec<StravaPoolApp>> {
                let sql = if only_enabled {
                    LIST_ENABLED_STRAVA_POOL_APPS_SQL
                } else {
                    LIST_STRAVA_POOL_APPS_SQL
                };
                let rows = sqlx::query(sql).fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to list Strava pool apps: {e}"))
                })?;

                rows.iter().map(strava_pool_app_from_row).collect()
            }

            async fn get_strava_pool_app_secret(
                &self,
                client_id: &str,
            ) -> AppResult<Option<String>> {
                let row = sqlx::query(GET_STRAVA_POOL_APP_SECRET_SQL)
                    .bind(client_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to load Strava pool app secret: {e}"))
                    })?;

                row.map(|row| {
                    let encrypted: String =
                        row.try_get("client_secret_encrypted").map_err(|e| {
                            AppError::database(format!(
                                "Failed to get client_secret_encrypted: {e}"
                            ))
                        })?;
                    HasEncryption::decrypt_data_with_aad(
                        self,
                        &encrypted,
                        &strava_pool_app_aad(client_id),
                    )
                })
                .transpose()
            }

            async fn list_strava_token_apps(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Vec<StravaTokenApp>> {
                let rows = sqlx::query(LIST_STRAVA_TOKEN_APPS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read Strava token attribution: {e}"))
                    })?;

                rows.iter()
                    .map(|row| {
                        let tenant_id: String = row.try_get("tenant_id").map_err(|e| {
                            AppError::database(format!("Failed to get tenant_id: {e}"))
                        })?;
                        let attribution: Option<String> = row
                            .try_get("app")
                            .map_err(|e| AppError::database(format!("Failed to get app: {e}")))?;
                        let holds_seat: i64 = row.try_get("holds_seat").map_err(|e| {
                            AppError::database(format!("Failed to get holds_seat: {e}"))
                        })?;
                        let grant_live: i64 = row.try_get("grant_live").map_err(|e| {
                            AppError::database(format!("Failed to get grant_live: {e}"))
                        })?;
                        Ok(StravaTokenApp {
                            tenant_id,
                            attribution,
                            holds_seat: holds_seat != 0,
                            grant_live: grant_live != 0,
                        })
                    })
                    .collect()
            }

            async fn count_strava_seat_usage_by_app(
                &self,
                excluded_user: Option<Uuid>,
            ) -> AppResult<Vec<(Option<String>, u32)>> {
                let rows = sqlx::query(COUNT_STRAVA_SEAT_USAGE_BY_APP_SQL)
                    .bind($ids::bind_opt(excluded_user))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count Strava seat usage by app: {e}"))
                    })?;

                rows.iter()
                    .map(|row| {
                        let app: Option<String> = row
                            .try_get("app")
                            .map_err(|e| AppError::database(format!("Failed to get app: {e}")))?;
                        let n: i64 = row
                            .try_get("n")
                            .map_err(|e| AppError::database(format!("Failed to get n: {e}")))?;
                        Ok((app, u32::try_from(n).unwrap_or(u32::MAX)))
                    })
                    .collect()
            }

            async fn list_strava_seat_holders(&self) -> AppResult<Vec<StravaSeatHolder>> {
                let rows = sqlx::query(LIST_STRAVA_SEAT_HOLDERS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list Strava seat holders: {e}"))
                    })?;

                rows.iter()
                    .map(|row| strava_seat_holder_from_row(row, $ids::read(row, "user_id")?))
                    .collect()
            }

            async fn list_token_providers(
                &self,
                user_id: Uuid,
            ) -> AppResult<Vec<(String, String)>> {
                let rows = sqlx::query(LIST_TOKEN_PROVIDERS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list user token providers: {e}"))
                    })?;

                rows.iter()
                    .map(|row| {
                        let tenant_id: String = row.try_get("tenant_id").map_err(|e| {
                            AppError::database(format!("Failed to get tenant_id: {e}"))
                        })?;
                        let provider: String = row.try_get("provider").map_err(|e| {
                            AppError::database(format!("Failed to get provider: {e}"))
                        })?;
                        Ok((tenant_id, provider))
                    })
                    .collect()
            }

            async fn upsert_strava_pool_app(
                &self,
                client_id: &str,
                client_secret: &str,
                seat_cap: u32,
                label: Option<&str>,
            ) -> AppResult<()> {
                let encrypted = HasEncryption::encrypt_data_with_aad(
                    self,
                    client_secret,
                    &strava_pool_app_aad(client_id),
                )?;
                sqlx::query(UPSERT_STRAVA_POOL_APP_SQL)
                    .bind(client_id)
                    .bind(&encrypted)
                    .bind(i32::try_from(seat_cap).unwrap_or(i32::MAX))
                    .bind(label)
                    .bind(Utc::now().timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert Strava pool app: {e}"))
                    })?;
                Ok(())
            }

            async fn set_strava_pool_app_enabled(
                &self,
                client_id: &str,
                enabled: bool,
            ) -> AppResult<()> {
                sqlx::query(SET_STRAVA_POOL_APP_ENABLED_SQL)
                    .bind(enabled)
                    .bind(Utc::now().timestamp())
                    .bind(client_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update Strava pool app: {e}"))
                    })?;
                Ok(())
            }

            async fn delete_strava_pool_app(&self, client_id: &str) -> AppResult<()> {
                sqlx::query(DELETE_STRAVA_POOL_APP_SQL)
                    .bind(client_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete Strava pool app: {e}"))
                    })?;
                Ok(())
            }

            async fn delete_token(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<()> {
                sqlx::query(DELETE_TOKEN_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete user OAuth token: {e}"))
                    })?;

                Ok(())
            }

            async fn delete_tokens(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()> {
                sqlx::query(DELETE_TOKENS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete user OAuth tokens: {e}"))
                    })?;

                Ok(())
            }

            async fn refresh_token(
                &self,
                stored: &UserOAuthToken,
                access_token: &str,
                refresh_token: Option<&str>,
                expires_at: Option<DateTime<Utc>>,
            ) -> AppResult<bool> {
                let refreshed = UserOAuthToken {
                    access_token: access_token.to_owned(),
                    refresh_token: refresh_token.map(str::to_owned),
                    ..stored.clone()
                };
                let (encrypted_access_token, encrypted_refresh_token) =
                    self.encrypted_token_columns(&refreshed)?;

                let result = sqlx::query(REFRESH_TOKEN_SQL)
                    .bind($ids::bind(stored.user_id))
                    .bind(&stored.tenant_id)
                    .bind(&stored.provider)
                    .bind(&encrypted_access_token)
                    .bind(encrypted_refresh_token.as_deref())
                    .bind(expires_at)
                    .bind(Utc::now())
                    .bind(&stored.id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to refresh user OAuth token: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn store_user_oauth_app(
                &self,
                user_id: Uuid,
                provider: &str,
                client_id: &str,
                client_secret: &str,
                redirect_uri: &str,
            ) -> AppResult<()> {
                sqlx::query(STORE_USER_OAUTH_APP_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind($ids::bind(user_id))
                    .bind(provider)
                    .bind(client_id)
                    .bind(client_secret)
                    .bind(redirect_uri)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store user OAuth app: {e}"))
                    })?;

                Ok(())
            }

            async fn get_user_oauth_app(
                &self,
                user_id: Uuid,
                provider: &str,
            ) -> AppResult<Option<UserOAuthApp>> {
                let row = sqlx::query(GET_USER_OAUTH_APP_SQL)
                    .bind($ids::bind(user_id))
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query user OAuth app: {e}"))
                    })?;

                row.map(|row| user_oauth_app_from_row(&row, $ids::read(&row, "user_id")?))
                    .transpose()
            }

            async fn list_user_oauth_apps(&self, user_id: Uuid) -> AppResult<Vec<UserOAuthApp>> {
                let rows = sqlx::query(LIST_USER_OAUTH_APPS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list user OAuth apps: {e}"))
                    })?;

                rows.iter()
                    .map(|row| user_oauth_app_from_row(row, $ids::read(row, "user_id")?))
                    .collect()
            }

            async fn remove_user_oauth_app(&self, user_id: Uuid, provider: &str) -> AppResult<()> {
                sqlx::query(REMOVE_USER_OAUTH_APP_SQL)
                    .bind($ids::bind(user_id))
                    .bind(provider)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to remove user OAuth app: {e}"))
                    })?;

                Ok(())
            }

            async fn get_provider_last_sync(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Option<DateTime<Utc>>> {
                // Outer Option: the row may not exist. Inner Option: last_sync is
                // NULL until the first successful sync.
                let last_sync: Option<Option<DateTime<Utc>>> =
                    sqlx::query_scalar(GET_PROVIDER_LAST_SYNC_SQL)
                        .bind($ids::bind(user_id))
                        .bind(tenant_id.to_string())
                        .bind(provider)
                        .fetch_optional(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to get provider last sync: {e}"))
                        })?;

                Ok(last_sync.flatten())
            }

            async fn update_provider_last_sync(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                sync_time: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(UPDATE_PROVIDER_LAST_SYNC_SQL)
                    .bind(sync_time)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update provider last sync: {e}"))
                    })?;

                Ok(())
            }
        }

        impl $ty {
            /// The access and refresh token of `token`, encrypted under its
            /// row's AAD, as the token columns store them.
            fn encrypted_token_columns(
                &self,
                token: &UserOAuthToken,
            ) -> AppResult<(String, Option<String>)> {
                let access = encrypt_oauth_token(
                    self,
                    &token.access_token,
                    &token.tenant_id,
                    token.user_id,
                    &token.provider,
                )?;
                let refresh = token
                    .refresh_token
                    .as_deref()
                    .map(|rt| {
                        encrypt_oauth_token(
                            self,
                            rt,
                            &token.tenant_id,
                            token.user_id,
                            &token.provider,
                        )
                    })
                    .transpose()?;
                Ok((access, refresh))
            }

            /// Decode a token row, reading `user_id` through this backend's codec
            /// and decrypting under the row's own AAD.
            fn token_from_row(&self, row: &$row) -> AppResult<UserOAuthToken> {
                user_oauth_token_from_row(self, row, $ids::read(row, "user_id")?)
            }
        }
    };
}
pub(crate) use impl_oauth_token_repository;
