// ABOUTME: Shared statements and body for OAuth client states — the CSRF `state` and PKCE verifier
// ABOUTME: minted at authorization start, consumed once at callback, and reaped once expired

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! OAuth client states, written once.
//!
//! Three operations form a closed lifecycle over `oauth_client_states` —
//! store, consume-exactly-once, reap-when-expired — and share no row mapper
//! or query with the OAuth token, OAuth2 server and provider connection
//! repositories they used to sit beside.
//!
//! Every column of this table is the same type on both backends but two:
//! `used` is `INTEGER` on `SQLite` and `BOOLEAN` on Postgres, and the two
//! timestamps are RFC 3339 text on `SQLite` and `TIMESTAMPTZ` on Postgres.
//! Neither needs a per-backend spelling: `TRUE`/`FALSE` are literals both
//! engines accept and a `bool` binds and decodes as 0/1 on `SQLite`, and a
//! `DateTime<Utc>` binds and decodes as RFC 3339 text there. `user_id` is
//! `TEXT` on both and holds the hyphenated uuid, so the id is stringified
//! on the way in and parsed on the way out. One body therefore serves both
//! drivers with no macro argument beyond the backend type.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthClientState;
use uuid::Uuid;

/// The columns every row carries, in the order [`client_state_from_row`]
/// reads them — one list for the insert and the consume's `RETURNING`.
macro_rules! client_state_columns {
    () => {
        "state, provider, user_id, tenant_id, redirect_uri, scope, pkce_code_verifier, \
         created_at, expires_at, used, oauth_app_client_id"
    };
}

/// Mint a state row.
pub(crate) const STORE_CLIENT_STATE_SQL: &str = concat!(
    "
            INSERT INTO oauth_client_states (",
    client_state_columns!(),
    ")
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "
);

/// Verify and mark used in one statement, so two concurrent callbacks
/// cannot both redeem the same state; the row comes back for the token
/// exchange that follows.
pub(crate) const CONSUME_CLIENT_STATE_SQL: &str = concat!(
    "
            UPDATE oauth_client_states
            SET used = TRUE
            WHERE state = $1
              AND provider = $2
              AND used = FALSE
              AND expires_at > $3
            RETURNING ",
    client_state_columns!(),
    "
            "
);

/// Delete the states that expired without ever being consumed, naming each
/// one's provider so the sweep can count per provider.
pub(crate) const REAP_EXPIRED_CLIENT_STATES_SQL: &str = r"
            DELETE FROM oauth_client_states
            WHERE expires_at < $1
              AND used = FALSE
            RETURNING provider
            ";

/// Decode one client-state row. `try_get` throughout, never `Row::get`, so
/// a corrupt row surfaces as a recoverable error rather than a panic, and
/// every nullable column decodes as an `Option` so a NULL reads back as
/// `None` on both backends.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded
/// or when `user_id` is present but not a uuid.
pub(crate) fn client_state_from_row<R>(row: &R) -> AppResult<OAuthClientState>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let text = |col: &str| -> AppResult<String> {
        row.try_get(col)
            .map_err(|e| AppError::database(format!("oauth_client_states {col}: {e}")))
    };
    let optional = |col: &str| -> AppResult<Option<String>> {
        row.try_get(col)
            .map_err(|e| AppError::database(format!("oauth_client_states {col}: {e}")))
    };
    let user_id = optional("user_id")?
        .map(|s| {
            Uuid::parse_str(&s)
                .map_err(|e| AppError::database(format!("Invalid user_id uuid '{s}': {e}")))
        })
        .transpose()?;
    Ok(OAuthClientState {
        state: text("state")?,
        provider: text("provider")?,
        user_id,
        tenant_id: optional("tenant_id")?,
        redirect_uri: text("redirect_uri")?,
        scope: optional("scope")?,
        pkce_code_verifier: optional("pkce_code_verifier")?,
        oauth_app_client_id: optional("oauth_app_client_id")?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::database(format!("oauth_client_states created_at: {e}")))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| AppError::database(format!("oauth_client_states expires_at: {e}")))?,
        used: row
            .try_get("used")
            .map_err(|e| AppError::database(format!("oauth_client_states used: {e}")))?,
    })
}

/// Emit the whole [`OAuthClientStateRepository`] implementation for one
/// backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_oauth_client_state_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl OAuthClientStateRepository for $ty {
            async fn store_oauth_client_state(&self, state: &OAuthClientState) -> AppResult<()> {
                sqlx::query(STORE_CLIENT_STATE_SQL)
                    .bind(&state.state)
                    .bind(&state.provider)
                    .bind(state.user_id.map(|u| u.to_string()))
                    .bind(&state.tenant_id)
                    .bind(&state.redirect_uri)
                    .bind(&state.scope)
                    .bind(&state.pkce_code_verifier)
                    .bind(state.created_at)
                    .bind(state.expires_at)
                    .bind(state.used)
                    .bind(&state.oauth_app_client_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth client state: {e}"))
                    })?;

                Ok(())
            }

            async fn consume_oauth_client_state(
                &self,
                state_value: &str,
                provider: &str,
                now: DateTime<Utc>,
            ) -> AppResult<Option<OAuthClientState>> {
                let row = sqlx::query(CONSUME_CLIENT_STATE_SQL)
                    .bind(state_value)
                    .bind(provider)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume OAuth client state: {e}"))
                    })?;

                row.map(|row| client_state_from_row(&row)).transpose()
            }

            async fn reap_expired_oauth_client_states(
                &self,
                now: DateTime<Utc>,
            ) -> AppResult<Vec<(String, u64)>> {
                let rows = sqlx::query(REAP_EXPIRED_CLIENT_STATES_SQL)
                    .bind(now)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to reap expired OAuth client states: {e}"
                        ))
                    })?;

                let mut per_provider: BTreeMap<String, u64> = BTreeMap::new();
                for row in &rows {
                    let provider: String = row.try_get("provider").map_err(|e| {
                        AppError::database(format!("oauth_client_states provider: {e}"))
                    })?;
                    *per_provider.entry(provider).or_default() += 1;
                }
                Ok(per_provider.into_iter().collect())
            }
        }
    };
}
pub(crate) use impl_oauth_client_state_repository;
