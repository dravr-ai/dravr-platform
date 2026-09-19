// ABOUTME: Repository trait for the mobility catalogue (stretching exercises, yoga poses, activity-muscle mappings)
// ABOUTME: Every statement written once as a shared const, one generic row parser per table, one macro emitting each backend's impl
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::mobility::{
    ActivityMuscleMapping, DifficultyLevel, ListStretchingFilter, ListYogaFilter,
    StretchingCategory, StretchingExercise, YogaCategory, YogaPose, YogaPoseType,
};
use serde::de::DeserializeOwned;
use sqlx::{ColumnIndex, Decode, Row, Type};

/// Mobility (stretching exercises and yoga poses) read-only repository
#[async_trait]
pub trait MobilityRepository: Send + Sync {
    /// Get a stretching exercise by ID
    async fn get_stretching_exercise(&self, id: &str) -> AppResult<Option<StretchingExercise>>;
    /// List stretching exercises with optional filtering
    async fn list_stretching_exercises(
        &self,
        filter: &ListStretchingFilter,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Search stretching exercises by text query
    async fn search_stretching_exercises(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Get stretches recommended for a specific activity type
    async fn get_stretches_for_activity(
        &self,
        activity_type: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>>;
    /// Get a yoga pose by ID
    async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>>;
    /// List yoga poses with optional filtering
    async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>>;
    /// Search yoga poses by text query
    async fn search_yoga_poses(&self, query: &str, limit: Option<u32>) -> AppResult<Vec<YogaPose>>;
    /// Get yoga poses recommended for a recovery context
    async fn get_poses_for_recovery(
        &self,
        recovery_context: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<YogaPose>>;
    /// Get muscle mapping for a specific activity type
    async fn get_activity_muscle_mapping(
        &self,
        activity_type: &str,
    ) -> AppResult<Option<ActivityMuscleMapping>>;
    /// List all activity-to-muscle mappings
    async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>>;
}

// ============================================================================
// Statements
// ============================================================================
//
// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
// Postgres, and every bind on these tables is a plain `&str`/`String`/`i32`,
// so one statement serves both backends and cannot drift between them.
//
// The one thing the two engines spell differently is the case-folding text
// match: Postgres has `ILIKE`, `SQLite` only `LIKE`, which folds case for
// ASCII letters alone. Each backend's shell passes its operator as the
// `$like` macro literal; see [`impl_mobility_repository`].

/// The eighteen columns every read of `stretching_exercises` returns, in the
/// order [`stretching_from_row`] reads them.
macro_rules! stretching_columns {
    () => {
        "id, name, description, category, difficulty, \
         primary_muscles, secondary_muscles, duration_seconds, \
         repetitions, sets, recommended_for_activities, contraindications, \
         instructions, cues, image_url, video_url, created_at, updated_at"
    };
}
pub(crate) use stretching_columns;

/// The twenty-six columns every read of `yoga_poses` returns, in the order
/// [`yoga_pose_from_row`] reads them.
macro_rules! yoga_columns {
    () => {
        "id, english_name, sanskrit_name, description, benefits, \
         category, difficulty, pose_type, primary_muscles, secondary_muscles, \
         chakras, hold_duration_seconds, breath_guidance, \
         recommended_for_activities, recommended_for_recovery, contraindications, \
         instructions, modifications, progressions, cues, \
         warmup_poses, followup_poses, image_url, video_url, \
         created_at, updated_at"
    };
}
pub(crate) use yoga_columns;

/// The eight columns every read of `activity_muscle_mapping` returns, in the
/// order [`muscle_mapping_from_row`] reads them.
macro_rules! muscle_mapping_columns {
    () => {
        "id, activity_type, primary_muscles, secondary_muscles, \
         recommended_stretch_categories, recommended_yoga_categories, \
         created_at, updated_at"
    };
}

/// One stretching exercise by id.
pub(crate) const GET_STRETCHING_EXERCISE_SQL: &str = concat!(
    "SELECT ",
    stretching_columns!(),
    " FROM stretching_exercises WHERE id = $1"
);

/// Stretching exercises whose name or description contains the query.
/// `$like` is the backend's case-folding match operator.
macro_rules! search_stretching_sql {
    ($like:literal) => {
        concat!(
            "SELECT ",
            stretching_columns!(),
            " FROM stretching_exercises WHERE name ",
            $like,
            " $1 OR description ",
            $like,
            " $1 ORDER BY name ASC LIMIT $2"
        )
    };
}
pub(crate) use search_stretching_sql;

/// Stretching exercises recommended for one activity type; `$1` is the
/// JSON-array element pattern from [`json_array_pattern`].
macro_rules! stretches_for_activity_sql {
    ($like:literal) => {
        concat!(
            "SELECT ",
            stretching_columns!(),
            " FROM stretching_exercises WHERE recommended_for_activities ",
            $like,
            " $1 ORDER BY category, name ASC LIMIT $2"
        )
    };
}
pub(crate) use stretches_for_activity_sql;

/// One yoga pose by id.
pub(crate) const GET_YOGA_POSE_SQL: &str =
    concat!("SELECT ", yoga_columns!(), " FROM yoga_poses WHERE id = $1");

/// Yoga poses whose English name, Sanskrit name or description contains the
/// query. `$like` is the backend's case-folding match operator.
macro_rules! search_yoga_sql {
    ($like:literal) => {
        concat!(
            "SELECT ",
            yoga_columns!(),
            " FROM yoga_poses WHERE english_name ",
            $like,
            " $1 OR sanskrit_name ",
            $like,
            " $1 OR description ",
            $like,
            " $1 ORDER BY english_name ASC LIMIT $2"
        )
    };
}
pub(crate) use search_yoga_sql;

/// Yoga poses recommended for one recovery context; `$1` is the JSON-array
/// element pattern from [`json_array_pattern`].
macro_rules! poses_for_recovery_sql {
    ($like:literal) => {
        concat!(
            "SELECT ",
            yoga_columns!(),
            " FROM yoga_poses WHERE recommended_for_recovery ",
            $like,
            " $1 ORDER BY category, english_name ASC LIMIT $2"
        )
    };
}
pub(crate) use poses_for_recovery_sql;

/// The muscle mapping for one activity type.
pub(crate) const GET_MUSCLE_MAPPING_SQL: &str = concat!(
    "SELECT ",
    muscle_mapping_columns!(),
    " FROM activity_muscle_mapping WHERE activity_type = $1"
);

/// Every muscle mapping, by activity type.
pub(crate) const LIST_MUSCLE_MAPPINGS_SQL: &str = concat!(
    "SELECT ",
    muscle_mapping_columns!(),
    " FROM activity_muscle_mapping ORDER BY activity_type ASC"
);

/// The `LIKE` pattern that matches one element of a JSON-array column.
///
/// `primary_muscles`, `recommended_for_activities` and
/// `recommended_for_recovery` hold JSON-array text (`["hamstrings"]`), so an
/// element is found by its quoted spelling anywhere in the text: the
/// trailing `%` is what lets the element sit before the closing `]`.
pub(crate) fn json_array_pattern(element: &str) -> String {
    format!("%\"{element}\"%")
}

/// The `%query%` pattern for a free-text search.
pub(crate) fn contains_pattern(query: &str) -> String {
    format!("%{query}%")
}

/// A `LIMIT` that a caller left unset or out of `i32` range falls to the
/// method's default.
pub(crate) fn limit_or(limit: Option<u32>, default: i32) -> i32 {
    limit
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(default)
}

/// A dynamic catalogue listing: the statement its filter built, the string
/// binds in placeholder order, then the page bounds bound last.
pub(crate) struct CatalogueListQuery {
    /// The `SELECT`, with `$n` placeholders numbered for the binds.
    pub sql: String,
    /// The filter values, one per `$n` in order.
    pub binds: Vec<String>,
    /// Bound after the filter values, as `$n+1`.
    pub limit: i32,
    /// Bound last, as `$n+2`.
    pub offset: i32,
}

/// Numbered `$n` conditions for the optional filters of a listing.
struct Conditions {
    clauses: Vec<String>,
    binds: Vec<String>,
}

impl Conditions {
    const fn new() -> Self {
        Self {
            clauses: Vec::new(),
            binds: Vec::new(),
        }
    }

    /// `column = value`.
    fn equals(&mut self, column: &str, value: Option<&str>) {
        if let Some(value) = value {
            self.binds.push(value.to_owned());
            let n = self.binds.len();
            self.clauses.push(format!("{column} = ${n}"));
        }
    }

    /// `column <like> <json-array element pattern>`.
    fn has_element(&mut self, column: &str, like: &str, value: Option<&str>) {
        if let Some(value) = value {
            self.binds.push(json_array_pattern(value));
            let n = self.binds.len();
            self.clauses.push(format!("{column} {like} ${n}"));
        }
    }

    /// `(primary_muscles <like> p OR secondary_muscles <like> p)`.
    fn works_muscle(&mut self, like: &str, value: Option<&str>) {
        if let Some(value) = value {
            let pattern = json_array_pattern(value);
            self.binds.push(pattern.clone());
            let p1 = self.binds.len();
            self.binds.push(pattern);
            let p2 = self.binds.len();
            self.clauses.push(format!(
                "(primary_muscles {like} ${p1} OR secondary_muscles {like} ${p2})"
            ));
        }
    }

    fn into_query(
        self,
        columns: &str,
        table: &str,
        order: &str,
        page: (i32, i32),
    ) -> CatalogueListQuery {
        let where_clause = if self.clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.clauses.join(" AND "))
        };
        let limit_param = self.binds.len() + 1;
        let offset_param = self.binds.len() + 2;
        CatalogueListQuery {
            sql: format!(
                "SELECT {columns} FROM {table}{where_clause} ORDER BY {order} LIMIT ${limit_param} OFFSET ${offset_param}"
            ),
            binds: self.binds,
            limit: page.0,
            offset: page.1,
        }
    }
}

/// The listing statement for [`MobilityRepository::list_stretching_exercises`].
/// `like` is the backend's case-folding match operator.
pub(crate) fn list_stretching_sql(filter: &ListStretchingFilter, like: &str) -> CatalogueListQuery {
    let mut conditions = Conditions::new();
    conditions.equals(
        "category",
        filter.category.as_ref().map(StretchingCategory::as_str),
    );
    conditions.equals(
        "difficulty",
        filter.difficulty.as_ref().map(DifficultyLevel::as_str),
    );
    conditions.works_muscle(like, filter.muscle_group.as_deref());
    conditions.has_element(
        "recommended_for_activities",
        like,
        filter.activity_type.as_deref(),
    );
    conditions.into_query(
        stretching_columns!(),
        "stretching_exercises",
        "name ASC",
        (limit_or(filter.limit, 50), limit_or(filter.offset, 0)),
    )
}

/// The listing statement for [`MobilityRepository::list_yoga_poses`].
/// `like` is the backend's case-folding match operator.
pub(crate) fn list_yoga_sql(filter: &ListYogaFilter, like: &str) -> CatalogueListQuery {
    let mut conditions = Conditions::new();
    conditions.equals(
        "category",
        filter.category.as_ref().map(YogaCategory::as_str),
    );
    conditions.equals(
        "difficulty",
        filter.difficulty.as_ref().map(DifficultyLevel::as_str),
    );
    conditions.equals(
        "pose_type",
        filter.pose_type.as_ref().map(YogaPoseType::as_str),
    );
    conditions.works_muscle(like, filter.muscle_group.as_deref());
    conditions.has_element(
        "recommended_for_activities",
        like,
        filter.activity_type.as_deref(),
    );
    conditions.has_element(
        "recommended_for_recovery",
        like,
        filter.recovery_context.as_deref(),
    );
    conditions.into_query(
        yoga_columns!(),
        "yoga_poses",
        "english_name ASC",
        (limit_or(filter.limit, 50), limit_or(filter.offset, 0)),
    )
}

// ============================================================================
// Row parsers
// ============================================================================

fn mobility_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read mobility column {name}: {e}"))
}

/// Read one column, naming it in the error.
fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    T: Decode<'r, R::Database> + Type<R::Database>,
{
    row.try_get(name)
        .map_err(|e| mobility_column_error(name, e))
}

/// A NOT NULL JSON column, parsed.
fn json_column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    T: DeserializeOwned,
{
    let raw: String = column(row, name)?;
    serde_json::from_str(&raw).map_err(|e| mobility_column_error(name, e))
}

/// A nullable JSON column; NULL reads as the type's empty value.
fn json_column_or_default<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    T: DeserializeOwned + Default,
{
    let raw: Option<String> = column(row, name)?;
    raw.map_or_else(
        || Ok(T::default()),
        |s| serde_json::from_str(&s).map_err(|e| mobility_column_error(name, e)),
    )
}

/// A seconds or count column: `INTEGER` is `int4` on Postgres and decodes
/// as `i32` on both engines; the domain holds it unsigned.
fn unsigned_column<'r, R>(row: &'r R, name: &str) -> AppResult<u32>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
{
    let raw: i32 = column(row, name)?;
    u32::try_from(raw).map_err(|e| mobility_column_error(name, e))
}

