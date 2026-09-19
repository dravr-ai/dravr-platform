// ABOUTME: PrescribedWorkoutRepository trait plus the one shared implementation both backends emit: the calendar-entry ledger
// ABOUTME: Upsert by id, tenant-scoped reads (by id, recent, live-per-provider), terminal status transitions; payload_json is an opaque blob
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CalendarEventSource, PrescribedWorkout, SportType, TenantId, UserId};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::uuid_column::UuidColumn;

/// CRUD for the `prescribed_workouts` table — the ledger of every calendar
/// entry Dravr wrote to a provider.
///
/// Each row records one write attempt: a single prescription, or one entry of
/// a plan push. Rows are never edited into a different entry; a re-push of the
/// same key writes a new `pushed` row and moves the old one to `replaced`, so
/// the partial unique index on (`tenant_id`, `user_id`, `provider`,
/// `external_id`) `WHERE status = 'pushed'` holds one live row per key.
#[async_trait]
pub trait PrescribedWorkoutRepository: Send + Sync {
    /// Insert a ledger row, or refresh an existing row's outcome fields
    /// (`provider_event_id`, `status`, payload, hash, `updated_at`) by id.
    async fn upsert_prescribed_workout(&self, prescribed: &PrescribedWorkout) -> AppResult<()>;

