// ABOUTME: Shared statements, row decode and body for the link-state lifecycle both backends serve
// ABOUTME: One copy of each operation; each backend shell supplies only its uuid bind cast and its text read cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Link states, written once.
//!
//! A link state is the one-time code that turns "someone messaged us from
//! this Telegram/Slack id" into "this id belongs to that Dravr account". It
//! is minted with a TTL, consumed exactly once, or completed by a webhook
//! that learned the user id after the fact, and those four operations are
//! what this module holds. The `MessagingRepository` impl of each backend
//! delegates its four link-state methods here.
//!
//! The two backends differ in one respect only: `tenant_id` and `user_id`
//! are `uuid` columns on Postgres and `TEXT` on `SQLite`. `tenant_id` binds
//! as a [`TenantId`], whose own sqlx encoding already follows that split; a
//! `user_id` arrives as `&str` and is bound through `$uuid`, the cast that
//! turns the text into a uuid on Postgres and is empty on `SQLite`; both are
//! read back through `$text`, the cast that turns the column into text on
//! Postgres and is empty on `SQLite`, so one row decode serves both.
//! `used` decodes as `bool` on both (`BOOLEAN` on Postgres, `INTEGER` 0/1 on
//! `SQLite`, where the `TRUE`/`FALSE` literals are 1 and 0). Timestamps bind
//! and decode as `DateTime<Utc>` on both: RFC 3339 text on `SQLite`,
//! byte-identical to the `to_rfc3339()` the rows were written with, and
//! `TIMESTAMPTZ` on Postgres.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

/// The eleven columns every read of `messaging_link_states` returns, in the
/// order [`link_state_from_row`] reads them, with `$text` appended to each
/// uuid column so it arrives as text on both backends.
macro_rules! link_state_columns {
    ($text:literal) => {
        concat!(
            "id, tenant_id",
            $text,
            " AS tenant_id, user_id",
            $text,
            " AS user_id, channel_type, code, method, used, \
             channel_user_id, sender_name, expires_at, created_at"
        )
    };
}
pub(crate) use link_state_columns;

/// Mint a pending link state. `$uuid` is the cast on the `user_id` bind.
macro_rules! create_link_state_sql {
    ($uuid:literal) => {
        concat!(
            "INSERT INTO messaging_link_states \
                 (id, tenant_id, user_id, channel_type, code, method, used, \
                  channel_user_id, sender_name, expires_at, created_at) \
             VALUES ($1, $2, $3",
            $uuid,
            ", $4, $5, $6, FALSE, $7, $8, $9, $10)"
        )
    };
}
pub(crate) use create_link_state_sql;

/// The row for a code within one tenant, whatever its state; the consume
/// path reads it first to tell an expired code from a used one.
macro_rules! link_state_for_tenant_sql {
    ($text:literal) => {
        concat!(
            "SELECT ",
            link_state_columns!($text),
            " FROM messaging_link_states WHERE code = $1 AND tenant_id = $2"
        )
    };
}
pub(crate) use link_state_for_tenant_sql;

/// The row for a code that is still live: unused and not yet expired.
macro_rules! live_link_state_sql {
    ($text:literal) => {
        concat!(
            "SELECT ",
            link_state_columns!($text),
            " FROM messaging_link_states WHERE code = $1 AND used = FALSE AND expires_at > $2"
        )
    };
}
pub(crate) use live_link_state_sql;

/// The row for a code across tenants, whatever its state; the webhook
/// completion path has no tenant yet, only the code the athlete typed.
macro_rules! link_state_by_code_sql {
    ($text:literal) => {
        concat!(
            "SELECT ",
            link_state_columns!($text),
            " FROM messaging_link_states WHERE code = $1"
        )
    };
}
pub(crate) use link_state_by_code_sql;

/// Consume a code exactly once: the guards on `used` and `expires_at` make
/// a second consumer, or a late one, update zero rows.
pub(crate) const CONSUME_LINK_STATE_SQL: &str = "UPDATE messaging_link_states SET used = TRUE \
     WHERE code = $1 AND tenant_id = $2 AND used = FALSE AND expires_at > $3";

/// Complete a webhook-initiated code: bind the user and consume it in one
/// statement, only while the code has no user yet. `$uuid` is the cast on
/// the `user_id` bind.
macro_rules! complete_link_state_sql {
    ($uuid:literal) => {
        concat!(
            "UPDATE messaging_link_states SET user_id = $1",
            $uuid,
            ", used = TRUE \
             WHERE code = $2 AND used = FALSE AND user_id IS NULL AND expires_at > $3"
        )
    };
}
pub(crate) use complete_link_state_sql;

