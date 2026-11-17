// ABOUTME: AthletePlaceRepository trait plus the one shared implementation both backends emit
// ABOUTME: The places an athlete trains from, the clustering run that last saw their rows, and the users due a recompute

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete places (carnet#1484, slice 6).
//!
//! A place is a cluster of an athlete's privacy-trimmed activity starts,
//! computed by the places worker (`pierre_services::athlete_places`). Its id
//! is kept across re-clustering, and every cached row whose start belongs to
//! it carries it in `cached_activities.place_id`. A centroid is never served
//! by any tool or route: a row serves its place as `{id, name}`, and only the
//! athlete's own export carries the centroid.
//!
//! A name has a source ([`PlaceNameSource`]). The athlete's own name is
//! written only by [`AthletePlaceRepository::set_name`] and
//! [`AthletePlaceRepository::create_athlete_place`], never by a clustering:
//! [`AthletePlaceRepository::replace_clustering`] keeps the name and source
//! the table holds for an athlete-named place, keeps an athlete-named place
//! its input leaves out, and writes no athlete name of its own. So a rename,
//! a clear or a single-visit place made while a recompute runs is never
//! undone by it.
//!
//! `$n` placeholders throughout; every id column is TEXT on both engines,
//! like `cached_activities`, so the ids bind as hyphenated text with no cast.
//! Timestamps bind as `DateTime<Utc>`: RFC 3339 text on `SQLite`,
//! `TIMESTAMPTZ` on Postgres, the form `cached_activities.synced_at` takes on
//! each, so the stamps compare alike on both engines.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CachedRowKey, TenantId};
use tracing::warn;
use uuid::Uuid;

/// Where a place's name came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaceNameSource {
    /// The athlete named it; served on every transport.
    Athlete,
    /// The most frequent city among its activities; served first-party only.
    City,
    /// The most frequent segment among its activities' efforts; served
    /// first-party only.
    Segment,
}

impl PlaceNameSource {
    /// The stored slug: `athlete`, `city` or `segment`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Athlete => "athlete",
            Self::City => "city",
            Self::Segment => "segment",
        }
    }

    /// The source a stored slug names, or `None` for an unknown one.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "athlete" => Some(Self::Athlete),
            "city" => Some(Self::City),
            "segment" => Some(Self::Segment),
            _ => None,
        }
    }
}

/// One `athlete_places` row. The tenant and user are the scope every method
/// takes, so the row does not repeat them.
#[derive(Debug, Clone, PartialEq)]
pub struct AthletePlace {
    /// The place id: a uuid, kept across re-clustering.
    pub id: String,
    /// The mean of the member starts. Never served by any tool or route.
    pub centroid_lat: f64,
    /// The mean of the member starts. Never served by any tool or route.
    pub centroid_lon: f64,
    /// The 90th-percentile member distance from the centroid, in metres.
    pub radius_m: f64,
    /// How many workouts start at the place; `0` for an athlete-named place
    /// no workout joins any more.
    pub activity_count: u32,
    /// The name, if the place has one.
    pub name: Option<String>,
    /// Where `name` came from; `None` exactly when `name` is.
    pub name_source: Option<PlaceNameSource>,
    /// The clustering version that computed the place.
    pub version: u32,
    /// When the place was last computed.
    pub computed_at: DateTime<Utc>,
    /// When the place was first created; kept across re-clustering.
    pub created_at: DateTime<Utc>,
}

/// One `athlete_place_runs` row: what the last clustering of a user saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceRun {
    /// How many cached rows the user held when the clustering read them.
    pub rows_seen: u32,
    /// The newest `synced_at` among those rows; `None` when there were none.
    pub newest_synced_at: Option<DateTime<Utc>>,
    /// The clustering version that ran.
    pub version: u32,
    /// When it ran. A failed recompute moves it too
    /// ([`AthletePlaceRepository::record_failed_recompute`]), so it reads as
    /// when the user was last tried.
    pub computed_at: DateTime<Utc>,
}