/// Convert a catalogue row to a [`StretchingExercise`].
///
/// Generic over the driver: `created_at`/`updated_at` decode as
/// `DateTime<Utc>` from `SQLite`'s RFC 3339 text and Postgres's
/// `TIMESTAMPTZ` alike, and every count column as `i32`.
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn stretching_from_row<'r, R>(row: &'r R) -> AppResult<StretchingExercise>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    DateTime<Utc>: Decode<'r, R::Database> + Type<R::Database>,
{
    let category: String = column(row, "category")?;
    let difficulty: String = column(row, "difficulty")?;
    let repetitions: Option<i32> = column(row, "repetitions")?;
    Ok(StretchingExercise {
        id: column(row, "id")?,
        name: column(row, "name")?,
        description: column(row, "description")?,
        category: StretchingCategory::parse(&category),
        difficulty: DifficultyLevel::parse(&difficulty),
        primary_muscles: json_column(row, "primary_muscles")?,
        secondary_muscles: json_column_or_default(row, "secondary_muscles")?,
        duration_seconds: unsigned_column(row, "duration_seconds")?,
        repetitions: repetitions
            .map(|r| u32::try_from(r).map_err(|e| mobility_column_error("repetitions", e)))
            .transpose()?,
        sets: unsigned_column(row, "sets")?,
        recommended_for_activities: json_column_or_default(row, "recommended_for_activities")?,
        contraindications: json_column_or_default(row, "contraindications")?,
        instructions: json_column(row, "instructions")?,
        cues: json_column_or_default(row, "cues")?,
        image_url: column(row, "image_url")?,
        video_url: column(row, "video_url")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
    })
}

