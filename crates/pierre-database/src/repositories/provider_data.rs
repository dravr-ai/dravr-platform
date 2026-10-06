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
//!   `health_snapshots`, the `data_point_series` points and their daily
//!   `data_point_series_archive` rollups, and the `data_sources` rows naming
//!   the provider's devices;
//! - its activities: `cached_activities`, the route each one drew in
//!   `activity_route_tracks`, the link from each to the thread its view
//!   opened in `activity_conversations` (the conversation itself is the
//!   athlete's and stays), the personal bests its runs set in
//!   `personal_best_efforts`, the record of which runs were measured for one
//!   in `best_effort_scans`, and how far the walk of the provider's history
//!   for them got in `personal_best_seeds`, so a reconnect walks it again;
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
//! Every id column compared here is `TEXT` on both engines except in
//! `personal_best_efforts`, `best_effort_scans`, `personal_best_seeds`,
//! `activity_fetch_freshness`, `activity_fetch_failures` and
//! `activity_backfill_coverage` (both ids) and
//! `activity_backfill_jobs` (`tenant_id`), which are `uuid` on Postgres. Those
//! compare `CAST(column AS TEXT)` against the hyphenated id, the same shape
//! [`super::user_references`] uses, so one statement serves both engines.
//!
//! Every purge writes one `provider_data_purges` row in the same transaction:
//! who, which provider, why ([`PurgeReason`]), how many rows and when, never
//! the data itself. That is the record a provider's terms can ask Dravr to
//! attest from (Nolio API terms §7.3).
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
use serde::Serialize;
use uuid::Uuid;

/// One user's rows for one provider under one tenant, on a table whose ids
/// are `TEXT` on both engines: `$1` user id, `$2` tenant id, `$3` provider.
macro_rules! user_rows {
    () => {
        "user_id = $1 AND tenant_id = $2 AND provider = $3"
    };
}

/// [`user_rows!`] on a table whose ids are `uuid` on Postgres.
macro_rules! user_rows_cast {
    () => {
        "CAST(user_id AS TEXT) = $1 AND CAST(tenant_id AS TEXT) = $2 AND provider = $3"
    };
}

/// The data sources a user's purge reaches, for the time-series tables that
/// reference them.
macro_rules! user_sources {
    () => {
        concat!(
            "data_source_id IN (SELECT id FROM data_sources WHERE ",
            user_rows!(),
            ")"
        )
    };
}

/// Every provider row, across every user and tenant: `$1` provider.
macro_rules! provider_rows {
    () => {
        "provider = $1"
    };
}

/// The data sources a whole-provider purge reaches.
macro_rules! provider_sources {
    () => {
        "data_source_id IN (SELECT id FROM data_sources WHERE provider = $1)"
    };
}

/// The rows a user's purge deletes, in order: `(table, statement)`, each
/// statement binding `$1` user id, `$2` tenant id and `$3` provider. The sync
/// state describing them, [`USER_SYNC_STATE_SQL`], goes after.
pub(crate) const USER_PROVIDER_ROWS_SQL: [(&str, &str); 12] = [
    (
        "data_point_series",
        concat!("DELETE FROM data_point_series WHERE ", user_sources!()),
    ),
    (
        "data_point_series_archive",
        concat!(
            "DELETE FROM data_point_series_archive WHERE ",
            user_sources!()
        ),
    ),
    (
        "sleep_sessions",
        concat!("DELETE FROM sleep_sessions WHERE ", user_rows!()),
    ),
    (
        "recovery_metrics",
        concat!("DELETE FROM recovery_metrics WHERE ", user_rows!()),
    ),
    (
        "health_snapshots",
        concat!("DELETE FROM health_snapshots WHERE ", user_rows!()),
    ),
    (
        "data_sources",
        concat!("DELETE FROM data_sources WHERE ", user_rows!()),
    ),
    (
        "cached_activities",
        concat!("DELETE FROM cached_activities WHERE ", user_rows!()),
    ),
    (
        "activity_route_tracks",
        concat!("DELETE FROM activity_route_tracks WHERE ", user_rows!()),
    ),
    (
        "activity_conversations",
        concat!("DELETE FROM activity_conversations WHERE ", user_rows!()),
    ),
    (
        "personal_best_efforts",
        concat!(
            "DELETE FROM personal_best_efforts WHERE ",
            user_rows_cast!()
        ),
    ),
    (
        "best_effort_scans",
        concat!("DELETE FROM best_effort_scans WHERE ", user_rows_cast!()),
    ),
    (
        "personal_best_seeds",
        concat!("DELETE FROM personal_best_seeds WHERE ", user_rows_cast!()),
    ),
];