/// One `messaging_link_states` row, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkStateRow {
    /// Row id.
    pub id: String,
    /// Owning tenant, as text.
    pub tenant_id: String,
    /// The bound user, absent until a webhook-initiated code is completed.
    pub user_id: Option<String>,
    /// Channel slug (`"telegram"`).
    pub channel_type: String,
    /// The one-time verification code.
    pub code: String,
    /// Linking method (`deep_link` or `oauth`).
    pub method: String,
    /// Whether the code has been consumed.
    pub used: bool,
    /// Sender's platform id, set on channel-initiated codes.
    pub channel_user_id: Option<String>,
    /// Sender's display name, set on channel-initiated codes.
    pub sender_name: Option<String>,
    /// When the code stops being consumable.
    pub expires_at: DateTime<Utc>,
    /// When the code was minted.
    pub created_at: DateTime<Utc>,
}

impl LinkStateRow {
    /// The shape the `MessagingRepository` link-state methods hand back:
    /// every column but `used`, timestamps as RFC 3339 text.
    pub(crate) fn into_json(self) -> Value {
        serde_json::json!({
            "id": self.id,
            "tenant_id": self.tenant_id,
            "user_id": self.user_id,
            "channel_type": self.channel_type,
            "code": self.code,
            "method": self.method,
            "channel_user_id": self.channel_user_id,
            "sender_name": self.sender_name,
            "expires_at": self.expires_at.to_rfc3339(),
            "created_at": self.created_at.to_rfc3339(),
        })
    }
}

/// The instant a caller's `expires_at` names. The wire shape is RFC 3339
/// text; anything else is refused before it reaches a column, because on
/// `SQLite` the column is text and is compared as text, where a value that
/// is not a timestamp would sort after every real instant and never expire.
///
/// # Errors
/// Returns an invalid-input error when the text is not RFC 3339.
pub(crate) fn parse_expires_at(text: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::invalid_input(format!("Invalid expires_at timestamp: {e}")))
}

/// Decode one link-state row from either backend via `try_get` only —
/// `Row::get` is `try_get().unwrap()` and would panic the read path on a
/// width or NULL surprise, so a corrupt row surfaces as a recoverable error
/// the caller can act on, never as a crash.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn link_state_from_row<R>(row: &R) -> AppResult<LinkStateRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str| -> AppResult<String> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("messaging_link_states {name}: {e}")))
    };
    let opt = |name: &str| -> AppResult<Option<String>> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("messaging_link_states {name}: {e}")))
    };
    let at = |name: &str| -> AppResult<DateTime<Utc>> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("messaging_link_states {name}: {e}")))
    };
    Ok(LinkStateRow {
        id: col("id")?,
        tenant_id: col("tenant_id")?,
        user_id: opt("user_id")?,
        channel_type: col("channel_type")?,
        code: col("code")?,
        method: col("method")?,
        used: row
            .try_get("used")
            .map_err(|e| AppError::database(format!("messaging_link_states used: {e}")))?,
        channel_user_id: opt("channel_user_id")?,
        sender_name: opt("sender_name")?,
        expires_at: at("expires_at")?,
        created_at: at("created_at")?,
    })
}