/// Convert a catalogue row to a [`YogaPose`]; see [`stretching_from_row`].
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn yoga_pose_from_row<'r, R>(row: &'r R) -> AppResult<YogaPose>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    DateTime<Utc>: Decode<'r, R::Database> + Type<R::Database>,
{
    let category: String = column(row, "category")?;
    let difficulty: String = column(row, "difficulty")?;
    let pose_type: String = column(row, "pose_type")?;
    Ok(YogaPose {
        id: column(row, "id")?,
        english_name: column(row, "english_name")?,
        sanskrit_name: column(row, "sanskrit_name")?,
        description: column(row, "description")?,
        benefits: json_column(row, "benefits")?,
        category: YogaCategory::parse(&category),
        difficulty: DifficultyLevel::parse(&difficulty),
        pose_type: YogaPoseType::parse(&pose_type),
        primary_muscles: json_column(row, "primary_muscles")?,
        secondary_muscles: json_column_or_default(row, "secondary_muscles")?,
        chakras: json_column_or_default(row, "chakras")?,
        hold_duration_seconds: unsigned_column(row, "hold_duration_seconds")?,
        breath_guidance: column(row, "breath_guidance")?,
        recommended_for_activities: json_column_or_default(row, "recommended_for_activities")?,
        recommended_for_recovery: json_column_or_default(row, "recommended_for_recovery")?,
        contraindications: json_column_or_default(row, "contraindications")?,
        instructions: json_column(row, "instructions")?,
        modifications: json_column_or_default(row, "modifications")?,
        progressions: json_column_or_default(row, "progressions")?,
        cues: json_column_or_default(row, "cues")?,
        warmup_poses: json_column_or_default(row, "warmup_poses")?,
        followup_poses: json_column_or_default(row, "followup_poses")?,
        image_url: column(row, "image_url")?,
        video_url: column(row, "video_url")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
    })
}

