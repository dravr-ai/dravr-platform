// ABOUTME: The rails a write to an athlete's live plan runs once it commits — telemetry, ramp, coverage, compliance, readiness
// ABOUTME: One sequence for the athlete's own save and a coach's publish; a coach's draft save gets the two safety checks, unreported

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The live-plan rails
//!
//! Two writes change what an athlete's season holds: their own agent's
//! `save_training_plan`, and a human coach publishing a draft
//! (`publish_training_plan`, or the Publish button in the coach's athlete
//! thread). Both run the same rails over what they wrote, here, after the
//! write commits: the write is reported (`training_plan.saved`, and the
//! vision's provenance when it laid a flavour), the opening week of a laid
//! outline is measured against the athlete's recent load, and the plan is
//! checked for coverage, week compliance and readiness.
//!
//! A coach's save into their draft changes nothing the athlete has, so none
//! of it is reported: no `training_plan.saved`, no verdict events, no
//! readiness. The two safety-shaped checks still run, on the draft view —
//! the draft's outline over the athlete's live weeks — and reach the coach's
//! turn alone ([`draft_plan_safety`]): a draft exists so the coach can review
//! it before the athlete sees anything, so a ramp spike or crowded hard days
//! belong at draft time, not after publish. Both paths run the same two
//! assessments; only the live one emits.
//!
//! Every rail is best-effort — the plan is already stored, so a rail that
//! cannot read its inputs degrades to silence, never to a failed write.

use std::sync::Arc;

use chrono::NaiveDate;
use pierre_core::models::TenantId;
use pierre_database::repositories::SavedDraftBundle;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{FlavourSelection, PlanWeek, TrainingPlan};
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{coach_plan_view, CoachDraftTarget, CoachPlanView};
use tracing::warn;
use uuid::Uuid;

use super::training_plan_compliance::{assess_weeks, crowded_weeks, emit_week_compliance};
use super::training_plan_ramp::assess_opening_ramp;
use super::training_plan_readiness::{emit_week_readiness, week_is_actionable};
use super::training_plan_telemetry::{
    emit_coverage_check, emit_plan_saved, emit_ramp_verdict, emit_vision_saved,
};
use super::training_plans_output::PlanSafetyReport;
use crate::runtime::ToolRuntime;

/// One committed write to an athlete's live season.
pub(super) struct LivePlanWrite<'a> {
    /// The athlete's tenant.
    pub(super) tenant: TenantId,
    /// The athlete.
    pub(super) user_id: Uuid,
    /// The live season the write landed on.
    pub(super) plan: &'a TrainingPlan,
    /// The weeks the write put live.
    pub(super) weeks: &'a [PlanWeek],
    /// Whether the write laid an outline — a new season with an opening week
    /// to measure.
    pub(super) laid_outline: bool,
    /// The flavour the write chose, when it stated one: its provenance is
    /// reported.
    pub(super) flavour: Option<&'a FlavourSelection>,
}

/// What the rails found that reaches the turn.
pub(super) struct LivePlanRails {
    /// The two safety-shaped findings, empty when there were none.
    pub(super) safety: PlanSafetyReport,
    /// The athlete's civil date the rails measured against — read once, so
    /// every rail and the caller's own report describe the same day even
    /// when the write straddles the athlete's midnight.
    pub(super) today: NaiveDate,
}

