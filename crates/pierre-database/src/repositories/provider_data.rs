// ABOUTME: Deletes every row one provider contributed: for one user in one tenant, or across every tenant
// ABOUTME: One transaction per purge, each with its attestation row; the retention sweep expires wellness the same way

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider data purge, written once.
//!
//! A provider's data lands in several tables, each keyed by a `provider`
//! column (or, for the time-series points, by a data source that carries
//! one). Disconnecting a provider owes the athlete the deletion of all of it,
//! and a provider's API terms can owe the whole platform's copy on
//! termination (WHOOP API Terms §7). Both purges run the same ordered list of
//! statements inside one transaction, so a failure part-way deletes nothing:
//!
//! - the provider's measurements: `sleep_sessions`, `recovery_metrics`,
//!   `health_snapshots`, `daily_activity`, the `data_point_series` points,
//!   and the `data_sources` rows naming the provider's devices;
//! - its activities: `cached_activities`, the route each one drew in
//!   `activity_route_tracks`, the samples a provider pushed for each in
//!   `activity_streams` (its `origin = 'push'` rows only: uploaded and
//!   imported samples stay, deleted with the athlete or their account), the link from each to the thread its view
//!   opened in `activity_conversations` (the conversation itself is the
//!   athlete's and stays), the personal bests its runs set in
//!   `personal_best_efforts`, the record of which runs were measured for one
//!   in `best_effort_scans`, and how far the walk of the provider's history
//!   for them got in `personal_best_seeds`, so a reconnect walks it again;
//!   and the shoe each one was attributed to in `activity_gear` (the pairs
//!   themselves are the athlete's declaration and stay);
//! - the sync state that describes those rows: `sync_state` cursors,
//!   `activity_fetch_freshness` marks, `activity_fetch_failures` records,
//!   `activity_backfill_coverage` depth and
//!   owed `activity_backfill_jobs`. Left behind, they would tell a reconnect
//!   that an emptied cache is fresh and fully backfilled, and it would never
//!   read the history again. A history walk filed under another key (the
//!   session's) whose API phase read this provider no longer claims that
//!   phase, for the same reason: its rows went with this purge.
//!
//! The points and rollups go first because they reference `data_sources`, and
//! the health rows go before `data_sources` for the same reason.
//!
//! A narrower purge deletes one provider's wellness alone for one user in one
//! tenant ([`ProviderDataRepository::purge_user_provider_wellness`]): every
//! row the health sync wrote (the measurements above, with the data sources
//! they reference) and the sync's cursors over them, leaving the activities
//! and the sync state describing those. It serves a member withdrawing a
//! coach link's wellness consent while the link itself, and the workouts it
//! reads, stand.
//!
//! Every id column compared here is `TEXT` on both engines except in
//! `activity_gear`, `activity_streams` (`user_id`), `personal_best_efforts`,
//! `best_effort_scans`, `personal_best_seeds`,
//! `activity_fetch_freshness`, `activity_fetch_failures` and
//! `activity_backfill_coverage` (both ids) and
//! `activity_backfill_jobs` (`tenant_id`), which are `uuid` on Postgres. Those
//! compare `CAST(column AS TEXT)` against the hyphenated id, the same shape
//! [`super::user_references`] uses, so one statement serves both engines.
//!
//! Every purge writes one `provider_data_purges` row in the same transaction:
//! who, which provider, why ([`PurgeReason`]), how many rows and when, never
//! the data itself. That is the record a provider's terms can ask Dravr to
//! attest from (Nolio API terms §7.3). An account deletion runs the same
//! per-user statements inside its own transaction without that row, which
//! would name a user the same transaction deletes: it records the counts on
//! its deletion receipt instead (`super::account_deletion`).
//!
//! The one retention policy (`activity_retention.*`, carnet#1482) reaches the
//! provider's wellness through
//! [`ProviderDataRepository::expire_provider_wellness`], which deletes the
//! copies last synced before the policy's cutoff, with the health sync's
//! cursors over them, so the next sync reads them again rather than trusting
//! a cursor over an emptied history. The provider's activities expire in the
//! activity store (dravr-barrage) under the same policy.
//!
//! A caller purges through [`super::ProviderPurge`], not this repository:
//! the activity store keeps a record of each activity a live provider served,
//! and that port deletes it with the rows (carnet#1482).
//!
//! Rows derived from a provider's data without naming it are out of reach
//! here: `training_history` rollups and `user_facts` carry no provider column,
//! so a purge cannot tell which of them a provider fed (carnet#769).

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, PERMISSION_WITHDRAWN};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod statements;

