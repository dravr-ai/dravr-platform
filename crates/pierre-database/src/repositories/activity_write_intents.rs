// ABOUTME: Repository trait, statements and shared body for walk steps' writes to the activity store, recorded before they land
// ABOUTME: A row a step could not clear after its write is settled by the resume sweep; written once, emitted per backend

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A walk step's write to the activity store, recorded before it lands
//! (registre#1555).
//!
//! A step checks its ledger row and the purge attestations after its write.
//! When that check stays unreadable, it cannot tell whether a purge of the
//! key it wrote under landed in between, and once its row is gone nothing
//! retries it. So the step records here, before its write, what it is about
//! to write and from when the attestations answer for it, and clears the
//! row once its check was read. A row still standing is settled by the
//! resume sweep once the attestations read again.

use std::fmt::Display;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// One walk step's write, recorded before it landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityWriteIntent {
    /// The intent's id (UUID string), minted by the step.
    pub id: String,
    /// The step's tenant.
    pub tenant_id: TenantId,
    /// The athlete.
    pub user_id: Uuid,
    /// The key the step wrote under.
    pub provider: String,
    /// The step's job kind, as the ledger names it.
    pub kind: String,
    /// Every activity id the step wrote.
    pub page_ids: Vec<String>,
    /// The ids the store neither served nor recorded before the write: what
    /// a retention expiry takes back.
    pub added_ids: Vec<String>,
    /// The write's sync stamp (unix microseconds): a copy stamped later is
    /// another writer's.
    pub written_at_us: i64,
    /// The attestations answering for the write are read from here (unix ms).
    pub purges_since_ms: i64,
    /// The purge reasons read before the step's first check: not the step's
    /// to answer.
    pub attested_before: Vec<String>,
    /// When the step recorded it (unix ms).
    pub created_at_ms: i64,
    /// When the sweep may settle it (unix ms).
    pub due_at_ms: i64,
    /// Failed attempts to settle it so far.
    pub attempts: i64,
}

/// Walk steps' writes, recorded before they land.
#[async_trait]
pub trait ActivityWriteIntentRepository: Send + Sync {
    /// Record `intent`, before the step writes.
    async fn record_write_intent(&self, intent: &ActivityWriteIntent) -> AppResult<()>;

    /// Remove the tenant's intent `id`: its step, or the sweep, settled the
    /// write. Returns whether a row was removed.
    async fn clear_write_intent(&self, tenant_id: TenantId, id: &str) -> AppResult<bool>;

    /// The intents due at `now_ms`, earliest first, at most `limit`. Across
    /// every tenant, for the resume sweep.
    async fn due_write_intents(
        &self,
        now_ms: i64,
        limit: i64,
    ) -> AppResult<Vec<ActivityWriteIntent>>;

    /// Count a failed attempt to settle the tenant's intent `id` and hold it
    /// until `due_at_ms`.
    async fn postpone_write_intent(
        &self,
        tenant_id: TenantId,
        id: &str,
        due_at_ms: i64,
    ) -> AppResult<()>;
}

/// Record an intent: `$1` id, `$2` tenant, `$3` user as text, `$4` provider,
/// `$5` kind, `$6` page ids, `$7` added ids (JSON arrays), `$8` write stamp
/// (µs), `$9` attestations' start (ms), `$10` reasons attested before (JSON
/// array), `$11` created (ms), `$12` due (ms).
pub(crate) const RECORD_WRITE_INTENT_SQL: &str = r"
            INSERT INTO activity_write_intents
                (id, tenant_id, user_id, provider, kind, page_ids, added_ids, written_at_us,
                 purges_since_ms, attested_before, created_at_ms, due_at_ms, attempts)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 0)
            ";

/// Remove the tenant `$2`'s intent `$1`.
pub(crate) const CLEAR_WRITE_INTENT_SQL: &str =
    "DELETE FROM activity_write_intents WHERE id = $1 AND tenant_id = $2";

/// ISOLATION EXEMPTION: the resume sweep settles every tenant's intents, as
/// it claims every tenant's lapsed walks; each row carries its own tenant
/// and user, and every write the sweep makes is scoped to them.
///
/// The intents due at `$1`, earliest first, capped at `$2`.
pub(crate) const DUE_WRITE_INTENTS_SQL: &str = r"
            SELECT id, tenant_id, user_id, provider, kind, page_ids, added_ids, written_at_us,
                   purges_since_ms, attested_before, created_at_ms, due_at_ms, attempts
            FROM activity_write_intents
            WHERE due_at_ms <= $1
            ORDER BY due_at_ms
            LIMIT $2
            ";

