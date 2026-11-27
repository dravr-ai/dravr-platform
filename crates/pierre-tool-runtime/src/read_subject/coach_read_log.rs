// ABOUTME: The coach-read record: one row on the athlete's own record per read a coach makes of them, or one a day for group views
// ABOUTME: record_coach_read and record_daily_coach_read, which read_subject re-exports

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_database::repositories::{CoachReadClass, NewCoachDataRead};
use pierre_database::RepositoryRegistry;
use pierre_services::coached_athlete::CoachedAthleteAccess;
use uuid::Uuid;

use super::peer_read;

/// Record on the athlete's own record that `coach` read their data of `class`
/// through `tool`, now (carnet#1445).
///
/// One row per read, as their coach in `access`'s group, under the athlete's
/// home tenant, which the athlete lists and which is purged after its
/// retention.
///
/// `tool` is the tool read through, or for a read made outside a tool call
/// its [`CoachReadSurface`](pierre_database::repositories::CoachReadSurface)
/// label. The transport the call declared is recorded beside it, so the
/// athlete's record says when a read was served to the coach's outside
/// assistant (EFVP 9.4b). So is the admin whose support session made the
/// read while acting as the coach ([`SUPPORT_SESSION`](crate::context::SUPPORT_SESSION)), so the athlete's
/// record says it was support's read, not their coach's.
///
/// Every coach read of a coached athlete records here once the gates have let
/// it through and before its data is read: the dispatch chokepoint for a
/// call that resolved its subject itself (never for the nested calls it
/// makes); each `groups` tool for the member it reads; the plan scope for
/// every plan tool acting for a coached athlete
/// ([`crate::implementations::plan_scope`]), and its wellness when
/// `get_training_plan` reads the plan's state; a coach's plan read outside a
/// tool call (`coached_plan_read_access`: the plan view and `/plan`); the
/// coach's REST roster; the athlete block of the coach's thread, once per
/// class per turn; and a training tool's read of the athlete's wellness
/// under their grant (`record_wellness_read`). The group views built from
/// every member's training record once a day instead
/// ([`record_daily_coach_read`]). A refused read records nothing. A record
/// that cannot be written fails the read: no read goes unrecorded.
///
/// # Errors
///
/// Returns the repository error from the insert.
pub async fn record_coach_read(
    repos: &RepositoryRegistry,
    access: &CoachedAthleteAccess,
    coach: Uuid,
    tool: &str,
    class: CoachReadClass,
) -> AppResult<()> {
    repos
        .coach_data_reads
        .record_coach_read(&new_coach_read(access, coach, tool, class))
        .await
}

/// [`record_coach_read`] at most once a UTC day per athlete, coach, group,
/// source and class.
///
/// For the group views a coach opens on every turn or page load — the room's
/// member cards, the group's stats, report and health flags, the coach's
/// weekly digest (carnet#1445).
///
/// `true` when the row was written, `false` when that day's row already was:
/// a repeat is no failure, and the read goes ahead either way. A read in a
/// support session is recorded each time instead: folded into the coach's
/// own row of the day, the athlete would never see that support read them.
///
/// # Errors
///
/// Returns the repository error from the insert: the read is refused rather
/// than left unrecorded.
pub async fn record_daily_coach_read(
    repos: &RepositoryRegistry,
    access: &CoachedAthleteAccess,
    coach: Uuid,
    tool: &str,
    class: CoachReadClass,
) -> AppResult<bool> {
    let read = new_coach_read(access, coach, tool, class);
    if read.impersonator.is_some() {
        repos.coach_data_reads.record_coach_read(&read).await?;
        return Ok(true);
    }
    repos.coach_data_reads.record_daily_coach_read(&read).await
}

/// The row recording `coach`'s read of `access`'s athlete now, with the
/// transport the call declared and the admin of the support session it runs
/// in, if any.
fn new_coach_read<'read>(
    access: &CoachedAthleteAccess,
    coach: Uuid,
    tool: &'read str,
    class: CoachReadClass,
) -> NewCoachDataRead<'read> {
    let (transport, impersonator) = peer_read::served_now();
    NewCoachDataRead {
        tenant: access.tenant,
        athlete: access.athlete_user_id,
        coach,
        group: access.group.id,
        tool,
        class,
        read_at: Timestamp::now(),
        transport,
        impersonator,
    }
}