pub(crate) use statements::{
    EXPIRED_WELLNESS_SCOPES_SQL, INSERT_PURGE_RECORD_SQL, LIST_PURGE_RECORDS_SQL,
    PROVIDER_PURGE_SQL, PROVIDER_WALK_PHASE_RESET_SQL, PURGE_REASONS_SINCE_SQL,
    USER_ACTIVITY_ROWS_SQL, USER_ACTIVITY_SYNC_STATE_SQL, USER_EXPIRED_ROUTE_TRACK_SQL,
    USER_EXPIRED_WELLNESS_SQL, USER_SOURCE_WELLNESS_ROWS_SQL, USER_SYNC_CURSORS_SQL,
    USER_SYNC_STATE_SQL, USER_WALK_PHASE_RESET_SQL, USER_WELLNESS_ROWS_SQL,
};

/// The statements one user's purge runs, in order.
pub(crate) struct PurgeStatements<'a> {
    /// Run first, each also binding the source marker as `$4`.
    pub marked: Option<(&'a [(&'static str, &'static str)], &'a str)>,
    /// Run next, in order.
    pub sets: &'a [&'a [(&'static str, &'static str)]],
    /// Whether the purge takes the provider's activities, and with them
    /// what a history walk's API phase brought in: the walk then no longer
    /// claims that phase ([`USER_WALK_PHASE_RESET_SQL`]).
    pub activities: bool,
}

/// What a purge deleted: rows removed per table. A table the purge found
/// nothing in is absent, so an empty map means the provider held no rows in
/// that scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProviderDataPurge {
    /// Rows removed, keyed by table name.
    pub rows_removed: BTreeMap<String, u64>,
}

impl ProviderDataPurge {
    /// Record `removed` rows from `table`; zero is not recorded.
    pub fn record(&mut self, table: &str, removed: u64) {
        if removed > 0 {
            self.rows_removed.insert(table.to_owned(), removed);
        }
    }

    /// Rows removed from `table`, zero when it held none in scope.
    #[must_use]
    pub fn removed_from(&self, table: &str) -> u64 {
        self.rows_removed.get(table).copied().unwrap_or(0)
    }

    /// Rows removed across every table.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.rows_removed.values().sum()
    }
}

/// Why a provider's data was deleted, as its attestation row records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PurgeReason {
    /// The athlete disconnected the provider.
    AthleteDisconnect,
    /// An operator disconnected it for them, or removed their account.
    OperatorDisconnect,
    /// The seat-reclaim sweeper freed an idle athlete's seat.
    SeatReclaim,
    /// The provider said the athlete withdrew the grant (a deauthorization
    /// notice), so nothing was left to revoke here.
    ProviderRevoked,
    /// The coach–athlete link the data was read through ended.
    LinkEnded,
    /// The member withdrew the wellness consent on the coach–athlete link
    /// their wellness was read through; the link itself stands.
    WellnessConsentWithdrawn,
    /// The athlete stopped sharing that kind of data with Dravr at the
    /// provider while the connection stands (Garmin's
    /// `userPermissionsChange`).
    PermissionWithdrawn,
    /// The operator's whole-provider purge, on termination of access.
    Termination,
    /// The retention policy's days passed since those copies were last read
    /// from the provider (`activity_retention.*`).
    CacheExpired,
    /// The athlete deleted a history they imported from their own export.
    AthleteDeletedImport,
    /// The activities were another athlete's, filed under the user's account
    /// by a read that misread whose they were (a `TrainingPeaks` coach
    /// account read before sciotte 0.14).
    MisfiledActivities,
}

impl PurgeReason {
    /// The `reason` value stored on the attestation row.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AthleteDisconnect => "athlete_disconnect",
            Self::OperatorDisconnect => "operator_disconnect",
            Self::SeatReclaim => "seat_reclaim",
            Self::ProviderRevoked => "provider_revoked",
            Self::LinkEnded => "link_ended",
            Self::WellnessConsentWithdrawn => "wellness_consent_withdrawn",
            Self::PermissionWithdrawn => PERMISSION_WITHDRAWN,
            Self::Termination => "termination",
            Self::CacheExpired => "cache_expired",
            Self::AthleteDeletedImport => "athlete_deleted_import",
            Self::MisfiledActivities => "misfiled_activities",
        }
    }
}

