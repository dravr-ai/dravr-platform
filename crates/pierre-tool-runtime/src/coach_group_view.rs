// ABOUTME: Whose training a group view built from every member's shows its viewer, and for the group's coach the gate and record of each read
// ABOUTME: The room's member cards, /stats, /report, /health and the coach's weekly digest all ask here before any snapshot is read (carnet#1445)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A group view, as its viewer may read it
//!
//! Several views are built from every member's training at once: the member
//! cards the agent reads on a turn in the group's room, the group's stats,
//! weekly report and health flags, and each manager's copy of the weekly
//! digest. Which members' training a viewer may see in them is
//! [`GroupService::visible_member_ids`]: the group's human coach the members
//! who share with the coach, anyone else the members who share with their
//! peers (carnet#1166).
//!
//! For the group's human coach that rule is the cheap first cut, from the
//! membership rows alone. Each member it lets through is then asked the one
//! answer every coach read of a coached athlete asks,
//! [`read_subject::coached_read`] (training class): the coach attachment, a
//! live membership, the member's `coach_sharing_consent` and one home tenant
//! (carnet#1425). A member it refuses is left out of the view and nothing of
//! theirs is read. Each read it lets through goes on the member's own record
//! before their training is read, at most once a UTC day per view
//! ([`read_subject::record_daily_coach_read`], carnet#1445), and their
//! training is read under their home tenant, as the coach's other reads of
//! them are.
//!
//! `visible_member_ids` stays a pure rule over the membership rows in
//! `pierre-groups`, which every crate reads and which reads no repository;
//! the gate's tenant lookup and the record are repository reads and writes,
//! so they run here, where the callers of every coach view can reach them.
//!
//! Any other viewer reads their peers' sharing, which is no coach read: the
//! view is the rule's set, with nothing gated or recorded.

use std::collections::HashSet;

use futures_util::stream::{self, StreamExt};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{CoachingGroup, GroupMember};
use pierre_core::models::TenantId;
use pierre_database::repositories::{CoachReadClass, CoachReadSurface};
use pierre_database::RepositoryRegistry;
use pierre_groups::GroupService;
use pierre_services::coach_roster::ROSTER_LOOKUP_CONCURRENCY;
use pierre_services::coached_athlete::CoachedAthleteAccess;
use tracing::warn;
use uuid::Uuid;

use crate::read_subject::{self, DataClass};

/// The members a group view shows its viewer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupView {
    /// The members whose training the view reads, in membership order, each
    /// with the tenant their training is read under: a member the coach's
    /// gate let through under their home tenant, anyone else under the
    /// viewer's.
    pub readable: Vec<(Uuid, TenantId)>,
    /// The members the coach's gate refused: nothing of theirs is read.
    pub refused: Vec<Uuid>,
    /// The members whose read could not be recorded, in a view that leaves
    /// them out rather than failing ([`gate_group_view_skipping`]): nothing
    /// of theirs is read.
    pub unrecorded: Vec<Uuid>,
}

impl GroupView {
    /// The members whose training the view reads.
    #[must_use]
    pub fn member_ids(&self) -> HashSet<Uuid> {
        self.readable.iter().map(|&(id, _)| id).collect()
    }

    /// The tenant `member`'s training is read under, when the view reads it.
    #[must_use]
    pub fn tenant_of(&self, member: Uuid) -> Option<TenantId> {
        self.readable
            .iter()
            .find(|&&(id, _)| id == member)
            .map(|&(_, tenant)| tenant)
    }
}

/// The members of `group` whose training `viewer` may see in the view
/// `surface` names, each read the group's human coach makes gated and on the
/// member's record before anything of theirs is read.
///
/// `viewer_tenant` is the tenant the view is opened under: the viewer's own
/// training, and every member's in a view that is not the coach's, is read
/// under it.
///
/// # Errors
///
/// Returns the repository error from a coach gate or from a record's insert:
/// the view is refused rather than read with a read left off a member's
/// record.
pub async fn gate_group_view(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    members: &[GroupMember],
    viewer: Uuid,
    viewer_tenant: TenantId,
    surface: CoachReadSurface,
) -> AppResult<GroupView> {
    let candidates = in_view(group, members, viewer);
    if group.coach_user_id != Some(viewer) {
        return Ok(uncoached_view(&candidates, viewer_tenant));
    }
    let mut view = GroupView::default();
    for (member, read) in gate_members(repos, group, viewer, &candidates, surface).await {
        match read? {
            MemberRead::Own => view.readable.push((member, viewer_tenant)),
            MemberRead::Coached(home) => view.readable.push((member, home)),
            MemberRead::Refused => view.refused.push(member),
        }
    }
    Ok(view)
}

