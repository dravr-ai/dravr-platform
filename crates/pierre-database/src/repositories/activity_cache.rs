// ABOUTME: Repository trait for the provider-agnostic activity cache (stale-while-revalidate)
// ABOUTME: Persists fetched Activity records so chat reads serve cached data instead of re-fetching
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use uuid::Uuid;

/// How deep a historical backfill has reached for a `(tenant, user, provider)`.
///
/// Lets the historical-activity gate distinguish "cached but only the recent
/// slice of a deep window" (re-backfill) from "cached down to the requested
/// floor" (covered, serve inline).
#[derive(Debug, Clone, Copy)]
pub struct BackfillCoverage {
    /// Deepest floor (unix seconds) a backfill has confirmed covered. When the
    /// scrape returned the whole requested window (not count-capped) this is the
    /// requested `after`, not the oldest activity fetched — so a sparse year
    /// whose oldest activity sits just after Jan 1 00:00 still reads as covered.
    pub oldest_reached_ts: i64,
    /// `true` only when the provider EXPLICITLY reports its feed exhausted
    /// (next-page disabled), so no older data exists and a deeper ask is covered
    /// without re-scraping. The date-floored scrape path cannot prove this (it
    /// stops at `after`, never below it), so it leaves this `false`; the gate
    /// still honors a `true` set by a provider that does report feed-end.
    pub hit_feed_end: bool,
    /// Which generation of the provider's capture wrote the rows this record
    /// vouches for (`pierre_core::constants::provider_capture`). Set by a
    /// completed backfill and left alone by the retention clamp, so it names
    /// the capture that produced the rows, not the last write to the record.
    /// A value below the provider's current version means those rows predate a
    /// capture fix and the window must be re-captured rather than served.
    pub capture_version: u32,
}

/// One live provider connection paired with the last time a fetch actually
/// reached its provider.
///
/// The two timestamps are what make a frozen capture visible, and they move for
/// different reasons. `last_used_at` is touched at the serve chokepoint on EVERY
/// serve, including one the durable cache answered; `last_fetch_at` advances only
/// when a live fetch genuinely succeeded. So a row whose `last_used_at` is recent
/// while `last_fetch_at` is days behind is an athlete being served from cache by a
/// provider that has stopped answering — the shape nobody could see when
/// jf@dravr.ai's sciotte capture froze on 2026-08-28 and stayed frozen for days.
#[derive(Debug, Clone)]
pub struct CaptureFreshness {
    /// Owning tenant, as stored on the connection row.
    pub tenant_id: String,
    /// Connection owner. Kept in the stored string form rather than parsed to
    /// [`Uuid`]: a malformed identifier must be REPORTED, not silently dropped
    /// by a failed parse, on the one surface whose whole purpose is to stop
    /// losing things quietly.
    pub user_id: String,
    /// Provider slug the connection resolves to.
    pub provider: String,
    /// When this connection last served the athlete — live fetch or cache alike.
    /// `None` for a connection that has never served.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When a fetch last actually reached the provider. `None` when no
    /// successful fetch has ever been recorded for this connection.
    pub last_fetch_at: Option<DateTime<Utc>>,
}

/// A cached activity with the provider key its row is stored under.
///
/// The key is not always the activity's own `provider()`: a row is filed
/// under the provider name the fetch that wrote it was made for, while a
/// mirror backend's activities name the backend (`sciotte`) whichever
/// provider they mirror. A caller that addresses the row again, or deletes it
/// with its provider's data, needs the key.
#[derive(Debug, Clone)]
pub struct CachedActivityRow {
    /// The provider key the row is stored under.
    pub provider: String,
    /// The cached activity.
    pub activity: Activity,
}

/// Persistence for activities fetched from any provider, enabling
/// stale-while-revalidate reads on the chat path.
///
/// Every provider fetch writes through here; chat reads serve the cached rows
/// immediately and trigger a background revalidation when the data is stale.
/// This removes the slow per-request scrape (notably Garmin/sciotte) from the
/// chat critical path and avoids redundant API calls for token providers.
#[async_trait]
pub trait ActivityCacheRepository: Send + Sync {
    /// Insert or update a batch of activities for a user+provider.
    ///
    /// Keyed on `(user_id, tenant_id, provider, activity_id)`; a later fetch of
    /// the same activity overwrites the stored copy. Returns the count of NET
    /// DISTINCT rows persisted (deduped by `activity_id`), not the raw input
    /// length — a provider feed that repeats an `activity_id` within one batch
    /// upserts the same row, so the input length would overstate the rows
    /// actually stored. This honest count feeds the backfill completion notice.
    async fn upsert_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<u64>;