/// Convert a catalogue row to an [`ActivityMuscleMapping`]; see
/// [`stretching_from_row`].
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn muscle_mapping_from_row<'r, R>(row: &'r R) -> AppResult<ActivityMuscleMapping>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    DateTime<Utc>: Decode<'r, R::Database> + Type<R::Database>,
{
    Ok(ActivityMuscleMapping {
        id: column(row, "id")?,
        activity_type: column(row, "activity_type")?,
        primary_muscles: json_column::<_, HashMap<String, u8>>(row, "primary_muscles")?,
        secondary_muscles: json_column_or_default::<_, HashMap<String, u8>>(
            row,
            "secondary_muscles",
        )?,
        recommended_stretch_categories: json_column_or_default(
            row,
            "recommended_stretch_categories",
        )?,
        recommended_yoga_categories: json_column_or_default(row, "recommended_yoga_categories")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
    })
}

// ============================================================================
// The implementation, emitted once per backend
// ============================================================================

/// Emit the whole [`MobilityRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion.
///
/// `$like` is the backend's case-folding text match, spliced into every
/// search and JSON-array filter: `"ILIKE"` on Postgres, `"LIKE"` on
/// `SQLite`. `SQLite`'s `LIKE` folds case for ASCII letters only, so a search
/// for a name with an accented letter is case-sensitive there and not on
/// Postgres; production runs Postgres, so no athlete sees the narrower fold.
macro_rules! impl_mobility_repository {
    ($ty:ty, $like:literal) => {
        #[async_trait::async_trait]
        impl MobilityRepository for $ty {
            async fn get_stretching_exercise(
                &self,
                id: &str,
            ) -> AppResult<Option<StretchingExercise>> {
                let row = sqlx::query(GET_STRETCHING_EXERCISE_SQL)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get stretching exercise: {e}"))
                    })?;
                row.as_ref().map(stretching_from_row).transpose()
            }

            async fn list_stretching_exercises(
                &self,
                filter: &ListStretchingFilter,
            ) -> AppResult<Vec<StretchingExercise>> {
                let listing = list_stretching_sql(filter, $like);
                let mut query = sqlx::query(&listing.sql);
                for value in &listing.binds {
                    query = query.bind(value);
                }
                let rows = query
                    .bind(listing.limit)
                    .bind(listing.offset)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list stretching exercises: {e}"))
                    })?;
                rows.iter().map(stretching_from_row).collect()
            }

            async fn search_stretching_exercises(
                &self,
                query: &str,
                limit: Option<u32>,
            ) -> AppResult<Vec<StretchingExercise>> {
                let rows = sqlx::query(search_stretching_sql!($like))
                    .bind(contains_pattern(query))
                    .bind(limit_or(limit, 20))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to search stretching exercises: {e}"))
                    })?;
                rows.iter().map(stretching_from_row).collect()
            }

            async fn get_stretches_for_activity(
                &self,
                activity_type: &str,
                limit: Option<u32>,
            ) -> AppResult<Vec<StretchingExercise>> {
                let rows = sqlx::query(stretches_for_activity_sql!($like))
                    .bind(json_array_pattern(activity_type))
                    .bind(limit_or(limit, 10))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get stretches for activity: {e}"))
                    })?;
                rows.iter().map(stretching_from_row).collect()
            }

            async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>> {
                let row = sqlx::query(GET_YOGA_POSE_SQL)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get yoga pose: {e}")))?;
                row.as_ref().map(yoga_pose_from_row).transpose()
            }

            async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>> {
                let listing = list_yoga_sql(filter, $like);
                let mut query = sqlx::query(&listing.sql);
                for value in &listing.binds {
                    query = query.bind(value);
                }
                let rows = query
                    .bind(listing.limit)
                    .bind(listing.offset)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list yoga poses: {e}")))?;
                rows.iter().map(yoga_pose_from_row).collect()
            }

            async fn search_yoga_poses(
                &self,
                query: &str,
                limit: Option<u32>,
            ) -> AppResult<Vec<YogaPose>> {
                let rows = sqlx::query(search_yoga_sql!($like))
                    .bind(contains_pattern(query))
                    .bind(limit_or(limit, 20))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to search yoga poses: {e}")))?;
                rows.iter().map(yoga_pose_from_row).collect()
            }

            async fn get_poses_for_recovery(
                &self,
                recovery_context: &str,
                limit: Option<u32>,
            ) -> AppResult<Vec<YogaPose>> {
                let rows = sqlx::query(poses_for_recovery_sql!($like))
                    .bind(json_array_pattern(recovery_context))
                    .bind(limit_or(limit, 10))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get poses for recovery: {e}"))
                    })?;
                rows.iter().map(yoga_pose_from_row).collect()
            }

            async fn get_activity_muscle_mapping(
                &self,
                activity_type: &str,
            ) -> AppResult<Option<ActivityMuscleMapping>> {
                let row = sqlx::query(GET_MUSCLE_MAPPING_SQL)
                    .bind(activity_type)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get activity muscle mapping: {e}"))
                    })?;
                row.as_ref().map(muscle_mapping_from_row).transpose()
            }

            async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>> {
                let rows = sqlx::query(LIST_MUSCLE_MAPPINGS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list activity muscle mappings: {e}"))
                    })?;
                rows.iter().map(muscle_mapping_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_mobility_repository;
