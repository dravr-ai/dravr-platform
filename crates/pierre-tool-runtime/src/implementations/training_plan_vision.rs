// ABOUTME: The vision half of a save_training_plan payload — flavour provenance, phase targets, template references
// ABOUTME: Split from training_plans.rs to keep that file under the 1200-line ceiling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Validation of what the Annual Vision adds to a plan outline.
//!
//! The flavour and who chose it, each phase's targets, the season window, a
//! week's phase index and a day's template. Every catalogue reference is
//! resolved against the live registry before anything is written.

use pierre_contremaitre::TrainingCatalogueRegistry;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::Share;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{FlavourSelection, PlanPhase, PlannedDay, SelectedBy};
use serde::Deserialize;
use uuid::Uuid;

use super::calendar::{bounded, MAX_SHORT_TEXT_LEN};
use super::training_plans::{plan_date, WeekPayload, MAX_TARGET_HOURS, MAX_TEXT_LEN};

/// Hard sessions a week no phase exceeds; the flavour caps are lower.
const MAX_HARD_SESSIONS_PER_WEEK: u32 = 7;

/// The flavour half of an outline as the model sends it: the id and who
/// chose it. Family, sequencing and modifiers are copied from the catalogue
/// at save time, never trusted from the payload.
#[derive(Deserialize)]
pub(super) struct FlavourPayload {
    pub(super) id: String,
    pub(super) selected_by: SelectedBy,
    #[serde(default)]
    pub(super) override_reason: Option<String>,
}

/// A flavour's provenance must be consistent: a rule selection carries no
/// override reason, a coach or athlete choice must give one.
pub(super) fn validate_flavour(flavour: &FlavourPayload) -> AppResult<()> {
    bounded("flavour.id", &flavour.id, MAX_SHORT_TEXT_LEN)?;
    match (flavour.selected_by, flavour.override_reason.as_deref()) {
        (SelectedBy::Rule, Some(reason)) if !reason.trim().is_empty() => {
            return Err(AppError::invalid_input(
                "flavour.override_reason is only for a coach or athlete choice; a rule \
                 selection carries none",
            ));
        }
        (SelectedBy::Coach | SelectedBy::Athlete, reason)
            if reason.is_none_or(|r| r.trim().is_empty()) =>
        {
            return Err(AppError::invalid_input(format!(
                "flavour.override_reason is required when selected_by is {}",
                flavour.selected_by.as_str()
            )));
        }
        _ => {}
    }
    if let Some(reason) = flavour.override_reason.as_deref() {
        bounded("flavour.override_reason", reason, MAX_TEXT_LEN)?;
    }
    Ok(())
}

/// The season window, when stated, is two plan dates in order.
pub(super) fn validate_season_window(start: Option<&str>, end: Option<&str>) -> AppResult<()> {
    if let Some(start) = start {
        plan_date("season_start", start)?;
    }
    if let Some(end) = end {
        plan_date("season_end", end)?;
    }
    if let (Some(start), Some(end)) = (start, end) {
        if start > end {
            return Err(AppError::invalid_input(format!(
                "season_start {start} is after season_end {end}"
            )));
        }
    }
    Ok(())
}

/// Validate one season phase: dates, length, prose bounds, and the targets a
/// phase may state (shares inside 0..=1 and a feasible time-in-zone split).
pub(super) fn validate_phase(phase: &PlanPhase) -> AppResult<()> {
    plan_date("phase start", &phase.start)?;
    if phase.weeks == 0 {
        return Err(AppError::invalid_input("phase weeks must be >= 1"));
    }
    bounded("phase.purpose", &phase.purpose, MAX_TEXT_LEN)?;
    bounded("phase.intent", &phase.intent, MAX_TEXT_LEN)?;
    if let Some(hours) = phase.target_hours {
        if !(0.0..=MAX_TARGET_HOURS).contains(&hours) {
            return Err(AppError::invalid_input(format!(
                "phase target_hours must be between 0 and {MAX_TARGET_HOURS}, got {hours}"
            )));
        }
    }
    if let Some(share) = phase.volume_share_of_peak.as_ref() {
        share_in_unit("phase.volume_share_of_peak", share)?;
    }
    if let Some(tid) = phase.tid_target.as_ref() {
        for (name, share) in [("z1", &tid.z1), ("z2", &tid.z2), ("z3", &tid.z3)] {
            share_in_unit(&format!("phase.tid_target.{name}"), share)?;
        }
        let min_sum = tid.z1.min + tid.z2.min + tid.z3.min;
        let max_sum = tid.z1.max + tid.z2.max + tid.z3.max;
        if min_sum > 1.0 || max_sum < 1.0 {
            return Err(AppError::invalid_input(format!(
                "phase.tid_target cannot be met: the three minimums sum to {min_sum} and the \
                 three maximums to {max_sum}; a split needs minimums <= 1 and maximums >= 1"
            )));
        }
    }
    if let Some(cap) = phase.hard_sessions_max {
        if cap > MAX_HARD_SESSIONS_PER_WEEK {
            return Err(AppError::invalid_input(format!(
                "phase hard_sessions_max {cap} exceeds {MAX_HARD_SESSIONS_PER_WEEK}"
            )));
        }
    }
    if let Some(id) = phase.skeleton_id.as_deref() {
        bounded("phase.skeleton_id", id, MAX_SHORT_TEXT_LEN)?;
    }
    Ok(())
}