/// How many cached rows a user holds in a tenant and the newest `synced_at`
/// among them: what a clustering records in its [`PlaceRun`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedRowsStamp {
    /// How many cached rows.
    pub rows: u32,
    /// The newest `synced_at` among them; `None` when there are none.
    pub newest_synced_at: Option<DateTime<Utc>>,
}

/// Persistence for athlete places.
#[async_trait]
pub trait AthletePlaceRepository: Send + Sync {
    /// Every place the user holds in the tenant, by id.
    ///
    /// # Errors
    /// Returns a database error when the read fails or a row does not decode.
    async fn list(&self, tenant_id: &TenantId, user_id: Uuid) -> AppResult<Vec<AthletePlace>>;

    /// The cached-row count and newest sync the user holds in the tenant.
    /// The places worker reads it before it reads the rows it clusters and
    /// records it as its [`PlaceRun`], so a row synced while the clustering
    /// runs makes the user due again instead of being taken as seen.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn cached_rows_stamp(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
    ) -> AppResult<CachedRowsStamp>;

    /// Replace the user's places with `places`, set each assigned cached
    /// row's `place_id` (`None` clears it), and record `run`, in one
    /// transaction.
    ///
    /// An athlete-named place keeps the name and source the table holds, and
    /// one `places` leaves out is kept as it is; no other place survives.
    /// A name `places` gives with [`PlaceNameSource::Athlete`] is not written:
    /// only [`Self::set_name`] and [`Self::create_athlete_place`] write an
    /// athlete's name. A row whose `place_id` names no place left is cleared.
    /// A place id another user's place holds is refused, and nothing is
    /// written.
    ///
    /// Only the places worker calls it (`check-activity-writers.sh`).
    ///
    /// # Errors
    /// Returns a database error when a write fails or an id is another
    /// user's; the transaction is then rolled back.
    async fn replace_clustering(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        places: &[AthletePlace],
        assignments: &[(CachedRowKey, Option<String>)],
        run: &PlaceRun,
    ) -> AppResult<()>;

    /// Name one of the user's places as the athlete asked (`Some`), or clear
    /// the athlete's name (`None`), which leaves the place unnamed until the
    /// next clustering gives it its automatic name. Clearing a place the
    /// athlete never named leaves its automatic name. Returns the place as
    /// stored, or `None` when the user holds no such place in the tenant.
    ///
    /// # Errors
    /// Returns a database error when the write or the read back fails.
    async fn set_name(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        place_id: &str,
        name: Option<&str>,
    ) -> AppResult<Option<AthletePlace>>;

    /// Create a place the athlete named at one activity's start, with that
    /// activity's cached row (`member`) as its only member, in one
    /// transaction: the single-visit path of `name_place`.
    ///
    /// Only the places service calls it (`check-activity-writers.sh`).
    ///
    /// # Errors
    /// Returns a not-found error, writing nothing, when the user holds no
    /// cached row under `member` in the tenant; a database error when a write
    /// fails.
    async fn create_athlete_place(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        place: &AthletePlace,
        member: &CachedRowKey,
    ) -> AppResult<()>;

    /// Up to `limit` users, across every tenant, whose places are due a
    /// recompute at `now`, in one statement over column aggregates: no runs
    /// row, a run of another `version`, a run older than 24 hours, a cached
    /// row count or newest sync that differs from the run's, or a run that
    /// saw rows the user no longer holds.
    ///
    /// Users never tried come first, then the one tried longest ago, so a
    /// user whose recompute keeps failing goes to the back of the queue
    /// ([`Self::record_failed_recompute`]) and never holds a slot others are
    /// due. A row whose tenant or user id is not a uuid is logged with its
    /// ids, skipped, and takes no slot.
    ///
    /// Cross-tenant, as the capture freshness snapshot is: each user carries
    /// its own tenant forward.
    ///
    /// # Errors
    /// Returns a database error when the read fails or a row does not decode.
    async fn due_users(
        &self,
        limit: u32,
        version: u32,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<(TenantId, Uuid)>>;

    /// Record that the user's places recompute failed at `now`: their run's
    /// `computed_at` moves to `now` and nothing else changes, so
    /// [`Self::due_users`] puts them behind every user tried earlier. A user
    /// due for new rows, another version or an erasure stays due; one due
    /// only for the 24-hour backstop waits another day. A user with no run
    /// gets one that saw nothing at `version`, which keeps them due while
    /// they hold rows.
    ///
    /// Only the places service calls it.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn record_failed_recompute(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        version: u32,
        now: DateTime<Utc>,
    ) -> AppResult<()>;
}

