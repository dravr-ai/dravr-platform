// ABOUTME: Repository trait, statements and shared body for the history of changes to an athlete's plan made in a coach's workspace
// ABOUTME: One row per change by the coach or the thread's agent, tenant-scoped under the athlete's tenant, listed to that coach newest first

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Plan change history (carnet#1815).
//!
//! A coach drafts a coached athlete's plan with the thread's agent: the coach
//! writes days by hand or from a family, adds, removes, restores, reverts a
//! date to the live plan and makes their call on a session; the agent saves
//! whole weeks into the draft; the coach publishes or discards it. Each of
//! those changes, once it landed, writes one row here naming who made it,
//! what kind of change it was and the dates it touched. The coach's thread
//! lists that coach's rows on the athlete's plan, newest first.
//!
//! Every statement filters on the athlete's tenant and the athlete; the
//! listing filters on the coach too, so no coach reads another coach's
//! changes. Rows are written after the change they record, never in its
//! transaction: the service in front of this repository logs a failed write
//! and never fails the change for it.

use std::fmt::Display;

use async_trait::async_trait;
use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Who made a change to an athlete's plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanChangeSource {
    /// The coach, from their own day editor or buttons.
    Coach,
    /// The thread's agent, on the coach's word.
    Agent,
}

impl PlanChangeSource {
    /// Every source, for a caller that maps each to its words.
    pub const ALL: [Self; 2] = [Self::Coach, Self::Agent];

    /// The value the `source` column stores, and the wire carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coach => "coach",
            Self::Agent => "agent",
        }
    }

    /// The source a stored `source` names.
    ///
    /// # Errors
    ///
    /// Returns a database error for a value no source stores.
    pub fn parse(stored: &str) -> AppResult<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.as_str() == stored)
            .ok_or_else(|| {
                AppError::database(format!(
                    "plan_change_events.source holds an unknown source {stored:?}"
                ))
            })
    }
}

/// What kind of change was made to an athlete's plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanChangeKind {
    /// A session of the draft rewritten: by hand, as a rest entry, or from a family.
    DayEdit,
    /// A session added to a date of the draft.
    DayAdd,
    /// A session removed from a date of the draft.
    DayRemove,
    /// A session put back as it stood before the coach's first edit, or taken out.
    ///
    /// A session the coach added is taken out; one they edited is put back.
    DayRestore,
    /// Every entry on a date of the draft replaced by the live plan's.
    DateRevert,
    /// The coach's call on a session made or taken off.
    CoachCall,
    /// Weeks or an outline the agent saved into the draft.
    AgentDraft,
    /// The draft made the athlete's live plan.
    Publish,
    /// The draft thrown away.
    Discard,
}

impl PlanChangeKind {
    /// Every kind, for a caller that maps each to its words.
    pub const ALL: [Self; 9] = [
        Self::DayEdit,
        Self::DayAdd,
        Self::DayRemove,
        Self::DayRestore,
        Self::DateRevert,
        Self::CoachCall,
        Self::AgentDraft,
        Self::Publish,
        Self::Discard,
    ];

    /// The value the `kind` column stores, and the wire carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DayEdit => "day_edit",
            Self::DayAdd => "day_add",
            Self::DayRemove => "day_remove",
            Self::DayRestore => "day_restore",
            Self::DateRevert => "date_revert",
            Self::CoachCall => "coach_call",
            Self::AgentDraft => "agent_draft",
            Self::Publish => "publish",
            Self::Discard => "discard",
        }
    }

    /// The kind a stored `kind` names.
    ///
    /// # Errors
    ///
    /// Returns a database error for a value no kind stores.
    pub fn parse(stored: &str) -> AppResult<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == stored)
            .ok_or_else(|| {
                AppError::database(format!(
                    "plan_change_events.kind holds an unknown kind {stored:?}"
                ))
            })
    }
}