/// The sync state describing a user's provider rows, deleted after them by a
/// purge and by a cache expiry, binding `$1` user id, `$2` tenant id and `$3`
/// provider. Left behind, it would tell the next read that a cache emptied of
/// those rows is fresh and fully backfilled, and nothing would read them
/// again.
pub(crate) const USER_SYNC_STATE_SQL: [(&str, &str); 5] = [
    (
        "sync_state",
        concat!("DELETE FROM sync_state WHERE ", user_rows!()),
    ),
    (
        "activity_fetch_freshness",
        concat!(
            "DELETE FROM activity_fetch_freshness WHERE ",
            user_rows_cast!()
        ),
    ),
    (
        "activity_fetch_failures",
        concat!(
            "DELETE FROM activity_fetch_failures WHERE ",
            user_rows_cast!()
        ),
    ),
    (
        "activity_backfill_coverage",
        concat!(
            "DELETE FROM activity_backfill_coverage WHERE ",
            user_rows_cast!()
        ),
    ),
    (
        "activity_backfill_jobs",
        concat!(
            "DELETE FROM activity_backfill_jobs WHERE ",
            user_rows_cast!()
        ),
    ),
];

/// A user's copies of a provider's data held past a cache TTL, in order:
/// `(table, statement)`, each binding `$1` user id, `$2` tenant id, `$3`
/// provider and `$4` the cutoff instant.
///
/// A copy's age is when it was last written from the provider: `synced_at`,
/// which every writer refreshes on each re-sync, and `created_at` on a route,
/// which its upsert refreshes the same way. A point stored before
/// `data_point_series` had a sync time cannot show its age and counts as
/// expired. Rows computed from the provider's data rather than copied from
/// it (the daily `data_point_series_archive` rollups, personal bests, the
/// record of which runs were measured) are derived content, not cached copies,
/// and stay: a disconnect deletes them, and their provenance is carnet#769's.
pub(crate) const USER_EXPIRED_ROWS_SQL: [(&str, &str); 6] = [
    (
        "data_point_series",
        concat!(
            "DELETE FROM data_point_series WHERE ",
            user_sources!(),
            " AND (synced_at IS NULL OR synced_at < $4)"
        ),
    ),
    (
        "sleep_sessions",
        concat!(
            "DELETE FROM sleep_sessions WHERE ",
            user_rows!(),
            " AND synced_at < $4"
        ),
    ),
    (
        "recovery_metrics",
        concat!(
            "DELETE FROM recovery_metrics WHERE ",
            user_rows!(),
            " AND synced_at < $4"
        ),
    ),
    (
        "health_snapshots",
        concat!(
            "DELETE FROM health_snapshots WHERE ",
            user_rows!(),
            " AND synced_at < $4"
        ),
    ),
    (
        "cached_activities",
        concat!(
            "DELETE FROM cached_activities WHERE ",
            user_rows!(),
            " AND synced_at < $4"
        ),
    ),
    (
        "activity_route_tracks",
        concat!(
            "DELETE FROM activity_route_tracks WHERE ",
            user_rows!(),
            " AND created_at < $4"
        ),
    ),
];

/// Every `(user_id, tenant_id)` holding a copy of `$1`'s data written before
/// `$2`, across the tables [`USER_EXPIRED_ROWS_SQL`] expires from.
///
/// ISOLATION EXEMPTION: this read spans every tenant on purpose. It serves
/// the cache TTL sweep, a system worker that enforces a provider's retention
/// cap wherever its data is held, and returns only ids; every delete it leads
/// to is scoped to one user in one tenant.
pub(crate) const EXPIRED_SCOPES_SQL: &str = "\
    SELECT user_id, tenant_id FROM sleep_sessions WHERE provider = $1 AND synced_at < $2 \
    UNION SELECT user_id, tenant_id FROM recovery_metrics WHERE provider = $1 AND synced_at < $2 \
    UNION SELECT user_id, tenant_id FROM health_snapshots WHERE provider = $1 AND synced_at < $2 \
    UNION SELECT user_id, tenant_id FROM cached_activities WHERE provider = $1 AND synced_at < $2 \
    UNION SELECT user_id, tenant_id FROM activity_route_tracks WHERE provider = $1 AND created_at < $2 \
    UNION SELECT ds.user_id, ds.tenant_id FROM data_sources ds \
        JOIN data_point_series p ON p.data_source_id = ds.id \
        WHERE ds.provider = $1 AND (p.synced_at IS NULL OR p.synced_at < $2)";

