// ABOUTME: Repository trait, statements and shared body for import tombstones: a deleted account's identities, by hash
// ABOUTME: Written in the account deletion's transaction; read by webhooks, provider connects and sign-up admission

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Import tombstones.
//!
//! When an account is deleted, its provider data goes, but the provider keeps
//! sending: a push event names the athlete's provider-side id, a reconnect
//! hands the same provider account back, and a sign-up with the same email
//! used to land Active through a standing pre-approval. The account deletion
//! writes a tombstone for every provider-side id its tokens held and one for
//! its email, in its own transaction, and each of those doors refuses the
//! identity while a live tombstone names it, until it is explicitly cleared.
//!
//! No identity is stored in clear. [`provider_identity_hash`] and
//! [`email_identity_hash`] are SHA-256 digests: a lookup recomputes the digest
//! of the identity it holds and asks for a live tombstone with it. A
//! provider-side id is often a small integer, so its digest is a pseudonym
//! that an exhaustive search over ids can reverse, not an anonymisation.
//!
//! ISOLATION EXEMPTION: the table carries neither `tenant_id` nor `user_id`.
//! An account deletion removes the account from every tenant, and a provider
//! account or an email belongs to a person, not to one tenant, so a tombstone
//! is global by design. Every read is keyed by an identity digest the caller
//! computed from an identity it already holds, or by a tombstone id the
//! refusal handed to the person it refused; the clearing surfaces check that
//! the caller holds the identity before they clear.

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::normalize_email;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// How a deletion receipt and a tombstone name an account: the SHA-256 of its
/// hyphenated lowercase id.
#[must_use]
pub fn subject_hash(user_id: Uuid) -> String {
    sha256_hex(user_id.to_string().as_bytes())
}

/// The digest a provider-side account id is tombstoned and looked up under:
/// the SHA-256 of the id with surrounding whitespace removed.
#[must_use]
pub fn provider_identity_hash(provider_user_id: &str) -> String {
    sha256_hex(provider_user_id.trim().as_bytes())
}

/// The digest an email is tombstoned and looked up under: the SHA-256 of the
/// normalized address ([`normalize_email`]: trimmed, lowercase).
#[must_use]
pub fn email_identity_hash(email: &str) -> String {
    sha256_hex(normalize_email(email).as_bytes())
}

/// How a tombstone's `claimed_by` and `cleared_by` name an athlete's account:
/// `athlete:` and the account's [`subject_hash`], never its id.
#[must_use]
pub fn athlete_marker(user_id: Uuid) -> String {
    format!("athlete:{}", subject_hash(user_id))
}

/// What a tombstone names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TombstoneKind {
    /// A provider-side account id (a Strava athlete id, an intervals.icu
    /// athlete id), with the provider it belongs to.
    ProviderAccount,
    /// The deleted account's email address.
    Email,
}

impl TombstoneKind {
    /// The `kind` column value. A persisted value: rows already hold it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderAccount => "provider_account",
            Self::Email => "email",
        }
    }

    /// The kind a stored `kind` value names.
    ///
    /// # Errors
    /// Returns a database error for a value no kind writes.
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "provider_account" => Ok(Self::ProviderAccount),
            "email" => Ok(Self::Email),
            other => Err(AppError::database(format!(
                "import_tombstones.kind holds an unknown value '{other}'"
            ))),
        }
    }
}

/// One tombstone, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportTombstone {
    /// Row id, which a refusal names so the person refused can clear it.
    pub id: String,
    /// The deleted account's [`subject_hash`].
    pub subject_hash: String,
    /// What the tombstone names.
    pub kind: TombstoneKind,
    /// The provider, for a [`TombstoneKind::ProviderAccount`].
    pub provider: Option<String>,
    /// The identity's digest.
    pub identity_hash: String,
    /// When the account deletion wrote it.
    pub created_at: DateTime<Utc>,
    /// When it was cleared; `None` while it is live.
    pub cleared_at: Option<DateTime<Utc>>,
    /// Who cleared it: an [`athlete_marker`] or `operator:` and the
    /// operator's token id.
    pub cleared_by: Option<String>,
    /// The [`athlete_marker`] of the last account that proved it holds the
    /// provider account and was refused.
    pub claimed_by: Option<String>,
    /// When that proof was made.
    pub claimed_at: Option<DateTime<Utc>>,
}

/// The tombstones of deleted accounts' identities.
#[async_trait]
pub trait ImportTombstoneRepository: Send + Sync {
    /// The live tombstone of `kind` naming `identity_hash` (under `provider`
    /// for a provider account), if there is one.
    async fn find_live(
        &self,
        kind: TombstoneKind,
        provider: Option<&str>,
        identity_hash: &str,
    ) -> AppResult<Option<ImportTombstone>>;

    /// The tombstone with `id`, live or cleared.
    async fn get(&self, id: &str) -> AppResult<Option<ImportTombstone>>;