/// One change to an athlete's plan, as it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPlanChangeEvent<'event> {
    /// The athlete's one home tenant, where the plan lives.
    pub tenant: TenantId,
    /// Whose plan changed.
    pub athlete: Uuid,
    /// The coach in whose workspace it was made: the listing reads a coach's
    /// own changes only, so every change names its coach.
    pub coach: Uuid,
    /// Who made it.
    pub source: PlanChangeSource,
    /// What kind of change it was.
    pub kind: PlanChangeKind,
    /// The dates it touched, in any order; stored ascending, each once.
    pub dates: &'event [CivilDate],
    /// When it landed.
    pub created_at: Timestamp,
}

/// One recorded change, as the coach lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanChangeEvent {
    /// What kind of change it was.
    pub kind: PlanChangeKind,
    /// Who made it.
    pub source: PlanChangeSource,
    /// The dates it touched, ascending; empty for a change that names none.
    pub dates: Vec<CivilDate>,
    /// When it landed.
    pub created_at: Timestamp,
}

/// Which changes a listing reads: one coach's on one athlete's plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanChangeQuery {
    /// The athlete's tenant.
    pub tenant: TenantId,
    /// Whose plan.
    pub athlete: Uuid,
    /// The coach whose changes are listed.
    pub coach: Uuid,
    /// Only the changes landed strictly before this instant; `None` for the newest.
    pub before: Option<Timestamp>,
    /// At most this many.
    pub limit: u32,
}

/// Record and list the changes made to an athlete's plan in a coach's workspace.
#[async_trait]
pub trait PlanChangeEventRepository: Send + Sync {
    /// Record one change.
    async fn record_plan_change(&self, event: &NewPlanChangeEvent<'_>) -> AppResult<()>;

    /// The changes `query` names, newest first.
    async fn list_plan_changes(&self, query: &PlanChangeQuery) -> AppResult<Vec<PlanChangeEvent>>;
}

/// Record one change. `$n` placeholders throughout: sqlx accepts them on
/// `SQLite` as well as Postgres, so one statement serves both backends. The
/// ids bind through the backend's uuid codec (see [`super::uuid_columns`]);
/// the tenant and the dates are `TEXT` on both.
pub(crate) const RECORD_PLAN_CHANGE_SQL: &str = r"
            INSERT INTO plan_change_events
                (id, tenant_id, athlete_user_id, coach_user_id, source, kind, dates, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ";

/// One coach's changes on one athlete's plan under the athlete's tenant,
/// newest first.
pub(crate) const LIST_PLAN_CHANGES_SQL: &str = r"
            SELECT source, kind, dates, created_at
            FROM plan_change_events
            WHERE tenant_id = $1 AND athlete_user_id = $2 AND coach_user_id = $3
            ORDER BY created_at DESC, id DESC
            LIMIT $4
            ";

/// The same, landed strictly before `$4`.
pub(crate) const LIST_PLAN_CHANGES_BEFORE_SQL: &str = r"
            SELECT source, kind, dates, created_at
            FROM plan_change_events
            WHERE tenant_id = $1 AND athlete_user_id = $2 AND coach_user_id = $3
              AND created_at < $4
            ORDER BY created_at DESC, id DESC
            LIMIT $5
            ";

/// The error for a read or write of this table that failed.
pub(crate) fn plan_change_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} plan change event: {e}"))
}

/// `dates` as the `dates` column stores them: a JSON array of `YYYY-MM-DD`,
/// ascending, each once.
///
/// # Errors
///
/// Returns a database error when the array does not encode.
pub(crate) fn encode_dates(dates: &[CivilDate]) -> AppResult<String> {
    let mut sorted = dates.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    serde_json::to_string(&sorted).map_err(|e| plan_change_error("encode", e))
}

/// The dates a stored `dates` column holds.
///
/// # Errors
///
/// Returns a database error for text that is not an array of dates.
pub(crate) fn decode_dates(stored: &str) -> AppResult<Vec<CivilDate>> {
    serde_json::from_str(stored).map_err(|e| plan_change_error("decode", e))
}

