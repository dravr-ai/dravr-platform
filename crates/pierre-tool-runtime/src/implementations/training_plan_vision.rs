// ABOUTME: The vision half of a save_training_plan payload — flavour provenance, phase targets, template references
// ABOUTME: Split from training_plans.rs to keep that file under the 1200-line ceiling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Validation of what the Annual Vision adds to a plan outline.
//!
//! The flavour and who chose it, each phase's targets, the season window, a
//! week's phase index and a day's template. Every catalogue reference is
//! resolved through the plan's agent package over the live registry before
//! anything is written, and the tier that answered is what the saved day
//! records.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::{
    envelope_check, EnvelopeInput, FlavourInputs, FlavourVerdict, InstanceRequest, PresetParams,
    PresetShape, Share, WorkoutTemplate,
};
use pierre_core::models::{SportType, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{
    DayEnvelope, FlavourSelection, PlanPhase, PlannedDay, SelectedBy, TemplateParams,
    TemplateSource,
};
use pierre_services::agent_package::PackagedCatalogue;
use pierre_services::plan_calendar_push::plan_sport;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use super::calendar::{
    bounded, validate_step, TargetRule, MAX_SESSION_STEPS, MAX_SHORT_TEXT_LEN,
    MAX_STEP_DURATION_SECONDS, MAX_STEP_REPEAT,
};
use super::training_plans::{
    plan_date, WeekPayload, MAX_DURATION_MIN, MAX_TARGET_HOURS, MAX_TEXT_LEN,
};

/// Hard sessions a week no phase exceeds; the flavour caps are lower.
const MAX_HARD_SESSIONS_PER_WEEK: u32 = 7;

/// The flavour half of an outline as the model sends it: the id, who chose
/// it, and — when `recommend_plan_flavour` ran — its verdict and inputs,
/// passed through verbatim. Family, sequencing and modifiers are copied from
/// the catalogue at save time, never trusted from the payload.
#[derive(Deserialize)]
pub(super) struct FlavourPayload {
    pub(super) id: String,
    pub(super) selected_by: SelectedBy,
    #[serde(default)]
    pub(super) override_reason: Option<String>,
    /// Raw, parsed by [`snapshot`]: an absent or empty object carries
    /// nothing, anything else must be the kernel's own shape.
    #[serde(default)]
    pub(super) verdict: Option<Value>,
    #[serde(default)]
    pub(super) inputs: Option<Value>,
}

/// A snapshot field as the kernel's type, or `None` when the payload left it
/// out — `null` and `{}` both read as absent, since neither carries a
/// verdict. Anything else that does not parse names the field, so an agent
/// passing the tool's output through a mangling step hears about it rather
/// than storing a snapshot that later reads as "the rule proposed nothing".
fn snapshot<T: DeserializeOwned>(field: &str, raw: Option<&Value>) -> AppResult<Option<T>> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) if map.is_empty() => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| AppError::invalid_input(format!("flavour.{field}: {e}"))),
    }
}

