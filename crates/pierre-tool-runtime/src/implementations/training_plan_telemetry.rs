// ABOUTME: Notify telemetry for training-plan writes — what was saved and what it leaves uncovered
// ABOUTME: Split from training_plans.rs to keep that file under the 1200-line ceiling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::NaiveDate;
use pierre_core::models::{LoadSnapshot, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanPhase, PlanWeek};
use pierre_services::ramp_check::RampVerdict;
use pierre_services::recent_load::recent_load_snapshot;
use tracing::{info, warn};
use uuid::Uuid;

/// Record that a calendar write to a provider landed.
///
/// A calendar write is a provider sync in the outbound direction, so it rides
/// the catalogued `sync.completed` event: `trigger` names the tool that wrote
/// (`prescribe_workout`, `withdraw_prescribed_workout`, `push_training_plan`)
/// and `records` counts the entries the provider accepted.
pub(super) fn emit_calendar_sync_completed(
    tenant_id: TenantId,
    user_id: Uuid,
    provider: &str,
    trigger: &str,
    records: usize,
) {
    info!(
        target: "notify",
        event = "sync.completed",
        tenant_id = %tenant_id,
        user_id = %user_id,
        provider = provider,
        records = records,
        trigger = trigger,
        direction = "outbound",
        "calendar write committed"
    );
}

/// Record that a calendar write to a provider was refused, or landed only in
/// part. `reason` is the provider's or the ledger's own words.
pub(super) fn emit_calendar_sync_failed(
    tenant_id: TenantId,
    user_id: Uuid,
    provider: &str,
    trigger: &str,
    reason: &str,
) {
    warn!(
        target: "notify",
        event = "sync.failed",
        tenant_id = %tenant_id,
        user_id = %user_id,
        provider = provider,
        trigger = trigger,
        reason = reason,
        direction = "outbound",
        "calendar write refused"
    );
}

/// Record that a plan write committed.
///
/// Emitted for **every** successful save, outline or weeks-only. The ramp
/// events below are gated on an outline being present, so before this existed a
/// weeks-only adjustment produced no notify output at all and was invisible to
/// anything but a raw argument-payload log read.
///
/// `week_starts` rides along because the count alone cannot tell a plan being
/// extended into the future from one rewriting a week already past — the exact
/// ambiguity that let a coverage gap sit unreported.
pub(super) fn emit_plan_saved(plan_id: &str, has_outline: bool, saved: &[PlanWeek]) {
    let week_starts = saved
        .iter()
        .map(|w| w.week_start.as_str())
        .collect::<Vec<_>>()
        .join(",");
    info!(
        target: "notify",
        event = "training_plan.saved",
        plan_id = %plan_id,
        weeks_saved = saved.len(),
        week_starts = %week_starts,
        has_outline = has_outline,
        "training plan write committed"
    );
}

/// Which way a stored plan fails to cover the athlete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoverageGap {
    /// No stored week spans the athlete's current date.
    UncoveredToday,
    /// The day-by-day weeks stop before the outline's last block ends.
    ShortOfOutline,
}

impl CoverageGap {
    /// Catalogue value for the event's `kind` field.
    const fn as_str(self) -> &'static str {
        match self {
            Self::UncoveredToday => "uncovered_today",
            Self::ShortOfOutline => "short_of_outline",
        }
    }
}

/// The civil date the athlete is living in, falling back to UTC.
///
/// A plan is "covering today" from the athlete's calendar, not the server's —
/// the same rule `/plan` applies when it picks which week to show.
pub(super) async fn athlete_today(repos: &RepositoryRegistry, user_id: &str) -> NaiveDate {
    let tz = match Uuid::parse_str(user_id) {
        Ok(uuid) => repos
            .users
            .get_global(uuid)
            .await
            .ok()
            .flatten()
            .and_then(|u| u.timezone),
        Err(_) => None,
    };
    tz.as_deref()
        .and_then(|t| t.parse::<chrono_tz::Tz>().ok())
        .map_or_else(
            || chrono::Utc::now().date_naive(),
            |t| chrono::Utc::now().with_timezone(&t).date_naive(),
        )
}

