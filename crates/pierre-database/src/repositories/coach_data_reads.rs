// ABOUTME: Repository trait, statements and shared body for the athlete's record of their human coach's reads of their data
// ABOUTME: One row per read, tenant-scoped under the athlete's tenant, listed to the athlete and purged after its retention

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach data reads (carnet#1445).
//!
//! A group's human coach reads a coached athlete's data through their agent:
//! in their thread about the athlete, over MCP or A2A with `athlete=`, through
//! the `groups` and plan tools; and outside any tool call, through the
//! surfaces [`CoachReadSurface`] names. Every such read the coaching gates let
//! through is recorded here, under the athlete's own tenant, and the athlete
//! lists their own rows: one row per read, or for a group view a coach opens
//! on every turn or page load at most one a UTC day
//! ([`CoachDataReadRepository::record_daily_coach_read`]). Rows older than
//! the retention are purged by an hourly sweep.

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Which kind of the athlete's data a coach's read reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoachReadClass {
    /// Their training: activities and what is derived from them.
    Training,
    /// Their wellness: sleep, HRV, resting heart rate, weight, readiness.
    Wellness,
}

impl CoachReadClass {
    /// The value the `data_class` column stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Training => "training",
            Self::Wellness => "wellness",
        }
    }

    /// The class a stored `data_class` names.
    ///
    /// # Errors
    ///
    /// Returns a database error for a value the column's check would refuse.
    pub fn parse(stored: &str) -> AppResult<Self> {
        match stored {
            "training" => Ok(Self::Training),
            "wellness" => Ok(Self::Wellness),
            other => Err(AppError::database(format!(
                "coach_data_reads.data_class holds an unknown class {other:?}"
            ))),
        }
    }
}

/// Where a coach read that is no tool call was made, as its row names it in
/// place of a tool.
///
/// A read through a tool names the tool. These reads run outside a tool call,
/// so each records a stable label of its own, which the athlete's clients
/// word in plain language rather than show as a tool name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoachReadSurface {
    /// Group info's read-only view of the athlete's live plan
    /// (`GET /api/chat/athletes/{id}/plan`).
    PlanView,
    /// `/plan <athlete>`, answered to the coach alone.
    PlanCommand,
    /// The coach's roster, Coach Home (`GET /api/coach/roster`).
    Roster,
    /// The coach's thread about them: the athlete block every turn carries
    /// (their roster line, plan and draft, and linked wellness), and the
    /// thread's plan card read and Publish button.
    CoachThread,
    /// The member cards the coach's agent reads on a turn in the group's
    /// room. Recorded at most once a day.
    GroupRoom,
    /// The group's stats (`GET /api/groups/{id}/stats`). Recorded at most
    /// once a day.
    GroupStats,
    /// The group's weekly report (`GET /api/groups/{id}/report`). Recorded at
    /// most once a day.
    GroupReport,
    /// The group's health flags (`GET /api/groups/{id}/health`). Recorded at
    /// most once a day.
    GroupHealth,
    /// The coach's copy of the group's weekly digest. Recorded at most once a
    /// day.
    WeeklyDigest,
}

impl CoachReadSurface {
    /// Every surface, for a caller that maps each to its words.
    pub const ALL: [Self; 9] = [
        Self::PlanView,
        Self::PlanCommand,
        Self::Roster,
        Self::CoachThread,
        Self::GroupRoom,
        Self::GroupStats,
        Self::GroupReport,
        Self::GroupHealth,
        Self::WeeklyDigest,
    ];

    /// The label the `tool_name` column stores for a read made here.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlanView => "plan_view",
            Self::PlanCommand => "plan_command",
            Self::Roster => "roster",
            Self::CoachThread => "coach_thread",
            Self::GroupRoom => "group_room",
            Self::GroupStats => "group_stats",
            Self::GroupReport => "group_report",
            Self::GroupHealth => "group_health",
            Self::WeeklyDigest => "weekly_digest",
        }
    }

    /// The surface a stored `tool_name` names, or `None` for a tool.
    #[must_use]
    pub fn of_label(stored: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|surface| surface.as_str() == stored)
    }
}

