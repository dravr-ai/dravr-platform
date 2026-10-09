// ABOUTME: The SQL both activity-cache backends run: shared statements, and macros for the ones that cast ids on Postgres
// ABOUTME: Emitted into each backend by impl_activity_cache_repository, which names these at its invocation site
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Insert or overwrite one cached activity.
///
/// `$n` placeholders throughout this module: sqlx accepts them on `SQLite` as
/// well as Postgres. `cached_activities` types its ids as TEXT on both
/// engines, so its statements need no cast; its timestamps bind as
/// `DateTime<Utc>` on both — sqlx-sqlite encodes one as
/// `to_rfc3339_opts(AutoSi, false)`, the text the column's earlier writes
/// hold, so the lexical order the reads rely on is unchanged.
///
/// `detail_json` is named neither in the insert nor in the update: a list
/// read replaces `data_json` whole and carries no splits or laps, so the
/// detail a detail read stored ([`STORE_CACHED_ACTIVITY_DETAIL_SQL`])
/// survives every later list sync of the same activity.
pub const UPSERT_CACHED_ACTIVITY_SQL: &str = r"
    INSERT INTO cached_activities (id, user_id, tenant_id, provider, activity_id, sport_type, start_date, synced_at, data_json)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
    ON CONFLICT(user_id, tenant_id, provider, activity_id) DO UPDATE SET
        sport_type = EXCLUDED.sport_type,
        start_date = EXCLUDED.start_date,
        synced_at = EXCLUDED.synced_at,
        data_json = EXCLUDED.data_json";

/// Store what a detail read of one cached activity found, on the row its
/// list read wrote, by the table's uniqueness key, with the instant it is
/// read again (`$6`, NULL when it stands). A row the user does not hold in
/// this tenant is left alone: the statement updates nothing.
pub const STORE_CACHED_ACTIVITY_DETAIL_SQL: &str = r"
    UPDATE cached_activities
    SET detail_json = $5, detail_recheck_at = $6
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";

/// Cached activities for a user within a window, newest first; `$5` NULL
/// means every provider. The parameter is bound once and named twice: both
/// engines give one bind to every occurrence of the same `$n`.
pub const GET_CACHED_ACTIVITIES_SQL: &str = r"
    SELECT data_json, detail_json
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND start_date >= $3 AND start_date <= $4
      AND ($5 IS NULL OR provider = $5)
    ORDER BY start_date DESC
    LIMIT $6";

/// Cached activities across every provider within a window, newest first,
/// with the provider key each row is stored under and the outcome of its
/// stored route read.
///
/// The route read is joined on the whole key of `activity_route_tracks`. Its
/// tenant and user are the bound parameters themselves, named in the join as
/// in the filter, so a read stored for the same provider and activity id
/// under another tenant or another user never reaches a row. LEFT JOIN, not
/// INNER: an activity whose route has not been read keeps its row, with both
/// route columns NULL, and so does one whose stored read has expired — its
/// `expires_at` is not after `$6` (now) — because that read is to be made
/// again. The track itself is not selected. Every id column on both tables is
/// TEXT on both engines, so the join carries no cast.
///
/// `detail_settled` is whether a stored detail read still answers the row: one
/// is stored and its recheck instant, if it has one, is after `$6` (now).
pub const GET_CACHED_ACTIVITY_ROWS_SQL: &str = r"
    SELECT ca.provider, ca.data_json, ca.detail_json,
           (ca.detail_json IS NOT NULL
            AND (ca.detail_recheck_at IS NULL OR ca.detail_recheck_at > $6)) AS detail_settled,
           rt.source AS route_source,
           rt.unavailable_reason AS route_unavailable_reason
    FROM cached_activities ca
    LEFT JOIN activity_route_tracks rt
           ON rt.tenant_id = $2
          AND rt.user_id = $1
          AND rt.provider = ca.provider
          AND rt.activity_id = ca.activity_id
          AND (rt.expires_at IS NULL OR rt.expires_at > $6)
    WHERE ca.user_id = $1 AND ca.tenant_id = $2
      AND ca.start_date >= $3 AND ca.start_date <= $4
    ORDER BY ca.start_date DESC
    LIMIT $5";

