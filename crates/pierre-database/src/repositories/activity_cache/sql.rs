// ABOUTME: The SQL both activity-cache backends run: shared statements, and macros for the ones that cast ids on Postgres
// ABOUTME: Emitted into each backend by impl_activity_cache_repository, which names these at its invocation site
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// How many times an upsert merges over a stored copy that kept changing under it.
pub(crate) const UPSERT_MERGE_ATTEMPTS: usize = 4;

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
///
/// The one statement every writer of the table runs — a provider read, a
/// push, an upload, an archive import. A copy lands only when it was synced
/// no earlier than the stored one (`$8`): a push batch retried after a later
/// push of the same activity leaves the later one standing. A read stamps
/// `now`, or, for a read short of the list head, the head's own last sync,
/// which is never older than a row the head wrote.
///
/// A copy merged over what was stored (`$11` true) lands only while the row
/// still holds the copy it was merged with (`$10`; NULL when the read found
/// no row, so a row that appeared since refuses it), the guard
/// [`AMEND_LISTED_READS_SQL`] has: a tag walk's amend that landed since is
/// never erased, and the caller merges again over what stands. A copy that
/// carries its own tags and gear is not merged (`$11` false) and is not
/// guarded.
pub(crate) const UPSERT_CACHED_ACTIVITY_SQL: &str = r"
    INSERT INTO cached_activities (id, user_id, tenant_id, provider, activity_id, sport_type, start_date, synced_at, data_json)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
    ON CONFLICT(user_id, tenant_id, provider, activity_id) DO UPDATE SET
        sport_type = EXCLUDED.sport_type,
        start_date = EXCLUDED.start_date,
        synced_at = EXCLUDED.synced_at,
        data_json = EXCLUDED.data_json
    WHERE cached_activities.synced_at <= EXCLUDED.synced_at
      AND (NOT $11 OR cached_activities.data_json = $10)";

/// Ids one stored-copy read asks for at most: an archive import writes a
/// whole history through one call, and a single `IN` list of that size would
/// pass `SQLite`'s bound-variable limit.
pub(crate) const STORED_DATA_JSON_CHUNK: usize = 500;

/// The list copies rows store, by the table's uniqueness key with the
/// activity ids in one `IN` list (`$4` on): what a later write of the same
/// activities keeps from them ([`stored_activity_json_over`](super::stored_json)).
/// One read for a page of ids, never one per activity.
pub(crate) fn stored_data_json_sql(ids: usize) -> String {
    let placeholders = (4..4 + ids)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "
    SELECT activity_id, data_json
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
      AND activity_id IN ({placeholders})"
    )
}

/// Replace the `data_json` (`$5`) of one row, by the table's uniqueness key,
/// only while it still holds the copy (`$6`) the amend was made from: a list
/// write that landed in between stands, and nothing but `data_json` moves —
/// not `synced_at`, which is part of the provider's freshness.
pub(crate) const AMEND_LISTED_READS_SQL: &str = r"
    UPDATE cached_activities
    SET data_json = $5
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4
      AND data_json = $6";

/// Store what a detail read of one cached activity found, on the row its
/// list read wrote, by the table's uniqueness key, with the instant it is
/// read again (`$6`, NULL when it stands). A row the user does not hold in
/// this tenant is left alone: the statement updates nothing.
pub(crate) const STORE_CACHED_ACTIVITY_DETAIL_SQL: &str = r"
    UPDATE cached_activities
    SET detail_json = $5, detail_recheck_at = $6
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";

/// Cached activities for a user within a window, newest first; `$5` NULL
/// means every provider. The parameter is bound once and named twice: both
/// engines give one bind to every occurrence of the same `$n`.
pub(crate) const GET_CACHED_ACTIVITIES_SQL: &str = r"
    SELECT data_json, detail_json
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND start_date >= $3 AND start_date <= $4
      AND ($5 IS NULL OR provider = $5)
    ORDER BY start_date DESC
    LIMIT $6";

