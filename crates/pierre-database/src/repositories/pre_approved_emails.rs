// ABOUTME: Shared statements and body for the pre-approved email allow-list — allow, remove, lookup, list
// ABOUTME: One SQL text per operation; each backend shell supplies how it binds a uuid and reads one back as text

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pre-approved emails, written once.
//!
//! An operator "allow" recorded before the person has an account. Emails
//! are stored and compared lowercase, so every lookup is case-insensitive;
//! `allow` is idempotent because the primary key turns a repeat into zero
//! rows affected.
//!
//! The two backends differ in one respect only: `allowed_by` is a `uuid`
//! column on Postgres and `TEXT` on `SQLite`. The id is therefore bound
//! through the macro's `$bind_id` function (native on Postgres, hyphenated
//! text on `SQLite`) and read back through `$text`, the cast that turns the
//! column into text on Postgres and is empty on `SQLite`, so one row parser
//! decodes both. `created_at` binds and decodes as `DateTime<Utc>` on both:
//! RFC 3339 text on `SQLite`, byte-identical to the `to_rfc3339()` the rows
//! were written with, and `TIMESTAMPTZ` on Postgres.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::PreApprovedEmail;
use uuid::Uuid;

/// The columns every read decodes, with `$text` appended to `allowed_by` so
/// the operator's uuid arrives as text on both backends.
macro_rules! pre_approved_email_columns {
    ($text:literal) => {
        concat!(
            "email, allowed_by",
            $text,
            " AS allowed_by, note, created_at"
        )
    };
}
pub(crate) use pre_approved_email_columns;

/// Record an allow; a repeat for the same address affects zero rows.
pub(crate) const ALLOW_EMAIL_SQL: &str = r"
            INSERT INTO pre_approved_emails (email, allowed_by, note, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (email) DO NOTHING
            ";

/// Remove the allow for an address.
pub(crate) const REMOVE_EMAIL_SQL: &str = "DELETE FROM pre_approved_emails WHERE email = $1";

/// The allow for one address.
macro_rules! get_email_sql {
    ($text:literal) => {
        concat!(
            "
            SELECT ",
            pre_approved_email_columns!($text),
            "
            FROM pre_approved_emails
            WHERE email = $1
            "
        )
    };
}
pub(crate) use get_email_sql;

/// Every allow, oldest first, ties broken by address.
macro_rules! list_emails_sql {
    ($text:literal) => {
        concat!(
            "
            SELECT ",
            pre_approved_email_columns!($text),
            "
            FROM pre_approved_emails
            ORDER BY created_at ASC, email ASC
            "
        )
    };
}
pub(crate) use list_emails_sql;

/// Decode one allow row. `allowed_by` arrives as text on both backends (see
/// [`pre_approved_email_columns`]) and is parsed here; `try_get` throughout,
/// never `Row::get`, so a corrupt row surfaces as a recoverable error rather
/// than a panic.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded
/// or when `allowed_by` is present but not a uuid.
pub(crate) fn entry_from_row<R>(row: &R) -> AppResult<PreApprovedEmail>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let allowed_by: Option<String> = row
        .try_get("allowed_by")
        .map_err(|e| AppError::database(format!("pre_approved_emails allowed_by: {e}")))?;
    let allowed_by = allowed_by
        .map(|s| {
            Uuid::parse_str(&s)
                .map_err(|e| AppError::database(format!("Invalid allowed_by uuid '{s}': {e}")))
        })
        .transpose()?;
    Ok(PreApprovedEmail {
        email: row
            .try_get("email")
            .map_err(|e| AppError::database(format!("pre_approved_emails email: {e}")))?,
        allowed_by,
        note: row
            .try_get("note")
            .map_err(|e| AppError::database(format!("pre_approved_emails note: {e}")))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::database(format!("pre_approved_emails created_at: {e}")))?,
    })
}

/// Emit the whole [`PreApprovedEmailRepository`] implementation for one
/// backend type.
///
/// `$bind_id` is the function turning a `Uuid` into whatever that backend's
/// `allowed_by` column accepts (the `bind` of its codec in
/// [`super::uuid_columns`]); `$text` is the cast that reads it back as text (`"::text"`
/// on Postgres, `""` on `SQLite`).
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_pre_approved_email_repository {
    ($ty:ty, $bind_id:path, $text:literal) => {
        #[async_trait::async_trait]
        impl PreApprovedEmailRepository for $ty {
            async fn allow(
                &self,
                email: &str,
                allowed_by: Option<Uuid>,
                note: Option<&str>,
            ) -> AppResult<bool> {
                let result = sqlx::query(ALLOW_EMAIL_SQL)
                    .bind(email.to_lowercase())
                    .bind(allowed_by.map($bind_id))
                    .bind(note)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record pre-approved email: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn remove(&self, email: &str) -> AppResult<bool> {
                let result = sqlx::query(REMOVE_EMAIL_SQL)
                    .bind(email.to_lowercase())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to remove pre-approved email: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn get(&self, email: &str) -> AppResult<Option<PreApprovedEmail>> {
                let row = sqlx::query(get_email_sql!($text))
                    .bind(email.to_lowercase())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch pre-approved email: {e}"))
                    })?;

                row.as_ref().map(entry_from_row).transpose()
            }

            async fn list(&self) -> AppResult<Vec<PreApprovedEmail>> {
                let rows = sqlx::query(list_emails_sql!($text))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list pre-approved emails: {e}"))
                    })?;

                rows.iter().map(entry_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_pre_approved_email_repository;
