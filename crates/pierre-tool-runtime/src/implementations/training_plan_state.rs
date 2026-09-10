// ABOUTME: What the readiness and compliance rails make of a plan, projected for the agent that asked
// ABOUTME: Both rails measured every save and logged the verdict; this is the same verdict, handed back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The plan's state block.
//!
//! Composes the two rails' readings into the shape `get_training_plan`
//! returns under `include_state`. It measures nothing of its own — both
//! rails already do, on every save — so the only thing decided here is what
//! an agent is shown.

use std::sync::Arc;

use chrono::NaiveDate;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use uuid::Uuid;

use super::training_plan_compliance::assess_saved_weeks;
use super::training_plan_readiness::read_ladder;
use super::training_plan_telemetry::coverage_gaps;
use crate::implementations::training_plans_output::{
    PlanStateBlock, WeekComplianceBlock, WeekReadiness,
};
use crate::runtime::ToolRuntime;

/// What the two rails make of a plan, for a caller that asked for it.
///
/// Best-effort: a rail that cannot read its inputs contributes nothing
/// rather than failing the read. An empty block therefore means *nothing to
/// say*, which is not the same as *everything is fine* — the same rule both
/// rails hold internally.
pub(super) async fn plan_state_block(
    state: &Arc<dyn ToolRuntime>,
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    plan: &TrainingPlan,
    weeks: &[PlanWeek],
    today: NaiveDate,
) -> PlanStateBlock {
    let reading = read_ladder(state, repos, tenant, user_id, plan, weeks).await;
    let compliance = assess_saved_weeks(state, repos, tenant, user_id, plan, weeks).await;

    PlanStateBlock {
        readiness: reading.as_ref().map(|r| r.level),
        alerts: reading
            .as_ref()
            .map(|r| r.alerts.clone())
            .unwrap_or_default(),
        readiness_weeks: reading.map_or_else(Vec::new, |r| {
            r.weeks
                .into_iter()
                .map(|(week_start, verdict)| WeekReadiness {
                    week_start,
                    verdict,
                })
                .collect()
        }),
        compliance_weeks: compliance
            .iter()
            .map(|(week_start, verdict)| WeekComplianceBlock {
                week_start: week_start.clone(),
                tid: verdict.tid_outcome().as_str().to_owned(),
                hard_sessions: verdict.hard_sessions_outcome().as_str().to_owned(),
                spacing: verdict.spacing_outcome().as_str().to_owned(),
                volume: verdict.volume_outcome().as_str().to_owned(),
                week_loading: verdict.week_loading.as_str().to_owned(),
                unclassified_days: verdict.unclassified_days,
            })
            .collect(),
        coverage_gaps: coverage_gaps(weeks, &plan.phases, today)
            .into_iter()
            .map(|gap| gap.as_str().to_owned())
            .collect(),
    }
}