    /// Fetch cached activities for a user within `[start, end]`, newest first.
    ///
    /// `provider = None` returns activities across all providers.
    async fn get_cached_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: Option<&str>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<Activity>>;

    /// Cached activities for a user within `[start, end]` across every
    /// provider, newest first, each with the provider key its row is stored
    /// under.
    async fn get_cached_activity_rows(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<CachedActivityRow>>;

    /// One cached activity by its provider and the provider's id for it, or
    /// `None` when the user holds no such row in this tenant.
    ///
    /// The key is the table's own uniqueness key: an activity id is unique
    /// only within its provider, so two providers can hold the same id.
    async fn get_cached_activity(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<Option<Activity>>;

    /// Delete every cached activity a provider contributed for a user.
    ///
    /// The provider-disconnect path calls this so revoking consent also
    /// removes the provider-derived rows we hold (Strava API Policy §7.4
    /// treats deletion on deauthorization as an obligation, not hygiene).
    /// Returns the number of rows removed.
    async fn delete_provider_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64>;

    /// Most recent successful activity fetch for a user+provider — the
    /// freshness signal that drives background revalidation. The later of the
    /// cached rows' `synced_at` and the fetch mark recorded by
    /// [`Self::record_activity_fetch`], so a fetch that truthfully returned
    /// nothing still reads as fresh. `None` when the provider has never been
    /// fetched.
    async fn latest_activity_sync(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>>;

    /// Most recent successful activity fetch for a user across **every**
    /// provider — the later of the cached rows' `synced_at` and the fetch
    /// marks recorded by [`Self::record_activity_fetch`]. `None` when nothing
    /// has ever been fetched for them.
    ///
    /// The provider-scoped sibling above answers "should I revalidate this
    /// provider"; this one answers "is an empty window real, or has the cache
    /// simply not caught up". A caller that has to distinguish *the athlete did
    /// not do it* from *we do not know yet* cannot enumerate providers to find
    /// out — an athlete who connected a second device mid-window would look
    /// stale on the first one forever.
    async fn latest_activity_sync_any(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<DateTime<Utc>>>;

    /// Record that a provider activity fetch completed successfully at
    /// `fetched_at`, whether or not it returned any activities.
    ///
    /// The cached rows' `synced_at` only advances when a fetch returns rows,
    /// so without this mark an athlete whose provider truthfully reports no
    /// activities looks forever stale — and a freshness-guarded reader (the
    /// commitment sweep) could never believe an honest zero. Both
    /// `latest_activity_sync*` reads take the later of the row signal and
    /// this mark.
    async fn record_activity_fetch(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        fetched_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Delete a user's cached activities whose `start_date` is older than
    /// `cutoff` (retention pruning). Returns the number of rows removed.
    async fn prune_activities_before(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        cutoff: DateTime<Utc>,
    ) -> AppResult<u64>;

    /// Record how deep a completed backfill reached for `(tenant, user,
    /// provider)`, and which capture version produced its rows. Overwrites any
    /// prior row. Within one capture version coverage only deepens, because a
    /// query shallower than the recorded floor is already covered and never
    /// reaches this write. Across versions the overwrite is what keeps the
    /// claim true: a record at an older version reads as not covered, so a
    /// shallower re-capture does reach this write, and it must replace the old
    /// deeper floor rather than inherit a depth the new capture never re-read.
    async fn upsert_backfill_coverage(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        coverage: BackfillCoverage,
    ) -> AppResult<()>;

    /// Raise every coverage floor for `(tenant, user)` that claims to reach
    /// below `cutoff`, across all providers. Returns the number of rows moved.
    ///
    /// Coverage records what a backfill FETCHED; the prune decides what is
    /// still STORED, and the two drift apart because `prune_activities_before`
    /// is keyed per `(user, tenant)` across every provider. One narrow
    /// write-through therefore reclaims a deep backfill's rows while the
    /// coverage row goes on naming the depth it once reached, and the
    /// historical gate serves that shallow cache as a complete window without
    /// calling a provider. Clamping at the moment of the prune keeps the claim
    /// true: a floor is only ever raised to what survived.
    ///
    /// `hit_feed_end` is cleared with it. That flag means the provider reported
    /// its feed exhausted — a statement about upstream, not about what the
    /// cache kept — so it must not go on excusing a depth the prune removed.
    async fn clamp_backfill_coverage(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        cutoff: DateTime<Utc>,
    ) -> AppResult<u64>;

    /// Read the recorded backfill coverage for `(tenant, user, provider)`, or
    /// `None` when no deep backfill has run for this athlete+provider yet.
    async fn get_backfill_coverage(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Option<BackfillCoverage>>;

    /// Every ACTIVE provider connection with the last time a fetch reached its
    /// provider, newest-served first, capped at `limit` rows.
    ///
    /// Cross-tenant, and deliberately so — the same exception
    /// [`CommitmentRepository::due_commitments`](crate::repositories::CommitmentRepository::due_commitments)
    /// takes. An operator asking "has any athlete's capture stopped" cannot ask
    /// it one tenant at a time: the tenants worth asking about are exactly the
    /// ones nobody thought to check. Each row carries its own `tenant_id`
    /// forward so every consumer stays tenant-aware.
    ///
    /// Returns no verdict. Staleness is a threshold decision that belongs to the
    /// caller, which is what lets an operator re-ask the same question with a
    /// different threshold without a deploy.
    ///
    /// Connections needing re-auth are excluded: a revoked or expired connection
    /// has a KNOWN reason to have stopped fetching, it already surfaces through
    /// the reconnect path, and leaving it in would bury the silent failures this
    /// exists to find under a pile of loud ones.
    async fn capture_freshness_snapshot(&self, limit: i64) -> AppResult<Vec<CaptureFreshness>>;
}

/// Insert or overwrite one cached activity.
///
/// `$n` placeholders throughout this module: sqlx accepts them on `SQLite` as
/// well as Postgres. `cached_activities` types its ids as TEXT on both
/// engines, so its statements need no cast; its timestamps bind as
/// `DateTime<Utc>` on both — sqlx-sqlite encodes one as
/// `to_rfc3339_opts(AutoSi, false)`, the text the column's earlier writes
/// hold, so the lexical order the reads rely on is unchanged.
pub(crate) const UPSERT_CACHED_ACTIVITY_SQL: &str = r"
    INSERT INTO cached_activities (id, user_id, tenant_id, provider, activity_id, sport_type, start_date, synced_at, data_json)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
    ON CONFLICT(user_id, tenant_id, provider, activity_id) DO UPDATE SET
        sport_type = EXCLUDED.sport_type,
        start_date = EXCLUDED.start_date,
        synced_at = EXCLUDED.synced_at,
        data_json = EXCLUDED.data_json";

/// Cached activities for a user within a window, newest first; `$5` NULL
/// means every provider. The parameter is bound once and named twice: both
/// engines give one bind to every occurrence of the same `$n`.
pub(crate) const GET_CACHED_ACTIVITIES_SQL: &str = r"
    SELECT data_json
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND start_date >= $3 AND start_date <= $4
      AND ($5 IS NULL OR provider = $5)
    ORDER BY start_date DESC
    LIMIT $6";

/// Cached activities across every provider within a window, newest first,
/// with the provider key each row is stored under.
pub(crate) const GET_CACHED_ACTIVITY_ROWS_SQL: &str = r"
    SELECT provider, data_json
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND start_date >= $3 AND start_date <= $4
    ORDER BY start_date DESC
    LIMIT $5";

/// One cached activity, by the table's uniqueness key.
pub(crate) const GET_CACHED_ACTIVITY_SQL: &str = r"
    SELECT data_json
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND activity_id = $4";

/// Latest `synced_at` a provider's cached rows carry for a user.
pub(crate) const LATEST_PROVIDER_SYNC_SQL: &str = r"
    SELECT synced_at
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
    ORDER BY synced_at DESC
    LIMIT 1";

/// Latest `synced_at` any cached row carries for a user.
pub(crate) const LATEST_ANY_SYNC_SQL: &str = r"
    SELECT synced_at
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2
    ORDER BY synced_at DESC
    LIMIT 1";

/// Delete every cached activity one provider contributed for a user.
pub(crate) const DELETE_PROVIDER_ACTIVITIES_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";

/// Delete a user's cached activities that started before a cutoff.
pub(crate) const PRUNE_ACTIVITIES_SQL: &str = r"
    DELETE FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND start_date < $3";

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

/// String form of an activity's sport type for the indexed column.
///
/// `SportType` is an externally-tagged enum: named variants serialize as a JSON
/// string (`"run"`, `"trail_running"`), but the catch-all `Other(String)`
/// serializes as an object (`{"other": "<provider_type>"}`). Unwrap that object
/// to the inner provider string so unmapped sports (e.g. a Strava type cageux
/// has no named variant for) get a real column value instead of `NULL` — the
/// canonical value always remains in `data_json` regardless.
#[must_use]
pub fn sport_type_string(activity: &Activity) -> Option<String> {
    match serde_json::to_value(activity.sport_type()).ok()? {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Object(map) => {
            map.get("other").and_then(|v| v.as_str()).map(str::to_owned)
        }
        _ => None,
    }
}

/// Later of the row-derived sync signal and the fetch-freshness mark.
pub(crate) fn later(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Read one timestamp column of either backend, via `try_get` only — a
/// corrupt row surfaces as a recoverable error naming the column, never as a
/// panic. On `SQLite` the column is RFC3339 text and sqlx parses it; on
/// Postgres it is a TIMESTAMPTZ.
///
/// # Errors
/// Returns a database error naming the column when it cannot be decoded.
pub(crate) fn timestamp_column<R>(row: &R, column: &str) -> AppResult<DateTime<Utc>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    row.try_get(column)
        .map_err(|e| AppError::database(format!("activity col {column}: {e}")))
}

/// Deserialize the stored `Activity` out of one `data_json` row.
///
/// # Errors
/// Returns a database error when the column is missing or does not hold an
/// `Activity`.
pub(crate) fn activity_from_row<R>(row: &R) -> AppResult<Activity>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let data_json: String = row
        .try_get("data_json")
        .map_err(|e| AppError::database(format!("activity col data_json: {e}")))?;
    serde_json::from_str::<Activity>(&data_json)
        .map_err(|e| AppError::database(format!("Failed to deserialize cached activity: {e}")))
}

/// Read a [`CachedActivityRow`] out of one `(provider, data_json)` row.
///
/// # Errors
/// Returns a database error when either column is missing or `data_json`
/// does not hold an `Activity`.
pub(crate) fn cached_activity_row_from_row<R>(row: &R) -> AppResult<CachedActivityRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let provider: String = row
        .try_get("provider")
        .map_err(|e| AppError::database(format!("activity col provider: {e}")))?;
    Ok(CachedActivityRow {
        provider,
        activity: activity_from_row(row)?,
    })
}

/// Extract a [`BackfillCoverage`] from a row of either backend.
/// `hit_feed_end` decodes as `bool` on both: a BOOLEAN on Postgres, an
/// INTEGER 0/1 on `SQLite`.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn backfill_coverage_from_row<R>(row: &R) -> AppResult<BackfillCoverage>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let capture_version: i64 = row
        .try_get("capture_version")
        .map_err(|e| AppError::database(format!("coverage col capture_version: {e}")))?;
    Ok(BackfillCoverage {
        oldest_reached_ts: row
            .try_get("oldest_reached_ts")
            .map_err(|e| AppError::database(format!("coverage col oldest_reached_ts: {e}")))?,
        hit_feed_end: row
            .try_get("hit_feed_end")
            .map_err(|e| AppError::database(format!("coverage col hit_feed_end: {e}")))?,
        // BIGINT on Postgres and INTEGER on `SQLite`, both of which decode as
        // i64. Written from a u32 and constrained NOT NULL, so a value outside
        // that range can only be a hand-edited row; reading it as the baseline
        // re-captures rather than trusts it.
        capture_version: u32::try_from(capture_version).unwrap_or(0),
    })
}

/// Extract a [`CaptureFreshness`] from a row of either backend. The two
/// nullable timestamps decode as `Option<DateTime<Utc>>`: a NULL is `None`
/// on both engines, and a value is RFC3339 text on `SQLite` — the only form
/// `provider_connections.last_used_at` is ever written in — or a
/// TIMESTAMPTZ on Postgres.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn capture_freshness_from_row<R>(row: &R) -> AppResult<CaptureFreshness>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str| -> AppResult<String> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("capture col {name}: {e}")))
    };
    let stamp = |name: &str| -> AppResult<Option<DateTime<Utc>>> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("capture col {name}: {e}")))
    };
    Ok(CaptureFreshness {
        tenant_id: col("tenant_id")?,
        user_id: col("user_id")?,
        provider: col("provider")?,
        last_used_at: stamp("last_used_at")?,
        last_fetch_at: stamp("fetched_at")?,
    })
}

