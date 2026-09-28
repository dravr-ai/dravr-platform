// ABOUTME: Repository trait for the persona-held notifications each digest returned, one row per held row
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What each persona digest handed back.
//!
//! An armed persona floor persists the pushes it withholds, and a digest on
//! the persona's cadence returns them. A digest *claims* the held rows it is
//! about to return, one row here per held notification, before it is sent:
//! the held notification's id is the primary key, so a row is claimed by one
//! digest only, however many sweeps or session landings race for it. A digest
//! that did not go out releases its claim, and its rows wait for the next one.
//!
//! The record lives here rather than on the digest notification because an
//! athlete can delete that notification from their feed; what the digest
//! returned must not come back because of it.
//!
//! Every statement is keyed by both `user_id` and `tenant_id`, the recipient
//! the held rows and their digests belong to.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The held notifications persona digests have returned.
#[async_trait]
pub trait PersonaDigestReturnRepository: Send + Sync {
    /// Claim `notification_ids` — held rows of `user_id` in `tenant_id` — for
    /// the digest `batch_id` about to return them at `claimed_at`.
    ///
    /// Returns the ids this call claimed. An id an earlier digest already
    /// claimed stays with that digest and is left out. The claim is one
    /// transaction: on error nothing is claimed.
    async fn claim_persona_digest_items(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        batch_id: Uuid,
        notification_ids: &[Uuid],
        claimed_at: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>>;

    /// Release every claim of the digest `batch_id`, which did not go out, so
    /// its rows are held again. Returns how many rows were released.
    async fn release_persona_digest_batch(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        batch_id: Uuid,
    ) -> AppResult<u64>;

    /// The held rows of `user_id` in `tenant_id` a digest returned at or after
    /// `since`.
    ///
    /// A row is always returned after it was held, so passing the time the
    /// oldest row in question was held yields every one of them a digest has
    /// already returned.
    async fn persona_digest_items_returned_since(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        since: DateTime<Utc>,
    ) -> AppResult<HashSet<Uuid>>;

    /// When the newest digest of `user_id` in `tenant_id` returned its rows,
    /// or `None` when no digest has returned any.
    async fn last_persona_digest_at(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<DateTime<Utc>>>;
}

/// `notification_id` is the key: a row an earlier digest claimed is left
/// alone, and the affected count says whether this insert claimed it.
pub(crate) const CLAIM_PERSONA_DIGEST_ITEM_SQL: &str = r"
            INSERT INTO persona_digest_returns (notification_id, user_id, tenant_id, batch_id, returned_at_ms)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (notification_id) DO NOTHING
            ";

pub(crate) const RELEASE_PERSONA_DIGEST_BATCH_SQL: &str = r"
            DELETE FROM persona_digest_returns
            WHERE user_id = $1 AND tenant_id = $2 AND batch_id = $3
            ";

pub(crate) const PERSONA_DIGEST_ITEMS_RETURNED_SINCE_SQL: &str = r"
            SELECT notification_id
            FROM persona_digest_returns
            WHERE user_id = $1 AND tenant_id = $2 AND returned_at_ms >= $3
            ";

pub(crate) const LAST_PERSONA_DIGEST_AT_SQL: &str = r"
            SELECT MAX(returned_at_ms)
            FROM persona_digest_returns
            WHERE user_id = $1 AND tenant_id = $2
            ";

/// A stored `returned_at_ms` as the instant it records.
///
/// # Errors
/// Returns a database error for a value outside chrono's representable range,
/// which only a hand-edited row can hold.
pub(crate) fn returned_at_from_ms(ms: i64) -> AppResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms).ok_or_else(|| {
        AppError::database(format!(
            "persona_digest_returns.returned_at_ms {ms} is not a representable instant"
        ))
    })
}

/// Emit the whole [`PersonaDigestReturnRepository`] implementation for one
/// backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]) for every id column.
/// `notification_id` has the type of the `notifications.id` it references:
/// `TEXT` on SQLite, `UUID` on PostgreSQL.
macro_rules! impl_persona_digest_return_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl PersonaDigestReturnRepository for $ty {
            async fn claim_persona_digest_items(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                batch_id: Uuid,
                notification_ids: &[Uuid],
                claimed_at: DateTime<Utc>,
            ) -> AppResult<Vec<Uuid>> {
                let mut tx =
                    self.pool().begin().await.map_err(|e| {
                        AppError::database(format!("begin persona digest claim: {e}"))
                    })?;
                let mut claimed = Vec::with_capacity(notification_ids.len());
                for id in notification_ids {
                    let result = sqlx::query(CLAIM_PERSONA_DIGEST_ITEM_SQL)
                        .bind($ids::bind(*id))
                        .bind($ids::bind(user_id))
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .bind($ids::bind(batch_id))
                        .bind(claimed_at.timestamp_millis())
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to claim persona digest item: {e}"))
                        })?;
                    if result.rows_affected() > 0 {
                        claimed.push(*id);
                    }
                }
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("commit persona digest claim: {e}")))?;
                Ok(claimed)
            }

            async fn release_persona_digest_batch(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                batch_id: Uuid,
            ) -> AppResult<u64> {
                let result = sqlx::query(RELEASE_PERSONA_DIGEST_BATCH_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(batch_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release persona digest batch: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn persona_digest_items_returned_since(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                since: DateTime<Utc>,
            ) -> AppResult<HashSet<Uuid>> {
                let rows = sqlx::query(PERSONA_DIGEST_ITEMS_RETURNED_SINCE_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind(since.timestamp_millis())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read persona digest returns: {e}"))
                    })?;
                rows.iter()
                    .map(|row| $ids::read(row, "notification_id"))
                    .collect()
            }

            async fn last_persona_digest_at(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<DateTime<Utc>>> {
                let (newest,): (Option<i64>,) = sqlx::query_as(LAST_PERSONA_DIGEST_AT_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the last persona digest: {e}"))
                    })?;
                newest.map(returned_at_from_ms).transpose()
            }
        }
    };
}
pub(crate) use impl_persona_digest_return_repository;