/// Count a failed attempt on the tenant `$3`'s intent `$1` and hold it
/// until `$2`.
pub(crate) const POSTPONE_WRITE_INTENT_SQL: &str = r"
            UPDATE activity_write_intents
            SET attempts = attempts + 1, due_at_ms = $2
            WHERE id = $1 AND tenant_id = $3
            ";

/// The error for a column of this table that would not decode.
pub(crate) fn write_intent_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read activity_write_intents.{name}: {e}"))
}

/// `ids` as the JSON array a list column holds.
pub(crate) fn encode_id_list(ids: &[String]) -> AppResult<String> {
    serde_json::to_string(ids)
        .map_err(|e| AppError::internal(format!("activity write intent ids: {e}")))
}

/// The list column `name` holding `json`, decoded.
pub(crate) fn decode_id_list(name: &str, json: &str) -> AppResult<Vec<String>> {
    serde_json::from_str(json).map_err(|e| write_intent_column_error(name, e))
}

/// Decode one `activity_write_intents` row.
pub(crate) fn write_intent_from_row<R>(row: &R) -> AppResult<ActivityWriteIntent>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    TenantId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let text = |name: &str| -> AppResult<String> {
        row.try_get(name)
            .map_err(|e| write_intent_column_error(name, e))
    };
    let number = |name: &str| -> AppResult<i64> {
        row.try_get(name)
            .map_err(|e| write_intent_column_error(name, e))
    };
    let user_id = text("user_id")?;
    Ok(ActivityWriteIntent {
        id: text("id")?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| write_intent_column_error("tenant_id", e))?,
        user_id: Uuid::parse_str(&user_id).map_err(|e| write_intent_column_error("user_id", e))?,
        provider: text("provider")?,
        kind: text("kind")?,
        page_ids: decode_id_list("page_ids", &text("page_ids")?)?,
        added_ids: decode_id_list("added_ids", &text("added_ids")?)?,
        written_at_us: number("written_at_us")?,
        purges_since_ms: number("purges_since_ms")?,
        attested_before: decode_id_list("attested_before", &text("attested_before")?)?,
        created_at_ms: number("created_at_ms")?,
        due_at_ms: number("due_at_ms")?,
        attempts: number("attempts")?,
    })
}

/// Emit the whole [`ActivityWriteIntentRepository`] implementation for one
/// backend type; sqlx resolves the driver from `self.pool()`.
macro_rules! impl_activity_write_intent_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl ActivityWriteIntentRepository for $ty {
            async fn record_write_intent(&self, intent: &ActivityWriteIntent) -> AppResult<()> {
                sqlx::query(RECORD_WRITE_INTENT_SQL)
                    .bind(&intent.id)
                    .bind(intent.tenant_id)
                    .bind(intent.user_id.to_string())
                    .bind(&intent.provider)
                    .bind(&intent.kind)
                    .bind(encode_id_list(&intent.page_ids)?)
                    .bind(encode_id_list(&intent.added_ids)?)
                    .bind(intent.written_at_us)
                    .bind(intent.purges_since_ms)
                    .bind(encode_id_list(&intent.attested_before)?)
                    .bind(intent.created_at_ms)
                    .bind(intent.due_at_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to record an activity write intent: {e}"
                        ))
                    })?;
                Ok(())
            }

            async fn clear_write_intent(&self, tenant_id: TenantId, id: &str) -> AppResult<bool> {
                let cleared = sqlx::query(CLEAR_WRITE_INTENT_SQL)
                    .bind(id)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to clear an activity write intent: {e}"))
                    })?;
                Ok(cleared.rows_affected() > 0)
            }

            async fn due_write_intents(
                &self,
                now_ms: i64,
                limit: i64,
            ) -> AppResult<Vec<ActivityWriteIntent>> {
                sqlx::query(DUE_WRITE_INTENTS_SQL)
                    .bind(now_ms)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list activity write intents: {e}"))
                    })?
                    .iter()
                    .map(write_intent_from_row)
                    .collect()
            }

            async fn postpone_write_intent(
                &self,
                tenant_id: TenantId,
                id: &str,
                due_at_ms: i64,
            ) -> AppResult<()> {
                sqlx::query(POSTPONE_WRITE_INTENT_SQL)
                    .bind(id)
                    .bind(due_at_ms)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to postpone an activity write intent: {e}"
                        ))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_activity_write_intent_repository;