/// Cached activities across every provider within a window, newest first,
/// with the provider key each row is stored under, the outcome of its stored
/// route read, and its place.
///
/// The route read is joined on the whole key of `activity_route_tracks`. Its
/// tenant and user are the bound parameters themselves, named in the join as
/// in the filter, so a read stored for the same provider and activity id
/// under another tenant or another user never reaches a row. LEFT JOIN, not
/// INNER: an activity whose route has not been read keeps its row, with both
/// route columns NULL, and so does one whose stored read has expired — its
/// `expires_at` is not after `$6` (now) — because that read is to be made
/// again. The track itself is not selected.
///
/// The place is joined the same way, on the bound tenant and user and the
/// row's `place_id`: a `place_id` whose place is gone joins nothing and reads
/// as no place. Every id column of the three tables is TEXT on both engines,
/// so no join carries a cast.
///
/// `detail_settled` is whether a stored detail read still answers the row: one
/// is stored and its recheck instant, if it has one, is after `$6` (now).
pub(crate) const GET_CACHED_ACTIVITY_ROWS_SQL: &str = r"
    SELECT ca.provider, ca.data_json, ca.detail_json, ca.created_at AS stored_at,
           (ca.detail_json IS NOT NULL
            AND (ca.detail_recheck_at IS NULL OR ca.detail_recheck_at > $6)) AS detail_settled,
           rt.source AS route_source,
           rt.unavailable_reason AS route_unavailable_reason,
           ap.id AS place_id, ap.name AS place_name, ap.name_source AS place_name_source
    FROM cached_activities ca
    LEFT JOIN activity_route_tracks rt
           ON rt.tenant_id = $2
          AND rt.user_id = $1
          AND rt.provider = ca.provider
          AND rt.activity_id = ca.activity_id
          AND (rt.expires_at IS NULL OR rt.expires_at > $6)
    LEFT JOIN athlete_places ap
           ON ap.tenant_id = $2
          AND ap.user_id = $1
          AND ap.id = ca.place_id
    WHERE ca.user_id = $1 AND ca.tenant_id = $2
      AND ca.start_date >= $3 AND ca.start_date <= $4";

/// [`GET_CACHED_ACTIVITY_ROWS_SQL`] as one read asks it
/// ([`CachedRowsQuery`](super::CachedRowsQuery)): below a page's cursor when
/// `below`, bound as `$7`–`$9`; over `keys` provider keys when non-zero, then
/// over `ids` activity ids when non-zero, one placeholder each from the next
/// free number, as `users_by_ids_sql` binds its batch. Only the placeholder
/// count is formatted; every value binds through the codec. Ordered by start,
/// key, then id, all descending, so the cursor's tie-break is the order's own
/// and a page's edge between two rows of one start never skips or repeats one.
pub(crate) fn cached_activity_rows_sql(keys: usize, ids: usize, below: bool) -> String {
    let mut sql = GET_CACHED_ACTIVITY_ROWS_SQL.to_owned();
    if below {
        sql.push_str(
            "
      AND (ca.start_date < $7
           OR (ca.start_date = $7
               AND (ca.provider < $8 OR (ca.provider = $8 AND ca.activity_id < $9))))",
        );
    }
    let mut next = if below { 10 } else { 7 };
    for (column, count) in [("ca.provider", keys), ("ca.activity_id", ids)] {
        if count == 0 {
            continue;
        }
        let placeholders = (next..next + count)
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str("\n      AND ");
        sql.push_str(column);
        sql.push_str(" IN (");
        sql.push_str(&placeholders);
        sql.push(')');
        next += count;
    }
    sql.push_str(
        "
    ORDER BY ca.start_date DESC, ca.provider DESC, ca.activity_id DESC
    LIMIT $5",
    );
    sql
}

// A row's place (slice 6, carnet#1484), written here with every other
// cached_activities write (check-one-writer.sh, G1). The places service
// writes it through `AthletePlaceRepository::replace_clustering` and
// `create_athlete_place`.

/// Set one cached row's place (`$5`, NULL to clear), by the table's
/// uniqueness key.
pub(crate) const ASSIGN_PLACE_SQL: &str = r"
    UPDATE cached_activities
    SET place_id = $5
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";

/// Clear the place of every cached row of `$2` (user) in `$1` (tenant)
/// whose place is gone.
pub(crate) const CLEAR_DANGLING_PLACES_SQL: &str = r"
    UPDATE cached_activities
    SET place_id = NULL
    WHERE tenant_id = $1 AND user_id = $2 AND place_id IS NOT NULL
      AND place_id NOT IN (
          SELECT ap.id FROM athlete_places ap WHERE ap.tenant_id = $1 AND ap.user_id = $2
      )";

/// One cached activity, by the table's uniqueness key.
pub(crate) const GET_CACHED_ACTIVITY_SQL: &str = r"
    SELECT data_json, detail_json
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";