/// A flavour's provenance must be consistent: a rule selection carries no
/// override reason, an agent or athlete choice must give one, and a rule
/// selection saved with the rule's verdict must be the flavour that verdict
/// ranked first — anything else is an override wearing the rule's name,
/// which is exactly what the override rate would then fail to count.
pub(super) fn validate_flavour(flavour: &FlavourPayload) -> AppResult<()> {
    bounded("flavour.id", &flavour.id, MAX_SHORT_TEXT_LEN)?;
    let verdict: Option<FlavourVerdict> = snapshot("verdict", flavour.verdict.as_ref())?;
    snapshot::<FlavourInputs>("inputs", flavour.inputs.as_ref())?;
    if flavour.selected_by == SelectedBy::Rule {
        if let Some(top) = verdict.as_ref().and_then(FlavourVerdict::top) {
            if top.id != flavour.id {
                return Err(AppError::invalid_input(format!(
                    "flavour '{}' is not what the rule ranked first ('{}'); a choice that departs \
                     from the verdict is selected_by coach or athlete, with its reason",
                    flavour.id, top.id
                )));
            }
        }
    }
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
    if let Some(share) = phase.volume_share_of_peak {
        share_in_unit("phase.volume_share_of_peak", share)?;
    }
    if let Some(tid) = phase.tid_target.as_ref() {
        for (name, share) in [("z1", tid.z1), ("z2", tid.z2), ("z3", tid.z3)] {
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
fn share_in_unit(field: &str, share: Share) -> AppResult<()> {
    for (bound, value) in [("min", share.min), ("max", share.max)] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(AppError::invalid_input(format!(
                "{field}.{bound} {value} is outside 0..=1"
            )));
        }
    }
    (share.min <= share.max).ok_or_else(|| {
        AppError::invalid_input(format!(
            "{field}.min {} is above {field}.max {}",
            share.min, share.max
        ))
    })
}

/// A day's template reference: a rest day has no session to build from a
/// template, and parameters or a preset without a template fill nothing.
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
    } else if day.template_preset.is_some() {
        return Err(AppError::invalid_input(format!(
            "day {} gives template_preset without a template_slug to name its family",
            day.date
        )));
    }
    if let Some(preset) = day.template_preset.as_deref() {
        bounded("day.template_preset", preset, MAX_SHORT_TEXT_LEN)?;
    }
    if let Some(params) = day.template_params.as_ref() {
        check_template_params(&format!("day {} template_params", day.date), params)?;
    }
    Ok(())
}