/// Every place of `$2` (user) in `$1` (tenant), by id.
pub(crate) const LIST_PLACES_SQL: &str = r"
    SELECT id, centroid_lat, centroid_lon, radius_m, activity_count, name, name_source,
           version, computed_at, created_at
    FROM athlete_places
    WHERE tenant_id = $1 AND user_id = $2
    ORDER BY id";

/// One place of `$2` (user) in `$1` (tenant).
pub(crate) const GET_PLACE_SQL: &str = r"
    SELECT id, centroid_lat, centroid_lon, radius_m, activity_count, name, name_source,
           version, computed_at, created_at
    FROM athlete_places
    WHERE tenant_id = $1 AND user_id = $2 AND id = $3";

/// The count and newest sync of the cached rows of `$2` (user) in `$1` (tenant).
pub(crate) const CACHED_ROWS_STAMP_SQL: &str = r"
    SELECT COUNT(*) AS n, MAX(synced_at) AS newest_synced_at
    FROM cached_activities
    WHERE tenant_id = $1 AND user_id = $2";

/// Drop the user's places no athlete named; an athlete-named place stays
/// until the athlete clears its name.
pub(crate) const DELETE_UNNAMED_PLACES_SQL: &str = r"
    DELETE FROM athlete_places
    WHERE tenant_id = $1 AND user_id = $2
      AND (name_source IS NULL OR name_source <> 'athlete')";

/// Write one clustered place (`$1` id, `$2` tenant, `$3` user, then its
/// columns). A place still in the table is an athlete-named one, and keeps
/// its name, source and creation; a clustering never writes an athlete name
/// (`$9` is the source, and an `athlete` one writes no name). The update
/// applies only to the same user's place in the same tenant, so an id
/// another user holds updates nothing.
pub(crate) const UPSERT_CLUSTERED_PLACE_SQL: &str = r"
    INSERT INTO athlete_places (
        id, tenant_id, user_id, centroid_lat, centroid_lon, radius_m, activity_count,
        name, name_source, version, computed_at, created_at
    )
    VALUES (
        $1, $2, $3, $4, $5, $6, $7,
        CASE WHEN $9 = 'athlete' THEN NULL ELSE $8 END,
        CASE WHEN $9 = 'athlete' THEN NULL ELSE $9 END,
        $10, $11, $12
    )
    ON CONFLICT (id) DO UPDATE SET
        centroid_lat = EXCLUDED.centroid_lat,
        centroid_lon = EXCLUDED.centroid_lon,
        radius_m = EXCLUDED.radius_m,
        activity_count = EXCLUDED.activity_count,
        version = EXCLUDED.version,
        computed_at = EXCLUDED.computed_at
    WHERE athlete_places.tenant_id = EXCLUDED.tenant_id
      AND athlete_places.user_id = EXCLUDED.user_id";

/// Write the athlete's own new place, with its name and source as given.
pub(crate) const INSERT_ATHLETE_PLACE_SQL: &str = r"
    INSERT INTO athlete_places (
        id, tenant_id, user_id, centroid_lat, centroid_lon, radius_m, activity_count,
        name, name_source, version, computed_at, created_at
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)";