/// Run every rail over a committed write to the athlete's live season.
pub(super) async fn run_live_plan_rails(
    state: &Arc<dyn ToolRuntime>,
    repos: &RepositoryRegistry,
    write: &LivePlanWrite<'_>,
) -> LivePlanRails {
    let tenant_id = write.tenant.to_string();
    let user_id = write.user_id.to_string();
    let plan = write.plan;

    // The one enforced rail on plan difficulty: does the plan's opening week
    // sit far above what this athlete actually does? A warning, never a
    // block — the agent may have good reason, and refusing the write would
    // strand a plan the athlete already agreed to. The event also reports
    // when the comparison could not be made, so a quiet log means "measured
    // and fine" rather than "never looked".
    let ramp = assess_opening_ramp(
        repos,
        state.provider_registry().as_ref(),
        write.tenant,
        &user_id,
        write.laid_outline,
        write.weeks,
    )
    .await;
    if let Some(verdict) = &ramp {
        emit_ramp_verdict(&plan.id, verdict);
    }

    // Every committed write is reported, so a weeks-only adjustment is not
    // silent, and the plan is then checked for whether it actually covers
    // the athlete it was just written for. A flavour reports its provenance
    // too — the override rate against the rule is the vision's
    // falsification test.
    emit_plan_saved(&plan.id, write.laid_outline, write.weeks);
    if let Some(flavour) = write.flavour {
        emit_vision_saved(&plan.id, flavour);
    }
    let today = athlete_today(repos, write.user_id).await;
    emit_coverage_check(repos, &tenant_id, &user_id, &plan.id, &plan.phases, today).await;
    // The compliance rail: every week written is measured against its
    // phase's targets and the flavour's caps, and every verdict reported for
    // the base-rate measurement. The weeks whose hard sessions crowd come
    // back and travel to the agent — a week the rail already knows crowds
    // its hard days should not reach the athlete unmentioned.
    let crowded_weeks =
        emit_week_compliance(state, repos, write.tenant, write.user_id, plan, write.weeks).await;
    // The readiness rail, the other half of the question: compliance asks
    // whether the week was built to its phase, this asks whether the athlete
    // can run it today. Only weeks with days left to change are read.
    let actionable: Vec<PlanWeek> = write
        .weeks
        .iter()
        .filter(|week| week_is_actionable(week, today))
        .cloned()
        .collect();
    if !actionable.is_empty() {
        emit_week_readiness(state, repos, write.tenant, write.user_id, plan, &actionable).await;
    }

    LivePlanRails {
        safety: PlanSafetyReport::from_findings(crowded_weeks, ramp),
        today,
    }
}

/// The two safety-shaped checks over a coach's committed save into their
/// draft of the athlete's plan, measured on the draft view and emitted
/// nowhere: what they find reaches the coach's turn alone.
///
/// The same assessments a live write runs, over what publishing the draft
/// would put in front of the athlete: the opening week of an outline the
/// save laid against the athlete's own recent load, and every week the save
/// wrote walked with the draft's outline over the athlete's live weeks, the
/// drafted ones in place. `laid_outline` is whether this save carried an
/// outline, as on the live path. Readiness stays at publish: it reads the
/// athlete's today, and is Log-only telemetry with nothing to hand back.
///
/// A draft view that cannot be read leaves the weeks unmeasured; the ramp,
/// which reads only the saved weeks and the athlete's load, still runs.
pub(super) async fn draft_plan_safety(
    state: &Arc<dyn ToolRuntime>,
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    saved: &SavedDraftBundle,
    laid_outline: bool,
) -> PlanSafetyReport {
    let ramp = assess_opening_ramp(
        repos,
        state.provider_registry().as_ref(),
        target.tenant(),
        &target.athlete().to_string(),
        laid_outline,
        &saved.weeks,
    )
    .await;
    let crowded = match coach_plan_view(repos, target).await {
        Ok(CoachPlanView {
            draft: Some(view), ..
        }) => crowded_weeks(
            assess_weeks(
                state,
                repos,
                target.tenant(),
                target.athlete(),
                &view.plan,
                &view.weeks,
                &saved.weeks,
            )
            .await,
        ),
        Ok(CoachPlanView { draft: None, .. }) => {
            warn!(
                draft_id = %saved.draft.plan.id,
                "draft safety: the draft was published or discarded under the save; its weeks \
                 go unmeasured"
            );
            Vec::new()
        }
        Err(e) => {
            warn!(error = %e, "draft safety: the draft view is unreadable; its weeks go unmeasured");
            Vec::new()
        }
    };
    PlanSafetyReport::from_findings(crowded, ramp)
}