/// The attestation row every purge writes in its own transaction: `$1` id,
/// `$2` tenant id and `$3` user id (both NULL on a whole-provider purge), `$4`
/// provider, `$5` reason, `$6` rows removed, `$7` when.
pub(crate) const INSERT_PURGE_RECORD_SQL: &str = "INSERT INTO provider_data_purges \
     (id, tenant_id, user_id, provider, reason, rows_removed, purged_at) \
     VALUES ($1, $2, $3, $4, $5, $6, $7)";

/// A provider's attestation rows, newest first: `$1` provider, `$2` limit.
///
/// ISOLATION EXEMPTION: the attestation a provider's terms ask for covers the
/// whole platform, so this read spans every tenant. It is reachable only from
/// the super-admin route, and the rows hold ids, counts and times, never
/// provider data.
pub(crate) const LIST_PURGE_RECORDS_SQL: &str = "SELECT id, tenant_id, user_id, provider, \
     reason, rows_removed, purged_at FROM provider_data_purges WHERE provider = $1 \
     ORDER BY purged_at DESC LIMIT $2";

/// What a whole-provider purge deletes, in the same order as
/// [`USER_PROVIDER_ROWS_SQL`] then [`USER_SYNC_STATE_SQL`], each statement
/// binding `$1` provider.
///
/// ISOLATION EXEMPTION: these statements carry neither `tenant_id` nor
/// `user_id` on purpose. They serve the operator's termination purge, which a
/// provider's terms can require across the whole platform (WHOOP API Terms
/// §7), and are reachable only from the super-admin route that audits the
/// call. No tenant- or user-facing path may run them.
pub(crate) const PROVIDER_PURGE_SQL: [(&str, &str); 17] = [
    (
        "data_point_series",
        concat!("DELETE FROM data_point_series WHERE ", provider_sources!()),
    ),
    (
        "data_point_series_archive",
        concat!(
            "DELETE FROM data_point_series_archive WHERE ",
            provider_sources!()
        ),
    ),
    (
        "sleep_sessions",
        concat!("DELETE FROM sleep_sessions WHERE ", provider_rows!()),
    ),
    (
        "recovery_metrics",
        concat!("DELETE FROM recovery_metrics WHERE ", provider_rows!()),
    ),
    (
        "health_snapshots",
        concat!("DELETE FROM health_snapshots WHERE ", provider_rows!()),
    ),
    (
        "data_sources",
        concat!("DELETE FROM data_sources WHERE ", provider_rows!()),
    ),
    (
        "cached_activities",
        concat!("DELETE FROM cached_activities WHERE ", provider_rows!()),
    ),
    (
        "activity_route_tracks",
        concat!("DELETE FROM activity_route_tracks WHERE ", provider_rows!()),
    ),
    (
        "activity_conversations",
        concat!(
            "DELETE FROM activity_conversations WHERE ",
            provider_rows!()
        ),
    ),
    (
        "personal_best_efforts",
        concat!("DELETE FROM personal_best_efforts WHERE ", provider_rows!()),
    ),
    (
        "best_effort_scans",
        concat!("DELETE FROM best_effort_scans WHERE ", provider_rows!()),
    ),
    (
        "personal_best_seeds",
        concat!("DELETE FROM personal_best_seeds WHERE ", provider_rows!()),
    ),
    (
        "sync_state",
        concat!("DELETE FROM sync_state WHERE ", provider_rows!()),
    ),
    (
        "activity_fetch_freshness",
        concat!(
            "DELETE FROM activity_fetch_freshness WHERE ",
            provider_rows!()
        ),
    ),
    (
        "activity_fetch_failures",
        concat!(
            "DELETE FROM activity_fetch_failures WHERE ",
            provider_rows!()
        ),
    ),
    (
        "activity_backfill_coverage",
        concat!(
            "DELETE FROM activity_backfill_coverage WHERE ",
            provider_rows!()
        ),
    ),
    (
        "activity_backfill_jobs",
        concat!(
            "DELETE FROM activity_backfill_jobs WHERE ",
            provider_rows!()
        ),
    ),
];

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
    /// The operator's whole-provider purge, on termination of access.
    Termination,
    /// The provider's cache TTL passed for those copies.
    CacheExpired,
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
            Self::Termination => "termination",
            Self::CacheExpired => "cache_expired",
        }
    }
}

/// One attestation row: that a provider's data was deleted, for whom, why,
/// how much and when. It holds no provider data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
        #[async_trait::async_trait]
        impl ProviderDataRepository for $ty {
            async fn purge_user_provider_data(
                &self,
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
                for (table, statement) in USER_PROVIDER_ROWS_SQL.iter().chain(&USER_SYNC_STATE_SQL)
                {
                    let removed = sqlx::query(statement)
                        .bind(&user)
                        .bind(&tenant)
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