/// Record what the user's clustering saw, replacing the run before it.
pub(crate) const UPSERT_PLACE_RUN_SQL: &str = r"
    INSERT INTO athlete_place_runs (tenant_id, user_id, rows_seen, newest_synced_at, version, computed_at)
    VALUES ($1, $2, $3, $4, $5, $6)
    ON CONFLICT (tenant_id, user_id) DO UPDATE SET
        rows_seen = EXCLUDED.rows_seen,
        newest_synced_at = EXCLUDED.newest_synced_at,
        version = EXCLUDED.version,
        computed_at = EXCLUDED.computed_at";

/// Name one place of `$2` (user) in `$1` (tenant) as the athlete asked.
pub(crate) const NAME_PLACE_SQL: &str = r"
    UPDATE athlete_places
    SET name = $4, name_source = 'athlete'
    WHERE tenant_id = $1 AND user_id = $2 AND id = $3";

/// Clear the athlete's name of one place; an automatic name stays.
pub(crate) const CLEAR_PLACE_NAME_SQL: &str = r"
    UPDATE athlete_places
    SET name = CASE WHEN name_source = 'athlete' THEN NULL ELSE name END,
        name_source = CASE WHEN name_source = 'athlete' THEN NULL ELSE name_source END
    WHERE tenant_id = $1 AND user_id = $2 AND id = $3";

/// Move the run of `$2` (user) in `$1` (tenant) to `$4` after a failed
/// recompute, or record one that saw nothing at version `$3`.
pub(crate) const RECORD_FAILED_PLACE_RUN_SQL: &str = r"
    INSERT INTO athlete_place_runs (tenant_id, user_id, rows_seen, newest_synced_at, version, computed_at)
    VALUES ($1, $2, 0, NULL, $3, $4)
    ON CONFLICT (tenant_id, user_id) DO UPDATE SET computed_at = EXCLUDED.computed_at";

/// The users whose places are due at `$2` (24 hours before now), up to `$3`,
/// for clustering version `$1`, across every tenant: never tried first, then
/// tried longest ago. The first branch aggregates each user's cached rows
/// against their run; the second finds a run that saw rows the user no
/// longer holds at all. No JSON is read. The NULL order is spelled out,
/// since the engines sort a NULL at opposite ends.
pub(crate) const DUE_PLACE_USERS_SQL: &str = r"
    SELECT tenant_id, user_id FROM (
        SELECT ca.tenant_id, ca.user_id, r.computed_at
        FROM cached_activities ca
        LEFT JOIN athlete_place_runs r
               ON r.tenant_id = ca.tenant_id AND r.user_id = ca.user_id
        GROUP BY ca.tenant_id, ca.user_id, r.rows_seen, r.newest_synced_at, r.version,
                 r.computed_at
        HAVING r.version IS NULL
            OR r.version <> $1
            OR r.computed_at < $2
            OR COUNT(*) <> r.rows_seen
            OR r.newest_synced_at IS NULL
            OR MAX(ca.synced_at) > r.newest_synced_at
        UNION
        SELECT r.tenant_id, r.user_id, r.computed_at
        FROM athlete_place_runs r
        WHERE r.rows_seen > 0
          AND NOT EXISTS (
              SELECT 1 FROM cached_activities ca
              WHERE ca.tenant_id = r.tenant_id AND ca.user_id = r.user_id
          )
    ) due
    ORDER BY CASE WHEN computed_at IS NULL THEN 0 ELSE 1 END, computed_at, tenant_id, user_id
    LIMIT $3";

/// One stored integer column as the `u32` it was written from.
fn count_column<R>(row: &R, column: &str) -> AppResult<u32>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let value: i32 = row
        .try_get(column)
        .map_err(|e| AppError::database(format!("athlete place col {column}: {e}")))?;
    u32::try_from(value)
        .map_err(|_| AppError::database(format!("athlete place col {column} is {value}")))
}

