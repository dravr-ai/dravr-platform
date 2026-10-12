// ABOUTME: Repository trait, statements and shared body for the athlete's gear ledger: declared pairs and per-activity attribution
// ABOUTME: Every statement is scoped by user_id; emitted per backend by macro with that backend's row type and uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The gear ledger's storage (carnet#711).
//!
//! `athlete_gear` holds the pairs an athlete declared, `activity_gear` the
//! pair each activity copy is attributed to. Neither table carries a
//! `tenant_id`: the ledger is the person's, whichever tenant they are signed
//! in to, and every statement here is scoped by `user_id`. A pair's distance
//! is read in the same statement as the pair: its declared distance plus the
//! distance stored on each attribution.
//!
//! Which pair a new activity takes, and which copies of one workout an
//! attribution covers, is decided by the caller; this layer stores what it is
//! told, refuses to delete a pair any activity is attributed to, and keeps at
//! most one pinned pair per sport.

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    parse_shoe_sport, ActivityGearKey, AthleteGear, AttributionSource, GearAttribution,
    GearDeclaration, GearKind,
};
use uuid::Uuid;

/// What a request to delete a pair did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GearDeletion {
    /// The pair is gone.
    Deleted,
    /// The athlete holds no such pair.
    NotFound,
    /// An activity is attributed to the pair, so it stays; retiring it keeps
    /// its history and takes it out of every choice.
    Attributed,
}

/// Read and write an athlete's gear ledger.
///
/// The tables carry no `tenant_id`; every statement is scoped by `user_id`.
#[async_trait]
pub trait AthleteGearRepository: Send + Sync {
    /// Every pair the athlete declared, retired ones included, newest first,
    /// each with the distance, count and latest start of the activities
    /// attributed to it.
    async fn list_gear(&self, user_id: Uuid) -> AppResult<Vec<AthleteGear>>;

    /// One of the athlete's pairs with its history, `None` when they hold no
    /// pair by that id.
    async fn get_gear(&self, user_id: Uuid, gear_id: Uuid) -> AppResult<Option<AthleteGear>>;

    /// Declare a pair, in use from `added_at`, and return its id. A pinned
    /// pair unpins whichever pair of its sport was pinned before.
    ///
    /// # Errors
    /// An invalid-input error when the athlete already holds
    /// [`pierre_core::models::MAX_GEAR_PER_ATHLETE`] pairs.
    async fn create_gear(
        &self,
        user_id: Uuid,
        declaration: &GearDeclaration,
        added_at: DateTime<Utc>,
    ) -> AppResult<Uuid>;

    /// Replace what the athlete declared about a pair; its history stays.
    /// Returns whether the athlete holds the pair. Pinning it unpins whichever
    /// other pair of its sport was pinned.
    async fn update_gear(
        &self,
        user_id: Uuid,
        gear_id: Uuid,
        declaration: &GearDeclaration,
    ) -> AppResult<bool>;

    /// Retire a pair at `retired_at`, which also unpins it, or put a retired
    /// pair back in use with `None`. Returns whether the athlete holds the
    /// pair.
    async fn set_gear_retired(
        &self,
        user_id: Uuid,
        gear_id: Uuid,
        retired_at: Option<DateTime<Utc>>,
    ) -> AppResult<bool>;

    /// Delete a pair no activity is attributed to.
    async fn delete_gear(&self, user_id: Uuid, gear_id: Uuid) -> AppResult<GearDeletion>;