/// One read of an athlete's data by their human coach, as it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCoachDataRead<'read> {
    /// The athlete's one home tenant, where the data read lives.
    pub tenant: TenantId,
    /// The athlete whose data was read.
    pub athlete: Uuid,
    /// The coach who read it.
    pub coach: Uuid,
    /// The group the coach coaches the athlete in.
    pub group: Uuid,
    /// The tool the coach's agent read through, or the
    /// [`CoachReadSurface`] label of a read made outside a tool call.
    pub tool: &'read str,
    /// Which kind of data the read reached.
    pub class: CoachReadClass,
    /// When the coach read it.
    pub read_at: DateTime<Utc>,
}

/// One recorded read, as the athlete lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoachDataRead {
    /// The coach who read; `None` once that account is deleted.
    pub coach: Option<Uuid>,
    /// The group the coach coaches the athlete in.
    pub group: Uuid,
    /// The tool the coach's agent read through.
    pub tool: String,
    /// Which kind of data the read reached.
    pub class: CoachReadClass,
    /// When the coach read it.
    pub read_at: DateTime<Utc>,
}

/// Record and list the athlete's record of their coach's reads.
#[async_trait]
pub trait CoachDataReadRepository: Send + Sync {
    /// Record one read.
    async fn record_coach_read(&self, read: &NewCoachDataRead<'_>) -> AppResult<()>;

    /// Record one read at most once a UTC day: a read of the same athlete by
    /// the same coach in the same group, through the same source and of the
    /// same class, already recorded on `read.read_at`'s UTC day writes
    /// nothing. `true` when this call wrote the row, `false` on such a
    /// repeat, which is no failure.
    ///
    /// For the group views a coach opens on every turn or page load (the
    /// room's member cards, the group's stats, report and health flags, the
    /// weekly digest), which would otherwise write a row per view.
    async fn record_daily_coach_read(&self, read: &NewCoachDataRead<'_>) -> AppResult<bool>;

    /// `athlete`'s recorded reads under `tenant` in `group`, newest first, at
    /// most `limit`.
    async fn list_coach_reads(
        &self,
        tenant: TenantId,
        athlete: Uuid,
        group: Uuid,
        limit: u32,
    ) -> AppResult<Vec<CoachDataRead>>;

    /// Delete every read recorded before `cutoff`, in every tenant, returning
    /// how many went.
    async fn purge_coach_reads_before(&self, cutoff: DateTime<Utc>) -> AppResult<u64>;
}

/// Record one read. `$n` placeholders throughout: sqlx accepts them on
/// `SQLite` as well as Postgres, so one statement serves both backends. The
/// ids bind through the backend's uuid codec (see [`super::uuid_columns`]);
/// the tenant, group and day are `TEXT` on both.
///
/// `$9`, the `read_day`, is the UTC day of a read recorded once a day and
/// `NULL` for one recorded each time. The conflict target is the unique
/// index `idx_coach_data_reads_daily`, which holds the day: a repeat that day
/// writes nothing, and a `NULL` day, distinct from every other on both
/// backends, never conflicts.
pub(crate) const RECORD_COACH_READ_SQL: &str = r"
            INSERT INTO coach_data_reads
                (id, tenant_id, user_id, coach_user_id, group_id, tool_name, data_class, read_at,
                 read_day)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (tenant_id, user_id, coach_user_id, group_id, tool_name, data_class, read_day)
            DO NOTHING
            ";

/// The UTC day a read recorded once a day is keyed on, as `read_day` stores
/// it.
pub(crate) fn read_day(read: &NewCoachDataRead<'_>) -> String {
    read.read_at.date_naive().format("%Y-%m-%d").to_string()
}

/// The athlete's reads in one group under their tenant, newest first.
pub(crate) const LIST_COACH_READS_SQL: &str = r"
            SELECT coach_user_id, group_id, tool_name, data_class, read_at
            FROM coach_data_reads
            WHERE tenant_id = $1 AND user_id = $2 AND group_id = $3
            ORDER BY read_at DESC
            LIMIT $4
            ";

/// The retention purge.
///
/// ISOLATION EXEMPTION: the purge spans every tenant on purpose. It serves
/// the one retention every tenant's athletes are under, carries no data out
/// and selects by age alone, so a tenant filter would only make the sweep
/// walk the tenants one by one to the same end.
pub(crate) const PURGE_COACH_READS_SQL: &str = r"
            DELETE FROM coach_data_reads WHERE read_at < $1
            ";

/// The error for a read or write of this table that failed.
pub(crate) fn coach_read_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} coach data read: {e}"))
}