/// One attestation row: that a provider's data was deleted, for whom, why,
/// how much and when. It holds no provider data.
///
/// `pierre-cli provider purges` reads it back from the admin route in this
/// same shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDataPurgeRecord {
    /// Row id.
    pub id: String,
    /// The tenant purged, `None` for a whole-provider purge.
    pub tenant_id: Option<String>,
    /// The user purged, `None` for a whole-provider purge.
    pub user_id: Option<String>,
    /// The provider whose data was deleted.
    pub provider: String,
    /// Why, as [`PurgeReason::as_str`] wrote it.
    pub reason: String,
    /// Rows deleted across every table.
    pub rows_removed: i64,
    /// When the purge committed.
    pub purged_at: DateTime<Utc>,
}

/// What one retention pass over a provider's wellness expired.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProviderWellnessExpiry {
    /// How many `(user, tenant)` scopes held an expired copy.
    pub scopes: u64,
    /// Rows expired per table across those scopes, the sync's cursors
    /// included.
    pub evicted: ProviderDataPurge,
}

/// Deletes the rows a provider contributed.
#[async_trait]
pub trait ProviderDataRepository: Send + Sync {
    /// Delete every row `provider` contributed for `user_id` under
    /// `tenant_id`, in one transaction that also writes the attestation row
    /// naming `reason`.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// and nothing recorded then.
    async fn purge_user_provider_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete the wellness `provider` contributed for `user_id` under
    /// `tenant_id` (its time-series points and their rollups, its sleep,
    /// recovery and health rows, the data sources they reference, and the
    /// sync's cursors over them) and nothing else, in one transaction that
    /// also writes the attestation row naming `reason`. The provider's
    /// activities, and the sync state describing them, stay.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// and nothing recorded then.
    async fn purge_user_provider_wellness(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete the activities `provider` contributed for `user_id` under
    /// `tenant_id` (the cached rows and everything kept beside them: routes,
    /// samples, conversations, best efforts, gear) and the sync state
    /// describing them, and nothing else, in one transaction that also writes
    /// the attestation row naming `reason`. The provider's wellness, and the
    /// health sync's cursors over it, stay.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// and nothing recorded then.
    async fn purge_user_provider_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete the wellness `provider` contributed for `user_id` under
    /// `tenant_id` through its data sources marked `source` (their series
    /// points and rollups, the sleep, recovery and health rows filed under
    /// them, and the sources themselves), in one transaction that also
    /// writes the attestation row naming `reason`. Wellness filed under the
    /// provider's other data sources, the health sync's cursors and the
    /// provider's activities stay.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// and nothing recorded then.
    async fn purge_user_provider_source_wellness(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete what one connection under `provider` contributed for
    /// `user_id` under `tenant_id` when another connection shares that
    /// provider key for its wellness: the provider's activities with the
    /// sync state describing them, as
    /// [`Self::purge_user_provider_activities`] deletes them, and the
    /// wellness filed under its data sources marked `source`, as
    /// [`Self::purge_user_provider_source_wellness`] deletes it, in one
    /// transaction that also writes the attestation row naming `reason`.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// and nothing recorded then.
    async fn purge_user_provider_source_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete every row `provider` contributed, for every user in every
    /// tenant, in one transaction that also writes its attestation row: the
    /// operator's termination purge.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// then.
    async fn purge_provider_data(&self, provider: &str) -> AppResult<ProviderDataPurge>;

    /// Expire every wellness copy of `provider`'s data last synced before
    /// `cutoff`, scope by scope: in each `(user, tenant)` holding one, the
    /// expired rows and the health sync's cursors over that provider go in
    /// one transaction with its `cache_expired` attestation row, so the next
    /// sync reads what expired again instead of trusting a cursor over an
    /// emptied history. Activities are not touched: the activity store
    /// expires them.
    ///
    /// # Errors
    /// Returns a database error when the scopes cannot be read or a scope's
    /// expiry fails; scopes already expired stay expired.
    async fn expire_provider_wellness(
        &self,
        provider: &str,
        cutoff: DateTime<Utc>,
    ) -> AppResult<ProviderWellnessExpiry>;

    /// Settle the retention expiry of `activity_ids`, the user's `provider`
    /// activities the activity store (dravr-barrage) expired and stopped
    /// serving: each one's route read goes, the sync state describing the
    /// provider's activities is reset (fetch freshness, fetch failures,
    /// backfill coverage and jobs), and one `cache_expired` attestation row
    /// counts the expired activities, in one transaction. Left behind, the
    /// sync state would tell the next read that a window emptied by the
    /// expiry is fresh and fully backfilled, and nothing would read it again.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// and nothing recorded then.
    async fn expire_provider_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        activity_ids: &[&str],
    ) -> AppResult<ProviderDataPurge>;

    /// The newest `limit` attestation rows for `provider`, across every
    /// tenant.
    ///
    /// # Errors
    /// Returns a database error when the rows cannot be read or decoded.
    async fn list_provider_data_purges(
        &self,
        provider: &str,
        limit: i64,
    ) -> AppResult<Vec<ProviderDataPurgeRecord>>;

    /// The reasons ([`PurgeReason::as_str`]) of every purge of `provider`'s
    /// data that reached `user_id` in `tenant_id` — theirs, or the
    /// operator's whole-provider purge — committed at or after `since`, read
    /// from the attestation rows.
    ///
    /// # Errors
    /// Returns a database error when the rows cannot be read.
    async fn purge_reasons_since(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        since: DateTime<Utc>,
    ) -> AppResult<Vec<String>>;
}

/// Decode one `provider_data_purges` row.
///
/// # Errors
/// Returns a database error when a column is missing or does not decode.
pub(crate) fn purge_record_from_row<R>(row: &R) -> AppResult<ProviderDataPurgeRecord>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |name: &str, e: sqlx::Error| {
        AppError::database(format!("provider_data_purges col {name}: {e}"))
    };
    Ok(ProviderDataPurgeRecord {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column("tenant_id", e))?,
        user_id: row.try_get("user_id").map_err(|e| column("user_id", e))?,
        provider: row.try_get("provider").map_err(|e| column("provider", e))?,
        reason: row.try_get("reason").map_err(|e| column("reason", e))?,
        rows_removed: row
            .try_get("rows_removed")
            .map_err(|e| column("rows_removed", e))?,
        purged_at: row
            .try_get("purged_at")
            .map_err(|e| column("purged_at", e))?,
    })
}