/// Read an [`AthletePlace`] out of one row of [`LIST_PLACES_SQL`] or
/// [`GET_PLACE_SQL`].
///
/// # Errors
/// Returns a database error naming the column that cannot be decoded.
pub(crate) fn athlete_place_from_row<R>(row: &R) -> AppResult<AthletePlace>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    fn col<R, T>(row: &R, column: &str) -> AppResult<T>
    where
        R: sqlx::Row,
        for<'a> &'a str: sqlx::ColumnIndex<R>,
        T: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    {
        row.try_get(column)
            .map_err(|e| AppError::database(format!("athlete place col {column}: {e}")))
    }
    let name_source = col::<R, Option<String>>(row, "name_source")?
        .map(|slug| {
            PlaceNameSource::from_slug(&slug).ok_or_else(|| {
                AppError::database(format!("athlete place name_source {slug:?} is unknown"))
            })
        })
        .transpose()?;
    Ok(AthletePlace {
        id: col(row, "id")?,
        centroid_lat: col(row, "centroid_lat")?,
        centroid_lon: col(row, "centroid_lon")?,
        radius_m: col(row, "radius_m")?,
        activity_count: count_column(row, "activity_count")?,
        name: col(row, "name")?,
        name_source,
        version: count_column(row, "version")?,
        computed_at: col(row, "computed_at")?,
        created_at: col(row, "created_at")?,
    })
}

/// The users one read of [`DUE_PLACE_USERS_SQL`] found, in order, and the
/// raw `(tenant_id, user_id)` of each row it skipped because an id is not a
/// uuid.
pub(crate) struct DueRead {
    pub(crate) users: Vec<(TenantId, Uuid)>,
    pub(crate) skipped: Vec<(String, String)>,
}

/// Read the rows of [`DUE_PLACE_USERS_SQL`], setting aside a row whose ids
/// are not uuids instead of failing the fleet's read on it.
///
/// # Errors
/// Returns a database error when an id column cannot be decoded.
pub(crate) fn due_users_from_rows<R>(rows: &[R]) -> AppResult<DueRead>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let mut read = DueRead {
        users: Vec::with_capacity(rows.len()),
        skipped: Vec::new(),
    };
    for row in rows {
        let id = |column: &str| -> AppResult<String> {
            row.try_get(column)
                .map_err(|e| AppError::database(format!("place run col {column}: {e}")))
        };
        let (tenant, user) = (id("tenant_id")?, id("user_id")?);
        match (TenantId::parse_str(&tenant), Uuid::parse_str(&user)) {
            (Ok(tenant_id), Ok(user_id)) => read.users.push((tenant_id, user_id)),
            _ => read.skipped.push((tenant, user)),
        }
    }
    Ok(read)
}

/// Log each row a due read skipped, by its ids alone.
pub(crate) fn log_skipped_due_users(skipped: &[(String, String)]) {
    for (tenant_id, user_id) in skipped {
        warn!(
            tenant_id = %tenant_id,
            user_id = %user_id,
            "a user due a places recompute has an id that is not a uuid; skipped"
        );
    }
}

/// Read a [`CachedRowsStamp`] out of the row of [`CACHED_ROWS_STAMP_SQL`].
///
/// # Errors
/// Returns a database error when a column cannot be decoded.
pub(crate) fn cached_rows_stamp_from_row<R>(row: &R) -> AppResult<CachedRowsStamp>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let rows: i64 = row
        .try_get("n")
        .map_err(|e| AppError::database(format!("cached rows stamp col n: {e}")))?;
    Ok(CachedRowsStamp {
        rows: u32::try_from(rows).unwrap_or(u32::MAX),
        newest_synced_at: row.try_get("newest_synced_at").map_err(|e| {
            AppError::database(format!("cached rows stamp col newest_synced_at: {e}"))
        })?,
    })
}

