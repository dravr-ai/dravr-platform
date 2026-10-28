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
//! carnet#1155), `upload` (decoded once from a file the athlete uploaded,
//! carnet#1219) or `archive`. A pushed set is the only place its samples can
//! be read from again; an uploaded set can also be decoded again from the
//! file it names in `source_ref`.
//! They are kept apart from the cached activity row on purpose: every list
//! read deserializes that row, and an hour of 1 Hz samples would multiply what
//! each list read carries; and the summary and the samples arrive in separate
//! pushes, where a later summary rewrites the cached row whole.
//!
//! The samples are stored as the [`TimeSeriesData`] JSON the activity model
//! carries, at [`STREAMS_FORMAT_VERSION`], so a read hands them back in the
//! shape every streams consumer already reads; a row at a version this build
//! does not know is refused rather than misread. How a row at an earlier
//! version is read is [`streams_from_row`]'s to say. `$n` placeholders
//! throughout: sqlx accepts them on `SQLite` as well as Postgres. `user_id`
//! is `uuid` on Postgres (a foreign key to `users`) and `TEXT` on `SQLite`;
//! it binds the hyphenated string on both, cast on Postgres. `synced_at`
//! binds as `DateTime<Utc>`, which sqlx stores as RFC 3339 text on `SQLite`
//! and as `TIMESTAMPTZ` on Postgres.
//!
//! A provider purge deletes only `origin = 'push'` rows of that provider:
//! an upload or an archive import is the athlete's own record, deleted with
//! the athlete or their account, or with the one activity the athlete
//! deletes, never with a provider connection.
//!
//! Every statement is scoped to a tenant and a user except
//! [`ActivityStreamRepository::list_activities_without_current_streams`], which the
//! backfill worker runs across tenants to find the activities still owed a
//! row, and which returns each one with the tenant and user it is held for.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TimeSeriesData};
use uuid::Uuid;

/// The version of the sample JSON this build writes.
///
/// The [`TimeSeriesData`] serialization, timestamps in seconds from the
/// start, every channel one entry per sample with `None` for a sample it
/// missed, GPS and cumulative distance included (carnet#1226).
pub const STREAMS_FORMAT_VERSION: i16 = 2;

/// The version written before GPS and distance carried gaps.
///
/// The same JSON shape, but a sample without a fix held a copied
/// neighbouring fix, and a FIT file's sample without a distance repeated the
/// last one.
pub const GAP_FILLED_FORMAT_VERSION: i16 = 1;

/// How a stored sample set reached Dravr: the `origin` column.
///
/// The column admits `push`, `upload` and `archive` (the migration's CHECK),
/// one variant each. Rows of every origin but `push` are never touched by a
/// provider purge, which deletes `origin = 'push'` rows only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamOrigin {
    /// A provider pushed it; a purge of that provider deletes it.
    Push,
    /// Decoded from a file the athlete uploaded, whose SHA-256 is the row's
    /// `source_ref`; deleted with the activity, the athlete or the account.
    Upload,
    /// Decoded from an original kept by an archive import (dravr-barrage),
    /// whose SHA-256 is the row's `source_ref`; deleted with the activity,
    /// the imported source or the account.
    Archive,
}

impl StreamOrigin {
    /// The value the `origin` column stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Upload => "upload",
            Self::Archive => "archive",
        }
    }
}

/// A cached activity no current sample row is stored for yet, as
/// [`ActivityStreamRepository::list_activities_without_current_streams`]
/// finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityWithoutStreams {
    /// The tenant the activity is held in, as the cache stores it.
    pub tenant_id: String,
    /// The activity's owner, as the cache stores it.
    pub user_id: String,
    /// The provider's id for the activity.
    pub activity_id: String,
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
    /// The stored samples of one activity, or `None` when none were stored,
    /// or the stored ones are to be decoded again from their source
    /// ([`streams_from_row`]).
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

    /// Delete the stored samples of one activity, whatever their origin: the
    /// athlete deleted the activity itself. Returns whether a row went.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn delete_activity_streams(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<bool>;

    /// Delete every stored sample set of one provider key for one user,
    /// whatever their origin: the athlete deleted a source they brought in
    /// (an imported archive). Returns how many rows went.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn delete_provider_activity_streams(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
    ) -> AppResult<u64>;

    /// Up to `limit` activities cached under `provider`, in every tenant,
    /// that hold no sample row at [`STREAMS_FORMAT_VERSION`] (none at all, or
    /// one at an earlier version), in a stable order: what a backfill still
    /// owes a current row. The one statement here that is not scoped to a
    /// tenant, since the worker that runs it serves them all.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn list_activities_without_current_streams(
        &self,
        provider: &str,
        limit: i64,
    ) -> AppResult<Vec<ActivityWithoutStreams>>;
}