/// Emit the whole [`CoachDataReadRepository`] implementation for one backend
/// type. `$ids` is the codec in [`super::uuid_columns`] for how that
/// backend's id columns bind and read.
macro_rules! impl_coach_data_read_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl CoachDataReadRepository for $ty {
            async fn record_coach_read(&self, read: &NewCoachDataRead<'_>) -> AppResult<()> {
                sqlx::query(RECORD_COACH_READ_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind(read.tenant.to_string())
                    .bind($ids::bind(read.athlete))
                    .bind($ids::bind(read.coach))
                    .bind(read.group.to_string())
                    .bind(read.tool)
                    .bind(read.class.as_str())
                    .bind(read.read_at)
                    .bind(None::<String>)
                    .execute(self.pool())
                    .await
                    .map_err(|e| coach_read_error("record", e))?;
                Ok(())
            }

            async fn record_daily_coach_read(
                &self,
                read: &NewCoachDataRead<'_>,
            ) -> AppResult<bool> {
                let done = sqlx::query(RECORD_COACH_READ_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind(read.tenant.to_string())
                    .bind($ids::bind(read.athlete))
                    .bind($ids::bind(read.coach))
                    .bind(read.group.to_string())
                    .bind(read.tool)
                    .bind(read.class.as_str())
                    .bind(read.read_at)
                    .bind(Some(read_day(read)))
                    .execute(self.pool())
                    .await
                    .map_err(|e| coach_read_error("record", e))?;
                Ok(done.rows_affected() > 0)
            }

            async fn list_coach_reads(
                &self,
                tenant: TenantId,
                athlete: Uuid,
                group: Uuid,
                limit: u32,
            ) -> AppResult<Vec<CoachDataRead>> {
                let rows = sqlx::query(LIST_COACH_READS_SQL)
                    .bind(tenant.to_string())
                    .bind($ids::bind(athlete))
                    .bind(group.to_string())
                    .bind(i64::from(limit))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| coach_read_error("list", e))?;
                rows.iter()
                    .map(|row| {
                        let group: String = row
                            .try_get("group_id")
                            .map_err(|e| coach_read_error("decode", e))?;
                        let class: String = row
                            .try_get("data_class")
                            .map_err(|e| coach_read_error("decode", e))?;
                        Ok(CoachDataRead {
                            coach: $ids::read_opt(row, "coach_user_id")?,
                            group: Uuid::parse_str(&group)
                                .map_err(|e| coach_read_error("decode", e))?,
                            tool: row
                                .try_get("tool_name")
                                .map_err(|e| coach_read_error("decode", e))?,
                            class: CoachReadClass::parse(&class)?,
                            read_at: row
                                .try_get("read_at")
                                .map_err(|e| coach_read_error("decode", e))?,
                        })
                    })
                    .collect()
            }

            async fn purge_coach_reads_before(&self, cutoff: DateTime<Utc>) -> AppResult<u64> {
                let done = sqlx::query(PURGE_COACH_READS_SQL)
                    .bind(cutoff)
                    .execute(self.pool())
                    .await
                    .map_err(|e| coach_read_error("purge", e))?;
                Ok(done.rows_affected())
            }
        }
    };
}
pub(crate) use impl_coach_data_read_repository;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_class_reads_back_as_itself_and_nothing_else_parses() {
        for class in [CoachReadClass::Training, CoachReadClass::Wellness] {
            assert_eq!(
                CoachReadClass::parse(class.as_str()).unwrap(),
                class,
                "bug: a stored class reads back as the class it stores"
            );
        }
        assert!(
            CoachReadClass::parse("memory").is_err(),
            "bug: a class the column never stores is a corrupt row"
        );
    }

    #[test]
    fn each_surface_label_reads_back_as_itself_and_a_tool_name_as_none() {
        for surface in CoachReadSurface::ALL {
            assert_eq!(
                CoachReadSurface::of_label(surface.as_str()),
                Some(surface),
                "bug: a stored surface label reads back as the surface"
            );
        }
        assert_eq!(
            CoachReadSurface::of_label("get_activities"),
            None,
            "bug: a tool name is no surface"
        );
    }
}