/// A count as the INTEGER column holds it.
///
/// # Errors
/// Returns a database error when `value` exceeds the column.
pub(crate) fn integer_column(value: u32, what: &str) -> AppResult<i32> {
    i32::try_from(value)
        .map_err(|_| AppError::database(format!("{what} {value} exceeds its column")))
}

/// Emit the whole [`AthletePlaceRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion. The body names its statements, helpers and types bare; each
/// shell imports them, as the `activity_route_tracks` shells do.
macro_rules! impl_athlete_place_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl AthletePlaceRepository for $ty {
            async fn list(
                &self,
                tenant_id: &TenantId,
                user_id: uuid::Uuid,
            ) -> AppResult<Vec<AthletePlace>> {
                let rows = sqlx::query(LIST_PLACES_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list athlete places: {e}"))
                    })?;
                rows.iter().map(athlete_place_from_row).collect()
            }

            async fn cached_rows_stamp(
                &self,
                tenant_id: &TenantId,
                user_id: uuid::Uuid,
            ) -> AppResult<CachedRowsStamp> {
                let row = sqlx::query(CACHED_ROWS_STAMP_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read cached rows stamp: {e}"))
                    })?;
                cached_rows_stamp_from_row(&row)
            }

            async fn replace_clustering(
                &self,
                tenant_id: &TenantId,
                user_id: uuid::Uuid,
                places: &[AthletePlace],
                assignments: &[(CachedRowKey, Option<String>)],
                run: &PlaceRun,
            ) -> AppResult<()> {
                let tenant = tenant_id.to_string();
                let user = user_id.to_string();
                let failed = |what: &str, e: sqlx::Error| {
                    AppError::database(format!("Failed to replace places ({what}): {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(|e| failed("begin", e))?;
                sqlx::query(DELETE_UNNAMED_PLACES_SQL)
                    .bind(&tenant)
                    .bind(&user)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| failed("delete", e))?;
                for place in places {
                    let written = sqlx::query(UPSERT_CLUSTERED_PLACE_SQL)
                        .bind(&place.id)
                        .bind(&tenant)
                        .bind(&user)
                        .bind(place.centroid_lat)
                        .bind(place.centroid_lon)
                        .bind(place.radius_m)
                        .bind(integer_column(place.activity_count, "activity count")?)
                        .bind(place.name.as_deref())
                        .bind(place.name_source.map(PlaceNameSource::as_str))
                        .bind(integer_column(place.version, "places version")?)
                        .bind(place.computed_at)
                        .bind(place.created_at)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| failed("place", e))?
                        .rows_affected();
                    if written == 0 {
                        return Err(AppError::database(format!(
                            "place id {} is held by another user's place",
                            place.id
                        )));
                    }
                }
                for (key, place_id) in assignments {
                    sqlx::query(ASSIGN_PLACE_SQL)
                        .bind(&user)
                        .bind(&tenant)
                        .bind(&key.provider)
                        .bind(&key.activity_id)
                        .bind(place_id.as_deref())
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| failed("assign", e))?;
                }
                sqlx::query(CLEAR_DANGLING_PLACES_SQL)
                    .bind(&tenant)
                    .bind(&user)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| failed("clear", e))?;
                sqlx::query(UPSERT_PLACE_RUN_SQL)
                    .bind(&tenant)
                    .bind(&user)
                    .bind(integer_column(run.rows_seen, "rows seen")?)
                    .bind(run.newest_synced_at)
                    .bind(integer_column(run.version, "places version")?)
                    .bind(run.computed_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| failed("run", e))?;
                tx.commit().await.map_err(|e| failed("commit", e))
            }

            async fn set_name(
                &self,
                tenant_id: &TenantId,
                user_id: uuid::Uuid,
                place_id: &str,
                name: Option<&str>,
            ) -> AppResult<Option<AthletePlace>> {
                let tenant = tenant_id.to_string();
                let user = user_id.to_string();
                let write = match name {
                    Some(name) => sqlx::query(NAME_PLACE_SQL)
                        .bind(&tenant)
                        .bind(&user)
                        .bind(place_id)
                        .bind(name),
                    None => sqlx::query(CLEAR_PLACE_NAME_SQL)
                        .bind(&tenant)
                        .bind(&user)
                        .bind(place_id),
                };
                write.execute(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to name athlete place: {e}"))
                })?;
                let row = sqlx::query(GET_PLACE_SQL)
                    .bind(&tenant)
                    .bind(&user)
                    .bind(place_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read athlete place: {e}"))
                    })?;
                row.as_ref().map(athlete_place_from_row).transpose()
            }

            async fn create_athlete_place(
                &self,
                tenant_id: &TenantId,
                user_id: uuid::Uuid,
                place: &AthletePlace,
                member: &CachedRowKey,
            ) -> AppResult<()> {
                let tenant = tenant_id.to_string();
                let user = user_id.to_string();
                let failed = |what: &str, e: sqlx::Error| {
                    AppError::database(format!("Failed to create athlete place ({what}): {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(|e| failed("begin", e))?;
                sqlx::query(INSERT_ATHLETE_PLACE_SQL)
                    .bind(&place.id)
                    .bind(&tenant)
                    .bind(&user)
                    .bind(place.centroid_lat)
                    .bind(place.centroid_lon)
                    .bind(place.radius_m)
                    .bind(integer_column(place.activity_count, "activity count")?)
                    .bind(place.name.as_deref())
                    .bind(place.name_source.map(PlaceNameSource::as_str))
                    .bind(integer_column(place.version, "places version")?)
                    .bind(place.computed_at)
                    .bind(place.created_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| failed("place", e))?;
                let assigned = sqlx::query(ASSIGN_PLACE_SQL)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(&member.provider)
                    .bind(&member.activity_id)
                    .bind(&place.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| failed("assign", e))?
                    .rows_affected();
                if assigned == 0 {
                    tx.rollback().await.map_err(|e| failed("rollback", e))?;
                    return Err(AppError::not_found(format!(
                        "cached activity {}/{}",
                        member.provider, member.activity_id
                    )));
                }
                tx.commit().await.map_err(|e| failed("commit", e))
            }

            async fn due_users(
                &self,
                limit: u32,
                version: u32,
                now: chrono::DateTime<chrono::Utc>,
            ) -> AppResult<Vec<(TenantId, uuid::Uuid)>> {
                let version = integer_column(version, "places version")?;
                let stale_before = now - chrono::Duration::hours(24);
                // A skipped row takes no slot: read again with room for every
                // row skipped so far, until a read skips no more than it made
                // room for or comes back short.
                let mut room: usize = 0;
                loop {
                    let asked = usize::try_from(limit).unwrap_or(usize::MAX) + room;
                    let rows = sqlx::query(DUE_PLACE_USERS_SQL)
                        .bind(version)
                        .bind(stale_before)
                        .bind(i64::try_from(asked).unwrap_or(i64::MAX))
                        .fetch_all(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to read users due places: {e}"))
                        })?;
                    let mut read = due_users_from_rows(&rows)?;
                    if read.skipped.len() <= room || rows.len() < asked {
                        log_skipped_due_users(&read.skipped);
                        read.users
                            .truncate(usize::try_from(limit).unwrap_or(usize::MAX));
                        return Ok(read.users);
                    }
                    room = read.skipped.len();
                }
            }

            async fn record_failed_recompute(
                &self,
                tenant_id: &TenantId,
                user_id: uuid::Uuid,
                version: u32,
                now: chrono::DateTime<chrono::Utc>,
            ) -> AppResult<()> {
                sqlx::query(RECORD_FAILED_PLACE_RUN_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(integer_column(version, "places version")?)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to record a failed places recompute: {e}"
                        ))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_athlete_place_repository;
