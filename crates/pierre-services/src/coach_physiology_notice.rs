// ABOUTME: Tells an athlete their human coach set one of their per-sport thresholds from Group info
// ABOUTME: Sent in the athlete's own tenant, naming the coach by the roster rule; best-effort, the saved value stands

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # An athlete told their human coach set a threshold (carnet#1561)
//!
//! A group's human coach may set a coached athlete's per-sport anchor from
//! Group info, through the one physiology writer. What the athlete's
//! workouts are set on then changes without the athlete doing anything, so
//! each such save sends `coach_physiology_set` to the athlete. The athlete's
//! own edits and their agent's `set_physiology` tell no one.
//!
//! As with the plan notices ([`crate::coach_plan_notice`]), the notice lands
//! in the athlete's tenant and names the coach by
//! [`roster_display_name`](pierre_core::models::roster_display_name). It is
//! best-effort: the value has been saved, so a coach whose account cannot be
//! read, or a deployment with no notification service, sends nothing, answers
//! `false`, and the save stands.

use std::sync::Arc;

use pierre_core::models::{SportAnchorName, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_notifications::triggers::trigger_coach_physiology_set;
use pierre_notifications::NotificationService;
use uuid::Uuid;

use crate::coach_plan_notice::{coach_name, notice_tenant};

/// A threshold a human coach set for one of their athletes.
#[derive(Debug, Clone, Copy)]
pub struct CoachThresholdSet {
    /// The athlete's one home tenant, where their profile and feed live.
    pub tenant: TenantId,
    /// The athlete whose threshold was set.
    pub athlete: Uuid,
    /// The coach who set it.
    pub coach: Uuid,
    /// Which anchor was set.
    pub anchor: SportAnchorName,
}

/// Tell the athlete of `set` that their coach set that threshold.
///
/// Returns whether the notice was dispatched: `false` with no notification
/// service, or when the coach's account cannot be read (logged).
pub async fn tell_threshold_set(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    set: &CoachThresholdSet,
) -> bool {
    let Some(service) = notifications else {
        return false;
    };
    let Some(name) = coach_name(repos, set.coach).await else {
        return false;
    };
    trigger_coach_physiology_set(
        service,
        set.athlete,
        notice_tenant(set.tenant),
        &name,
        set.anchor.as_str(),
    );
    true
}
