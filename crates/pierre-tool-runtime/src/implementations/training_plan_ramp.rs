// ABOUTME: The opening week's ramp against the athlete's real recent load, measured on the save path
// ABOUTME: Best-effort by design — an unreadable activity cache degrades the verdict, it never fails the save

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::parse_plan_date;
use pierre_services::ramp_check::{assess_ramp, RampVerdict};

use super::training_plan_telemetry::{emit_ramp_verdict, ramp_baseline};
use super::training_plans::WeekPayload;

/// The chronologically first week of a payload — the plan's opening week.
///
/// Payload order is the model's, not the calendar's: nothing sorts `weeks`, and
/// a plan may be sent newest-first or in any order at all. The ramp check grades
/// the week the athlete starts on, so it is selected by date.
pub(super) fn earliest_week(weeks: &[WeekPayload]) -> Option<&WeekPayload> {
    weeks
        .iter()
        .filter_map(|w| parse_plan_date(&w.week_start).map(|date| (date, w)))
        .min_by_key(|(date, _)| *date)
        .map(|(_, week)| week)
}

/// Measure the saved plan's opening week against the athlete's real recent
/// load and emit the result.
///
/// Best-effort by design: a plan the athlete already agreed to must not fail to
/// save because the activity cache was unreadable, so every failure path here
/// degrades to an unmeasurable verdict rather than an error.
pub(super) async fn emit_ramp_check(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    plan_id: &str,
    opening_week: Option<&WeekPayload>,
) -> RampVerdict {
    let baseline = ramp_baseline(repos, tenant, user_id).await;
    let durations: Vec<Option<u32>> = opening_week
        .map(|w| w.days.iter().map(|d| d.duration_min).collect())
        .unwrap_or_default();
    let verdict = assess_ramp(&durations, baseline.as_ref());
    emit_ramp_verdict(plan_id, &verdict);
    verdict
}