    /// Record that the account `claimed_by` names proved it holds the
    /// identity of the live tombstone `id`, at `at`. Returns whether a live
    /// tombstone was there to claim.
    async fn record_claim(&self, id: &str, claimed_by: &str, at: DateTime<Utc>) -> AppResult<bool>;

    /// Clear the tombstone `id` as `cleared_by` at `at`, so its identity is
    /// accepted again. Idempotent: a tombstone already cleared keeps its
    /// first clearing. Returns the tombstone as it now stands, `None` when no
    /// tombstone has that id.
    async fn clear(
        &self,
        id: &str,
        cleared_by: &str,
        at: DateTime<Utc>,
    ) -> AppResult<Option<ImportTombstone>>;

    /// The newest `limit` tombstones, live ones only unless `include_cleared`.
    async fn list(&self, include_cleared: bool, limit: i64) -> AppResult<Vec<ImportTombstone>>;
}

/// Every column of a tombstone, in the order the decoder reads them.
macro_rules! tombstone_columns {
    () => {
        "id, subject_hash, kind, provider, identity_hash, created_at, cleared_at, cleared_by, claimed_by, claimed_at"
    };
}

/// The live tombstone of `$1` kind naming `$3` under `$2` (NULL for an email).
pub(crate) const FIND_LIVE_TOMBSTONE_SQL: &str = concat!(
    "SELECT ",
    tombstone_columns!(),
    " FROM import_tombstones WHERE kind = $1 AND COALESCE(provider, '') = COALESCE($2, '') AND identity_hash = $3 AND cleared_at IS NULL"
);

/// The tombstone with id `$1`.
pub(crate) const GET_TOMBSTONE_SQL: &str = concat!(
    "SELECT ",
    tombstone_columns!(),
    " FROM import_tombstones WHERE id = $1"
);

/// Write one tombstone inside the account deletion: `$1` id, `$2` subject
/// hash, `$3` kind, `$4` provider, `$5` identity hash, `$6` now.
pub(crate) const INSERT_TOMBSTONE_SQL: &str = r"
            INSERT INTO import_tombstones
                (id, subject_hash, kind, provider, identity_hash, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ";

/// Record a claim on the live tombstone `$1`: `$2` claimant, `$3` when.
pub(crate) const CLAIM_TOMBSTONE_SQL: &str = r"
            UPDATE import_tombstones SET claimed_by = $2, claimed_at = $3
            WHERE id = $1 AND cleared_at IS NULL
            ";

/// Clear the live tombstone `$1`: `$2` who, `$3` when. A cleared one is left
/// as its first clearing left it.
pub(crate) const CLEAR_TOMBSTONE_SQL: &str = r"
            UPDATE import_tombstones SET cleared_at = $3, cleared_by = $2
            WHERE id = $1 AND cleared_at IS NULL
            ";

/// The newest `$1` live tombstones.
pub(crate) const LIST_LIVE_TOMBSTONES_SQL: &str = concat!(
    "SELECT ",
    tombstone_columns!(),
    " FROM import_tombstones WHERE cleared_at IS NULL ORDER BY created_at DESC, id LIMIT $1"
);

/// The newest `$1` tombstones, cleared ones included.
pub(crate) const LIST_ALL_TOMBSTONES_SQL: &str = concat!(
    "SELECT ",
    tombstone_columns!(),
    " FROM import_tombstones ORDER BY created_at DESC, id LIMIT $1"
);

/// The error for a column of this table that would not decode.
pub(crate) fn tombstone_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read import_tombstones.{name}: {e}"))
}

/// Decode one tombstone row.
///
/// # Errors
/// Returns a database error when a column is missing or does not decode.
pub(crate) fn tombstone_from_row<R>(row: &R) -> AppResult<ImportTombstone>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let kind: String = row
        .try_get("kind")
        .map_err(|e| tombstone_column_error("kind", e))?;
    Ok(ImportTombstone {
        id: row
            .try_get("id")
            .map_err(|e| tombstone_column_error("id", e))?,
        subject_hash: row
            .try_get("subject_hash")
            .map_err(|e| tombstone_column_error("subject_hash", e))?,
        kind: TombstoneKind::parse(&kind)?,
        provider: row
            .try_get("provider")
            .map_err(|e| tombstone_column_error("provider", e))?,
        identity_hash: row
            .try_get("identity_hash")
            .map_err(|e| tombstone_column_error("identity_hash", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| tombstone_column_error("created_at", e))?,
        cleared_at: row
            .try_get("cleared_at")
            .map_err(|e| tombstone_column_error("cleared_at", e))?,
        cleared_by: row
            .try_get("cleared_by")
            .map_err(|e| tombstone_column_error("cleared_by", e))?,
        claimed_by: row
            .try_get("claimed_by")
            .map_err(|e| tombstone_column_error("claimed_by", e))?,
        claimed_at: row
            .try_get("claimed_at")
            .map_err(|e| tombstone_column_error("claimed_at", e))?,
    })
}