/// Emit the four link-state operations for one backend, as free functions
/// over that backend's pool — the shape a `MessagingRepository` impl can
/// delegate to from another module, since a trait impl cannot be split
/// across files.
///
/// `$db` is the sqlx database type (`Sqlite` or `Postgres`); `$uuid` is the
/// cast on a `user_id` bind (`"::uuid"` on Postgres, `""` on `SQLite`);
/// `$text` is the cast that reads a uuid column back as text (`"::text"` on
/// Postgres, `""` on `SQLite`).
///
/// The body is written once here; each backend's shell invokes it with its
/// own arguments, and sqlx resolves the driver from the pool type per
/// expansion. The body names its consts, helpers and types unqualified, so
/// the invoking shell must `use` every one of them.
macro_rules! impl_link_state_functions {
    ($db:ty, $uuid:literal, $text:literal) => {
        /// Mint a pending link state with a verification code.
        ///
        /// # Errors
        /// Returns an invalid-input error when `expires_at` is not RFC 3339,
        /// or a database error when the insert fails.
        pub(crate) async fn create_link_state(
            pool: &Pool<$db>,
            params: &CreateLinkStateParams<'_>,
        ) -> AppResult<()> {
            let expires_at = parse_expires_at(params.expires_at)?;
            sqlx::query(create_link_state_sql!($uuid))
                .bind(params.id)
                .bind(params.tenant_id)
                .bind(params.user_id)
                .bind(params.channel_type)
                .bind(params.code)
                .bind(params.method)
                .bind(params.channel_user_id)
                .bind(params.sender_name)
                .bind(expires_at)
                .bind(Utc::now())
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to create link state: {e}")))?;
            Ok(())
        }

        /// Consume a link state by verification code, exactly once.
        ///
        /// Reads the row first so an expired code and a used one are told
        /// apart, then updates under `used = FALSE` and expiry guards and
        /// checks `rows_affected`, so two consumers racing on one code see
        /// one success and one `LinkCodeAlreadyUsed`.
        ///
        /// # Errors
        /// Returns `MessagingError::LinkCodeExpired` when the code has expired
        /// or does not exist for this tenant, `MessagingError::LinkCodeAlreadyUsed`
        /// when it was already consumed, or a database error.
        pub(crate) async fn consume_link_state(
            pool: &Pool<$db>,
            code: &str,
            tenant_id: TenantId,
        ) -> AppResult<Value> {
            let now = Utc::now();
            let existing = sqlx::query(link_state_for_tenant_sql!($text))
                .bind(code)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;
            let Some(row) = existing else {
                return Err(MessagingError::LinkCodeExpired.into());
            };
            let state = link_state_from_row(&row)?;
            if state.used {
                return Err(MessagingError::LinkCodeAlreadyUsed.into());
            }
            if state.expires_at < now {
                return Err(MessagingError::LinkCodeExpired.into());
            }

            let result = sqlx::query(CONSUME_LINK_STATE_SQL)
                .bind(code)
                .bind(tenant_id)
                .bind(now)
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to consume link state: {e}")))?;
            if result.rows_affected() == 0 {
                // Another consumer took the code between the read and the update.
                return Err(MessagingError::LinkCodeAlreadyUsed.into());
            }
            Ok(state.into_json())
        }

        /// Read-only lookup of a live link state by code, for rendering the
        /// login page. Returns `None` when the code does not exist, has
        /// expired or has been used; never consumes it.
        ///
        /// # Errors
        /// Returns a database error when the query or the decode fails.
        pub(crate) async fn get_link_state(
            pool: &Pool<$db>,
            code: &str,
        ) -> AppResult<Option<Value>> {
            let row = sqlx::query(live_link_state_sql!($text))
                .bind(code)
                .bind(Utc::now())
                .fetch_optional(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;
            row.map(|r| link_state_from_row(&r).map(LinkStateRow::into_json))
                .transpose()
        }

        /// Complete a webhook-initiated link state by binding its `user_id`,
        /// which also consumes it. Only a code that exists, is unexpired,
        /// unused and has no user yet completes.
        ///
        /// # Errors
        /// Returns `MessagingError::LinkCodeExpired` when the code has expired
        /// or does not exist, `MessagingError::LinkCodeAlreadyUsed` when it was
        /// already consumed, `MessagingError::LinkCodeNotCompletable` when it
        /// already has a user, or a database error.
        pub(crate) async fn complete_link_state(
            pool: &Pool<$db>,
            code: &str,
            user_id: &str,
        ) -> AppResult<Value> {
            let now = Utc::now();
            let existing = sqlx::query(link_state_by_code_sql!($text))
                .bind(code)
                .fetch_optional(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to look up link state: {e}")))?;
            let Some(row) = existing else {
                return Err(MessagingError::LinkCodeExpired.into());
            };
            let mut state = link_state_from_row(&row)?;
            if state.used {
                return Err(MessagingError::LinkCodeAlreadyUsed.into());
            }
            if state.expires_at < now {
                return Err(MessagingError::LinkCodeExpired.into());
            }
            if state.user_id.is_some() {
                return Err(MessagingError::LinkCodeNotCompletable {
                    code: code.to_owned(),
                    reason: "Link code already has a user_id set".to_owned(),
                }
                .into());
            }

            let result = sqlx::query(complete_link_state_sql!($uuid))
                .bind(user_id)
                .bind(code)
                .bind(now)
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to complete link state: {e}")))?;
            if result.rows_affected() == 0 {
                return Err(MessagingError::LinkCodeAlreadyUsed.into());
            }
            state.user_id = Some(user_id.to_owned());
            Ok(state.into_json())
        }
    };
}
pub(crate) use impl_link_state_functions;
