// ABOUTME: UserExportBundleRepository trait plus the one shared implementation both backends emit
// ABOUTME: The durable state of each originals bundle an athlete asked for: building, ready or failed, and when it expires
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Originals bundles (carnet#1541).
//!
//! `GET /api/user/export` carries the athlete's rows; the bytes it leaves out
//! (kept originals, uploads, feedback screenshots) are built on request into
//! one ZIP in the blob store. A row here is that build's durable state, so a
//! bundle survives an instance restart: `building` while a task writes it, its
//! heartbeat moving forward; `ready` once stored, until it expires; `failed`
//! with why. One `building` row per athlete (a partial unique index), so a
//! second request while one builds reads that one back.
//!
//! Every statement is scoped to the tenant and the user, save the two the
//! hourly sweep runs across every athlete ([`UserExportBundleRepository::stale_building`],
//! [`UserExportBundleRepository::expired`]), as retention does. `$n`
//! placeholders throughout, and every id column is `TEXT` on both engines.

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Where a bundle's build stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleStatus {
    /// A task is writing it.
    Building,
    /// Stored and downloadable until it expires.
    Ready,
    /// The build failed; its blob is gone.
    Failed,
}

impl BundleStatus {
    /// The stored and wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    /// The status `text` names.
    ///
    /// # Errors
    /// Returns a database error for any other text.
    pub fn parse(text: &str) -> AppResult<Self> {
        match text {
            "building" => Ok(Self::Building),
            "ready" => Ok(Self::Ready),
            "failed" => Ok(Self::Failed),
            other => Err(AppError::database(format!(
                "unknown export bundle status `{other}`"
            ))),
        }
    }
}

/// One originals bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportBundle {
    /// Its id, also its blob key's.
    pub id: Uuid,
    /// The tenant its blob is keyed under.
    pub tenant_id: TenantId,
    /// The athlete.
    pub user_id: Uuid,
    /// Where its build stands.
    pub status: BundleStatus,
    /// Whether the athlete sealed it with a passphrase.
    pub sealed: bool,
    /// Files it holds, once ready.
    pub files: Option<u64>,
    /// Files the store no longer held, once ready.
    pub missing: Option<u64>,
    /// Files whose bytes did not match their digest, once ready.
    pub damaged: Option<u64>,
    /// The stored file's size, once ready.
    pub byte_size: Option<u64>,
    /// The stored file's SHA-256, hex, once ready.
    pub sha256: Option<String>,
    /// Why the build failed.
    pub error: Option<String>,
    /// When it was asked for.
    pub created_at: Timestamp,
    /// The building task's last sign of life.
    pub heartbeat_at: Timestamp,
    /// When it became ready or failed.
    pub finished_at: Option<Timestamp>,
    /// When the sweep deletes it.
    pub expires_at: Timestamp,
}

/// What a finished build holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleContents {
    /// Files written.
    pub files: u64,
    /// Files the store no longer held.
    pub missing: u64,
    /// Files whose bytes did not match their digest.
    pub damaged: u64,
    /// The stored file's size.
    pub byte_size: u64,
    /// The stored file's SHA-256, hex.
    pub sha256: String,
}

/// The durable state of the athletes' originals bundles.
#[async_trait]
pub trait UserExportBundleRepository: Send + Sync {
    /// Insert `bundle`, which is `building`. When the athlete already has a
    /// bundle building, nothing is written and that one is returned.
    ///
    /// # Errors
    /// Returns a database error when the write or the read back fails.
    async fn insert_building(&self, bundle: &ExportBundle) -> AppResult<ExportBundle>;

    /// The athlete's newest bundle in `tenant_id`.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn latest(&self, tenant_id: TenantId, user_id: Uuid) -> AppResult<Option<ExportBundle>>;

    /// The athlete's bundle `id` in `tenant_id`; `None` when it is not theirs.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn get(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: Uuid,
    ) -> AppResult<Option<ExportBundle>>;

    /// Move a building bundle's heartbeat to `at`; `false` when it is no
    /// longer building (or gone).
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn heartbeat(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: Uuid,
        at: Timestamp,
    ) -> AppResult<bool>;

    /// Mark a building bundle ready with `contents`, expiring at
    /// `expires_at`; `false` when it is no longer building (or gone).
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn finish_ready(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: Uuid,
        contents: &BundleContents,
        expires_at: Timestamp,
    ) -> AppResult<bool>;

    /// Mark a building bundle failed for `error`; `false` when it is no
    /// longer building (or gone).
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn finish_failed(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: Uuid,
        error: &str,
    ) -> AppResult<bool>;

    /// Bundles of every athlete still building whose heartbeat is older than
    /// `before`: an instance that died mid-build. The sweep's read, across
    /// scopes by design.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn stale_building(&self, before: Timestamp) -> AppResult<Vec<ExportBundle>>;

    /// Bundles of every athlete past their expiry at `now`. The sweep's read,
    /// across scopes by design.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn expired(&self, now: Timestamp) -> AppResult<Vec<ExportBundle>>;

    /// Delete the athlete's bundle row `id`; `false` when there was none.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn delete(&self, tenant_id: TenantId, user_id: Uuid, id: Uuid) -> AppResult<bool>;
}

