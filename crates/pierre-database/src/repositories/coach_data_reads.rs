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
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::transport::{Transport, UNDECLARED_TRANSPORT};
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
    /// Group info's view of the athlete's thresholds, where the coach also
    /// sets one (`GET`/`PUT /api/chat/athletes/{id}/physiology`, carnet#1561).
    PhysiologyView,
}

impl CoachReadSurface {
    /// Every surface, for a caller that maps each to its words.
    pub const ALL: [Self; 10] = [
        Self::PlanView,
        Self::PlanCommand,
        Self::Roster,
        Self::CoachThread,
        Self::GroupRoom,
        Self::GroupStats,
        Self::GroupReport,
        Self::GroupHealth,
        Self::WeeklyDigest,
        Self::PhysiologyView,
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
            Self::PhysiologyView => "physiology_view",
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
    pub read_at: Timestamp,
    /// The transport the read was served over: an external one when the
    /// coach's outside assistant (an MCP client, an A2A agent, an API key)
    /// received the data, a first-party one for Dravr's own surfaces.
    /// `None` when the read declared no transport, which is stored under its
    /// own label ([`UNDECLARED_TRANSPORT`]).
    pub transport: Option<Transport>,
    /// The admin whose support session made the read while acting as the
    /// coach; `None` for a read the coach made themselves.
    pub impersonator: Option<Uuid>,
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
    pub read_at: Timestamp,
    /// What the row says the read was served over.
    pub transport: RecordedTransport,
    /// The admin whose support session made the read while acting as the
    /// coach; `None` for the coach's own read.
    pub impersonator: Option<Uuid>,
    /// The read is a group view recorded at most once a UTC day
    /// ([`CoachDataReadRepository::record_daily_coach_read`]), so it stands
    /// for every view of that source that day; `false` for a read recorded
    /// each time it is made.
    pub once_a_day: bool,
}

/// What a recorded read says it was served over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedTransport {
    /// The transport the read declared.
    Served(Transport),
    /// The read declared no transport.
    Undeclared,
    /// The read was recorded before the transport was.
    NotRecorded,
}

impl RecordedTransport {
    /// Whether the read was served to a client outside Dravr's own surfaces —
    /// the coach's outside assistant over MCP, A2A or an API key — so the
    /// company running it received the data. Only a declared external
    /// transport says so; an undeclared read and one recorded before the
    /// transport was are not marked, as nothing shows where they went.
    #[must_use]
    pub const fn served_to_outside_client(self) -> bool {
        matches!(self, Self::Served(transport) if !transport.is_first_party())
    }
}

/// What a stored `transport` says a read was served over, in this table or
/// in `peer_data_reads`.
///
/// # Errors
///
/// Returns a database error for a label no transport stores.
pub(crate) fn parse_transport(stored: Option<&str>) -> AppResult<RecordedTransport> {
    match stored {
        None => Ok(RecordedTransport::NotRecorded),
        Some(UNDECLARED_TRANSPORT) => Ok(RecordedTransport::Undeclared),
        Some(label) => Transport::of_label(label)
            .map(RecordedTransport::Served)
            .ok_or_else(|| {
                AppError::database(format!(
                    "a read record's transport holds an unknown transport {label:?}"
                ))
            }),
    }
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

    /// `athlete`'s recorded reads under `tenant` in every group, made at or
    /// after `since`, newest first, at most `limit`: what the athlete's "who
    /// read my data" feed counts per day (carnet#1543).
    async fn list_coach_reads_since(
        &self,
        tenant: TenantId,
        athlete: Uuid,
        since: Timestamp,
        limit: u32,
    ) -> AppResult<Vec<CoachDataRead>>;

    /// Delete every read recorded before `cutoff`, in every tenant, returning
    /// how many went.
    async fn purge_coach_reads_before(&self, cutoff: Timestamp) -> AppResult<u64>;
}