    /// List the most recent `limit` rows for a (`tenant_id`, `user_id`),
    /// newest first.
    async fn list_prescribed_workouts(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<PrescribedWorkout>>;

    /// Fetch one row by id. Returns `None` when no row matches the
    /// (`tenant_id`, `user_id`, `id`) tuple — a row of another athlete is
    /// indistinguishable from a missing one, by design.
    async fn get_prescribed_workout(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: Uuid,
    ) -> AppResult<Option<PrescribedWorkout>>;

    /// List the rows whose entry is live on `provider` (`status = pushed`),
    /// on or after `from` when given, in calendar order.
    async fn list_live_calendar_events(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        provider: &str,
        from: Option<NaiveDate>,
    ) -> AppResult<Vec<PrescribedWorkout>>;

    /// Move a row to `status` (`replaced` or `withdrawn`), stamping
    /// `updated_at`. Errors when no row matches the (`tenant_id`, `id`) pair.
    async fn set_prescribed_workout_status(
        &self,
        tenant_id: TenantId,
        id: Uuid,
        status: &str,
    ) -> AppResult<()>;
}

/// The most rows one list call returns, whatever `limit` asked for.
pub(crate) const MAX_LIST_LIMIT: u32 = 200;

/// The column list every read projects, in the order [`prescribed_from_row`]
/// names them.
macro_rules! prescribed_columns {
    () => {
        "id, tenant_id, user_id, agent_id, template_slug, sport, \
         prescribed_for_date, provider, provider_event_id, external_id, \
         source, plan_week_id, replaces_id, payload_json, payload_hash, \
         status, created_at, updated_at"
    };
}

/// Insert a ledger row, or refresh the outcome fields of the row sharing
/// its id.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres. `id` and `replaces_id` bind as [`UuidColumn`], `tenant_id` as
/// [`TenantId`] and `user_id` as [`UserId`], each of which encodes as
/// hyphenated text on `SQLite` and as a native `uuid` on Postgres.
/// `prescribed_for_date` binds as a `NaiveDate`: `%Y-%m-%d` text on
/// `SQLite`, `DATE` on Postgres. The timestamps bind as `DateTime<Utc>`:
/// RFC3339 text on `SQLite`, `TIMESTAMPTZ` on Postgres. `payload_json`
/// binds as [`Value`], which sqlx stores as `TEXT` on `SQLite` and `jsonb`
/// on Postgres: the stored JSON is an opaque blob. Its readers parse it —
/// the calendar view takes `name` out of it — and the change detector
/// compares `payload_hash`, a separate column, never the stored text.
pub(crate) const UPSERT_PRESCRIBED_WORKOUT_SQL: &str = concat!(
    "
            INSERT INTO prescribed_workouts (",
    prescribed_columns!(),
    ")
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
            ON CONFLICT (id) DO UPDATE SET
                provider_event_id = EXCLUDED.provider_event_id,
                status = EXCLUDED.status,
                payload_json = EXCLUDED.payload_json,
                payload_hash = EXCLUDED.payload_hash,
                updated_at = EXCLUDED.updated_at
            "
);

/// The newest rows of one athlete, capped by the bound `LIMIT`.
pub(crate) const LIST_PRESCRIBED_WORKOUTS_SQL: &str = concat!(
    "
            SELECT ",
    prescribed_columns!(),
    "
            FROM prescribed_workouts
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY created_at DESC
            LIMIT $3
            "
);

/// One row by id, within the athlete's own rows.
pub(crate) const GET_PRESCRIBED_WORKOUT_SQL: &str = concat!(
    "
            SELECT ",
    prescribed_columns!(),
    "
            FROM prescribed_workouts
            WHERE tenant_id = $1 AND user_id = $2 AND id = $3
            "
);

/// The rows live on one provider, on or after `$5` when it is bound, in
/// calendar order. `$5` is one optional date bound once; sqlx declares its
/// type up front on Postgres and `SQLite` compares the text, so neither
/// engine needs a cast.
pub(crate) const LIST_LIVE_CALENDAR_EVENTS_SQL: &str = concat!(
    "
            SELECT ",
    prescribed_columns!(),
    "
            FROM prescribed_workouts
            WHERE tenant_id = $1 AND user_id = $2 AND provider = $3
              AND status = $4
              AND ($5 IS NULL OR prescribed_for_date >= $5)
            ORDER BY prescribed_for_date ASC, created_at ASC
            "
);

/// Move one row to a new status, stamping `updated_at`.
pub(crate) const SET_PRESCRIBED_WORKOUT_STATUS_SQL: &str = r"
            UPDATE prescribed_workouts
            SET status = $1, updated_at = $2
            WHERE tenant_id = $3 AND id = $4
            ";

/// Rebuild a [`PrescribedWorkout`] from one row of [`prescribed_columns`].
///
/// Every nullable text column is decoded as `Option<String>`, never
/// `try_get::<String>().ok()`: `SQLite` hands a NULL TEXT column back as an
/// empty string rather than an error, so the `.ok()` form silently turns
/// "no value" into `Some("")`. For `provider_event_id` that is the
/// difference between a prescription the provider never created and one
/// whose calendar event id is the empty string.
///
/// # Errors
/// Returns a database error when a column cannot be decoded, a stored
/// `source` is not one the application knows, or the payload cannot be
/// re-rendered.
pub(crate) fn prescribed_from_row<R>(row: &R) -> AppResult<PrescribedWorkout>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Value: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    UuidColumn: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    TenantId: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    UserId: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let id: UuidColumn = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("read id: {e}")))?;
    let tenant_id: TenantId = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("read tenant_id: {e}")))?;
    let user_id: UserId = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("read user_id: {e}")))?;
    let sport_str: String = row
        .try_get("sport")
        .map_err(|e| AppError::database(format!("read sport: {e}")))?;
    let sport: SportType = serde_json::from_str(&sport_str)
        .map_err(|e| AppError::database(format!("parse sport: {e}")))?;
    let prescribed_for_date: NaiveDate = row
        .try_get("prescribed_for_date")
        .map_err(|e| AppError::database(format!("read prescribed_for_date: {e}")))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("read created_at: {e}")))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("read updated_at: {e}")))?;
    let payload: Value = row
        .try_get("payload_json")
        .map_err(|e| AppError::database(format!("read payload_json: {e}")))?;
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| AppError::database(format!("serialize payload_json: {e}")))?;
    let source_str: String = row
        .try_get("source")
        .map_err(|e| AppError::database(format!("read source: {e}")))?;
    let source = CalendarEventSource::parse(&source_str)
        .ok_or_else(|| AppError::database(format!("unknown source '{source_str}'")))?;
    let replaces_id: Option<UuidColumn> = row
        .try_get("replaces_id")
        .map_err(|e| AppError::database(format!("read replaces_id: {e}")))?;

    Ok(PrescribedWorkout {
        id: id.into(),
        tenant_id: tenant_id.as_uuid(),
        user_id: user_id.as_uuid(),
        agent_id: row
            .try_get("agent_id")
            .map_err(|e| AppError::database(format!("read agent_id: {e}")))?,
        template_slug: row
            .try_get("template_slug")
            .map_err(|e| AppError::database(format!("read template_slug: {e}")))?,
        sport,
        prescribed_for_date,
        provider: row
            .try_get("provider")
            .map_err(|e| AppError::database(format!("read provider: {e}")))?,
        provider_event_id: row
            .try_get("provider_event_id")
            .map_err(|e| AppError::database(format!("read provider_event_id: {e}")))?,
        external_id: row
            .try_get("external_id")
            .map_err(|e| AppError::database(format!("read external_id: {e}")))?,
        source,
        plan_week_id: row
            .try_get("plan_week_id")
            .map_err(|e| AppError::database(format!("read plan_week_id: {e}")))?,
        replaces_id: replaces_id.map(Uuid::from),
        payload_hash: row
            .try_get("payload_hash")
            .map_err(|e| AppError::database(format!("read payload_hash: {e}")))?,
        payload_json,
        status: row
            .try_get("status")
            .map_err(|e| AppError::database(format!("read status: {e}")))?,
        created_at,
        updated_at,
    })
}

