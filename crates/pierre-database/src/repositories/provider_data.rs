// ABOUTME: Deletes every row one provider contributed: for one user in one tenant, or across every tenant
// ABOUTME: One transaction per purge over the health, time-series, activity-cache and sync-state tables

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
//! - its activities: `cached_activities`, and the route each one drew in
//!   `activity_route_tracks`;
//! - the sync state that describes those rows: `sync_state` cursors,
//!   `activity_fetch_freshness` marks, `activity_backfill_coverage` depth and
//!   owed `activity_backfill_jobs`. Left behind, they would tell a reconnect
//!   that an emptied cache is fresh and fully backfilled, and it would never
//!   read the history again.
//!
//! The points and rollups go first because they reference `data_sources`, and
//! the health rows go before `data_sources` for the same reason.
//!
//! Every id column compared here is `TEXT` on both engines except in
//! `activity_fetch_freshness` and `activity_backfill_coverage` (both ids) and
//! `activity_backfill_jobs` (`tenant_id`), which are `uuid` on Postgres. Those
//! compare `CAST(column AS TEXT)` against the hyphenated id, the same shape
//! [`super::user_references`] uses, so one statement serves both engines.
//!
//! Rows derived from a provider's data without naming it are out of reach
//! here: `training_history` rollups and `user_facts` carry no provider column,
//! so a purge cannot tell which of them a provider fed.

use std::collections::BTreeMap;

use async_trait::async_trait;
use pierre_core::errors::AppResult;
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

/// What a user's disconnect deletes, in order: `(table, statement)`, each
/// statement binding `$1` user id, `$2` tenant id and `$3` provider.
pub(crate) const USER_PROVIDER_PURGE_SQL: [(&str, &str); 12] = [
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

/// What a whole-provider purge deletes, in the same order as
/// [`USER_PROVIDER_PURGE_SQL`], each statement binding `$1` provider.
///
/// ISOLATION EXEMPTION: these statements carry neither `tenant_id` nor
/// `user_id` on purpose. They serve the operator's termination purge, which a
/// provider's terms can require across the whole platform (WHOOP API Terms
/// §7), and are reachable only from the super-admin route that audits the
/// call. No tenant- or user-facing path may run them.
pub(crate) const PROVIDER_PURGE_SQL: [(&str, &str); 12] = [
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

/// Deletes the rows a provider contributed.
#[async_trait]
pub trait ProviderDataRepository: Send + Sync {
    /// Delete every row `provider` contributed for `user_id` under
    /// `tenant_id`, in one transaction: the disconnect purge.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// then.
    async fn purge_user_provider_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<ProviderDataPurge>;

    /// Delete every row `provider` contributed, for every user in every
    /// tenant, in one transaction: the operator's termination purge.
    ///
    /// # Errors
    /// Returns a database error when any statement fails; nothing is deleted
    /// then.
    async fn purge_provider_data(&self, provider: &str) -> AppResult<ProviderDataPurge>;
}

/// Emit the [`ProviderDataRepository`] implementation for one backend type.
///
/// The statements are identical on both engines, so the shell passes only its
/// type; sqlx resolves the driver from `self.pool()` per expansion. The body
/// names its consts and types unqualified, so the invoking shell must `use`
/// every one of them.
macro_rules! impl_provider_data_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl ProviderDataRepository for $ty {
            async fn purge_user_provider_data(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
            ) -> AppResult<ProviderDataPurge> {
                let user = user_id.to_string();
                let tenant = tenant_id.to_string();
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to begin the provider data purge: {e}"))
                })?;
                let mut purge = ProviderDataPurge::default();
                for (table, statement) in USER_PROVIDER_PURGE_SQL {
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
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to commit the provider data purge: {e}"))
                })?;
                Ok(purge)
            }
        }
    };
}
pub(crate) use impl_provider_data_repository;
