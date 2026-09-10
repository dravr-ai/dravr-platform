// ABOUTME: The compliance rail's platform half — a saved week resolved into the kernel's inputs, measured, reported
// ABOUTME: Log-only: every verdict is a training_plan.week_assessed event; nothing here warns the agent or refuses a save
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Week compliance, after the write commits.
//!
//! The kernel (`periodization::assess_week_compliance`) decides every rule;
//! this module only resolves what a stored week refers to — the phase it
//! instantiates, the flavour the plan runs on, each day's template from the
//! catalogue or the athlete's own bank, the athlete's recovery arm — hands
//! the kernel a borrowed view, and reports the verdict. The whole plan's
//! weeks are walked in date order so the last hard day of one week bounds
//! the first gap of the next; only the weeks this save wrote are reported.
//!
//! Log-only is the ledger's first phase (`feature-phases.yaml`,
//! `training-plan-week-compliance-rail`): the verdict is data for the
//! base-rate measurement, and nothing downstream reads it yet.

use pierre_core::models::periodization::{
    assess_week_compliance, PhaseTargets, PlannedSession, RecoverySpeed, SessionParams, WeekInput,
    WeekVerdict, WorkoutTemplate,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanPhase, PlanWeek, TrainingPlan};
use pierre_services::coach_package::{load_coach_package, PackagedCatalogue};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Measure every week of the plan and report the ones this save wrote.
///
/// Best-effort and after the commit, like the ramp check: an unreadable
/// store logs and returns, never fails the save. Weeks whose `week_start`
/// does not parse are skipped, since they cannot be ordered.
pub(super) async fn emit_week_compliance(
    state: &Arc<dyn ToolRuntime>,
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    plan: &TrainingPlan,
    saved: &[PlanWeek],
) {
    let tenant_id = tenant.to_string();
    let user = user_id.to_string();
    let mut weeks = match repos
        .training_plans
        .list_plan_weeks(&tenant_id, &user, &plan.id, false)
        .await
    {
        Ok(weeks) => weeks,
        Err(e) => {
            warn!(error = %e, "week compliance: plan weeks unreadable");
            return;
        }
    };
    weeks.retain(|w| parse_plan_date(&w.week_start).is_some());
    weeks.sort_by(|a, b| a.week_start.cmp(&b.week_start));

    // The plan's coach package first, then the catalogue — the same view
    // the save resolved through, so a house flavour or a package template
    // is measured against what it was saved as.
    let package = match load_coach_package(repos, tenant, user_id, plan.coach_slug.as_deref()).await
    {
        Ok(package) => package,
        Err(e) => {
            warn!(error = %e, "week compliance: coach package unreadable; catalogue alone");
            None
        }
    };
    let catalogue = PackagedCatalogue::new(state.training_catalogue(), package);
    let flavour = plan
        .flavour
        .as_ref()
        .and_then(|selection| catalogue.flavour(&selection.id))
        .map(|(flavour, _)| flavour);
    // Masters-style loading triggers on the athlete's own recovery-speed
    // answer, never on age; the rule's inputs snapshot is where a saved plan
    // keeps it. A plan saved without running the rule reads as typical.
    let recovery_limited = plan
        .flavour
        .as_ref()
        .and_then(|selection| selection.inputs_snapshot.as_ref())
        .is_some_and(|inputs| inputs.recovery_speed == RecoverySpeed::Limited);

    let templates = resolve_templates(&catalogue, repos, tenant, user_id, &weeks).await;
    // Resolved once: the lookup is per phase, and a season is a handful of phases
    // against a catalogue that rebuilds this vector on every call.
    let skeletons = catalogue.skeletons();
    let reported: Vec<&str> = saved.iter().map(|w| w.id.as_str()).collect();

    let mut previous_hard = None;
    for week in &weeks {
        let phase = week
            .phase_index
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| plan.phases.get(i));
        let targets = PhaseTargets {
            kind: phase.map(|p| p.kind),
            tid_target: phase.and_then(|p| p.tid_target.as_ref()),
            hard_sessions_max: phase
                .and_then(|p| p.hard_sessions_max)
                .and_then(|cap| u8::try_from(cap).ok()),
            target_hours: phase.and_then(|p| p.target_hours),
            // The saved phase carries its own pattern; a plan written before
            // the field existed, or by a coach who stated none, holds None and
            // the kernel reads every week as a load week.
            loading_pattern: phase.and_then(|p| p.loading_pattern),
            // The cut is the skeleton's, not the phase's — the phase records
            // which skeleton it was laid out from, and that is where the share
            // a recovery week drops is authored. A phase laid out from no
            // skeleton, or from one the catalogue has since dropped, leaves it
            // unmeasured rather than inventing a default cut.
            recovery_week_cut: phase
                .and_then(|p| p.skeleton_id.as_deref())
                .and_then(|id| skeletons.iter().find(|s| s.id == id))
                .map(|s| s.recovery_week_cut),
        };
        let sessions: Vec<PlannedSession<'_>> = week
            .days
            .iter()
            .filter(|day| !day.is_rest())
            .filter_map(|day| {
                Some(PlannedSession {
                    date: parse_plan_date(&day.date)?,
                    minutes: day.duration_min,
                    steps: &day.steps,
                    intensity: (!day.intensity.trim().is_empty()).then_some(day.intensity.as_str()),
                    template: day
                        .template_slug
                        .as_deref()
                        .and_then(|slug| templates.get(slug)),
                    params: day.template_params.as_ref().map(|p| SessionParams {
                        sets: p.sets,
                        reps: p.reps,
                        work_seconds: p.work_seconds,
                        rest_seconds: p.rest_seconds,
                        duration_minutes: p.duration_minutes,
                    }),
                })
            })
            .collect();
        // 0-based position inside the phase, taken from the calendar rather
        // than from this week's position in the vector: the saved weeks are
        // whatever the athlete has, so counting them would make a phase whose
        // first weeks were never written start at its third. Both dates parse
        // or the position is unknown, which the kernel reads as a load week.
        let week_index_in_phase = phase
            .and_then(PlanPhase::start_date)
            .zip(parse_plan_date(&week.week_start))
            .map(|(phase_start, week_start)| (week_start - phase_start).num_days() / 7)
            .filter(|weeks| *weeks >= 0)
            .and_then(|weeks| u8::try_from(weeks).ok());
        let verdict = assess_week_compliance(&WeekInput {
            sessions: &sessions,
            previous_hard,
            targets,
            flavour: flavour.as_ref(),
            recovery_limited,
            week_index_in_phase,
        });
        previous_hard = verdict.hard_days.last().copied().or(previous_hard);
        if reported.contains(&week.id.as_str()) {
            emit_week_assessed(&plan.id, &week.week_start, &verdict);
        }
    }
}