/// Hold the values a day or a prescription fills a template with to the
/// bounds a written session keeps. A family's generator builds one step
/// group per set before anything measures the result, so an unbounded
/// `sets` would build millions of steps first; each other value is the
/// bound its step field already has. A value inside these bounds but
/// outside the preset's range is the envelope check's to warn about.
pub(super) fn check_template_params(at: &str, params: &TemplateParams) -> AppResult<()> {
    let session_steps = u32::try_from(MAX_SESSION_STEPS).unwrap_or(u32::MAX);
    for (key, value, max) in [
        ("sets", params.sets, session_steps),
        ("reps", params.reps, MAX_STEP_REPEAT),
        (
            "work_seconds",
            params.work_seconds,
            MAX_STEP_DURATION_SECONDS,
        ),
        (
            "rest_seconds",
            params.rest_seconds,
            MAX_STEP_DURATION_SECONDS,
        ),
        (
            "set_rest_seconds",
            params.set_rest_seconds,
            MAX_STEP_DURATION_SECONDS,
        ),
        (
            "duration_minutes",
            params.duration_minutes,
            MAX_DURATION_MIN,
        ),
    ] {
        if let Some(value) = value.filter(|v| *v > max) {
            return Err(AppError::invalid_input(format!(
                "{at}.{key} {value} is above the most a session holds ({max})"
            )));
        }
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
/// flavour the agent's package or the catalogue carries, whose family,
/// sequencing and modifiers are copied so the stored plan still says what it
/// was built on after the catalogue moves.
pub(super) fn resolve_flavour(
    catalogue: &PackagedCatalogue<'_>,
    payload: &FlavourPayload,
) -> AppResult<FlavourSelection> {
    let (flavour, _tier) = catalogue.flavour(&payload.id).ok_or_else(|| {
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
        verdict_snapshot: snapshot("verdict", payload.verdict.as_ref())?,
        inputs_snapshot: snapshot("inputs", payload.inputs.as_ref())?,
    })
}

/// Build or measure every day that names a parametric family (plan step 11).
///
/// A day that names a family — or a legacy slug a family absorbed — and
/// writes no steps gets its preset's generated steps and duration; the
/// day's stated duration sizes a continuous or insert preset. A day that
/// writes its own steps is measured against its preset. Either way the
/// envelope check's result is stamped on the day: it warns and relabels,
/// and never refuses the save (founder decision D3). A non-family template
/// is left as the agent wrote it.
///
/// # Errors
///
/// Invalid input when a day names a `template_preset` for a template that
/// has no presets — the athlete's own sessions included.
pub(super) fn instantiate_template_days(
    catalogue: &PackagedCatalogue<'_>,
    weeks: &mut [WeekPayload],
) -> AppResult<()> {
    for day in weeks.iter_mut().flat_map(|w| &mut w.days) {
        day.envelope = None;
        let Some(slug) = day.template_slug.clone() else {
            continue;
        };
        let resolved = catalogue.resolve_workout(&slug).map(|(r, _)| r);
        let Some(resolved) = resolved.filter(|r| r.template.is_family()) else {
            if let Some(preset) = day.template_preset.as_deref() {
                return Err(AppError::invalid_input(format!(
                    "day {} names preset '{preset}' for template '{slug}', which has no presets",
                    day.date
                )));
            }
            continue;
        };
        let template = &resolved.template;
        let preset = day
            .template_preset
            .clone()
            .or_else(|| resolved.request.preset.clone());
        let sport = plan_sport(&day.sport);
        let stated = day.template_params.map(|p| p.preset_params());
        if day.steps.is_empty() {
            generate_day(day, template, preset.clone(), sport, stated);
        } else {
            let report = envelope_check(&EnvelopeInput {
                template,
                preset: preset.as_deref(),
                sport: &sport,
                steps: &day.steps,
                params: stated.as_ref(),
                generated: false,
                anchors: None,
            });
            day.envelope = Some(DayEnvelope::from_report(report, false));
        }
        // Record the preset the day resolved to — its key, not the legacy
        // slug that named it — so later reads need no alias table.
        if day.template_preset.is_none() {
            day.template_preset = day.envelope.as_ref().and_then(|e| e.preset.clone());
        }
    }
    Ok(())
}

/// Fill a family day's steps and duration from its preset, and stamp what
/// the envelope check found on them.
fn generate_day(
    day: &mut PlannedDay,
    template: &WorkoutTemplate,
    preset: Option<String>,
    sport: SportType,
    stated: Option<PresetParams>,
) {
    let mut params = stated.unwrap_or_default();
    let sized_by_duration = preset
        .as_deref()
        .and_then(|key| template.resolve_preset(key))
        .or_else(|| template.default_preset().map(|p| (p, None)))
        .is_some_and(|(p, _)| matches!(p.shape, PresetShape::Continuous | PresetShape::Insert));
    if sized_by_duration && params.duration_minutes.is_none() {
        params.duration_minutes = day.duration_min;
    }
    let instance = template.instantiate(
        &InstanceRequest {
            preset,
            sport: Some(sport),
            params,
        },
        None,
    );
    let steps_ok = instance.generated.steps.len() <= MAX_SESSION_STEPS
        && instance.generated.duration_minutes <= MAX_DURATION_MIN
        && instance
            .generated
            .steps
            .iter()
            .all(|step| validate_step("steps[]", step, TargetRule::Resolvable).is_ok());
    if !steps_ok {
        warn!(
            template = %template.slug,
            date = %day.date,
            "family generator produced steps outside the session bounds; the day keeps no steps"
        );
        day.envelope = Some(DayEnvelope {
            preset: instance.report.preset,
            relabelled_purpose: None,
            badge: None,
            generated: false,
            findings: Vec::new(),
            not_generated: Some(format!(
                "the steps built from these values leave a session's bounds ({} min, {} steps; \
                 at most {MAX_DURATION_MIN} min and {MAX_SESSION_STEPS} steps): write the steps, \
                 or shorten the day",
                instance.generated.duration_minutes,
                instance.generated.steps.len()
            )),
        });
        return;
    }
    day.steps = instance.generated.steps;
    day.duration_min = Some(instance.generated.duration_minutes);
    day.envelope = Some(DayEnvelope::from_report(instance.report, true));
}

/// Every `template_slug` a day names must be a template of the agent's
/// package, of the catalogue, or one of this athlete's own saved sessions —
/// the same set `list_workout_templates` shows — so a saved day never points
/// at a template nobody can read. The tier that answered is stamped on the
/// day as its `template_source`; whatever the payload carried there is
/// overwritten, and a day with no template records none.
pub(super) async fn check_template_slugs(
    catalogue: &PackagedCatalogue<'_>,
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    weeks: &mut [WeekPayload],
) -> AppResult<()> {
    for day in weeks.iter_mut().flat_map(|w| &mut w.days) {
        let Some(slug) = day.template_slug.as_deref() else {
            day.template_source = None;
            continue;
        };
        if let Some((_, tier)) = catalogue.workout(slug) {
            day.template_source = Some(tier.template_source());
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
        day.template_source = Some(TemplateSource::Athlete);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::implementations::tests::{catalogue_with_family, FAMILY_SLUG};
    use crate::implementations::training_plans_output::PlanEnvelopeReport;
    use pierre_core::models::periodization::WorkoutPurpose;
    use pierre_core::models::WorkoutStep;
    use serde_json::json;

    /// Zwift's tabata-style 3 × 8 × (20 s at 119 % FTP, 10 s at 50 %): its
    /// cycle mean sits under the family's floor, so it trains threshold.
    fn tabata() -> Vec<WorkoutStep> {
        let step = |label: &str, seconds: u32, target: &str, repeat: u32, group: Option<u32>| {
            WorkoutStep {
                label: label.to_owned(),
                duration_seconds: seconds,
                distance_meters: None,
                target_zone: target.to_owned(),
                repeat,
                repeat_group: group,
                note: None,
            }
        };
        let mut steps = vec![step("Warm-up", 900, "Z2", 1, None)];
        for set in 1..=3 {
            steps.push(step("Work", 20, "119% FTP", 8, Some(set)));
            steps.push(step("Float", 10, "50% FTP", 8, Some(set)));
        }
        steps.push(step("Cool-down", 600, "Z1", 1, None));
        steps
    }

    fn week(days: &Value) -> Vec<WeekPayload> {
        vec![serde_json::from_value(json!({ "week_start": "2026-11-16", "days": days })).unwrap()]
    }

    #[test]
    fn a_family_day_is_built_or_measured_and_stamped() {
        let registry = catalogue_with_family();
        let catalogue = PackagedCatalogue::catalogue_only(&registry);
        let plain = registry
            .workouts()
            .into_iter()
            .find(|w| !w.is_family())
            .unwrap()
            .slug;
        let mut weeks = week(&json!([
            { "date": "2026-11-16", "sport": "ride", "workout": "30/15",
              "template_slug": "fixture_alias_30_15" },
            { "date": "2026-11-18", "sport": "ride", "workout": "Tabata",
              "template_slug": FAMILY_SLUG, "template_preset": "fx_30_15",
              "steps": serde_json::to_value(tabata()).unwrap() },
            { "date": "2026-11-20", "sport": "ride", "workout": "Plain", "template_slug": plain,
              "envelope": { "generated": true, "relabelled_purpose": "threshold" } },
        ]));
        instantiate_template_days(&catalogue, &mut weeks).unwrap();
        let days = &weeks[0].days;

        // Warm-up, 3 × (work, float), two set rests, cool-down: 61 minutes.
        assert_eq!(days[0].steps.len(), 10, "{:?}", days[0].steps);
        assert_eq!(days[0].duration_min, Some(61));
        assert_eq!(
            days[0].template_preset.as_deref(),
            Some("fx_30_15"),
            "the preset key is recorded, not the legacy slug that named it"
        );
        let built = days[0].envelope.as_ref().unwrap();
        assert!(built.generated);
        assert!(built.findings.is_empty(), "{:?}", built.findings);

        assert_eq!(days[1].steps, tabata(), "written steps are kept as written");
        let measured = days[1].envelope.as_ref().unwrap();
        assert!(!measured.generated);
        assert_eq!(measured.relabelled_purpose, Some(WorkoutPurpose::Threshold));

        assert_eq!(
            days[2].envelope, None,
            "an envelope in the payload never survives the save"
        );
        assert_eq!(days[2].template_preset, None);

        let report = PlanEnvelopeReport::from_days(days).unwrap();
        assert_eq!(report.days.len(), 1, "only the relabelled day has findings");
        assert_eq!(report.days[0].date, "2026-11-18");
        assert_eq!(report.days[0].template_slug, FAMILY_SLUG);
        assert_eq!(
            report.days[0].relabelled_purpose,
            Some(WorkoutPurpose::Threshold)
        );
    }

    #[test]
    fn a_generated_session_past_a_days_length_keeps_no_steps() {
        let registry = catalogue_with_family();
        let catalogue = PackagedCatalogue::catalogue_only(&registry);
        // Six-hour reps, 3 × 13 of them: far past the 24 hours a day holds.
        let mut weeks = week(&json!([
            { "date": "2026-11-16", "sport": "ride", "workout": "30/15",
              "template_slug": FAMILY_SLUG, "template_params": { "work_seconds": 21_600 } },
        ]));
        instantiate_template_days(&catalogue, &mut weeks).unwrap();
        let day = &weeks[0].days[0];
        assert!(day.steps.is_empty(), "{:?}", day.steps);
        let envelope = day.envelope.as_ref().expect("the agent is told why");
        assert!(!envelope.generated);
        let why = envelope.not_generated.as_deref().unwrap_or_default();
        assert!(why.contains("write the steps"), "{why}");
        let report = PlanEnvelopeReport::from_days(&weeks[0].days).unwrap();
        assert_eq!(report.days[0].not_generated.as_deref(), Some(why));
    }

    #[test]
    fn a_preset_on_a_template_without_presets_is_refused() {
        let registry = catalogue_with_family();
        let catalogue = PackagedCatalogue::catalogue_only(&registry);
        let plain = registry
            .workouts()
            .into_iter()
            .find(|w| !w.is_family())
            .unwrap()
            .slug;
        let mut weeks = week(&json!([
            { "date": "2026-11-16", "sport": "ride", "workout": "x",
              "template_slug": plain, "template_preset": "fx_30_15" },
        ]));
        let refused = instantiate_template_days(&catalogue, &mut weeks)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("has no presets"), "{refused}");
    }

    #[test]
    fn a_days_preset_and_values_need_a_template_and_a_sessions_bounds() {
        let day = |value: Value| -> PlannedDay { serde_json::from_value(value).unwrap() };
        let orphan = day(json!({
            "date": "2026-11-16", "sport": "ride", "workout": "x", "template_preset": "fx_30_15",
        }));
        let refused = validate_day_template(&orphan).unwrap_err().to_string();
        assert!(refused.contains("template_preset"), "{refused}");
        let over = day(json!({
            "date": "2026-11-16", "sport": "ride", "workout": "x",
            "template_slug": FAMILY_SLUG, "template_params": { "sets": 51 },
        }));
        let refused = validate_day_template(&over).unwrap_err().to_string();
        assert!(refused.contains("template_params.sets 51"), "{refused}");
        let at_bounds = day(json!({
            "date": "2026-11-16", "sport": "ride", "workout": "x",
            "template_slug": FAMILY_SLUG, "template_preset": "fx_30_15",
            "template_params": { "sets": 50, "reps": 100, "duration_minutes": 1440 },
        }));
        assert!(validate_day_template(&at_bounds).is_ok());
    }
}
