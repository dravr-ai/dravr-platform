// ABOUTME: Deletes every row one provider contributed: for one user in one tenant, or across every tenant
// ABOUTME: One transaction per purge, each with its attestation row; a cache TTL evicts expired copies the same way

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
//!   `health_snapshots`, `daily_activity`, the `data_point_series` points and
//!   their daily `data_point_series_archive` rollups, and the `data_sources`
//!   rows naming the provider's devices;
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
//!   read the history again.
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
//! A provider whose terms cap how long a copy may be held declares a cache
//! TTL on its descriptor, and [`ProviderDataRepository::expire_provider_cache`]
//! evicts the copies written before it, with the sync state that described
//! them, so the next read fetches them again rather than trusting a coverage
//! mark over an emptied cache.
//!
//! Rows derived from a provider's data without naming it are out of reach
//! here: `training_history` rollups and `user_facts` carry no provider column,
//! so a purge cannot tell which of them a provider fed (carnet#769).

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod statements;

pub(crate) use statements::{
    EXPIRED_SCOPES_SQL, INSERT_PURGE_RECORD_SQL, LIST_PURGE_RECORDS_SQL, PROVIDER_PURGE_SQL,
    USER_ACTIVITY_ROWS_SQL, USER_ACTIVITY_SYNC_STATE_SQL, USER_EXPIRED_ROWS_SQL,
    USER_SOURCE_WELLNESS_ROWS_SQL, USER_SYNC_CURSORS_SQL, USER_SYNC_STATE_SQL,
    USER_WELLNESS_ROWS_SQL,
};

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
    /// The provider's cache TTL passed for those copies.
    CacheExpired,
    /// The athlete deleted a history they imported from their own export.
    AthleteDeletedImport,
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
            Self::PermissionWithdrawn => "permission_withdrawn",
            Self::Termination => "termination",
            Self::CacheExpired => "cache_expired",
            Self::AthleteDeletedImport => "athlete_deleted_import",
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

/// What one cache TTL sweep of a provider evicted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProviderCacheExpiry {
    /// How many `(user, tenant)` scopes held an expired copy.
    pub scopes: u64,
    /// Rows evicted per table across those scopes, sync state included.
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

    /// Evict every copy of `provider`'s data written before `cutoff`, scope
    /// by scope: in each `(user, tenant)` holding one, the expired rows and
    /// the sync state describing that provider go in one transaction with
    /// its attestation row, so the next read fetches what was evicted again
    /// instead of trusting a coverage mark over an emptied cache.
    ///
    /// # Errors
    /// Returns a database error when the scopes cannot be read or a scope's
    /// eviction fails; scopes already evicted stay evicted.
    async fn expire_provider_cache(
        &self,
        provider: &str,
        cutoff: DateTime<Utc>,
    ) -> AppResult<ProviderCacheExpiry>;

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
            /// attestation row naming `reason`. `marked` statements run
            /// first, each also binding the source marker as `$4`.
            async fn purge_user_rows(
                &self,
                marked: Option<(&[(&'static str, &'static str)], &str)>,
                statements: &[&[(&'static str, &'static str)]],
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
                if let Some((set, source)) = marked {
                    ordered.extend(set.iter().map(|&(table, sql)| (table, sql, Some(source))));
                }
                for set in statements {
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
                    None,
                    &[
                        &USER_WELLNESS_ROWS_SQL,
                        &USER_ACTIVITY_ROWS_SQL,
                        &USER_SYNC_STATE_SQL,
                    ],
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
                    None,
                    &[&USER_WELLNESS_ROWS_SQL, &[USER_SYNC_CURSORS_SQL]],
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
                    None,
                    &[&USER_ACTIVITY_ROWS_SQL, &USER_ACTIVITY_SYNC_STATE_SQL],
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
                    Some((&USER_SOURCE_WELLNESS_ROWS_SQL, source)),
                    &[],
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
                    Some((&USER_SOURCE_WELLNESS_ROWS_SQL, source)),
                    &[&USER_ACTIVITY_ROWS_SQL, &USER_ACTIVITY_SYNC_STATE_SQL],
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

            async fn expire_provider_cache(
                &self,
                provider: &str,
                cutoff: DateTime<Utc>,
            ) -> AppResult<ProviderCacheExpiry> {
                let scopes = sqlx::query(EXPIRED_SCOPES_SQL)
                    .bind(provider)
                    .bind(cutoff)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to find expired {provider} cache copies: {e}"
                        ))
                    })?;
                let mut expiry = ProviderCacheExpiry::default();
                for row in scopes {
                    let user: String = sqlx::Row::try_get(&row, "user_id").map_err(|e| {
                        AppError::database(format!("expired scope col user_id: {e}"))
                    })?;
                    let tenant: String = sqlx::Row::try_get(&row, "tenant_id").map_err(|e| {
                        AppError::database(format!("expired scope col tenant_id: {e}"))
                    })?;
                    let mut tx = self.pool().begin().await.map_err(|e| {
                        AppError::database(format!("Failed to begin the cache expiry: {e}"))
                    })?;
                    let mut evicted = ProviderDataPurge::default();
                    for (table, statement) in USER_EXPIRED_ROWS_SQL {
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
                    for (table, statement) in USER_SYNC_STATE_SQL {
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
                        evicted.record(table, removed);
                    }
                    insert_purge_record!(
                        tx,
                        Some(&tenant),
                        Some(&user),
                        provider,
                        PurgeReason::CacheExpired,
                        evicted.total()
                    )?;
                    tx.commit().await.map_err(|e| {
                        AppError::database(format!("Failed to commit the cache expiry: {e}"))
                    })?;
                    expiry.scopes += 1;
                    for (table, removed) in evicted.rows_removed {
                        *expiry.evicted.rows_removed.entry(table).or_insert(0) += removed;
                    }
                }
                Ok(expiry)
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
        }
    };
}
pub(crate) use impl_provider_data_repository;