/// One cached activity, by the table's uniqueness key.
pub const GET_CACHED_ACTIVITY_SQL: &str = r"
    SELECT data_json, detail_json
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";

/// Latest `synced_at` a provider's cached rows carry for a user.
pub const LATEST_PROVIDER_SYNC_SQL: &str = r"
    SELECT synced_at
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
    ORDER BY synced_at DESC
    LIMIT 1";

/// Latest `synced_at` any provider's cached row carries for a user.
///
/// An uploaded file's rows (`provider = 'upload'`,
/// `pierre_core::constants::oauth_providers::UPLOAD`) are left out: their
/// `synced_at` is when the athlete uploaded, which says nothing about how
/// current any provider's copy is, and counting it would show a stale cache
/// as just synced.
pub const LATEST_ANY_SYNC_SQL: &str = r"
    SELECT synced_at
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider <> 'upload'
    ORDER BY synced_at DESC
    LIMIT 1";

/// Delete one cached activity, by the table's uniqueness key.
pub const DELETE_CACHED_ACTIVITY_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";

/// Delete one provider's cached activities for a user that started within a
/// window, both ends included — the window [`GET_CACHED_ACTIVITIES_SQL`]
/// reads.
pub const DELETE_CACHED_ACTIVITIES_BETWEEN_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
      AND start_date >= $4 AND start_date <= $5";

/// Delete every cached activity one provider contributed for a user.
pub const DELETE_PROVIDER_ACTIVITIES_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";

/// Delete a user's cached provider activities that started before a cutoff.
///
/// An uploaded file's rows (`provider = 'upload'`,
/// `pierre_core::constants::oauth_providers::UPLOAD`) are kept: a provider's
/// row is a copy the next fetch can bring back, an upload's is the record
/// itself, and pruning it would lose the athlete's workout for good.
pub const PRUNE_ACTIVITIES_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND start_date < $3 AND provider <> 'upload'";

/// The most recent fetch mark for a user, optionally scoped to one provider.
///
/// `activity_fetch_freshness` and `activity_backfill_coverage` type
/// `tenant_id`/`user_id` as UUID on Postgres, to match `users.id`, and as
/// TEXT on `SQLite`; the id binds the hyphenated string on both and `$uuid`
/// is the cast Postgres needs on it (`"::uuid"`), `""` on `SQLite`.
macro_rules! latest_fetch_mark_sql {
    ($uuid:literal) => {
        concat!(
            "SELECT fetched_at FROM activity_fetch_freshness \
             WHERE user_id = $1",
            $uuid,
            " AND tenant_id = $2",
            $uuid,
            " AND ($3 IS NULL OR provider = $3) \
             ORDER BY fetched_at DESC LIMIT 1"
        )
    };
}
pub(crate) use latest_fetch_mark_sql;

/// Record a successful fetch; a later fetch overwrites the mark.
macro_rules! record_activity_fetch_sql {
    ($uuid:literal) => {
        concat!(
            "INSERT INTO activity_fetch_freshness (tenant_id, user_id, provider, fetched_at) \
             VALUES ($1",
            $uuid,
            ", $2",
            $uuid,
            ", $3, $4) \
             ON CONFLICT (tenant_id, user_id, provider) DO UPDATE SET \
                 fetched_at = EXCLUDED.fetched_at"
        )
    };
}
pub(crate) use record_activity_fetch_sql;

/// Record a fetch that did not count as a sync; a later one overwrites it.
macro_rules! record_activity_fetch_failure_sql {
    ($uuid:literal) => {
        concat!(
            "INSERT INTO activity_fetch_failures \
                 (tenant_id, user_id, provider, failed_at, reason, consecutive, streak, \
                  streak_started_at) \
             VALUES ($1",
            $uuid,
            ", $2",
            $uuid,
            ", $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (tenant_id, user_id, provider) DO UPDATE SET \
                 failed_at = EXCLUDED.failed_at, \
                 reason = EXCLUDED.reason, \
                 consecutive = EXCLUDED.consecutive, \
                 streak = EXCLUDED.streak, \
                 streak_started_at = EXCLUDED.streak_started_at"
        )
    };
}
pub(crate) use record_activity_fetch_failure_sql;

