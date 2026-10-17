// ABOUTME: The opening week's ramp against the athlete's real recent load, measured on every plan write that lays an outline
// ABOUTME: Assessed here and reported by the caller — a live write emits it, a coach's draft hands it to the coach alone

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::ai_policy::ProviderTerms;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek};
use pierre_services::ramp_check::{assess_ramp, RampVerdict};

use super::training_plan_telemetry::ramp_baseline;

/// The chronologically first of the weeks a write stored — the plan's
/// opening week.
///
/// Write order is the model's, not the calendar's: nothing sorts a payload's
/// `weeks`, and a plan may be sent newest-first or in any order at all. The
/// ramp check grades the week the athlete starts on, so it is selected by
/// date.
fn earliest_week(weeks: &[PlanWeek]) -> Option<&PlanWeek> {
    weeks
        .iter()
        .filter_map(|w| parse_plan_date(&w.week_start).map(|date| (date, w)))
        .min_by_key(|(date, _)| *date)
        .map(|(_, week)| week)
}

/// Measure a write's opening week against the athlete's real recent load,
/// or `None` for a write that laid no outline. Reports nothing: the caller
/// does, a live write by emitting the verdict and a coach's draft save by
/// handing it to the coach's turn alone.
///
/// Only a write that lays an outline is a new plan with an opening week. A
/// weeks-only write adjusts weeks of a plan already measured, and grading one
/// as an opening week reports a ramp the athlete is not being asked to make.
///
/// Best-effort by design: a plan must not fail to save because the activity
/// cache was unreadable, so every failure path here degrades to an
/// unmeasurable verdict rather than an error.
pub(super) async fn assess_opening_ramp(
    repos: &RepositoryRegistry,
    policies: &dyn ProviderTerms,
    tenant: TenantId,
    user_id: &str,
    laid_outline: bool,
    written: &[PlanWeek],
) -> Option<RampVerdict> {
    if !laid_outline {
        return None;
    }
    let baseline = ramp_baseline(repos, policies, tenant, user_id).await;
    let durations: Vec<Option<u32>> = earliest_week(written)
        .map(|w| w.days.iter().map(|d| d.duration_min).collect())
        .unwrap_or_default();
    Some(assess_ramp(&durations, baseline.as_ref()))
}