/// Emit the whole [`PlanChangeEventRepository`] implementation for one
/// backend type, whose rows are `$row`. `$ids` is the codec in
/// [`super::uuid_columns`] for how that backend's id columns bind.
macro_rules! impl_plan_change_event_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// One listed row as the change it records.
        fn plan_change_from_row(row: &$row) -> AppResult<PlanChangeEvent> {
            let text = |col: &str| -> AppResult<String> {
                row.try_get(col).map_err(|e| plan_change_error("decode", e))
            };
            Ok(PlanChangeEvent {
                kind: PlanChangeKind::parse(&text("kind")?)?,
                source: PlanChangeSource::parse(&text("source")?)?,
                dates: decode_dates(&text("dates")?)?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| plan_change_error("decode", e))?,
            })
        }

        #[async_trait::async_trait]
        impl PlanChangeEventRepository for $ty {
            async fn record_plan_change(&self, event: &NewPlanChangeEvent<'_>) -> AppResult<()> {
                sqlx::query(RECORD_PLAN_CHANGE_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind(event.tenant.to_string())
                    .bind($ids::bind(event.athlete))
                    .bind($ids::bind(event.coach))
                    .bind(event.source.as_str())
                    .bind(event.kind.as_str())
                    .bind(encode_dates(event.dates)?)
                    .bind(event.created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| plan_change_error("record", e))?;
                Ok(())
            }

            async fn list_plan_changes(
                &self,
                query: &PlanChangeQuery,
            ) -> AppResult<Vec<PlanChangeEvent>> {
                let statement = match query.before {
                    Some(before) => sqlx::query(LIST_PLAN_CHANGES_BEFORE_SQL)
                        .bind(query.tenant.to_string())
                        .bind($ids::bind(query.athlete))
                        .bind($ids::bind(query.coach))
                        .bind(before),
                    None => sqlx::query(LIST_PLAN_CHANGES_SQL)
                        .bind(query.tenant.to_string())
                        .bind($ids::bind(query.athlete))
                        .bind($ids::bind(query.coach)),
                };
                let rows = statement
                    .bind(i64::from(query.limit))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| plan_change_error("list", e))?;
                rows.iter().map(plan_change_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_plan_change_event_repository;

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_tronc::dates::Elapsed;

    #[test]
    fn each_source_and_kind_reads_back_as_itself_and_nothing_else_parses() {
        for source in PlanChangeSource::ALL {
            assert_eq!(
                PlanChangeSource::parse(source.as_str()).unwrap(),
                source,
                "bug: a stored source reads back as the source it stores"
            );
        }
        for kind in PlanChangeKind::ALL {
            assert_eq!(
                PlanChangeKind::parse(kind.as_str()).unwrap(),
                kind,
                "bug: a stored kind reads back as the kind it stores"
            );
        }
        assert!(
            PlanChangeSource::parse("athlete").is_err(),
            "bug: a source the server never writes is a corrupt row"
        );
        assert!(
            PlanChangeKind::parse("day_moved").is_err(),
            "bug: a kind the server never writes is a corrupt row"
        );
    }

    #[test]
    fn dates_are_stored_ascending_each_once_and_read_back() {
        let tuesday: CivilDate = "2026-11-24".parse().unwrap();
        let monday = tuesday - Elapsed::days(1);
        let stored = encode_dates(&[tuesday, monday, tuesday]).unwrap();
        assert_eq!(
            stored, r#"["2026-11-23","2026-11-24"]"#,
            "bug: the column holds the dates ascending, each once, as YYYY-MM-DD"
        );
        assert_eq!(
            decode_dates(&stored).unwrap(),
            [monday, tuesday],
            "bug: the stored dates read back as written"
        );
        assert_eq!(
            encode_dates(&[]).unwrap(),
            "[]",
            "bug: a change that names no date stores an empty array"
        );
        assert!(
            decode_dates(r#"["2026-11-31"]"#).is_err(),
            "bug: a stored date that is no date is a corrupt row"
        );
    }
}