    /// The athlete's attributions of activities that started in `[from, to]`.
    async fn list_gear_attributions(
        &self,
        user_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> AppResult<Vec<GearAttribution>>;

    /// Record the default pair for each activity copy that holds no
    /// attribution yet, in one transaction; a copy already attributed keeps
    /// what it holds. Returns how many were recorded.
    async fn record_default_attributions(
        &self,
        user_id: Uuid,
        attributions: &[GearAttribution],
    ) -> AppResult<u64>;

    /// Record the athlete's own choice for one activity, replacing what that
    /// copy held, and clear the attributions of `replaces` — the workout's
    /// other copies — in the same transaction, so one workout counts once.
    async fn declare_gear_attribution(
        &self,
        user_id: Uuid,
        attribution: &GearAttribution,
        replaces: &[ActivityGearKey],
    ) -> AppResult<()>;

    /// Let go of what the copy `released` is attributed to, before that copy
    /// is deleted: `successor` — another copy of the same workout — takes the
    /// attribution over unless it holds one of its own, and `released` holds
    /// nothing afterwards, in one transaction. A workout the athlete deleted
    /// then adds nothing to a pair, and one still held by another copy keeps
    /// its pair without counting twice.
    async fn release_gear_attribution(
        &self,
        user_id: Uuid,
        released: &ActivityGearKey,
        successor: Option<&ActivityGearKey>,
    ) -> AppResult<()>;
}

/// The columns a pair is read with: what was declared, then its history from
/// the attributions that name it. `$1` binds the user.
macro_rules! gear_select {
    () => {
        r"
            SELECT g.id, g.kind, g.name, g.sport, g.initial_distance_meters,
                   g.warn_distance_meters, g.pinned, g.added_at, g.retired_at,
                   (SELECT COALESCE(SUM(a.distance_meters), CAST(0 AS DOUBLE PRECISION))
                      FROM activity_gear a
                     WHERE a.user_id = g.user_id AND a.gear_id = g.id) AS attributed_distance_meters,
                   (SELECT COUNT(*)
                      FROM activity_gear a
                     WHERE a.user_id = g.user_id AND a.gear_id = g.id) AS activity_count,
                   (SELECT MAX(a.start_date)
                      FROM activity_gear a
                     WHERE a.user_id = g.user_id AND a.gear_id = g.id) AS last_used_at
            FROM athlete_gear g
            WHERE g.user_id = $1
            "
    };
}

/// Every pair of `$1`, newest first.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends. The ids bind through the
/// backend's uuid codec (see [`super::uuid_columns`]); timestamps bind as
/// `DateTime<Utc>`, the flag as `bool`, and each driver encodes them for its
/// own column type. The zero the sum takes over no attribution is cast to the
/// floating type on both engines, so the column always decodes as `f64`.
pub(crate) const LIST_GEAR_SQL: &str = concat!(gear_select!(), "ORDER BY g.added_at DESC, g.id");

/// One pair of `$1` by its id `$2`.
pub(crate) const GET_GEAR_SQL: &str = concat!(gear_select!(), "AND g.id = $2");

/// How many pairs `$1` holds.
pub(crate) const COUNT_GEAR_SQL: &str =
    "SELECT COUNT(*) AS pairs FROM athlete_gear WHERE user_id = $1";

/// Declare a pair: `$1` id, `$2` user, `$3` kind, `$4` name, `$5` sport, `$6`
/// declared distance, `$7` warning distance, `$8` pinned, `$9` when.
pub(crate) const INSERT_GEAR_SQL: &str = r"
            INSERT INTO athlete_gear (
                id, user_id, kind, name, sport, initial_distance_meters,
                warn_distance_meters, pinned, added_at, retired_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, $9)
            ";

/// Unpin `$1`'s pinned pair of sport `$2` unless it is `$3`, at `$4`.
pub(crate) const UNPIN_SPORT_SQL: &str = r"
            UPDATE athlete_gear
            SET pinned = FALSE, updated_at = $4
            WHERE user_id = $1 AND sport = $2 AND pinned = TRUE AND id <> $3
            ";

/// Replace what `$1` declared about pair `$2`: `$3` kind, `$4` name, `$5`
/// sport, `$6` declared distance, `$7` warning distance, `$8` pinned, `$9`
/// when.
pub(crate) const UPDATE_GEAR_SQL: &str = r"
            UPDATE athlete_gear
            SET kind = $3, name = $4, sport = $5, initial_distance_meters = $6,
                warn_distance_meters = $7, pinned = $8, updated_at = $9
            WHERE user_id = $1 AND id = $2
            ";

/// Retire `$1`'s pair `$2` at `$3`, unpinning it.
pub(crate) const RETIRE_GEAR_SQL: &str = r"
            UPDATE athlete_gear
            SET retired_at = $3, pinned = FALSE, updated_at = $3
            WHERE user_id = $1 AND id = $2
            ";

/// Put `$1`'s retired pair `$2` back in use, at `$3`.
pub(crate) const RESTORE_GEAR_SQL: &str = r"
            UPDATE athlete_gear
            SET retired_at = NULL, updated_at = $3
            WHERE user_id = $1 AND id = $2
            ";

/// Delete `$1`'s pair `$2` while no activity is attributed to it.
pub(crate) const DELETE_GEAR_SQL: &str = r"
            DELETE FROM athlete_gear
            WHERE user_id = $1 AND id = $2
              AND NOT EXISTS (
                  SELECT 1 FROM activity_gear WHERE user_id = $1 AND gear_id = $2
              )
            ";

/// `$1`'s attributions of activities that started in `[$2, $3]`.
pub(crate) const LIST_ATTRIBUTIONS_SQL: &str = r"
            SELECT provider, activity_id, gear_id, source, start_date, distance_meters
            FROM activity_gear
            WHERE user_id = $1 AND start_date >= $2 AND start_date <= $3
            ";

/// Record a default for one copy unless it already holds an attribution: `$1`
/// user, `$2` provider, `$3` activity, `$4` pair, `$5` source, `$6` start,
/// `$7` distance, `$8` when.
pub(crate) const INSERT_DEFAULT_ATTRIBUTION_SQL: &str = r"
            INSERT INTO activity_gear (
                user_id, provider, activity_id, gear_id, source, start_date,
                distance_meters, attributed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (user_id, provider, activity_id) DO NOTHING
            ";

/// Record the athlete's choice for one copy, replacing what it held; binds as
/// [`INSERT_DEFAULT_ATTRIBUTION_SQL`].
pub(crate) const UPSERT_DECLARED_ATTRIBUTION_SQL: &str = r"
            INSERT INTO activity_gear (
                user_id, provider, activity_id, gear_id, source, start_date,
                distance_meters, attributed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (user_id, provider, activity_id) DO UPDATE SET
                gear_id = EXCLUDED.gear_id,
                source = EXCLUDED.source,
                start_date = EXCLUDED.start_date,
                distance_meters = EXCLUDED.distance_meters,
                attributed_at = EXCLUDED.attributed_at
            ";

/// Clear `$1`'s attribution of copy `$3` from provider `$2`.
pub(crate) const DELETE_ATTRIBUTION_SQL: &str = r"
            DELETE FROM activity_gear
            WHERE user_id = $1 AND provider = $2 AND activity_id = $3
            ";

/// Copy `$1`'s attribution of copy `$3` from provider `$2` onto copy `$5`
/// from provider `$4`, unless that copy already holds one. The `WHERE` clause
/// is what lets `SQLite` read the `ON CONFLICT` as the insert's upsert rather
/// than a join constraint of the `SELECT`.
pub(crate) const HAND_OVER_ATTRIBUTION_SQL: &str = r"
            INSERT INTO activity_gear (
                user_id, provider, activity_id, gear_id, source, start_date,
                distance_meters, attributed_at
            )
            SELECT user_id, $4, $5, gear_id, source, start_date, distance_meters,
                   attributed_at
            FROM activity_gear
            WHERE user_id = $1 AND provider = $2 AND activity_id = $3
            ON CONFLICT (user_id, provider, activity_id) DO NOTHING
            ";

/// The error for a read or write of the ledger that failed.
pub(crate) fn gear_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} the gear ledger: {e}"))
}

/// Decode a pair row; its id arrives already read through the backend's
/// codec. `try_get` throughout, so a corrupt row surfaces as a recoverable
/// error naming its column.
///
/// # Errors
/// A database error naming the first column that cannot be decoded, or a
/// kind or sport no pair is stored with.
pub(crate) fn gear_from_row<R>(row: &R, id: Uuid) -> AppResult<AthleteGear>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| gear_error(&format!("decode {col} of"), e);
    let kind: String = row.try_get("kind").map_err(|e| column("kind", e))?;
    let sport: String = row.try_get("sport").map_err(|e| column("sport", e))?;
    let activity_count: i64 = row
        .try_get("activity_count")
        .map_err(|e| column("activity_count", e))?;
    Ok(AthleteGear {
        id,
        declaration: GearDeclaration {
            kind: GearKind::parse(&kind)
                .ok_or_else(|| AppError::database(format!("Unknown gear kind: {kind}")))?,
            name: row.try_get("name").map_err(|e| column("name", e))?,
            sport: parse_shoe_sport(&sport)
                .ok_or_else(|| AppError::database(format!("Unknown gear sport: {sport}")))?,
            initial_distance_meters: row
                .try_get("initial_distance_meters")
                .map_err(|e| column("initial_distance_meters", e))?,
            warn_distance_meters: row
                .try_get("warn_distance_meters")
                .map_err(|e| column("warn_distance_meters", e))?,
            pinned: row.try_get("pinned").map_err(|e| column("pinned", e))?,
        },
        added_at: row.try_get("added_at").map_err(|e| column("added_at", e))?,
        retired_at: row
            .try_get("retired_at")
            .map_err(|e| column("retired_at", e))?,
        attributed_distance_meters: row
            .try_get("attributed_distance_meters")
            .map_err(|e| column("attributed_distance_meters", e))?,
        activity_count: u64::try_from(activity_count).unwrap_or_default(),
        last_used_at: row
            .try_get("last_used_at")
            .map_err(|e| column("last_used_at", e))?,
    })
}

