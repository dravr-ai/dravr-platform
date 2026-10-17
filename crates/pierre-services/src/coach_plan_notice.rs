// ABOUTME: Tells an athlete what their human coach did to their plan — published the coach's draft, or put it on their calendar
// ABOUTME: Sent in the athlete's own tenant, naming the coach by the roster rule; best-effort, the write it reports stands

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # An athlete told what their human coach did to their plan (carnet#1069)
//!
//! A group's human coach builds a coached athlete's plan in a draft the
//! athlete never sees. Two of the coach's acts then change what the athlete
//! reads without the athlete doing anything, and each tells them:
//!
//! - publishing the draft ([`publish_coach_draft`]) sends
//!   `coach_plan_published` ([`tell_plan_published`]);
//! - pushing the plan onto the athlete's provider calendar through the
//!   coach's link sends `coach_plan_pushed` ([`tell_plan_pushed`]), once the
//!   push wrote at least one workout there.
//!
//! Nothing else does: a draft save, a discard, a refused or conflicted
//! publish, a push that wrote nothing and the athlete's own push tell no one,
//! and the coach is never the recipient.
//!
//! The notice lands in the athlete's tenant — the one the coaching gates
//! found them in, where their feed is read — never the coach's. It names the
//! coach by [`roster_display_name`], the rule every roster names a member by.
//!
//! Both are best-effort. The write they report has landed, so a coach whose
//! account cannot be read, or a deployment with no notification service,
//! sends nothing, answers `false`, and the write stands.
//!
//! [`publish_coach_draft`]: crate::coach_plan_draft::publish_coach_draft

use std::sync::Arc;

use pierre_core::models::{roster_display_name, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::TrainingPlan;
use pierre_notifications::triggers::{
    trigger_coach_plan_published, trigger_coach_plan_pushed, LinkPlatform,
};
use pierre_notifications::{NotificationService, TenantId as NoticeTenantId};
use pierre_providers::backend_resolver::{brand_name, user_facing_name};
use pierre_providers::ProviderRegistry;
use tracing::warn;
use uuid::Uuid;

use crate::coach_plan_draft::CoachDraftTarget;
use crate::plan_calendar_push::PushReport;

/// Tell the athlete of `target` that its coach published their draft as
/// `plan`, the athlete's live season now.
///
/// Returns whether the notice was dispatched: `false` with no notification
/// service, or when the coach's account cannot be read (logged). The notice
/// carries the plan's own transport stamp, since it names the plan's race.
pub async fn tell_plan_published(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    target: &CoachDraftTarget,
    plan: &TrainingPlan,
) -> bool {
    let Some(service) = notifications else {
        return false;
    };
    let Some(coach_name) = coach_name(repos, target.coach()).await else {
        return false;
    };
    trigger_coach_plan_published(
        service,
        target.athlete(),
        notice_tenant(target.tenant()),
        &coach_name,
        &plan.goal_race.name,
        plan.transport_policy,
    );
    true
}

/// A human coach's push of a coached athlete's plan onto the athlete's
/// calendar, through the coach's link.
#[derive(Debug, Clone, Copy)]
pub struct CoachCalendarPush<'a> {
    /// The athlete's tenant: their ledger, plan and feed live under it.
    pub tenant: TenantId,
    /// The athlete whose calendar was written.
    pub athlete: Uuid,
    /// The coach who pushed.
    pub coach: Uuid,
    /// The calendar backend the push wrote, as the ledger files it.
    pub backend: &'a str,
}

/// Tell the athlete of `push` that their coach put their plan's workouts on
/// their calendar, when `report` says the push created or updated at least
/// one there.
///
/// A push that wrote nothing — every entry already current, only removals,
/// or every write refused — tells no one: the calendar holds no workout the
/// coach put there. Returns whether the notice was dispatched: `false` for
/// such a push, with no notification service, or when the coach's account
/// cannot be read (logged). The notice names the platform by the brand the
/// athlete connected and opens their connections on its card.
pub async fn tell_plan_pushed(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    notifications: Option<&Arc<NotificationService>>,
    push: &CoachCalendarPush<'_>,
    report: &PushReport,
) -> bool {
    if report.created + report.updated == 0 {
        return false;
    }
    let Some(service) = notifications else {
        return false;
    };
    let Some(coach_name) = coach_name(repos, push.coach).await else {
        return false;
    };
    let provider = user_facing_name(push.backend);
    trigger_coach_plan_pushed(
        service,
        push.athlete,
        notice_tenant(push.tenant),
        &coach_name,
        LinkPlatform {
            provider,
            name: brand_name(registry, push.backend).unwrap_or(provider),
        },
    );
    true
}

/// The coach's name as a roster names a member, or `None` (logged) when
/// their account cannot be read.
async fn coach_name(repos: &RepositoryRegistry, coach: Uuid) -> Option<String> {
    match repos.users.get_global(coach).await {
        Ok(Some(user)) => Some(roster_display_name(
            user.display_name.as_deref(),
            &user.email,
        )),
        Ok(None) => {
            warn!(coach_user_id = %coach, "Coach plan notice skipped: the coach's account is gone");
            None
        }
        Err(e) => {
            warn!(coach_user_id = %coach, error = %e, "Coach plan notice skipped: coach read failed");
            None
        }
    }
}

/// A platform tenant as the notification pipeline names it.
const fn notice_tenant(tenant: TenantId) -> NoticeTenantId {
    NoticeTenantId(tenant.as_uuid())
}
