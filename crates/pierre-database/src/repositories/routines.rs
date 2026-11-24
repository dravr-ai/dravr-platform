// ABOUTME: Repository trait for routines — what makes the platform speak first, and when each fires next
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routines (ADR-036, carnet#1790).
//!
//! One row per routine. The engine reads a tenant's active routines when it
//! reconciles, reads one routine when its wake is delivered, and moves its
//! next firing forward ([`RoutineRepository::schedule_routine`]) — deferred
//! while the recipient is mid-conversation, or on to the next slot once it
//! fired.
//!
//! A built-in routine is written by the preference it encodes: setting the
//! athlete's check-in cadence writes their check-in routine in the same
//! transaction ([`UPSERT_CHECK_IN_ROUTINE_SQL`], used by
//! [`super::check_in_preferences`]), and resets its next firing so the engine
//! schedules it afresh.
//!
//! Every statement is keyed by `tenant_id`.

use std::fmt;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::check_in::CHECK_IN_HOUR;
use pierre_core::models::{
    CheckInCadence, Routine, RoutineKind, RoutineScope, RoutineState, RoutineTrigger, Schedule,
    TenantId, Weekdays,
};
use uuid::Uuid;

/// The routines the engine reads and schedules.
#[async_trait]
pub trait RoutineRepository: Send + Sync {
    /// Every active routine of `tenant_id`, oldest first.
    async fn active_routines(&self, tenant_id: TenantId) -> AppResult<Vec<Routine>>;

    /// One routine of `tenant_id`, or `None` when it is gone.
    async fn routine(&self, tenant_id: TenantId, id: Uuid) -> AppResult<Option<Routine>>;

    /// The built-in routine of `kind` that `owner_user_id` has, if any.
    async fn builtin_routine(
        &self,
        tenant_id: TenantId,
        owner_user_id: Uuid,
        kind: RoutineKind,
    ) -> AppResult<Option<Routine>>;

    /// Move the routine's next firing to `next` — its slot, when it fires,
    /// and how many times that firing was deferred — but only while its next
    /// firing is still `expected`. `true` when the row moved; `false` when a
    /// schedule change or another delivery moved it first.
    async fn schedule_routine(
        &self,
        tenant_id: TenantId,
        id: Uuid,
        expected: Option<Timestamp>,
        next: NextFiring,
        at: Timestamp,
    ) -> AppResult<bool>;

    /// Record that the routine fired at `fired_at` and move it to `next`,
    /// under the same guard as [`Self::schedule_routine`].
    async fn record_routine_fired(
        &self,
        tenant_id: TenantId,
        id: Uuid,
        expected: Option<Timestamp>,
        fired_at: Timestamp,
        next: NextFiring,
    ) -> AppResult<bool>;
}

/// A routine's next firing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NextFiring {
    /// The schedule slot it fires for; `None` leaves it unscheduled.
    pub slot: Option<Timestamp>,
    /// When it fires: the slot, or later while deferred.
    pub fire: Option<Timestamp>,
    /// How many times that firing was deferred.
    pub deferrals: u32,
}

/// The columns every routine read selects, in the order a row is decoded.
macro_rules! routine_columns {
    () => {
        "id, tenant_id, owner_user_id, kind, scope, trigger_kind, days_mask, local_hour, state, \
         next_slot_ms, next_fire_ms, deferrals, last_fired_at_ms"
    };
}

pub(crate) const ACTIVE_ROUTINES_SQL: &str = concat!(
    "SELECT ",
    routine_columns!(),
    " FROM routines WHERE tenant_id = $1 AND state = 'active' ORDER BY created_at_ms, id"
);

pub(crate) const ROUTINE_SQL: &str = concat!(
    "SELECT ",
    routine_columns!(),
    " FROM routines WHERE tenant_id = $1 AND id = $2"
);

pub(crate) const BUILTIN_ROUTINE_SQL: &str = concat!(
    "SELECT ",
    routine_columns!(),
    " FROM routines WHERE tenant_id = $1 AND owner_user_id = $2 AND kind = $3"
);