/// One activity's samples and their format version. `$uuid` is the cast the
/// `user_id` column needs on the bound string: `"::uuid"` on Postgres, `""` on
/// `SQLite`.
macro_rules! get_activity_streams_sql {
    ($uuid:literal) => {
        concat!(
            "SELECT streams_json, format_version, source_ref FROM activity_streams \
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

/// Delete one activity's samples. `$uuid` as in [`get_activity_streams_sql`].
macro_rules! delete_activity_streams_sql {
    ($uuid:literal) => {
        concat!(
            "DELETE FROM activity_streams WHERE tenant_id = $1 AND user_id = $2",
            $uuid,
            " AND provider = $3 AND activity_id = $4"
        )
    };
}

/// Delete one provider key's samples for one user. `$uuid` as in
/// [`get_activity_streams_sql`].
macro_rules! delete_provider_activity_streams_sql {
    ($uuid:literal) => {
        concat!(
            "DELETE FROM activity_streams WHERE tenant_id = $1 AND user_id = $2",
            $uuid,
            " AND provider = $3"
        )
    };
}

/// The cached activities of one provider, across tenants, with no sample row
/// at the format version `$3`. `$user` is how the stored row's `user_id` is
/// compared with the cache's `TEXT` one: as it is on `SQLite`, cast down to
/// text on Postgres, where it is `uuid`.
macro_rules! list_activities_without_current_streams_sql {
    ($user:literal) => {
        concat!(
            "SELECT c.tenant_id, c.user_id, c.activity_id FROM cached_activities c \
             WHERE c.provider = $1 AND NOT EXISTS (SELECT 1 FROM activity_streams s \
             WHERE s.tenant_id = c.tenant_id AND ",
            $user,
            " = c.user_id AND s.provider = c.provider AND s.activity_id = c.activity_id \
             AND s.format_version = $3) \
             ORDER BY c.tenant_id, c.user_id, c.activity_id LIMIT $2"
        )
    };
}

/// [`get_activity_streams_sql`] on `SQLite`, where `user_id` is `TEXT`.
pub(crate) const GET_ACTIVITY_STREAMS_SQL: &str = get_activity_streams_sql!("");

/// [`upsert_activity_streams_sql`] on `SQLite`.
pub(crate) const UPSERT_ACTIVITY_STREAMS_SQL: &str = upsert_activity_streams_sql!("");

/// [`delete_activity_streams_sql`] on `SQLite`.
pub(crate) const DELETE_ACTIVITY_STREAMS_SQL: &str = delete_activity_streams_sql!("");

/// [`delete_provider_activity_streams_sql`] on `SQLite`.
pub(crate) const DELETE_PROVIDER_ACTIVITY_STREAMS_SQL: &str =
    delete_provider_activity_streams_sql!("");

/// [`list_activities_without_current_streams_sql`] on `SQLite`.
pub(crate) const LIST_ACTIVITIES_WITHOUT_CURRENT_STREAMS_SQL: &str =
    list_activities_without_current_streams_sql!("s.user_id");

/// [`get_activity_streams_sql`] on Postgres, where `user_id` is `uuid`.
#[cfg(feature = "postgresql")]
pub(crate) const GET_ACTIVITY_STREAMS_PG_SQL: &str = get_activity_streams_sql!("::uuid");

/// [`upsert_activity_streams_sql`] on Postgres.
#[cfg(feature = "postgresql")]
pub(crate) const UPSERT_ACTIVITY_STREAMS_PG_SQL: &str = upsert_activity_streams_sql!("::uuid");

/// [`delete_activity_streams_sql`] on Postgres.
#[cfg(feature = "postgresql")]
pub(crate) const DELETE_ACTIVITY_STREAMS_PG_SQL: &str = delete_activity_streams_sql!("::uuid");

/// [`delete_provider_activity_streams_sql`] on Postgres.
#[cfg(feature = "postgresql")]
pub(crate) const DELETE_PROVIDER_ACTIVITY_STREAMS_PG_SQL: &str =
    delete_provider_activity_streams_sql!("::uuid");

/// [`list_activities_without_current_streams_sql`] on Postgres.
#[cfg(feature = "postgresql")]
pub(crate) const LIST_ACTIVITIES_WITHOUT_CURRENT_STREAMS_PG_SQL: &str =
    list_activities_without_current_streams_sql!("CAST(s.user_id AS TEXT)");

/// The samples one stored row holds, at `format_version`, decoded from
/// `source_ref` when it names one.
///
/// A row at [`STREAMS_FORMAT_VERSION`] is read as it is. A row at
/// [`GAP_FILLED_FORMAT_VERSION`] holds GPS fixes copied into the samples
/// that had none, and in the FIT path distances repeated into the samples
/// that had none, which nothing in the stored row tells apart from recorded
/// ones; such rows were written between the sample store's landing and
/// carnet#1226. One that names its source (an upload's file) is not read at
/// all, `None`: its reader decodes the source again, and the backfill
/// rewrites the row at the current version. One without a source (samples a
/// provider pushed as JSON or as a file it did not keep) cannot be decoded
/// again, so it is read without its GPS and distance channels, and every
/// other channel, which already kept its gaps, as stored.
///
/// # Errors
/// Returns a database error when the version is not one this build reads or
/// the stored JSON is not a sample set.
pub(crate) fn streams_from_row(
    json: &str,
    format_version: i16,
    source_ref: Option<&str>,
) -> AppResult<Option<TimeSeriesData>> {
    let parse = || -> AppResult<TimeSeriesData> {
        serde_json::from_str(json)
            .map_err(|e| AppError::database(format!("stored activity streams do not parse: {e}")))
    };
    match (format_version, source_ref) {
        (STREAMS_FORMAT_VERSION, _) => parse().map(Some),
        (GAP_FILLED_FORMAT_VERSION, Some(_)) => Ok(None),
        (GAP_FILLED_FORMAT_VERSION, None) => {
            let mut streams = parse()?;
            streams.gps_coordinates = None;
            streams.distance = None;
            Ok(Some(streams))
        }
        _ => Err(AppError::database(format!(
            "stored activity streams are at format version {format_version}, this build \
             reads {GAP_FILLED_FORMAT_VERSION} and {STREAMS_FORMAT_VERSION}"
        ))),
    }
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
/// its own type and its backend's statements (`GET_ACTIVITY_STREAMS_SQL`,
/// `UPSERT_ACTIVITY_STREAMS_SQL`, `DELETE_ACTIVITY_STREAMS_SQL`,
/// `DELETE_PROVIDER_ACTIVITY_STREAMS_SQL`,
/// `LIST_ACTIVITIES_WITHOUT_CURRENT_STREAMS_SQL`, or their `_PG_` forms), and sqlx resolves
/// the driver from `self.pool()` per expansion. The body names its helpers
/// and types unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_activity_stream_repository {
    ($ty:ty, $get:expr, $upsert:expr, $delete:expr, $delete_provider:expr, $unstreamed:expr) => {
        #[async_trait::async_trait]
        impl ActivityStreamRepository for $ty {
            async fn get_activity_streams(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<Option<TimeSeriesData>> {
                let row: Option<(String, i16, Option<String>)> = sqlx::query_as($get)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_activity_streams: {e}")))?;
                match row {
                    Some((json, version, source_ref)) => {
                        streams_from_row(&json, version, source_ref.as_deref())
                    }
                    None => Ok(None),
                }
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

            async fn delete_activity_streams(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<bool> {
                let removed = sqlx::query($delete)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("delete_activity_streams: {e}")))?
                    .rows_affected();
                Ok(removed > 0)
            }

            async fn delete_provider_activity_streams(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
            ) -> AppResult<u64> {
                let removed = sqlx::query($delete_provider)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("delete_provider_activity_streams: {e}"))
                    })?
                    .rows_affected();
                Ok(removed)
            }

            async fn list_activities_without_current_streams(
                &self,
                provider: &str,
                limit: i64,
            ) -> AppResult<Vec<ActivityWithoutStreams>> {
                let rows: Vec<(String, String, String)> = sqlx::query_as($unstreamed)
                    .bind(provider)
                    .bind(limit)
                    .bind(STREAMS_FORMAT_VERSION)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("list_activities_without_current_streams: {e}"))
                    })?;
                Ok(rows
                    .into_iter()
                    .map(|(tenant_id, user_id, activity_id)| ActivityWithoutStreams {
                        tenant_id,
                        user_id,
                        activity_id,
                    })
                    .collect())
            }
        }
    };
}
pub(crate) use impl_activity_stream_repository;

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample set as a gap-filled row stores it: plain fixes and distances.
    const GAP_FILLED_JSON: &str = r#"{
        "timestamps": [0, 1, 2],
        "heart_rate": [120, null, 122],
        "gps_coordinates": [[45.5, -73.6], [45.5, -73.6], [45.5002, -73.6]],
        "distance": [0.0, 0.0, 22.2]
    }"#;

    #[test]
    fn a_current_row_reads_as_stored_gaps_included() {
        let json = r#"{
            "timestamps": [0, 1, 2],
            "gps_coordinates": [[45.5, -73.6], null, [45.5002, -73.6]],
            "distance": [0.0, null, 22.2]
        }"#;
        let streams = streams_from_row(json, STREAMS_FORMAT_VERSION, None)
            .unwrap_or_else(|e| panic!("a current row parses: {e}"))
            .unwrap_or_else(|| panic!("a current row is read"));
        assert_eq!(
            streams.gps_coordinates,
            Some(vec![Some((45.5, -73.6)), None, Some((45.5002, -73.6))]),
            "a missing fix stays a gap"
        );
        assert_eq!(
            streams.distance,
            Some(vec![Some(0.0), None, Some(22.2)]),
            "a missing distance stays a gap"
        );
    }

    #[test]
    fn a_gap_filled_row_with_a_source_is_decoded_again_from_it() {
        let read = streams_from_row(GAP_FILLED_JSON, GAP_FILLED_FORMAT_VERSION, Some("ab12"))
            .unwrap_or_else(|e| panic!("a gap-filled row is a known version: {e}"));
        assert_eq!(read, None, "its reader decodes the source instead");
    }

    #[test]
    fn a_gap_filled_row_without_a_source_drops_only_its_filled_channels() {
        let streams = streams_from_row(GAP_FILLED_JSON, GAP_FILLED_FORMAT_VERSION, None)
            .unwrap_or_else(|e| panic!("a gap-filled row parses: {e}"))
            .unwrap_or_else(|| panic!("a row without a source is still read"));
        assert_eq!(
            streams.gps_coordinates, None,
            "a copied fix cannot be told from a recorded one"
        );
        assert_eq!(
            streams.distance, None,
            "a repeated distance cannot be told from a recorded one"
        );
        assert_eq!(
            streams.heart_rate,
            Some(vec![Some(120), None, Some(122)]),
            "the channels that already kept their gaps are read as stored"
        );
        assert_eq!(streams.timestamps, vec![0, 1, 2], "the time axis is kept");
    }

    #[test]
    fn a_version_this_build_does_not_know_is_refused() {
        let refused = streams_from_row(GAP_FILLED_JSON, STREAMS_FORMAT_VERSION + 1, None);
        assert!(refused.is_err(), "a later version is never misread");
        let refused = streams_from_row(GAP_FILLED_JSON, 0, Some("ab12"));
        assert!(
            refused.is_err(),
            "an unknown earlier version is never misread"
        );
    }
}