/// Record one read. `$n` placeholders throughout: sqlx accepts them on
/// `SQLite` as well as Postgres, so one statement serves both backends. The
/// ids bind through the backend's uuid codec (see [`super::uuid_columns`]);
/// the tenant, group and day are `TEXT` on both.
///
/// `$10` is the transport the read was served over ([`Transport::label_of`]:
/// [`UNDECLARED_TRANSPORT`] when it declared none). `NULL` is left to the
/// rows recorded before the column.
///
/// `$11` is the admin whose support session made the read, `NULL` for the
/// coach's own.
///
/// `$9`, the `read_day`, is the UTC day of a read recorded once a day and
/// `NULL` for one recorded each time. The conflict target is the unique
/// index `idx_coach_data_reads_daily`, which holds the day: a repeat that day
/// writes nothing, and a `NULL` day, distinct from every other on both
/// backends, never conflicts.
pub(crate) const RECORD_COACH_READ_SQL: &str = r"
            INSERT INTO coach_data_reads
                (id, tenant_id, user_id, coach_user_id, group_id, tool_name, data_class, read_at,
                 read_day, transport, impersonator_user_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (tenant_id, user_id, coach_user_id, group_id, tool_name, data_class, read_day)
            DO NOTHING
            ";

/// The UTC day a read recorded once a day is keyed on, as `read_day` stores
/// it.
pub(crate) fn read_day(read: &NewCoachDataRead<'_>) -> String {
    read.read_at.utc_date().to_string()
}

/// The athlete's reads in one group under their tenant, newest first.
pub(crate) const LIST_COACH_READS_SQL: &str = r"
            SELECT coach_user_id, group_id, tool_name, data_class, read_at, transport,
                   impersonator_user_id, read_day
            FROM coach_data_reads
            WHERE tenant_id = $1 AND user_id = $2 AND group_id = $3
            ORDER BY read_at DESC
            LIMIT $4
            ";

/// The athlete's reads under their tenant in every group since `$3`, newest
/// first.
pub(crate) const LIST_COACH_READS_SINCE_SQL: &str = r"
            SELECT coach_user_id, group_id, tool_name, data_class, read_at, transport,
                   impersonator_user_id, read_day
            FROM coach_data_reads
            WHERE tenant_id = $1 AND user_id = $2 AND read_at >= $3
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
/// type, whose rows are `$row`. `$ids` is the codec in [`super::uuid_columns`] for how that
/// backend's id columns bind and read.
macro_rules! impl_coach_data_read_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// One listed row as the read it records.
        fn coach_read_from_row(row: &$row) -> AppResult<CoachDataRead> {
            let text = |col: &str| -> AppResult<String> {
                row.try_get(col).map_err(|e| coach_read_error("decode", e))
            };
            let optional = |col: &str| -> AppResult<Option<String>> {
                row.try_get(col).map_err(|e| coach_read_error("decode", e))
            };
            Ok(CoachDataRead {
                coach: $ids::read_opt(row, "coach_user_id")?,
                group: Uuid::parse_str(&text("group_id")?)
                    .map_err(|e| coach_read_error("decode", e))?,
                tool: text("tool_name")?,
                class: CoachReadClass::parse(&text("data_class")?)?,
                read_at: row
                    .try_get("read_at")
                    .map_err(|e| coach_read_error("decode", e))?,
                transport: parse_transport(optional("transport")?.as_deref())?,
                impersonator: $ids::read_opt(row, "impersonator_user_id")?,
                once_a_day: optional("read_day")?.is_some(),
            })
        }

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
                    .bind(Transport::label_of(read.transport))
                    .bind($ids::bind_opt(read.impersonator))
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
                    .bind(Transport::label_of(read.transport))
                    .bind($ids::bind_opt(read.impersonator))
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
                rows.iter().map(coach_read_from_row).collect()
            }

            async fn list_coach_reads_since(
                &self,
                tenant: TenantId,
                athlete: Uuid,
                since: Timestamp,
                limit: u32,
            ) -> AppResult<Vec<CoachDataRead>> {
                let rows = sqlx::query(LIST_COACH_READS_SINCE_SQL)
                    .bind(tenant.to_string())
                    .bind($ids::bind(athlete))
                    .bind(since)
                    .bind(i64::from(limit))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| coach_read_error("list", e))?;
                rows.iter().map(coach_read_from_row).collect()
            }

            async fn purge_coach_reads_before(&self, cutoff: Timestamp) -> AppResult<u64> {
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

    #[test]
    fn only_a_read_served_over_an_external_transport_is_marked_as_outside() {
        for transport in Transport::ALL {
            assert_eq!(
                RecordedTransport::Served(transport).served_to_outside_client(),
                !transport.is_first_party(),
                "bug: {transport:?} is marked by its class"
            );
        }
        assert!(
            !RecordedTransport::Undeclared.served_to_outside_client(),
            "bug: an undeclared read shows nowhere it went"
        );
        assert!(
            !RecordedTransport::NotRecorded.served_to_outside_client(),
            "bug: a read from before the column shows nowhere it went"
        );
    }
}