/// Every template the plan's days name, resolved once: the package, then
/// the catalogue, then the athlete's own bank. A slug that resolves nowhere
/// leaves the day template-less, which the kernel reads as unclassified —
/// the honest outcome for a reference the save-time check let through and
/// the catalogue has since dropped.
async fn resolve_templates(
    catalogue: &PackagedCatalogue<'_>,
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    weeks: &[PlanWeek],
) -> HashMap<String, WorkoutTemplate> {
    let mut templates = HashMap::new();
    for slug in weeks
        .iter()
        .flat_map(|w| w.days.iter())
        .filter_map(|d| d.template_slug.as_deref())
    {
        if templates.contains_key(slug) {
            continue;
        }
        let resolved = match catalogue.workout(slug) {
            Some((template, _)) => Some(template),
            None => repos
                .workout_templates
                .get_user_workout_template(tenant, user_id, slug)
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, slug, "week compliance: user template unreadable");
                    None
                }),
        };
        if let Some(template) = resolved {
            templates.insert(slug.to_owned(), template);
        }
    }
    templates
}

/// Report one week's verdict. Each check is a word — within, off,
/// unmeasured — so the base rate is a count over events, and the details
/// (which zone, which gap, which parameter) ride in the message for a
/// reader chasing one plan.
fn emit_week_assessed(plan_id: &str, week_start: &str, verdict: &WeekVerdict) {
    // The verdict as JSON, so a reader chasing one plan gets the zone
    // shares, the short gaps and the parameter names, not a Debug dump.
    let detail = serde_json::to_string(verdict).unwrap_or_default();
    info!(
        target: "notify",
        event = "training_plan.week_assessed",
        plan_id = %plan_id,
        week_start = %week_start,
        tid = verdict.tid_outcome().as_str(),
        hard_sessions = verdict.hard_sessions_outcome().as_str(),
        spacing = verdict.spacing_outcome().as_str(),
        template_ranges = verdict.template_ranges.len(),
        volume = verdict.volume_outcome().as_str(),
        unclassified_days = verdict.unclassified_days,
        hard_days = verdict.hard_days.len(),
        detail = %detail,
        "saved week measured against its phase and flavour"
    );
}