/// How many rows one sweep read returns at most.
pub(crate) const SWEEP_BATCH: i64 = 200;

macro_rules! bundle_columns {
    () => {
        "id, tenant_id, user_id, status, sealed, files, missing, damaged, byte_size, sha256, \
         error, created_at, heartbeat_at, finished_at, expires_at"
    };
}

/// Insert a building bundle; a second one building for the athlete is a
/// conflict on `one_building_bundle_per_user` and writes nothing.
pub(crate) const INSERT_BUILDING_SQL: &str = concat!(
    "INSERT INTO user_export_bundles (",
    bundle_columns!(),
    ") VALUES ($1, $2, $3, 'building', $4, NULL, NULL, NULL, NULL, NULL, NULL, $5, $5, NULL, $6) \
     ON CONFLICT DO NOTHING"
);

/// The athlete's bundle building now.
pub(crate) const BUILDING_SQL: &str = concat!(
    "SELECT ",
    bundle_columns!(),
    " FROM user_export_bundles WHERE tenant_id = $1 AND user_id = $2 AND status = 'building'"
);

/// The athlete's newest bundle.
pub(crate) const LATEST_SQL: &str = concat!(
    "SELECT ",
    bundle_columns!(),
    " FROM user_export_bundles WHERE tenant_id = $1 AND user_id = $2 \
     ORDER BY created_at DESC, id DESC LIMIT 1"
);

/// One of the athlete's bundles.
pub(crate) const GET_SQL: &str = concat!(
    "SELECT ",
    bundle_columns!(),
    " FROM user_export_bundles WHERE tenant_id = $1 AND user_id = $2 AND id = $3"
);

pub(crate) const HEARTBEAT_SQL: &str = "UPDATE user_export_bundles SET heartbeat_at = $4 \
     WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND status = 'building'";

pub(crate) const FINISH_READY_SQL: &str = "UPDATE user_export_bundles \
     SET status = 'ready', files = $4, missing = $5, damaged = $6, byte_size = $7, sha256 = $8, \
     finished_at = $9, heartbeat_at = $9, expires_at = $10 \
     WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND status = 'building'";

pub(crate) const FINISH_FAILED_SQL: &str = "UPDATE user_export_bundles \
     SET status = 'failed', error = $4, finished_at = $5, heartbeat_at = $5 \
     WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND status = 'building'";

/// The sweep: building rows whose heartbeat stopped, every athlete's.
pub(crate) const STALE_BUILDING_SQL: &str = concat!(
    "SELECT ",
    bundle_columns!(),
    " FROM user_export_bundles WHERE status = 'building' AND heartbeat_at < $1 \
     ORDER BY heartbeat_at LIMIT $2"
);

/// The sweep: rows past their expiry, every athlete's.
pub(crate) const EXPIRED_SQL: &str = concat!(
    "SELECT ",
    bundle_columns!(),
    " FROM user_export_bundles WHERE expires_at < $1 ORDER BY expires_at LIMIT $2"
);

pub(crate) const DELETE_SQL: &str =
    "DELETE FROM user_export_bundles WHERE tenant_id = $1 AND user_id = $2 AND id = $3";