/// `$3` is the expected next firing; NULL matches an unscheduled routine.
pub(crate) const SCHEDULE_ROUTINE_SQL: &str = r"
            UPDATE routines
            SET next_slot_ms = $4, next_fire_ms = $5, deferrals = $6, updated_at_ms = $7
            WHERE tenant_id = $1 AND id = $2
              AND (next_fire_ms = $3 OR (next_fire_ms IS NULL AND $3 IS NULL))
            ";

pub(crate) const RECORD_ROUTINE_FIRED_SQL: &str = r"
            UPDATE routines
            SET last_fired_at_ms = $7, next_slot_ms = $4, next_fire_ms = $5, deferrals = $6,
                updated_at_ms = $7
            WHERE tenant_id = $1 AND id = $2
              AND (next_fire_ms = $3 OR (next_fire_ms IS NULL AND $3 IS NULL))
            ";

/// Write an athlete's check-in routine from the cadence they chose: `$4` the
/// days mask, `$5` the local hour, `$6` the state. Run in the cadence's own
/// transaction. A changed schedule is unscheduled until the engine schedules
/// it again, so a wake queued for the old one finds nothing to fire.
pub(crate) const UPSERT_CHECK_IN_ROUTINE_SQL: &str = r"
            INSERT INTO routines (id, tenant_id, owner_user_id, kind, scope, trigger_kind,
                                  days_mask, local_hour, state, created_at_ms, updated_at_ms)
            VALUES ($1, $2, $3, 'check_in', 'me', 'schedule', $4, $5, $6, $7, $7)
            ON CONFLICT (tenant_id, owner_user_id, kind) WHERE kind <> 'custom' DO UPDATE SET
                days_mask = excluded.days_mask,
                local_hour = excluded.local_hour,
                state = excluded.state,
                next_slot_ms = NULL,
                next_fire_ms = NULL,
                deferrals = 0,
                updated_at_ms = excluded.updated_at_ms
            ";

/// The check-in routine's stored days mask, local hour and state for
/// `cadence`: its weekdays at the check-in hour, paused for `off`.
pub(crate) fn check_in_routine_columns(cadence: CheckInCadence) -> (i64, i64, &'static str) {
    let state = if Schedule::of_check_in(cadence).is_some() {
        RoutineState::Active
    } else {
        RoutineState::Paused
    };
    (
        i64::from(Weekdays::of_cadence(cadence).bits()),
        i64::from(CHECK_IN_HOUR),
        state.as_str(),
    )
}

/// One routine row, decoded column by column by the backend that read it.
#[derive(Debug)]
pub(crate) struct RoutineColumns {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub owner_user_id: Uuid,
    pub kind: String,
    pub scope: String,
    pub trigger_kind: String,
    pub days_mask: i64,
    pub local_hour: i64,
    pub state_label: String,
    pub next_slot_ms: Option<i64>,
    pub next_fire_ms: Option<i64>,
    pub deferrals: i64,
    pub last_fired_at_ms: Option<i64>,
}

impl RoutineColumns {
    /// The routine the columns describe.
    ///
    /// # Errors
    ///
    /// A database error naming the column when a stored spelling is not one
    /// the model reads, or a timestamp is out of range.
    pub(crate) fn into_routine(self) -> AppResult<Routine> {
        let bad = |column: &str, value: &dyn fmt::Debug| {
            AppError::database(format!("routines.{column} {value:?} is not readable"))
        };
        let kind = RoutineKind::parse(&self.kind).ok_or_else(|| bad("kind", &self.kind))?;
        let scope = RoutineScope::parse(&self.scope).ok_or_else(|| bad("scope", &self.scope))?;
        let state = RoutineState::parse(&self.state_label)
            .ok_or_else(|| bad("state", &self.state_label))?;
        let days = u8::try_from(self.days_mask)
            .ok()
            .and_then(Weekdays::from_bits)
            .ok_or_else(|| bad("days_mask", &self.days_mask))?;
        let hour =
            u32::try_from(self.local_hour).map_err(|_| bad("local_hour", &self.local_hour))?;
        let trigger = match self.trigger_kind.as_str() {
            // A schedule with no day is a paused check-in's: no trigger to read.
            "schedule" => Schedule::try_new(days, hour).map(RoutineTrigger::Schedule),
            other => return Err(bad("trigger_kind", &other)),
        };
        let at = |column: &str, ms: Option<i64>| {
            ms.map(|ms| Timestamp::from_epoch_millis(ms).map_err(|_| bad(column, &ms)))
                .transpose()
        };
        Ok(Routine {
            id: self.id,
            tenant_id: TenantId::from_uuid(self.tenant_id),
            owner_user_id: self.owner_user_id,
            kind,
            scope,
            trigger,
            state,
            next_slot: at("next_slot_ms", self.next_slot_ms)?,
            next_fire: at("next_fire_ms", self.next_fire_ms)?,
            deferrals: u32::try_from(self.deferrals)
                .map_err(|_| bad("deferrals", &self.deferrals))?,
            last_fired_at: at("last_fired_at_ms", self.last_fired_at_ms)?,
        })
    }
}