/// The last failed fetch of one provider for a user.
macro_rules! latest_fetch_failure_sql {
    ($uuid:literal) => {
        concat!(
            "SELECT failed_at, reason, consecutive, streak, streak_started_at \
             FROM activity_fetch_failures \
             WHERE user_id = $1",
            $uuid,
            " AND tenant_id = $2",
            $uuid,
            " AND provider = $3"
        )
    };
}
pub(crate) use latest_fetch_failure_sql;

/// Raise every coverage floor below a cutoff and clear its feed-end flag.
/// `hit_feed_end` is BOOLEAN on Postgres and an INTEGER 0/1 on `SQLite`,
/// which reads the `FALSE` keyword as 0.
macro_rules! clamp_backfill_coverage_sql {
    ($uuid:literal) => {
        concat!(
            "UPDATE activity_backfill_coverage \
             SET oldest_reached_ts = $1, hit_feed_end = FALSE, updated_at = $2 \
             WHERE user_id = $3",
            $uuid,
            " AND tenant_id = $4",
            $uuid,
            " AND oldest_reached_ts < $1"
        )
    };
}
pub(crate) use clamp_backfill_coverage_sql;

/// Record how deep a backfill reached; overwrites any prior row.
macro_rules! upsert_backfill_coverage_sql {
    ($uuid:literal) => {
        concat!(
            "INSERT INTO activity_backfill_coverage \
                 (tenant_id, user_id, provider, oldest_reached_ts, hit_feed_end, updated_at, \
                  capture_version) \
             VALUES ($1",
            $uuid,
            ", $2",
            $uuid,
            ", $3, $4, $5, $6, $7) \
             ON CONFLICT (tenant_id, user_id, provider) DO UPDATE SET \
                 oldest_reached_ts = EXCLUDED.oldest_reached_ts, \
                 hit_feed_end = EXCLUDED.hit_feed_end, \
                 updated_at = EXCLUDED.updated_at, \
                 capture_version = EXCLUDED.capture_version"
        )
    };
}
pub(crate) use upsert_backfill_coverage_sql;

/// The coverage row for one `(tenant, user, provider)`.
macro_rules! get_backfill_coverage_sql {
    ($uuid:literal) => {
        concat!(
            "SELECT oldest_reached_ts, hit_feed_end, capture_version \
             FROM activity_backfill_coverage \
             WHERE tenant_id = $1",
            $uuid,
            " AND user_id = $2",
            $uuid,
            " AND provider = $3"
        )
    };
}
pub(crate) use get_backfill_coverage_sql;

/// Every active connection with its last use and its last successful fetch.
///
/// On Postgres the join legs straddle a type split: `provider_connections`
/// types `tenant_id`/`user_id` as TEXT, `activity_fetch_freshness` as UUID. The
/// cast direction is load-bearing. Casting the TEXT side UP
/// (`pc.user_id::uuid`) throws `invalid input syntax for type uuid` on the
/// first non-UUID-shaped value and takes the whole report down; casting the
/// UUID side DOWN to text can never fail, because every UUID renders. So a
/// tenant whose id is not UUID-shaped simply matches nothing and is reported
/// as never-fetched — which is the truth for it, since the UUID column could
/// not have held a row for it either. `$text` is that cast (`"::text"`); on
/// `SQLite` every identifier column is TEXT and it is `""`.
///
/// LEFT JOIN, not INNER: a connection with no freshness row at all has never
/// had a successful fetch recorded, which is the most alarming state this
/// can report and the one an INNER JOIN would delete. `NULLS LAST` is what
/// `SQLite` already does for `DESC`, and what Postgres does only when told.
macro_rules! capture_freshness_snapshot_sql {
    ($text:literal) => {
        concat!(
            "SELECT pc.tenant_id, pc.user_id, pc.provider, pc.last_used_at, f.fetched_at \
             FROM provider_connections pc \
             LEFT JOIN activity_fetch_freshness f \
                    ON f.user_id",
            $text,
            " = pc.user_id \
                   AND f.tenant_id",
            $text,
            " = pc.tenant_id \
                   AND f.provider = pc.provider \
             WHERE pc.status = 'active' \
             ORDER BY pc.last_used_at DESC NULLS LAST \
             LIMIT $1"
        )
    };
}
pub(crate) use capture_freshness_snapshot_sql;