/// The gaps a stored plan leaves, given its active weeks and outline phases.
///
/// A week or block whose date does not parse, or whose span leaves the
/// calendar, is skipped rather than guessed at — `parse_plan_date` is a format
/// check, so a stored `+262142-12-31` reaches here intact.
fn coverage_gaps(weeks: &[PlanWeek], phases: &[PlanPhase], today: NaiveDate) -> Vec<CoverageGap> {
    let spans: Vec<(NaiveDate, NaiveDate)> = weeks
        .iter()
        .filter_map(|w| {
            let start = parse_plan_date(&w.week_start)?;
            let end = start.checked_add_days(chrono::Days::new(6))?;
            Some((start, end))
        })
        .collect();
    let mut gaps = Vec::new();
    // Phases the outline claims are running right now. A plan that says it is
    // in a phase today has asserted it should be prescribing today.
    let block_spans_today = phases.iter().any(|p| p.covers(today));
    // A week that has already ended is proof the plan was under way.
    let plan_has_started = block_spans_today || spans.iter().any(|(_, e)| *e < today);
    // Not covering today only means something once the plan claims to be
    // running. A coach who lays out next week's schedule on a Wednesday, or
    // saves an outline before its weeks, has written a perfectly good plan that
    // simply has not started — reporting those as gaps would bury the case this
    // exists to catch, a plan that was covering the athlete and stopped.
    if plan_has_started && !spans.iter().any(|(s, e)| (*s..=*e).contains(&today)) {
        gaps.push(CoverageGap::UncoveredToday);
    }
    // Only meaningful once the plan has both a phased outline and some weeks:
    // an outline with no phases promises nothing, and a plan with no weeks at
    // all cannot be measured against one.
    let last_week_end = spans.iter().map(|(_, e)| *e).max();
    // Both sides must be the block's / week's LAST COVERED DAY. A block spans
    // `[start, start + 7 * weeks)` — exclusive — while a week's `end` is
    // inclusive, so comparing them raw reports every exactly-covered plan as
    // one day short and the signal fires on every save.
    let outline_last_day = phases
        .iter()
        .filter_map(|p| p.end_exclusive()?.checked_sub_days(chrono::Days::new(1)))
        .max();
    if let (Some(week_end), Some(block_end)) = (last_week_end, outline_last_day) {
        if block_end > week_end {
            gaps.push(CoverageGap::ShortOfOutline);
        }
    }
    gaps
}

/// Check whether the plan just written actually covers the athlete, and report
/// each way it does not.
///
/// Best-effort like the ramp check: the plan is already committed, so an
/// unreadable week list degrades to silence rather than failing the save.
/// Reads the plan's full active week set rather than the payload, because a
/// save that superseded an outline carries earlier weeks forward and those
/// count toward coverage just as much as the ones in this call.
pub(super) async fn emit_coverage_check(
    repos: &RepositoryRegistry,
    tenant_id: &str,
    user_id: &str,
    plan_id: &str,
    phases: &[PlanPhase],
) {
    let weeks = match repos
        .training_plans
        .list_plan_weeks(tenant_id, user_id, plan_id, false)
        .await
    {
        Ok(weeks) => weeks,
        Err(e) => {
            warn!(error = %e, "coverage check: plan weeks unreadable");
            return;
        }
    };
    let today = athlete_today(repos, user_id).await;
    let last_week_start = weeks
        .iter()
        .filter(|w| parse_plan_date(&w.week_start).is_some())
        .max_by_key(|w| w.week_start.clone())
        .map_or_else(String::new, |w| w.week_start.clone());
    for gap in coverage_gaps(&weeks, phases, today) {
        info!(
            target: "notify",
            event = "training_plan.coverage_gap",
            plan_id = %plan_id,
            kind = gap.as_str(),
            last_week_start = %last_week_start,
            "stored plan does not cover the athlete"
        );
    }
}

/// The athlete's recent-load baseline for the ramp check, or `None` when it
/// cannot be read. Never propagates an error: the plan is already saved.
pub(super) async fn ramp_baseline(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
) -> Option<LoadSnapshot> {
    let uuid = match Uuid::parse_str(user_id) {
        Ok(uuid) => uuid,
        Err(e) => {
            warn!(error = %e, "ramp check: unparseable user id");
            return None;
        }
    };
    recent_load_snapshot(repos.activity_cache.as_ref(), uuid, &tenant)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "ramp check: activity cache unreadable");
            None
        })
}

/// Emit the ramp verdict, including the explicit "could not measure" case so a
/// quiet log means the comparison ran and passed.
pub(super) fn emit_ramp_verdict(plan_id: &str, verdict: &RampVerdict) {
    match verdict {
        RampVerdict::Exceeded {
            planned_hours,
            baseline_hours,
            increase,
        } => {
            warn!(
                target: "notify",
                event = "training_plan.ramp_warning",
                plan_id = %plan_id,
                planned_hours = format!("{planned_hours:.1}"),
                baseline_hours = format!("{baseline_hours:.1}"),
                increase_pct = format!("{:.0}", increase * 100.0),
                "planned opening week exceeds the athlete's recent weekly load"
            );
        }
        RampVerdict::WithinThreshold {
            planned_hours,
            baseline_hours,
        } => {
            info!(
                target: "notify",
                event = "training_plan.ramp_checked",
                plan_id = %plan_id,
                planned_hours = format!("{planned_hours:.1}"),
                baseline_hours = format!("{baseline_hours:.1}"),
                "planned opening week is within the athlete's recent weekly load"
            );
        }
        RampVerdict::Unmeasurable(reason) => {
            info!(
                target: "notify",
                event = "training_plan.ramp_unmeasurable",
                plan_id = %plan_id,
                reason = reason.as_str(),
                "could not compare the planned opening week against recent load"
            );
        }
    }
}
