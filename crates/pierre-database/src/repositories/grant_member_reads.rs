// ABOUTME: Repository trait for when an outside client's grant last read another member, one row per grant
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The grant-scoped member-read taint (carnet#1511 call 30).
//!
//! A tool call over `/mcp` or A2A carries no topic: nothing ties one call's
//! read of another member to the next call's write. The taint is the
//! grant's instead. Each row records the newest read of another member a
//! grant made; the executor refuses a free-text write by that grant until
//! the window after it ends. Every statement is keyed by `user_id`, the
//! account the grant acts for.

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

/// When each outside client's grant last read another member.
#[async_trait]
pub trait GrantMemberReadRepository: Send + Sync {
    /// The newest read of another member that `grant_key`, acting for
    /// `user_id`, made; `None` when it made none.
    async fn grant_member_read_at(
        &self,
        user_id: Uuid,
        grant_key: &str,
    ) -> AppResult<Option<Timestamp>>;

    /// Record that `grant_key`, acting for `user_id`, read another member
    /// at `read_at`, replacing an older read; an older `read_at` keeps the
    /// newer one stored.
    async fn record_grant_member_read(
        &self,
        user_id: Uuid,
        grant_key: &str,
        read_at: Timestamp,
    ) -> AppResult<()>;
}

pub(crate) const GRANT_MEMBER_READ_AT_SQL: &str = r"
            SELECT read_at_ms
            FROM grant_member_reads
            WHERE user_id = $1 AND grant_key = $2
            ";

/// `excluded` names the row the insert proposed on both engines; the
/// newer instant wins, so two calls racing keep the later read.
pub(crate) const RECORD_GRANT_MEMBER_READ_SQL: &str = r"
            INSERT INTO grant_member_reads (user_id, grant_key, read_at_ms)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id, grant_key) DO UPDATE SET
                read_at_ms = CASE
                    WHEN excluded.read_at_ms > grant_member_reads.read_at_ms
                        THEN excluded.read_at_ms
                    ELSE grant_member_reads.read_at_ms
                END
            ";

/// A stored `read_at_ms` as the instant it records.
///
/// # Errors
/// Returns a database error for a value outside the range the date types
/// hold, which only a hand-edited row can hold.
pub(crate) fn read_at_from_ms(ms: i64) -> AppResult<Timestamp> {
    Timestamp::from_epoch_millis(ms).ok().ok_or_else(|| {
        AppError::database(format!(
            "grant_member_reads.read_at_ms {ms} is not a representable instant"
        ))
    })
}

/// Emit the whole [`GrantMemberReadRepository`] implementation for one
/// backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres.
macro_rules! impl_grant_member_read_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl GrantMemberReadRepository for $ty {
            async fn grant_member_read_at(
                &self,
                user_id: Uuid,
                grant_key: &str,
            ) -> AppResult<Option<Timestamp>> {
                let row: Option<(i64,)> = sqlx::query_as(GRANT_MEMBER_READ_AT_SQL)
                    .bind($ids::bind(user_id))
                    .bind(grant_key)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read a grant's member read: {e}"))
                    })?;
                row.map(|(ms,)| read_at_from_ms(ms)).transpose()
            }

            async fn record_grant_member_read(
                &self,
                user_id: Uuid,
                grant_key: &str,
                read_at: Timestamp,
            ) -> AppResult<()> {
                sqlx::query(RECORD_GRANT_MEMBER_READ_SQL)
                    .bind($ids::bind(user_id))
                    .bind(grant_key)
                    .bind(read_at.epoch_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a grant's member read: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_grant_member_read_repository;