/// Write one attestation row on `$tx`, mapping a failure to a database error.
///
/// The invoking shell must `use` [`INSERT_PURGE_RECORD_SQL`], `AppError`,
/// `Utc` and `Uuid`.
macro_rules! insert_purge_record {
    ($tx:expr, $tenant:expr, $user:expr, $provider:expr, $reason:expr, $removed:expr) => {
        sqlx::query(INSERT_PURGE_RECORD_SQL)
            .bind(Uuid::new_v4().to_string())
            .bind($tenant)
            .bind($user)
            .bind($provider)
            .bind($reason.as_str())
            .bind(i64::try_from($removed).unwrap_or(i64::MAX))
            .bind(Utc::now())
            .execute(&mut *$tx)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to record the provider data purge: {e}"))
            })
    };
}
pub(crate) use insert_purge_record;

/// Emit the [`ProviderDataRepository`] implementation for one backend type.
///
/// The statements are identical on both engines, so the shell passes only its
/// type; sqlx resolves the driver from `self.pool()` per expansion. The body
/// names its consts, types and helpers unqualified, so the invoking shell must
/// `use` every one of them.
macro_rules! impl_provider_data_repository {
    ($ty:ty) => {
        impl $ty {
            /// Run each of `statements`, in order, over one user's `provider`
            /// rows under one tenant, in one transaction that also writes the
            /// attestation row naming `reason`.
            async fn purge_user_rows(
                &self,
                statements: $crate::repositories::provider_data::PurgeStatements<'_>,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                reason: PurgeReason,
            ) -> AppResult<ProviderDataPurge> {
                let user = user_id.to_string();
                let tenant = tenant_id.to_string();
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin the provider data purge: {e}"))
                })?;
                let mut purge = ProviderDataPurge::default();
                let mut ordered: Vec<(&'static str, &'static str, Option<&str>)> = Vec::new();
                if let Some((set, source)) = statements.marked {
                    ordered.extend(set.iter().map(|&(table, sql)| (table, sql, Some(source))));
                }
                for set in statements.sets {
                    ordered.extend(set.iter().map(|&(table, sql)| (table, sql, None)));
                }
                for (table, statement, source) in ordered {
                    let mut query = sqlx::query(statement)
                        .bind(&user)
                        .bind(&tenant)
                        .bind(provider);
                    if let Some(source) = source {
                        query = query.bind(source);
                    }
                    let removed = query
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to purge {provider} rows from {table}: {e}"
                            ))
                        })?
                        .rows_affected();
                    purge.record(table, removed);
                }
                if statements.activities {
                    sqlx::query($crate::repositories::provider_data::USER_WALK_PHASE_RESET_SQL)
                        .bind(&user)
                        .bind(&tenant)
                        .bind(provider)
                        .bind($crate::repositories::BackfillJobKind::HistoryWalk.as_str())
                        .bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to release the history walks that read {provider}: {e}"
                            ))
                        })?;
                }
                insert_purge_record!(
                    tx,
                    Some(&tenant),
                    Some(&user),
                    provider,
                    reason,
                    purge.total()
                )?;
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to commit the provider data purge: {e}"))
                })?;
                Ok(purge)
            }
        }

        #[async_trait::async_trait]
        impl ProviderDataRepository for $ty {
            async fn purge_user_provider_data(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                reason: PurgeReason,
            ) -> AppResult<ProviderDataPurge> {
                self.purge_user_rows(
                    $crate::repositories::provider_data::PurgeStatements {
                        marked: None,
                        sets: &[
                            &USER_WELLNESS_ROWS_SQL,
                            &USER_ACTIVITY_ROWS_SQL,
                            &USER_SYNC_STATE_SQL,
                        ],
                        activities: true,
                    },
                    user_id,
                    tenant_id,
                    provider,
                    reason,
                )
                .await
            }

            async fn purge_user_provider_wellness(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                reason: PurgeReason,
            ) -> AppResult<ProviderDataPurge> {
                self.purge_user_rows(
                    $crate::repositories::provider_data::PurgeStatements {
                        marked: None,
                        sets: &[&USER_WELLNESS_ROWS_SQL, &[USER_SYNC_CURSORS_SQL]],
                        activities: false,
                    },
                    user_id,
                    tenant_id,
                    provider,
                    reason,
                )
                .await
            }

            async fn purge_user_provider_activities(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                reason: PurgeReason,
            ) -> AppResult<ProviderDataPurge> {
                self.purge_user_rows(
                    $crate::repositories::provider_data::PurgeStatements {
                        marked: None,
                        sets: &[&USER_ACTIVITY_ROWS_SQL, &USER_ACTIVITY_SYNC_STATE_SQL],
                        activities: true,
                    },
                    user_id,
                    tenant_id,
                    provider,
                    reason,
                )
                .await
            }

            async fn purge_user_provider_source_wellness(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                source: &str,
                reason: PurgeReason,
            ) -> AppResult<ProviderDataPurge> {
                self.purge_user_rows(
                    $crate::repositories::provider_data::PurgeStatements {
                        marked: Some((&USER_SOURCE_WELLNESS_ROWS_SQL, source)),
                        sets: &[],
                        activities: false,
                    },
                    user_id,
                    tenant_id,
                    provider,
                    reason,
                )
                .await
            }

            async fn purge_user_provider_source_data(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                source: &str,
                reason: PurgeReason,
            ) -> AppResult<ProviderDataPurge> {
                self.purge_user_rows(
                    $crate::repositories::provider_data::PurgeStatements {
                        marked: Some((&USER_SOURCE_WELLNESS_ROWS_SQL, source)),
                        sets: &[&USER_ACTIVITY_ROWS_SQL, &USER_ACTIVITY_SYNC_STATE_SQL],
                        activities: true,
                    },
                    user_id,
                    tenant_id,
                    provider,
                    reason,
                )
                .await
            }

            async fn purge_provider_data(&self, provider: &str) -> AppResult<ProviderDataPurge> {
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin the provider data purge: {e}"))
                })?;
                let mut purge = ProviderDataPurge::default();
                for (table, statement) in PROVIDER_PURGE_SQL {
                    let removed = sqlx::query(statement)
                        .bind(provider)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to purge {provider} rows from {table}: {e}"
                            ))
                        })?
                        .rows_affected();
                    purge.record(table, removed);
                }
                sqlx::query($crate::repositories::provider_data::PROVIDER_WALK_PHASE_RESET_SQL)
                    .bind(provider)
                    .bind($crate::repositories::BackfillJobKind::HistoryWalk.as_str())
                    .bind($crate::repositories::activity_backfill_jobs::walked_outcome())
                    .bind($crate::repositories::activity_backfill_jobs::phase_purged_outcome())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to release the history walks that read {provider}: {e}"
                        ))
                    })?;
                insert_purge_record!(
                    tx,
                    None::<String>,
                    None::<String>,
                    provider,
                    PurgeReason::Termination,
                    purge.total()
                )?;
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to commit the provider data purge: {e}"))
                })?;
                Ok(purge)
            }

            async fn expire_provider_wellness(
                &self,
                provider: &str,
                cutoff: DateTime<Utc>,
            ) -> AppResult<ProviderWellnessExpiry> {
                let scopes = sqlx::query(EXPIRED_WELLNESS_SCOPES_SQL)
                    .bind(provider)
                    .bind(cutoff)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to find expired {provider} wellness copies: {e}"
                        ))
                    })?;
                let mut expiry = ProviderWellnessExpiry::default();
                for row in scopes {
                    let user: String = sqlx::Row::try_get(&row, "user_id").map_err(|e| {
                        AppError::database(format!("expired scope col user_id: {e}"))
                    })?;
                    let tenant: String = sqlx::Row::try_get(&row, "tenant_id").map_err(|e| {
                        AppError::database(format!("expired scope col tenant_id: {e}"))
                    })?;
                    let mut tx = self.pool().begin().await.map_err(|e| {
                        AppError::database(format!("Failed to begin the wellness expiry: {e}"))
                    })?;
                    let mut evicted = ProviderDataPurge::default();
                    for (table, statement) in USER_EXPIRED_WELLNESS_SQL {
                        let removed = sqlx::query(statement)
                            .bind(&user)
                            .bind(&tenant)
                            .bind(provider)
                            .bind(cutoff)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| {
                                AppError::database(format!(
                                    "Failed to expire {provider} rows from {table}: {e}"
                                ))
                            })?
                            .rows_affected();
                        evicted.record(table, removed);
                    }
                    let (table, statement) = USER_SYNC_CURSORS_SQL;
                    let removed = sqlx::query(statement)
                        .bind(&user)
                        .bind(&tenant)
                        .bind(provider)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to reset {provider} sync cursors in {table}: {e}"
                            ))
                        })?
                        .rows_affected();
                    evicted.record(table, removed);
                    insert_purge_record!(
                        tx,
                        Some(&tenant),
                        Some(&user),
                        provider,
                        PurgeReason::CacheExpired,
                        evicted.total()
                    )?;
                    tx.commit().await.map_err(|e| {
                        AppError::database(format!("Failed to commit the wellness expiry: {e}"))
                    })?;
                    expiry.scopes += 1;
                    for (table, removed) in evicted.rows_removed {
                        *expiry.evicted.rows_removed.entry(table).or_insert(0) += removed;
                    }
                }
                Ok(expiry)
            }

            async fn expire_provider_activities(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                activity_ids: &[&str],
            ) -> AppResult<ProviderDataPurge> {
                let user = user_id.to_string();
                let tenant = tenant_id.to_string();
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin the activity expiry: {e}"))
                })?;
                let mut expired = ProviderDataPurge::default();
                let (table, statement) = USER_EXPIRED_ROUTE_TRACK_SQL;
                let mut routes = 0_u64;
                for activity_id in activity_ids {
                    routes += sqlx::query(statement)
                        .bind(&user)
                        .bind(&tenant)
                        .bind(provider)
                        .bind(*activity_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to expire {provider} rows from {table}: {e}"
                            ))
                        })?
                        .rows_affected();
                }
                expired.record(table, routes);
                for (table, statement) in USER_ACTIVITY_SYNC_STATE_SQL {
                    let removed = sqlx::query(statement)
                        .bind(&user)
                        .bind(&tenant)
                        .bind(provider)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to reset {provider} sync state in {table}: {e}"
                            ))
                        })?
                        .rows_affected();
                    expired.record(table, removed);
                }
                insert_purge_record!(
                    tx,
                    Some(&tenant),
                    Some(&user),
                    provider,
                    PurgeReason::CacheExpired,
                    u64::try_from(activity_ids.len()).unwrap_or(u64::MAX)
                )?;
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to commit the activity expiry: {e}"))
                })?;
                Ok(expired)
            }

            async fn list_provider_data_purges(
                &self,
                provider: &str,
                limit: i64,
            ) -> AppResult<Vec<ProviderDataPurgeRecord>> {
                sqlx::query(LIST_PURGE_RECORDS_SQL)
                    .bind(provider)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list {provider} purge records: {e}"))
                    })?
                    .iter()
                    .map(purge_record_from_row)
                    .collect()
            }

            async fn purge_reasons_since(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                since: DateTime<Utc>,
            ) -> AppResult<Vec<String>> {
                sqlx::query_scalar($crate::repositories::provider_data::PURGE_REASONS_SINCE_SQL)
                    .bind(provider)
                    .bind(since)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the {provider} purges: {e}"))
                    })
            }
        }
    };
}
pub(crate) use impl_provider_data_repository;