/// Emit the whole [`ActivityCacheRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, the cast its UUID columns need on a bound id (`"::uuid"` or
/// `""`) and the cast that joins those columns to a TEXT id (`"::text"` or
/// `""`), and sqlx resolves the driver from `self.pool()` per expansion.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_activity_cache_repository {
    ($ty:ty, $uuid:literal, $text:literal) => {
        /// Most recent `activity_fetch_freshness` mark for the user, optionally
        /// scoped to one provider. `None` when no successful fetch has been
        /// recorded.
        async fn latest_fetch_mark(
            db: &$ty,
            user_id: &str,
            tenant_id: &str,
            provider: Option<&str>,
        ) -> AppResult<Option<DateTime<Utc>>> {
            // `fetched_at` is RFC3339 UTC text on SQLite, so its lexical DESC
            // order is also the chronological one — the same assumption the
            // `synced_at` reads make.
            let row = sqlx::query(latest_fetch_mark_sql!($uuid))
                .bind(user_id)
                .bind(tenant_id)
                .bind(provider)
                .fetch_optional(db.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to read activity fetch mark: {e}"))
                })?;
            row.map(|r| timestamp_column(&r, "fetched_at")).transpose()
        }

        #[async_trait::async_trait]
        impl ActivityCacheRepository for $ty {
            async fn upsert_activities(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                activities: &[Activity],
            ) -> AppResult<u64> {
                let user_id_str = user_id.to_string();
                let tenant_str = tenant_id.to_string();
                let now = Utc::now();
                // Count NET DISTINCT rows persisted, not raw input length. A provider feed
                // can return the same `activity_id` twice in one batch; each ON CONFLICT
                // upsert overwrites the prior copy, so the input length overstates the
                // distinct rows actually stored. Dedup by the upsert key (activity_id) to
                // report the honest figure — the backfill completion notice surfaces this
                // count to the user.
                let mut distinct_ids = HashSet::new();

                for activity in activities {
                    let id = Uuid::new_v4().to_string();
                    let sport = sport_type_string(activity);
                    let data_json = serde_json::to_string(activity).map_err(|e| {
                        AppError::database(format!("Failed to serialize activity: {e}"))
                    })?;

                    sqlx::query(UPSERT_CACHED_ACTIVITY_SQL)
                        .bind(&id)
                        .bind(&user_id_str)
                        .bind(&tenant_str)
                        .bind(provider)
                        .bind(activity.id())
                        .bind(&sport)
                        .bind(activity.start_date())
                        .bind(now)
                        .bind(&data_json)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to upsert activity: {e}"))
                        })?;

                    distinct_ids.insert(activity.id().to_owned());
                }

                Ok(u64::try_from(distinct_ids.len()).unwrap_or(u64::MAX))
            }

            async fn get_cached_activities(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: Option<&str>,
                start: DateTime<Utc>,
                end: DateTime<Utc>,
                limit: i64,
            ) -> AppResult<Vec<Activity>> {
                let rows = sqlx::query(GET_CACHED_ACTIVITIES_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(start)
                    .bind(end)
                    .bind(provider)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get cached activities: {e}"))
                    })?;
                rows.iter().map(activity_from_row).collect()
            }

            async fn get_cached_activity_rows(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                start: DateTime<Utc>,
                end: DateTime<Utc>,
                limit: i64,
            ) -> AppResult<Vec<CachedActivityRow>> {
                let rows = sqlx::query(GET_CACHED_ACTIVITY_ROWS_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(start)
                    .bind(end)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get cached activity rows: {e}"))
                    })?;
                rows.iter().map(cached_activity_row_from_row).collect()
            }

            async fn get_cached_activity(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<Option<Activity>> {
                let row = sqlx::query(GET_CACHED_ACTIVITY_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get cached activity: {e}"))
                    })?;
                row.as_ref().map(activity_from_row).transpose()
            }

            async fn latest_activity_sync(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
            ) -> AppResult<Option<DateTime<Utc>>> {
                let user_id_str = user_id.to_string();
                let row = sqlx::query(LATEST_PROVIDER_SYNC_SQL)
                    .bind(&user_id_str)
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read activity sync time: {e}"))
                    })?;
                let from_rows = row.map(|r| timestamp_column(&r, "synced_at")).transpose()?;
                let mark =
                    latest_fetch_mark(self, &user_id_str, &tenant_id.to_string(), Some(provider))
                        .await?;
                Ok(later(from_rows, mark))
            }

            async fn latest_activity_sync_any(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
            ) -> AppResult<Option<DateTime<Utc>>> {
                let user_id_str = user_id.to_string();
                // `synced_at` is RFC3339 UTC text on SQLite, so the lexical DESC
                // order is also the chronological one — the same assumption every
                // other read here makes.
                let row = sqlx::query(LATEST_ANY_SYNC_SQL)
                    .bind(&user_id_str)
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read activity sync time: {e}"))
                    })?;
                let from_rows = row.map(|r| timestamp_column(&r, "synced_at")).transpose()?;
                let mark =
                    latest_fetch_mark(self, &user_id_str, &tenant_id.to_string(), None).await?;
                Ok(later(from_rows, mark))
            }

            async fn record_activity_fetch(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                fetched_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(record_activity_fetch_sql!($uuid))
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(fetched_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record activity fetch: {e}"))
                    })?;
                Ok(())
            }

            async fn delete_provider_activities(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
            ) -> AppResult<u64> {
                let result = sqlx::query(DELETE_PROVIDER_ACTIVITIES_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to delete provider cached activities: {e}"
                        ))
                    })?;
                Ok(result.rows_affected())
            }

            async fn prune_activities_before(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                cutoff: DateTime<Utc>,
            ) -> AppResult<u64> {
                let result = sqlx::query(PRUNE_ACTIVITIES_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(cutoff)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to prune cached activities: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn clamp_backfill_coverage(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                cutoff: DateTime<Utc>,
            ) -> AppResult<u64> {
                let result = sqlx::query(clamp_backfill_coverage_sql!($uuid))
                    .bind(cutoff.timestamp())
                    .bind(Utc::now())
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to clamp backfill coverage: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn upsert_backfill_coverage(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                coverage: BackfillCoverage,
            ) -> AppResult<()> {
                sqlx::query(upsert_backfill_coverage_sql!($uuid))
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(coverage.oldest_reached_ts)
                    .bind(coverage.hit_feed_end)
                    .bind(Utc::now())
                    .bind(i64::from(coverage.capture_version))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert backfill coverage: {e}"))
                    })?;
                Ok(())
            }

            async fn get_backfill_coverage(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
            ) -> AppResult<Option<BackfillCoverage>> {
                let row = sqlx::query(get_backfill_coverage_sql!($uuid))
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read backfill coverage: {e}"))
                    })?;
                row.map(|r| backfill_coverage_from_row(&r)).transpose()
            }

            async fn capture_freshness_snapshot(
                &self,
                limit: i64,
            ) -> AppResult<Vec<CaptureFreshness>> {
                let rows = sqlx::query(capture_freshness_snapshot_sql!($text))
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read capture freshness: {e}"))
                    })?;
                rows.iter().map(capture_freshness_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_activity_cache_repository;
