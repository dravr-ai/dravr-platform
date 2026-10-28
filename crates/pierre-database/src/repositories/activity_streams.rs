// ABOUTME: ActivityStreamRepository trait plus the one shared implementation both backends emit
// ABOUTME: The one per-activity sample store: pushed, uploaded or imported samples per (tenant, user, provider, activity)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stored activity samples: the one per-activity sample store.
//!
//! Every activity whose samples Dravr holds itself keeps them here, one row
//! per activity, its `origin` column saying how they arrived: `push` (a
//! provider pushed them, as the Garmin Health API pushes activity details,
//! carnet#1155), `upload` or `archive`. Only pushed samples are written here
//! today ([`StreamOrigin`]); a pushed set is the only place the samples can
//! be read from again.
//! They are kept apart from the cached activity row on purpose: every list
//! read deserializes that row, and an hour of 1 Hz samples would multiply what
//! each list read carries; and the summary and the samples arrive in separate
//! pushes, where a later summary rewrites the cached row whole.
//!
//! The samples are stored as the [`TimeSeriesData`] JSON the activity model
//! carries, at [`STREAMS_FORMAT_VERSION`], so a read hands them back in the
//! shape every streams consumer already reads; a row at a version this build
//! does not know is refused rather than misread. `$n` placeholders
//! throughout: sqlx accepts them on `SQLite` as well as Postgres. `user_id`
//! is `uuid` on Postgres (a foreign key to `users`) and `TEXT` on `SQLite`;
//! it binds the hyphenated string on both, cast on Postgres. `synced_at`
//! binds as `DateTime<Utc>`, which sqlx stores as RFC 3339 text on `SQLite`
//! and as `TIMESTAMPTZ` on Postgres.
//!
//! A provider purge deletes only `origin = 'push'` rows of that provider:
//! an upload or an archive import is the athlete's own record, deleted with
//! the athlete or their account, never with a provider connection.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TimeSeriesData};
use uuid::Uuid;

/// The version of the sample JSON this build writes and reads: the
/// [`TimeSeriesData`] serialization, timestamps in seconds from the start.
pub const STREAMS_FORMAT_VERSION: i16 = 1;

/// How a stored sample set reached Dravr: the `origin` column.
///
/// The column admits `push`, `upload` and `archive` (the migration's CHECK),
/// but only `Push` has a writer today, so it is the only variant. Uploaded
/// and imported samples are still read from their files; the change that
/// moves them onto this store adds `Upload` and `Archive` here, with no
/// schema change. Rows of those origins are never touched by a provider
/// purge, which deletes `origin = 'push'` rows only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamOrigin {
    /// A provider pushed it; a purge of that provider deletes it.
    Push,
}

impl StreamOrigin {
    /// The value the `origin` column stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
        }
    }
}

/// One activity's samples, as they are written.
#[derive(Debug, Clone, Copy)]
pub struct ActivityStreamsWrite<'a> {
    /// Tenant the activity is held in.
    pub tenant_id: &'a TenantId,
    /// The activity's owner.
    pub user_id: Uuid,
    /// The provider key the activity is read under.
    pub provider: &'a str,
    /// That provider's id for the activity.
    pub activity_id: &'a str,
    /// How the samples reached Dravr.
    pub origin: StreamOrigin,
    /// What they were decoded from, when it is named: the SHA-256 of the
    /// original file, or a blob id.
    pub source_ref: Option<&'a str>,
    /// The samples.
    pub streams: &'a TimeSeriesData,
    /// When they were received.
    pub synced_at: DateTime<Utc>,
}