/// [`gate_group_view`] for a view that goes out to many readers in one pass.
///
/// For the weekly digest: a member whose gate or record fails is left out of
/// the coach's view with a warning (`unrecorded`) and nothing of theirs is
/// read, rather than the whole view failing for the others.
pub async fn gate_group_view_skipping(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    members: &[GroupMember],
    viewer: Uuid,
    viewer_tenant: TenantId,
    surface: CoachReadSurface,
) -> GroupView {
    let candidates = in_view(group, members, viewer);
    if group.coach_user_id != Some(viewer) {
        return uncoached_view(&candidates, viewer_tenant);
    }
    let mut view = GroupView::default();
    for (member, read) in gate_members(repos, group, viewer, &candidates, surface).await {
        match read {
            Ok(MemberRead::Own) => view.readable.push((member, viewer_tenant)),
            Ok(MemberRead::Coached(home)) => view.readable.push((member, home)),
            Ok(MemberRead::Refused) => view.refused.push(member),
            Err(e) => {
                warn!(
                    group_id = %group.id,
                    coach = %viewer,
                    member = %member,
                    surface = surface.as_str(),
                    error = %e,
                    "coach group view: the member's read could not be gated or recorded; \
                     their training is left out"
                );
                view.unrecorded.push(member);
            }
        }
    }
    view
}

/// What the coach's view reads of one member.
enum MemberRead {
    /// The coach themself: their own training, which no record covers.
    Own,
    /// A member the coach's gate lets them read, their read on their record:
    /// their home tenant, which their training is read under.
    Coached(TenantId),
    /// A member the coach's gate refuses.
    Refused,
}

/// The members [`GroupService::visible_member_ids`] lets `viewer` see, in
/// membership order.
fn in_view(group: &CoachingGroup, members: &[GroupMember], viewer: Uuid) -> Vec<Uuid> {
    let visible = GroupService::visible_member_ids(group, members, viewer);
    members
        .iter()
        .map(|member| member.user_id)
        .filter(|id| visible.contains(id))
        .collect()
}

/// A view that is no coach's: every candidate, read under the viewer's tenant.
fn uncoached_view(candidates: &[Uuid], viewer_tenant: TenantId) -> GroupView {
    GroupView {
        readable: candidates.iter().map(|&id| (id, viewer_tenant)).collect(),
        ..GroupView::default()
    }
}

/// Ask the coach's gate for each candidate, a bounded number at a time, in
/// their order, and put each read it lets through on the member's record.
async fn gate_members(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    coach: Uuid,
    candidates: &[Uuid],
    surface: CoachReadSurface,
) -> Vec<(Uuid, AppResult<MemberRead>)> {
    stream::iter(candidates.iter().copied())
        .map(|member| async move {
            (
                member,
                gate_member(repos, group, coach, member, surface).await,
            )
        })
        .buffered(ROSTER_LOOKUP_CONCURRENCY)
        .collect()
        .await
}

/// What the coach's view reads of `member`: the coach's own training, or the
/// gate's answer with the read on the member's record.
async fn gate_member(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    coach: Uuid,
    member: Uuid,
    surface: CoachReadSurface,
) -> AppResult<MemberRead> {
    if member == coach {
        return Ok(MemberRead::Own);
    }
    match read_subject::coached_read(repos, coach, member, group.id, DataClass::Training).await? {
        Ok(access) => {
            record_view_read(repos, &access, coach, surface).await?;
            Ok(MemberRead::Coached(access.tenant))
        }
        Err(_) => Ok(MemberRead::Refused),
    }
}

/// Put the coach's read of `access`'s member through the view `surface`
/// names on the member's record, once a day; a repeat that day is no
/// failure.
async fn record_view_read(
    repos: &RepositoryRegistry,
    access: &CoachedAthleteAccess,
    coach: Uuid,
    surface: CoachReadSurface,
) -> AppResult<()> {
    read_subject::record_daily_coach_read(
        repos,
        access,
        coach,
        surface.as_str(),
        CoachReadClass::Training,
    )
    .await
    .map(drop)
}