macro_rules! impl_routine_repository {
    ($ty:ty, $ids:ident, $row:ty) => {
        fn routine_from_row(row: &$row) -> AppResult<Routine> {
            fn column<'r, T>(row: &'r $row, name: &str) -> AppResult<T>
            where
                T: sqlx::Decode<'r, <$row as sqlx::Row>::Database>
                    + sqlx::Type<<$row as sqlx::Row>::Database>,
            {
                row.try_get(name)
                    .map_err(|e| AppError::database(format!("Invalid routines.{name} column: {e}")))
            }
            $crate::repositories::routines::RoutineColumns {
                id: $ids::read(row, "id")?,
                tenant_id: $ids::read(row, "tenant_id")?,
                owner_user_id: $ids::read(row, "owner_user_id")?,
                kind: column(row, "kind")?,
                scope: column(row, "scope")?,
                trigger_kind: column(row, "trigger_kind")?,
                days_mask: column(row, "days_mask")?,
                local_hour: column(row, "local_hour")?,
                state_label: column(row, "state")?,
                next_slot_ms: column(row, "next_slot_ms")?,
                next_fire_ms: column(row, "next_fire_ms")?,
                deferrals: column(row, "deferrals")?,
                last_fired_at_ms: column(row, "last_fired_at_ms")?,
            }
            .into_routine()
        }

        #[async_trait::async_trait]
        impl RoutineRepository for $ty {
            async fn active_routines(&self, tenant_id: TenantId) -> AppResult<Vec<Routine>> {
                let rows = sqlx::query(ACTIVE_ROUTINES_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list routines: {e}")))?;
                rows.iter().map(routine_from_row).collect()
            }

            async fn routine(&self, tenant_id: TenantId, id: Uuid) -> AppResult<Option<Routine>> {
                let row = sqlx::query(ROUTINE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read a routine: {e}")))?;
                row.as_ref().map(routine_from_row).transpose()
            }

            async fn builtin_routine(
                &self,
                tenant_id: TenantId,
                owner_user_id: Uuid,
                kind: RoutineKind,
            ) -> AppResult<Option<Routine>> {
                let row = sqlx::query(BUILTIN_ROUTINE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(owner_user_id))
                    .bind(kind.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read a built-in routine: {e}"))
                    })?;
                row.as_ref().map(routine_from_row).transpose()
            }

            async fn schedule_routine(
                &self,
                tenant_id: TenantId,
                id: Uuid,
                expected: Option<Timestamp>,
                next: NextFiring,
                at: Timestamp,
            ) -> AppResult<bool> {
                let result = sqlx::query(SCHEDULE_ROUTINE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(id))
                    .bind(expected.map(Timestamp::epoch_millis))
                    .bind(next.slot.map(Timestamp::epoch_millis))
                    .bind(next.fire.map(Timestamp::epoch_millis))
                    .bind(i64::from(next.deferrals))
                    .bind(at.epoch_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to schedule a routine: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn record_routine_fired(
                &self,
                tenant_id: TenantId,
                id: Uuid,
                expected: Option<Timestamp>,
                fired_at: Timestamp,
                next: NextFiring,
            ) -> AppResult<bool> {
                let result = sqlx::query(RECORD_ROUTINE_FIRED_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(id))
                    .bind(expected.map(Timestamp::epoch_millis))
                    .bind(next.slot.map(Timestamp::epoch_millis))
                    .bind(next.fire.map(Timestamp::epoch_millis))
                    .bind(i64::from(next.deferrals))
                    .bind(fired_at.epoch_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a routine firing: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_routine_repository;
