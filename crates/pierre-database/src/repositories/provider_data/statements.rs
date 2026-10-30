// ABOUTME: The ordered DELETE statements every provider-data purge and cache expiry runs
// ABOUTME: One table per entry, children before the data sources they reference; both engines share them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The statements of [`super`]'s purges, as data: which table each deletes
//! from and under which scope. See the parent module for what each purge
//! owes and why the order matters.

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

/// One user's rows for one provider on a table the person owns whichever
/// tenant they are signed in to, whose `user_id` is `uuid` on Postgres: `$1`
/// user id and `$3` provider. `$2` is bound with every other statement's and
/// read by none here: the gear ledger has no tenant to narrow it to.
macro_rules! person_rows_cast {
    () => {
        "CAST(user_id AS TEXT) = $1 AND provider = $3"
    };
}

/// The `activity_streams` rows a provider purge reaches: only those the
/// provider pushed. Uploaded and imported samples are the athlete's own
/// record, deleted with the athlete or their account, never with a provider.
macro_rules! pushed_streams {
    () => {
        "origin = 'push'"
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

/// A user's wellness rows for one provider, in order: `(table, statement)`,
/// each binding `$1` user id, `$2` tenant id and `$3` provider. They are
/// everything the health sync writes: the time-series points and their daily
/// rollups, the sleep, recovery and health rows, then the data sources all of
/// them reference. A user's purge runs them first, then
/// [`USER_ACTIVITY_ROWS_SQL`]; a wellness purge runs them alone, with the
/// sync's cursors ([`USER_SYNC_CURSORS_SQL`]).
pub(crate) const USER_WELLNESS_ROWS_SQL: [(&str, &str); 7] = [
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
        "daily_activity",
        concat!("DELETE FROM daily_activity WHERE ", user_rows!()),
    ),
    (
        "data_sources",
        concat!("DELETE FROM data_sources WHERE ", user_rows!()),
    ),
];

/// The data sources of one provider a user's source-scoped purge reaches:
/// those whose `source` marker is `$4`.
macro_rules! user_marked_sources {
    () => {
        concat!(
            "data_source_id IN (SELECT id FROM data_sources WHERE ",
            user_rows!(),
            " AND source = $4)"
        )
    };
}

/// A user's wellness rows for one provider filed under its data sources
/// marked `$4`, in order, binding `$1` user id, `$2` tenant id, `$3`
/// provider and `$4` the source marker: [`USER_WELLNESS_ROWS_SQL`] narrowed
/// to one connection when two connections store wellness under the same
/// provider key (the Garmin API and the connection using usr/pwd from Dravr
/// both under `garmin`). The rows the other connection filed under its own
/// data source, and the health sync's cursors over them, stay.
pub(crate) const USER_SOURCE_WELLNESS_ROWS_SQL: [(&str, &str); 7] = [
    (
        "data_point_series",
        concat!(
            "DELETE FROM data_point_series WHERE ",
            user_marked_sources!()
        ),
    ),
    (
        "data_point_series_archive",
        concat!(
            "DELETE FROM data_point_series_archive WHERE ",
            user_marked_sources!()
        ),
    ),
    (
        "sleep_sessions",
        concat!(
            "DELETE FROM sleep_sessions WHERE ",
            user_rows!(),
            " AND ",
            user_marked_sources!()
        ),
    ),
    (
        "recovery_metrics",
        concat!(
            "DELETE FROM recovery_metrics WHERE ",
            user_rows!(),
            " AND ",
            user_marked_sources!()
        ),
    ),
    (
        "health_snapshots",
        concat!(
            "DELETE FROM health_snapshots WHERE ",
            user_rows!(),
            " AND ",
            user_marked_sources!()
        ),
    ),
    (
        "daily_activity",
        concat!(
            "DELETE FROM daily_activity WHERE ",
            user_rows!(),
            " AND ",
            user_marked_sources!()
        ),
    ),
    (
        "data_sources",
        concat!(
            "DELETE FROM data_sources WHERE ",
            user_rows!(),
            " AND source = $4"
        ),
    ),
];

/// A user's activity rows for one provider, which a user's purge deletes
/// after [`USER_WELLNESS_ROWS_SQL`], in order: `(table, statement)`, each
/// binding `$1` user id, `$2` tenant id and `$3` provider. The sync state
/// describing them, [`USER_SYNC_STATE_SQL`], goes after.
pub(crate) const USER_ACTIVITY_ROWS_SQL: [(&str, &str); 9] = [
    (
        "cached_activities",
        concat!("DELETE FROM cached_activities WHERE ", user_rows!()),
    ),
    (
        "activity_route_tracks",
        concat!("DELETE FROM activity_route_tracks WHERE ", user_rows!()),
    ),
    (
        "activity_streams",
        concat!(
            "DELETE FROM activity_streams WHERE ",
            user_rows_cast!(),
            " AND ",
            pushed_streams!()
        ),
    ),
    (
        "pushed_activity_files",
        concat!("DELETE FROM pushed_activity_files WHERE ", user_rows!()),
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
    (
        "activity_gear",
        concat!("DELETE FROM activity_gear WHERE ", person_rows_cast!()),
    ),
];

/// The health sync's cursors over a user's provider rows, binding `$1` user
/// id, `$2` tenant id and `$3` provider. Every `sync_state` row is one of
/// them (the sync writes no other), whichever data type it walks. Left behind
/// by a wellness purge, a cursor would tell a later sync that the emptied
/// history was already read.
pub(crate) const USER_SYNC_CURSORS_SQL: (&str, &str) = (
    "sync_state",
    concat!("DELETE FROM sync_state WHERE ", user_rows!()),
);

/// The sync state describing a user's provider rows, deleted after them by a
/// purge and by a cache expiry, binding `$1` user id, `$2` tenant id and `$3`
/// provider. Left behind, it would tell the next read that a cache emptied of
/// those rows is fresh and fully backfilled, and nothing would read them
/// again.
pub(crate) const USER_SYNC_STATE_SQL: [(&str, &str); 5] = [
    USER_SYNC_CURSORS_SQL,
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

/// The sync state describing a user's provider activities alone: every
/// [`USER_SYNC_STATE_SQL`] statement but the health sync's cursors, which an
/// activity purge leaves to the wellness it walks.
pub(crate) const USER_ACTIVITY_SYNC_STATE_SQL: [(&str, &str); 4] = [
    USER_SYNC_STATE_SQL[1],
    USER_SYNC_STATE_SQL[2],
    USER_SYNC_STATE_SQL[3],
    USER_SYNC_STATE_SQL[4],
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
pub(crate) const USER_EXPIRED_ROWS_SQL: [(&str, &str); 8] = [
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
        "daily_activity",
        concat!(
            "DELETE FROM daily_activity WHERE ",
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
    (
        "activity_streams",
        concat!(
            "DELETE FROM activity_streams WHERE ",
            user_rows_cast!(),
            " AND ",
            pushed_streams!(),
            " AND synced_at < $4"
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
    UNION SELECT user_id, tenant_id FROM daily_activity WHERE provider = $1 AND synced_at < $2 \
    UNION SELECT user_id, tenant_id FROM cached_activities WHERE provider = $1 AND synced_at < $2 \
    UNION SELECT user_id, tenant_id FROM activity_route_tracks WHERE provider = $1 AND created_at < $2 \
    UNION SELECT CAST(user_id AS TEXT), tenant_id FROM activity_streams \
        WHERE provider = $1 AND origin = 'push' AND synced_at < $2 \
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
/// [`USER_WELLNESS_ROWS_SQL`], [`USER_ACTIVITY_ROWS_SQL`] then
/// [`USER_SYNC_STATE_SQL`], each statement binding `$1` provider.
///
/// ISOLATION EXEMPTION: these statements carry neither `tenant_id` nor
/// `user_id` on purpose. They serve the operator's termination purge, which a
/// provider's terms can require across the whole platform (WHOOP API Terms
/// §7), and are reachable only from the super-admin route that audits the
/// call. No tenant- or user-facing path may run them.
pub(crate) const PROVIDER_PURGE_SQL: [(&str, &str); 21] = [
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
        "daily_activity",
        concat!("DELETE FROM daily_activity WHERE ", provider_rows!()),
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
        "activity_streams",
        concat!(
            "DELETE FROM activity_streams WHERE ",
            provider_rows!(),
            " AND ",
            pushed_streams!()
        ),
    ),
    (
        "pushed_activity_files",
        concat!("DELETE FROM pushed_activity_files WHERE ", provider_rows!()),
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
        "activity_gear",
        concat!("DELETE FROM activity_gear WHERE ", provider_rows!()),
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