/// A stored counter as an unsigned count; a negative one, which no write
/// stores, reads as zero.
pub(crate) fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// An unsigned count as a stored counter.
pub(crate) fn stored(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Emit the whole [`UserExportBundleRepository`] implementation for one
/// backend type, as `uploaded_activity_files` does. The body names its consts
/// and types unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_user_export_bundle_repository {
    ($ty:ty, $row:ty) => {
        impl $ty {
            fn bundle_from_row(row: &$row) -> AppResult<ExportBundle> {
                let column = |e: sqlx::Error| AppError::database(format!("export bundle row: {e}"));
                let uuid = |name: &str| -> AppResult<Uuid> {
                    let text: String = row.try_get(name).map_err(column)?;
                    Uuid::parse_str(&text)
                        .map_err(|e| AppError::database(format!("export bundle {name}: {e}")))
                };
                let tenant: String = row.try_get("tenant_id").map_err(column)?;
                let status: String = row.try_get("status").map_err(column)?;
                Ok(ExportBundle {
                    id: uuid("id")?,
                    tenant_id: TenantId::parse_str(&tenant)
                        .map_err(|e| AppError::database(format!("export bundle tenant: {e}")))?,
                    user_id: uuid("user_id")?,
                    status: BundleStatus::parse(&status)?,
                    sealed: row.try_get("sealed").map_err(column)?,
                    files: row
                        .try_get::<Option<i64>, _>("files")
                        .map_err(column)?
                        .map(count),
                    missing: row
                        .try_get::<Option<i64>, _>("missing")
                        .map_err(column)?
                        .map(count),
                    damaged: row
                        .try_get::<Option<i64>, _>("damaged")
                        .map_err(column)?
                        .map(count),
                    byte_size: row
                        .try_get::<Option<i64>, _>("byte_size")
                        .map_err(column)?
                        .map(count),
                    sha256: row.try_get("sha256").map_err(column)?,
                    error: row.try_get("error").map_err(column)?,
                    created_at: row.try_get("created_at").map_err(column)?,
                    heartbeat_at: row.try_get("heartbeat_at").map_err(column)?,
                    finished_at: row.try_get("finished_at").map_err(column)?,
                    expires_at: row.try_get("expires_at").map_err(column)?,
                })
            }

            async fn bundles(
                &self,
                sql: &'static str,
                at: Timestamp,
            ) -> AppResult<Vec<ExportBundle>> {
                let rows = sqlx::query(sql)
                    .bind(at)
                    .bind(SWEEP_BATCH)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("export bundle sweep read: {e}")))?;
                rows.iter().map(Self::bundle_from_row).collect()
            }

            async fn one(
                &self,
                sql: &'static str,
                tenant_id: TenantId,
                user_id: Uuid,
                id: Option<Uuid>,
            ) -> AppResult<Option<ExportBundle>> {
                let mut query = sqlx::query(sql)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string());
                if let Some(id) = id {
                    query = query.bind(id.to_string());
                }
                let row = query
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("export bundle read: {e}")))?;
                row.as_ref().map(Self::bundle_from_row).transpose()
            }
        }

        #[async_trait::async_trait]
        impl UserExportBundleRepository for $ty {
            async fn insert_building(&self, bundle: &ExportBundle) -> AppResult<ExportBundle> {
                let written = sqlx::query(INSERT_BUILDING_SQL)
                    .bind(bundle.id.to_string())
                    .bind(bundle.tenant_id.to_string())
                    .bind(bundle.user_id.to_string())
                    .bind(bundle.sealed)
                    .bind(bundle.created_at)
                    .bind(bundle.expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("export bundle insert: {e}")))?;
                let id = (written.rows_affected() > 0).then_some(bundle.id);
                let sql = if id.is_some() { GET_SQL } else { BUILDING_SQL };
                self.one(sql, bundle.tenant_id, bundle.user_id, id)
                    .await?
                    .ok_or_else(|| AppError::database("export bundle insert: no row read back"))
            }

            async fn latest(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Option<ExportBundle>> {
                self.one(LATEST_SQL, tenant_id, user_id, None).await
            }

            async fn get(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                id: Uuid,
            ) -> AppResult<Option<ExportBundle>> {
                self.one(GET_SQL, tenant_id, user_id, Some(id)).await
            }

            async fn heartbeat(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                id: Uuid,
                at: Timestamp,
            ) -> AppResult<bool> {
                let done = sqlx::query(HEARTBEAT_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(id.to_string())
                    .bind(at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("export bundle heartbeat: {e}")))?;
                Ok(done.rows_affected() > 0)
            }

            async fn finish_ready(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                id: Uuid,
                contents: &BundleContents,
                expires_at: Timestamp,
            ) -> AppResult<bool> {
                let done = sqlx::query(FINISH_READY_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(id.to_string())
                    .bind(stored(contents.files))
                    .bind(stored(contents.missing))
                    .bind(stored(contents.damaged))
                    .bind(stored(contents.byte_size))
                    .bind(contents.sha256.as_str())
                    .bind(Timestamp::now())
                    .bind(expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("export bundle ready: {e}")))?;
                Ok(done.rows_affected() > 0)
            }

            async fn finish_failed(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                id: Uuid,
                error: &str,
            ) -> AppResult<bool> {
                let done = sqlx::query(FINISH_FAILED_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(id.to_string())
                    .bind(error)
                    .bind(Timestamp::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("export bundle failed: {e}")))?;
                Ok(done.rows_affected() > 0)
            }

            async fn stale_building(&self, before: Timestamp) -> AppResult<Vec<ExportBundle>> {
                self.bundles(STALE_BUILDING_SQL, before).await
            }

            async fn expired(&self, now: Timestamp) -> AppResult<Vec<ExportBundle>> {
                self.bundles(EXPIRED_SQL, now).await
            }

            async fn delete(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                id: Uuid,
            ) -> AppResult<bool> {
                let done = sqlx::query(DELETE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("export bundle delete: {e}")))?;
                Ok(done.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_user_export_bundle_repository;

#[cfg(test)]
mod tests {
    use super::BundleStatus;

    #[test]
    fn every_status_round_trips_and_an_unknown_one_is_refused() {
        for status in [
            BundleStatus::Building,
            BundleStatus::Ready,
            BundleStatus::Failed,
        ] {
            assert_eq!(
                BundleStatus::parse(status.as_str()).ok(),
                Some(status),
                "bug: {} reads back as itself",
                status.as_str()
            );
        }
        assert!(
            BundleStatus::parse("deleted").is_err(),
            "bug: a status the table's CHECK refuses is refused here too"
        );
    }
}