/// Emit the whole [`PrescribedWorkoutRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion.
macro_rules! impl_prescribed_workout_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl PrescribedWorkoutRepository for $ty {
            async fn upsert_prescribed_workout(
                &self,
                prescribed: &PrescribedWorkout,
            ) -> AppResult<()> {
                let sport = serde_json::to_string(&prescribed.sport)
                    .map_err(|e| AppError::database(format!("serialize sport: {e}")))?;
                let payload: Value = serde_json::from_str(&prescribed.payload_json)
                    .map_err(|e| AppError::database(format!("parse payload_json: {e}")))?;
                sqlx::query(UPSERT_PRESCRIBED_WORKOUT_SQL)
                    .bind(UuidColumn(prescribed.id))
                    .bind(TenantId::from_uuid(prescribed.tenant_id))
                    .bind(UserId::from_uuid(prescribed.user_id))
                    .bind(prescribed.agent_id.as_deref())
                    .bind(prescribed.template_slug.as_deref())
                    .bind(&sport)
                    .bind(prescribed.prescribed_for_date)
                    .bind(&prescribed.provider)
                    .bind(prescribed.provider_event_id.as_deref())
                    .bind(prescribed.external_id.as_deref())
                    .bind(prescribed.source.as_str())
                    .bind(prescribed.plan_week_id.as_deref())
                    .bind(prescribed.replaces_id.map(UuidColumn))
                    .bind(payload)
                    .bind(prescribed.payload_hash.as_deref())
                    .bind(&prescribed.status)
                    .bind(prescribed.created_at)
                    .bind(prescribed.updated_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert_prescribed_workout: {e}")))?;
                Ok(())
            }

            async fn list_prescribed_workouts(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                limit: u32,
            ) -> AppResult<Vec<PrescribedWorkout>> {
                let bounded = limit.clamp(1, MAX_LIST_LIMIT);
                let rows = sqlx::query(LIST_PRESCRIBED_WORKOUTS_SQL)
                    .bind(tenant_id)
                    .bind(UserId::from_uuid(user_id))
                    .bind(i64::from(bounded))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list_prescribed_workouts: {e}")))?;
                rows.iter().map(prescribed_from_row).collect()
            }

            async fn get_prescribed_workout(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                id: Uuid,
            ) -> AppResult<Option<PrescribedWorkout>> {
                let row = sqlx::query(GET_PRESCRIBED_WORKOUT_SQL)
                    .bind(tenant_id)
                    .bind(UserId::from_uuid(user_id))
                    .bind(UuidColumn(id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_prescribed_workout: {e}")))?;
                row.as_ref().map(prescribed_from_row).transpose()
            }

            async fn list_live_calendar_events(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                provider: &str,
                from: Option<NaiveDate>,
            ) -> AppResult<Vec<PrescribedWorkout>> {
                let rows = sqlx::query(LIST_LIVE_CALENDAR_EVENTS_SQL)
                    .bind(tenant_id)
                    .bind(UserId::from_uuid(user_id))
                    .bind(provider)
                    .bind(PrescribedWorkout::STATUS_PUSHED)
                    .bind(from)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list_live_calendar_events: {e}")))?;
                rows.iter().map(prescribed_from_row).collect()
            }

            async fn set_prescribed_workout_status(
                &self,
                tenant_id: TenantId,
                id: Uuid,
                status: &str,
            ) -> AppResult<()> {
                let result = sqlx::query(SET_PRESCRIBED_WORKOUT_STATUS_SQL)
                    .bind(status)
                    .bind(Utc::now())
                    .bind(tenant_id)
                    .bind(UuidColumn(id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("set_prescribed_workout_status: {e}"))
                    })?;
                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!(
                        "prescribed workout {id} not found in tenant {tenant_id}"
                    )));
                }
                Ok(())
            }
        }
    };
}
pub(crate) use impl_prescribed_workout_repository;