/// Latest `synced_at` a provider's cached rows carry for a user.
pub(crate) const LATEST_PROVIDER_SYNC_SQL: &str = r"
    SELECT synced_at
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
    ORDER BY synced_at DESC
    LIMIT 1";

/// Latest `synced_at` any provider's cached row carries for a user.
///
/// The rows the athlete brought in themselves (an uploaded file's, an
/// imported archive's: `pierre_core::constants::oauth_providers::USER_PROVIDED_PROVIDERS`)
/// are left out: their `synced_at` is when the athlete uploaded or imported,
/// which says nothing about how current any provider's copy is, and counting
/// it would show a stale cache as just synced.
pub(crate) const LATEST_ANY_SYNC_SQL: &str = r"
    SELECT synced_at
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider NOT IN ('upload', 'strava_archive', 'garmin_archive', 'apple_health_archive', 'coros_archive', 'whoop_archive')
    ORDER BY synced_at DESC
    LIMIT 1";

/// Latest `start_date` any cached activity of a user carries, uploads and
/// imports included: the day they last trained, as far as the cache knows.
pub(crate) const LATEST_ACTIVITY_START_SQL: &str = r"
    SELECT start_date
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2
    ORDER BY start_date DESC
    LIMIT 1";

/// Oldest `start_date` a user's cached activities carry under one key: how
/// far back that key's history reaches.
pub(crate) const OLDEST_PROVIDER_START_SQL: &str = r"
    SELECT start_date
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
    ORDER BY start_date ASC
    LIMIT 1";

/// The user-provided keys (`USER_PROVIDED_PROVIDERS`) under which a user holds
/// at least one cached activity: the sources an athlete with no connection
/// still has to answer from.
pub(crate) const HELD_USER_PROVIDED_SQL: &str = r"
    SELECT DISTINCT provider
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider IN ('upload', 'strava_archive', 'garmin_archive', 'apple_health_archive', 'coros_archive', 'whoop_archive')";

/// How many cached activities the user holds under each key in a tenant.
pub(crate) const COUNT_BY_PROVIDER_SQL: &str = r"
    SELECT provider, COUNT(*) AS n
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2
    GROUP BY provider
    ORDER BY provider";

/// How many cached activities each user holds under each key in a tenant.
pub(crate) const COUNT_BY_USER_AND_PROVIDER_SQL: &str = r"
    SELECT user_id, provider, COUNT(*) AS n
    FROM cached_activities
    WHERE tenant_id = $1
    GROUP BY user_id, provider
    ORDER BY user_id, provider";

/// Every row a user holds under one key in a tenant, as stored, oldest
/// sync first: the list copy without its detail read, and its stamp.
pub(crate) const GET_STORED_ACTIVITY_ROWS_SQL: &str = r"
    SELECT data_json, synced_at
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
    ORDER BY synced_at, activity_id";

/// Delete one cached activity, by the table's uniqueness key.
pub(crate) const DELETE_CACHED_ACTIVITY_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";

/// Delete every cached activity one provider contributed for a user.
pub(crate) const DELETE_PROVIDER_ACTIVITIES_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";

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

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::constants::oauth_providers::USER_PROVIDED_PROVIDERS;

    /// The quoted keys inside the one `IN (...)` list `sql` carries.
    fn listed_keys(sql: &str) -> Vec<&str> {
        let start = sql.find("IN (").map(|at| at + 4);
        let list =
            start.and_then(|start| sql[start..].find(')').map(|end| &sql[start..start + end]));
        list.map_or_default(|list| {
            list.split(',')
                .map(|key| key.trim().trim_matches('\''))
                .collect()
        })
    }

    /// Every statement that names the user-provided keys names exactly
    /// `USER_PROVIDED_PROVIDERS`: a key added there cannot be missed by one
    /// statement and pruned or counted as a sync by it.
    #[test]
    fn every_statement_naming_user_provided_keys_lists_exactly_the_set() {
        for (name, sql) in [
            ("LATEST_ANY_SYNC_SQL", LATEST_ANY_SYNC_SQL),
            ("HELD_USER_PROVIDED_SQL", HELD_USER_PROVIDED_SQL),
        ] {
            assert_eq!(
                listed_keys(sql),
                USER_PROVIDED_PROVIDERS,
                "{name} must list exactly the user-provided keys"
            );
        }
    }
}