/// The one per-activity sample store.
#[async_trait]
pub trait ActivityStreamRepository: Send + Sync {
    /// The stored samples of one activity, or `None` when none were stored.
    ///
    /// # Errors
    /// Returns a database error when the read fails, the row is at a format
    /// version this build does not read, or the stored JSON is not a sample
    /// set.
    async fn get_activity_streams(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<Option<TimeSeriesData>>;

    /// Store the samples of one activity, unless the stored ones were
    /// received later: a provider re-sends an activity's samples as an
    /// update, and the latest receipt wins, whichever order they are
    /// written in. Returns whether they were written.
    ///
    /// # Errors
    /// Returns a database error when the samples do not serialize or the
    /// write fails.
    async fn upsert_activity_streams(&self, write: &ActivityStreamsWrite<'_>) -> AppResult<bool>;
}

/// One activity's samples and their format version. `$uuid` is the cast the
/// `user_id` column needs on the bound string: `"::uuid"` on Postgres, `""` on
/// `SQLite`.
macro_rules! get_activity_streams_sql {
    ($uuid:literal) => {
        concat!(
            "SELECT streams_json, format_version FROM activity_streams \
             WHERE tenant_id = $1 AND user_id = $2",
            $uuid,
            " AND provider = $3 AND activity_id = $4"
        )
    };
}

/// Write one activity's samples unless the stored ones were received later.
/// `$uuid` as in [`get_activity_streams_sql`].
macro_rules! upsert_activity_streams_sql {
    ($uuid:literal) => {
        concat!(
            "INSERT INTO activity_streams (tenant_id, user_id, provider, activity_id, \
             origin, source_ref, format_version, streams_json, synced_at) \
             VALUES ($1, $2",
            $uuid,
            ", $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (tenant_id, user_id, provider, activity_id) DO UPDATE SET \
             origin = EXCLUDED.origin, source_ref = EXCLUDED.source_ref, \
             format_version = EXCLUDED.format_version, \
             streams_json = EXCLUDED.streams_json, synced_at = EXCLUDED.synced_at \
             WHERE activity_streams.synced_at <= EXCLUDED.synced_at"
        )
    };
}

/// [`get_activity_streams_sql`] on `SQLite`, where `user_id` is `TEXT`.
pub(crate) const GET_ACTIVITY_STREAMS_SQL: &str = get_activity_streams_sql!("");

/// [`upsert_activity_streams_sql`] on `SQLite`.
pub(crate) const UPSERT_ACTIVITY_STREAMS_SQL: &str = upsert_activity_streams_sql!("");

/// [`get_activity_streams_sql`] on Postgres, where `user_id` is `uuid`.
#[cfg(feature = "postgresql")]
pub(crate) const GET_ACTIVITY_STREAMS_PG_SQL: &str = get_activity_streams_sql!("::uuid");

/// [`upsert_activity_streams_sql`] on Postgres.
#[cfg(feature = "postgresql")]
pub(crate) const UPSERT_ACTIVITY_STREAMS_PG_SQL: &str = upsert_activity_streams_sql!("::uuid");

/// The samples one stored row holds, at `format_version`.
///
/// # Errors
/// Returns a database error when the version is not one this build reads or
/// the stored JSON is not a sample set.
pub(crate) fn streams_from_row(json: &str, format_version: i16) -> AppResult<TimeSeriesData> {
    if format_version != STREAMS_FORMAT_VERSION {
        return Err(AppError::database(format!(
            "stored activity streams are at format version {format_version}, this build \
             reads {STREAMS_FORMAT_VERSION}"
        )));
    }
    serde_json::from_str(json)
        .map_err(|e| AppError::database(format!("stored activity streams do not parse: {e}")))
}

/// The JSON one row stores for `streams`.
///
/// # Errors
/// Returns an internal error when the samples do not serialize.
pub(crate) fn streams_to_json(streams: &TimeSeriesData) -> AppResult<String> {
    serde_json::to_string(streams)
        .map_err(|e| AppError::internal(format!("serialize activity streams: {e}")))
}

/// Emit the whole [`ActivityStreamRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type and its backend's statements (`GET_ACTIVITY_STREAMS_SQL` /
/// `UPSERT_ACTIVITY_STREAMS_SQL`, or their `_PG_` forms), and sqlx resolves
/// the driver from `self.pool()` per expansion. The body names its helpers
/// and types unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_activity_stream_repository {
    ($ty:ty, $get:expr, $upsert:expr) => {
        #[async_trait::async_trait]
        impl ActivityStreamRepository for $ty {
            async fn get_activity_streams(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<Option<TimeSeriesData>> {
                let row: Option<(String, i16)> = sqlx::query_as($get)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_activity_streams: {e}")))?;
                row.map(|(json, version)| streams_from_row(&json, version))
                    .transpose()
            }

            async fn upsert_activity_streams(
                &self,
                write: &ActivityStreamsWrite<'_>,
            ) -> AppResult<bool> {
                let json = streams_to_json(write.streams)?;
                let written = sqlx::query($upsert)
                    .bind(write.tenant_id.to_string())
                    .bind(write.user_id.to_string())
                    .bind(write.provider)
                    .bind(write.activity_id)
                    .bind(write.origin.as_str())
                    .bind(write.source_ref)
                    .bind(STREAMS_FORMAT_VERSION)
                    .bind(json)
                    .bind(write.synced_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert_activity_streams: {e}")))?
                    .rows_affected();
                Ok(written > 0)
            }
        }
    };
}
pub(crate) use impl_activity_stream_repository;