/// A share must be a finite value inside `0..=1` with `min <= max`.
fn share_in_unit(field: &str, share: &Share) -> AppResult<()> {
    for (bound, value) in [("min", share.min), ("max", share.max)] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(AppError::invalid_input(format!(
                "{field}.{bound} {value} is outside 0..=1"
            )));
        }
    }
    if share.min > share.max {
        return Err(AppError::invalid_input(format!(
            "{field}.min {} is above {field}.max {}",
            share.min, share.max
        )));
    }
    Ok(())
}

/// A day's template reference: a rest day has no session to build from a
/// template, and parameters without a template fill nothing.
pub(super) fn validate_day_template(day: &PlannedDay) -> AppResult<()> {
    if let Some(slug) = day.template_slug.as_deref() {
        bounded("day.template_slug", slug, MAX_SHORT_TEXT_LEN)?;
        if day.is_rest() {
            return Err(AppError::invalid_input(format!(
                "day {} is a rest day and names template {slug} — there is no session to \
                 build from it",
                day.date
            )));
        }
    } else if day.template_params.is_some_and(|p| !p.is_empty()) {
        return Err(AppError::invalid_input(format!(
            "day {} gives template_params without a template_slug to fill",
            day.date
        )));
    }
    Ok(())
}

/// A week's `phase_index` must name a phase the plan will have once the save
/// lands — the outline's when one is sent, else the active plan's.
pub(super) fn check_phase_indexes(weeks: &[WeekPayload], phase_count: usize) -> AppResult<()> {
    for week in weeks {
        let Some(index) = week.phase_index else {
            continue;
        };
        if usize::try_from(index).is_ok_and(|i| i < phase_count) {
            continue;
        }
        return Err(AppError::invalid_input(format!(
            "week {} names phase_index {index} but the plan has {phase_count} phase(s)",
            week.week_start
        )));
    }
    Ok(())
}

/// Turn the payload's flavour into the stored selection: the id must name a
/// catalogue flavour, whose family, sequencing and modifiers are copied so the
/// stored plan still says what it was built on after the catalogue moves.
pub(super) fn resolve_flavour(
    catalogue: &TrainingCatalogueRegistry,
    payload: &FlavourPayload,
) -> AppResult<FlavourSelection> {
    let flavour = catalogue.flavour(&payload.id).ok_or_else(|| {
        let known: Vec<String> = catalogue.flavours().into_iter().map(|f| f.id).collect();
        AppError::invalid_input(format!(
            "flavour '{}' is not in the training catalogue; one of: {}",
            payload.id,
            known.join(", ")
        ))
    })?;
    Ok(FlavourSelection {
        id: flavour.id,
        family: flavour.family,
        sequencing: flavour.sequencing,
        modifiers: flavour.modifiers,
        selected_by: payload.selected_by,
        override_reason: payload
            .override_reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned),
    })
}

/// Every `template_slug` a day names must be a catalogue template or one of
/// this athlete's own saved sessions — the same set `list_workout_templates`
/// shows — so a saved day never points at a template nobody can read.
pub(super) async fn check_template_slugs(
    catalogue: &TrainingCatalogueRegistry,
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    weeks: &[WeekPayload],
) -> AppResult<()> {
    for day in weeks.iter().flat_map(|w| &w.days) {
        let Some(slug) = day.template_slug.as_deref() else {
            continue;
        };
        if catalogue.workout(slug).is_some() {
            continue;
        }
        let own = repos
            .workout_templates
            .get_user_workout_template(tenant, user_id, slug)
            .await?;
        if own.is_none() {
            return Err(AppError::invalid_input(format!(
                "day {} names template '{slug}', which is neither in the catalogue nor \
                 among this athlete's saved sessions — list_workout_templates names both",
                day.date
            )));
        }
    }
    Ok(())
}