/// Decode an attribution row; its pair id arrives already read through the
/// backend's codec.
///
/// # Errors
/// A database error naming the first column that cannot be decoded, or a
/// source no attribution is stored with.
pub(crate) fn attribution_from_row<R>(row: &R, gear_id: Option<Uuid>) -> AppResult<GearAttribution>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| gear_error(&format!("decode {col} of"), e);
    let source: String = row.try_get("source").map_err(|e| column("source", e))?;
    Ok(GearAttribution {
        key: ActivityGearKey {
            provider: row.try_get("provider").map_err(|e| column("provider", e))?,
            activity_id: row
                .try_get("activity_id")
                .map_err(|e| column("activity_id", e))?,
        },
        gear_id,
        source: AttributionSource::parse(&source)
            .ok_or_else(|| AppError::database(format!("Unknown attribution source: {source}")))?,
        start_date: row
            .try_get("start_date")
            .map_err(|e| column("start_date", e))?,
        distance_meters: row
            .try_get("distance_meters")
            .map_err(|e| column("distance_meters", e))?,
    })
}

/// Emit the whole [`AthleteGearRepository`] implementation for one backend
/// type. `$row` is the driver's row type and `$ids` the codec in
/// [`super::uuid_columns`] for how that backend's uuid columns bind. The body
/// names its consts, types and helpers unqualified, so the invoking shell must
/// `use` every one of them.
macro_rules! impl_athlete_gear_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        impl $ty {
            /// Decode a pair row through the backend's uuid codec.
            fn athlete_gear_row(row: &$row) -> AppResult<AthleteGear> {
                gear_from_row(row, $ids::read(row, "id")?)
            }

            /// Decode an attribution row through the backend's uuid codec.
            fn gear_attribution_row(row: &$row) -> AppResult<GearAttribution> {
                attribution_from_row(row, $ids::read_opt(row, "gear_id")?)
            }
        }

        #[async_trait::async_trait]
        impl AthleteGearRepository for $ty {
            async fn list_gear(&self, user_id: Uuid) -> AppResult<Vec<AthleteGear>> {
                let rows = sqlx::query(LIST_GEAR_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| gear_error("read", e))?;
                rows.iter().map(Self::athlete_gear_row).collect()
            }

            async fn get_gear(
                &self,
                user_id: Uuid,
                gear_id: Uuid,
            ) -> AppResult<Option<AthleteGear>> {
                let row = sqlx::query(GET_GEAR_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(gear_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| gear_error("read", e))?;
                row.map(|row| Self::athlete_gear_row(&row)).transpose()
            }

            async fn create_gear(
                &self,
                user_id: Uuid,
                declaration: &GearDeclaration,
                added_at: DateTime<Utc>,
            ) -> AppResult<Uuid> {
                let sport = shoe_sport_slug(&declaration.sport)
                    .ok_or_else(|| AppError::invalid_field("sport", "is not a shoe sport"))?;
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| gear_error("begin a write to", e))?;
                let pairs: i64 = sqlx::query_scalar(COUNT_GEAR_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| gear_error("count", e))?;
                if pairs >= MAX_GEAR_PER_ATHLETE {
                    return Err(AppError::invalid_input(format!(
                        "An athlete can declare at most {MAX_GEAR_PER_ATHLETE} pairs; delete or retire one first"
                    )));
                }
                let id = Uuid::new_v4();
                if declaration.pinned {
                    sqlx::query(UNPIN_SPORT_SQL)
                        .bind($ids::bind(user_id))
                        .bind(sport)
                        .bind($ids::bind(id))
                        .bind(added_at)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| gear_error("unpin a pair in", e))?;
                }
                sqlx::query(INSERT_GEAR_SQL)
                    .bind($ids::bind(id))
                    .bind($ids::bind(user_id))
                    .bind(declaration.kind.as_str())
                    .bind(declaration.name.as_str())
                    .bind(sport)
                    .bind(declaration.initial_distance_meters)
                    .bind(declaration.warn_distance_meters)
                    .bind(declaration.pinned)
                    .bind(added_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| gear_error("declare a pair in", e))?;
                tx.commit()
                    .await
                    .map_err(|e| gear_error("commit a write to", e))?;
                Ok(id)
            }

            async fn update_gear(
                &self,
                user_id: Uuid,
                gear_id: Uuid,
                declaration: &GearDeclaration,
            ) -> AppResult<bool> {
                let sport = shoe_sport_slug(&declaration.sport)
                    .ok_or_else(|| AppError::invalid_field("sport", "is not a shoe sport"))?;
                let now = Utc::now();
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| gear_error("begin a write to", e))?;
                if declaration.pinned {
                    sqlx::query(UNPIN_SPORT_SQL)
                        .bind($ids::bind(user_id))
                        .bind(sport)
                        .bind($ids::bind(gear_id))
                        .bind(now)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| gear_error("unpin a pair in", e))?;
                }
                let updated = sqlx::query(UPDATE_GEAR_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(gear_id))
                    .bind(declaration.kind.as_str())
                    .bind(declaration.name.as_str())
                    .bind(sport)
                    .bind(declaration.initial_distance_meters)
                    .bind(declaration.warn_distance_meters)
                    .bind(declaration.pinned)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| gear_error("update a pair in", e))?
                    .rows_affected();
                tx.commit()
                    .await
                    .map_err(|e| gear_error("commit a write to", e))?;
                Ok(updated == 1)
            }

            async fn set_gear_retired(
                &self,
                user_id: Uuid,
                gear_id: Uuid,
                retired_at: Option<DateTime<Utc>>,
            ) -> AppResult<bool> {
                let statement = match retired_at {
                    Some(at) => sqlx::query(RETIRE_GEAR_SQL)
                        .bind($ids::bind(user_id))
                        .bind($ids::bind(gear_id))
                        .bind(at),
                    None => sqlx::query(RESTORE_GEAR_SQL)
                        .bind($ids::bind(user_id))
                        .bind($ids::bind(gear_id))
                        .bind(Utc::now()),
                };
                let changed = statement
                    .execute(self.pool())
                    .await
                    .map_err(|e| gear_error("retire a pair in", e))?
                    .rows_affected();
                Ok(changed == 1)
            }

            async fn delete_gear(&self, user_id: Uuid, gear_id: Uuid) -> AppResult<GearDeletion> {
                let Some(pair) = self.get_gear(user_id, gear_id).await? else {
                    return Ok(GearDeletion::NotFound);
                };
                if pair.activity_count > 0 {
                    return Ok(GearDeletion::Attributed);
                }
                let deleted = sqlx::query(DELETE_GEAR_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(gear_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| gear_error("delete a pair from", e))?
                    .rows_affected();
                // Zero rows here means an activity was attributed to the pair
                // between the read and the delete: the statement's own guard
                // kept it.
                Ok(if deleted == 1 {
                    GearDeletion::Deleted
                } else {
                    GearDeletion::Attributed
                })
            }

            async fn list_gear_attributions(
                &self,
                user_id: Uuid,
                from: DateTime<Utc>,
                to: DateTime<Utc>,
            ) -> AppResult<Vec<GearAttribution>> {
                let rows = sqlx::query(LIST_ATTRIBUTIONS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(from)
                    .bind(to)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| gear_error("read the attributions of", e))?;
                rows.iter().map(Self::gear_attribution_row).collect()
            }

            async fn record_default_attributions(
                &self,
                user_id: Uuid,
                attributions: &[GearAttribution],
            ) -> AppResult<u64> {
                if attributions.is_empty() {
                    return Ok(0);
                }
                let now = Utc::now();
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| gear_error("begin a write to", e))?;
                let mut recorded = 0;
                for attribution in attributions {
                    recorded += sqlx::query(INSERT_DEFAULT_ATTRIBUTION_SQL)
                        .bind($ids::bind(user_id))
                        .bind(attribution.key.provider.as_str())
                        .bind(attribution.key.activity_id.as_str())
                        .bind($ids::bind_opt(attribution.gear_id))
                        .bind(attribution.source.as_str())
                        .bind(attribution.start_date)
                        .bind(attribution.distance_meters)
                        .bind(now)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| gear_error("record a default in", e))?
                        .rows_affected();
                }
                tx.commit()
                    .await
                    .map_err(|e| gear_error("commit a write to", e))?;
                Ok(recorded)
            }

            async fn declare_gear_attribution(
                &self,
                user_id: Uuid,
                attribution: &GearAttribution,
                replaces: &[ActivityGearKey],
            ) -> AppResult<()> {
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| gear_error("begin a write to", e))?;
                for key in replaces.iter().filter(|key| **key != attribution.key) {
                    sqlx::query(DELETE_ATTRIBUTION_SQL)
                        .bind($ids::bind(user_id))
                        .bind(key.provider.as_str())
                        .bind(key.activity_id.as_str())
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| gear_error("clear an attribution in", e))?;
                }
                sqlx::query(UPSERT_DECLARED_ATTRIBUTION_SQL)
                    .bind($ids::bind(user_id))
                    .bind(attribution.key.provider.as_str())
                    .bind(attribution.key.activity_id.as_str())
                    .bind($ids::bind_opt(attribution.gear_id))
                    .bind(attribution.source.as_str())
                    .bind(attribution.start_date)
                    .bind(attribution.distance_meters)
                    .bind(Utc::now())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| gear_error("record a choice in", e))?;
                tx.commit()
                    .await
                    .map_err(|e| gear_error("commit a write to", e))?;
                Ok(())
            }

            async fn release_gear_attribution(
                &self,
                user_id: Uuid,
                released: &ActivityGearKey,
                successor: Option<&ActivityGearKey>,
            ) -> AppResult<()> {
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| gear_error("begin a write to", e))?;
                if let Some(successor) = successor.filter(|key| *key != released) {
                    sqlx::query(HAND_OVER_ATTRIBUTION_SQL)
                        .bind($ids::bind(user_id))
                        .bind(released.provider.as_str())
                        .bind(released.activity_id.as_str())
                        .bind(successor.provider.as_str())
                        .bind(successor.activity_id.as_str())
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| gear_error("hand an attribution over in", e))?;
                }
                sqlx::query(DELETE_ATTRIBUTION_SQL)
                    .bind($ids::bind(user_id))
                    .bind(released.provider.as_str())
                    .bind(released.activity_id.as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| gear_error("clear an attribution in", e))?;
                tx.commit()
                    .await
                    .map_err(|e| gear_error("commit a write to", e))?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_athlete_gear_repository;