/// Inside the account deletion's open transaction (`$guard`), make sure a
/// live tombstone names `$identity_hash` of `$kind` (under `$provider`):
/// reuse the one already live, or write one for `$subject` at `$now`.
/// Evaluates to the tombstone's id; a failure returns the enclosing
/// function's error, rolling the whole deletion back.
macro_rules! ensure_tombstone_in_transaction {
    ($guard:ident, $subject:expr, $kind:expr, $provider:expr, $identity_hash:expr, $now:expr) => {{
        let kind: TombstoneKind = $kind;
        let provider: Option<&str> = $provider;
        let identity_hash: &str = $identity_hash;
        let live = sqlx::query(FIND_LIVE_TOMBSTONE_SQL)
            .bind(kind.as_str())
            .bind(provider)
            .bind(identity_hash)
            .fetch_optional($guard.executor()?)
            .await
            .map_err(|e| AppError::database(format!("Failed to read a live tombstone: {e}")))?;
        if let Some(row) = live {
            row.try_get::<String, _>("id")
                .map_err(|e| tombstone_column_error("id", e))?
        } else {
            let id = Uuid::new_v4().to_string();
            sqlx::query(INSERT_TOMBSTONE_SQL)
                .bind(&id)
                .bind($subject)
                .bind(kind.as_str())
                .bind(provider)
                .bind(identity_hash)
                .bind($now)
                .execute($guard.executor()?)
                .await
                .map_err(|e| AppError::database(format!("Failed to write a tombstone: {e}")))?;
            id
        }
    }};
}
pub(crate) use ensure_tombstone_in_transaction;

/// Emit the whole [`ImportTombstoneRepository`] implementation for one
/// backend type; sqlx resolves the driver from `self.pool()`. The invoking
/// shell must `use` every statement, `tombstone_from_row`, the trait and its
/// types, `AppError`, `AppResult`, `DateTime` and `Utc`.
macro_rules! impl_import_tombstone_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl ImportTombstoneRepository for $ty {
            async fn find_live(
                &self,
                kind: TombstoneKind,
                provider: Option<&str>,
                identity_hash: &str,
            ) -> AppResult<Option<ImportTombstone>> {
                let row = sqlx::query(FIND_LIVE_TOMBSTONE_SQL)
                    .bind(kind.as_str())
                    .bind(provider)
                    .bind(identity_hash)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read a live tombstone: {e}"))
                    })?;
                row.as_ref().map(tombstone_from_row).transpose()
            }

            async fn get(&self, id: &str) -> AppResult<Option<ImportTombstone>> {
                let row = sqlx::query(GET_TOMBSTONE_SQL)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read a tombstone: {e}")))?;
                row.as_ref().map(tombstone_from_row).transpose()
            }

            async fn record_claim(
                &self,
                id: &str,
                claimed_by: &str,
                at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let done = sqlx::query(CLAIM_TOMBSTONE_SQL)
                    .bind(id)
                    .bind(claimed_by)
                    .bind(at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a tombstone claim: {e}"))
                    })?;
                Ok(done.rows_affected() > 0)
            }

            async fn clear(
                &self,
                id: &str,
                cleared_by: &str,
                at: DateTime<Utc>,
            ) -> AppResult<Option<ImportTombstone>> {
                sqlx::query(CLEAR_TOMBSTONE_SQL)
                    .bind(id)
                    .bind(cleared_by)
                    .bind(at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to clear a tombstone: {e}")))?;
                self.get(id).await
            }

            async fn list(
                &self,
                include_cleared: bool,
                limit: i64,
            ) -> AppResult<Vec<ImportTombstone>> {
                let statement = if include_cleared {
                    LIST_ALL_TOMBSTONES_SQL
                } else {
                    LIST_LIVE_TOMBSTONES_SQL
                };
                let rows = sqlx::query(statement)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list tombstones: {e}")))?;
                rows.iter().map(tombstone_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_import_tombstone_repository;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_digest_ignores_case_and_surrounding_space() {
        assert_eq!(
            email_identity_hash("  Jane.Doe@Example.COM "),
            email_identity_hash("jane.doe@example.com")
        );
        assert_ne!(
            email_identity_hash("jane.doe@example.com"),
            email_identity_hash("jane.roe@example.com")
        );
    }

    #[test]
    fn a_digest_never_carries_the_identity_in_clear() {
        let digest = provider_identity_hash("12345678");
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains("12345678"));
        assert_eq!(digest, provider_identity_hash(" 12345678\n"));
    }

    #[test]
    fn an_athlete_marker_names_the_subject_hash_not_the_id() {
        let id = Uuid::new_v4();
        let marker = athlete_marker(id);
        assert_eq!(marker, format!("athlete:{}", subject_hash(id)));
        assert!(!marker.contains(&id.to_string()));
    }

    #[test]
    fn a_kind_round_trips_its_persisted_value_and_refuses_another() {
        for kind in [TombstoneKind::ProviderAccount, TombstoneKind::Email] {
            assert_eq!(TombstoneKind::parse(kind.as_str()).unwrap(), kind);
        }
        assert!(TombstoneKind::parse("phone").is_err());
    }
}
