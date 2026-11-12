// ABOUTME: Argument parsing for the endurance workout tools: prescription ids, list detail, vocabularies, sport
// ABOUTME: Also the summary row an agent picks a template by, and the session a family's preset arguments build

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeSet;
use std::slice;

use pierre_contremaitre::{session_view, ResolvedWorkout};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::{
    AthleteAnchors, EnvelopeReport, InstanceRequest, PresetParams,
};
use pierre_core::models::{SportType, WorkoutTemplate};
use pierre_memory::training_plans::TemplateParams;
use pierre_services::plan_calendar_push::plan_sport;
use serde_json::Value;
use uuid::Uuid;

use super::endurance_workouts::structure_minutes;
use super::training_plan_vision::check_template_params;
use crate::implementations::endurance_workouts_output::{
    PresetSummary, TemplateFit, TemplateProgression, WorkoutTemplateSummary,
};

/// Parse an optional prescription-id argument.
pub(super) fn optional_prescription_id(args: &Value, key: &str) -> AppResult<Option<Uuid>> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            Uuid::parse_str(s).map_err(|e| {
                AppError::invalid_input(format!("{key} must be a prescription id (UUID): {e}"))
            })
        })
        .transpose()
}

/// The two shapes a listed template comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListDetail {
    /// The fields an agent picks a session by.
    Summary,
    /// The whole template, steps and target zones included.
    Full,
}

impl ListDetail {
    pub(super) const ALL: &[Self] = &[Self::Summary, Self::Full];

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Full => "full",
        }
    }
}

/// A trimmed optional string argument; `None` when absent, null or blank.
pub(super) fn optional_text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The vocabulary names, comma-separated, for an error that has to name
/// every allowed value.
pub(super) fn vocabulary<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> String {
    all.iter()
        .map(|value| name(*value))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve an optional closed-vocabulary argument, refusing a value outside
/// the vocabulary with the whole vocabulary in the message.
pub(super) fn vocab_arg<T: Copy>(
    args: &Value,
    key: &str,
    all: &[T],
    name: fn(T) -> &'static str,
) -> AppResult<Option<T>> {
    let Some(raw) = optional_text(args, key) else {
        return Ok(None);
    };
    all.iter()
        .copied()
        .find(|value| name(*value) == raw)
        .map(Some)
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "{key} '{raw}' is not in the vocabulary; one of: {}",
                vocabulary(all, name)
            ))
        })
}

/// Resolve the optional `sport` argument: a `snake_case` [`SportType`] name.
///
/// An unknown name is refused with the sports the bank is written for, so
/// the agent can pick one that returns something.
pub(super) fn sport_arg(args: &Value, bank: &[WorkoutTemplate]) -> AppResult<Option<SportType>> {
    let Some(raw) = optional_text(args, "sport") else {
        return Ok(None);
    };
    serde_json::from_value::<SportType>(Value::String(raw.to_owned()))
        .map(Some)
        .map_err(|_| {
            let mut sports = BTreeSet::new();
            for template in bank {
                for sport in slice::from_ref(&template.sport)
                    .iter()
                    .chain(&template.sport_variants)
                {
                    if let Ok(Value::String(name)) = serde_json::to_value(sport) {
                        sports.insert(name);
                    }
                }
            }
            AppError::invalid_input(format!(
                "sport '{raw}' is not a sport name; the bank is written for: {}",
                sports.into_iter().collect::<Vec<_>>().join(", ")
            ))
        })
}

/// The fields an agent picks a session by, without the steps.
pub(super) fn summary_row(template: &WorkoutTemplate) -> AppResult<WorkoutTemplateSummary> {
    let params = serde_json::to_value(&template.params)
        .map_err(|e| AppError::internal(format!("serialize template params: {e}")))?;
    // The ENUMS, not their Debug names: every one is a serde enum and the
    // untyped payload serialized them, so `{:?}` would change the wire.
    Ok(WorkoutTemplateSummary {
        slug: template.slug.clone(),
        name: template.name.clone(),
        purpose: template.purpose,
        sport: template.sport.clone(),
        sport_variants: template.sport_variants.clone(),
        duration_minutes: template.duration_minutes,
        intensity_distribution: template.intensity_distribution,
        evidence_tier: template.evidence_tier,
        caveat: template.caveat.clone(),
        params,
        fit: TemplateFit {
            phases: template.fit.phases.clone(),
            readiness_min: template.fit.readiness_min,
            max_per_week: template.fit.max_per_week,
            min_spacing_hours: template.fit.min_spacing_hours,
        },
        progression: TemplateProgression {
            order: template.progression.order.clone(),
            max_weekly_step: template.progression.max_weekly_step,
        },
        is_compiled_in: template.is_compiled_in,
        presets: preset_summaries(template),
        aliases: template
            .aliases
            .iter()
            .map(|(slug, alias)| (slug.clone(), alias.preset.clone()))
            .collect(),
    })
}

/// A family's presets as summary rows: key, floor, quality flag and the
/// evidence badge per sport.
fn preset_summaries(template: &WorkoutTemplate) -> Vec<PresetSummary> {
    template
        .presets
        .iter()
        .map(|preset| {
            let fit = template.effective_fit(Some(&preset.key));
            PresetSummary {
                key: preset.key.clone(),
                name: preset.name.clone(),
                readiness_min: fit.readiness_min,
                counts_as_quality: fit.counts_as_quality,
                badges: preset
                    .families()
                    .filter_map(|family| {
                        preset
                            .realization(family)
                            .map(|r| (family.as_str().to_owned(), r.badge.status))
                    })
                    .collect(),
            }
        })
        .collect()
}

/// A family template made into the one session `prescribe_workout` pushes.
///
/// The preset is the `preset` argument, else the legacy slug the call named
/// (an alias or preset key the family absorbed), else the family's default;
/// the sport is the `sport` argument, else the preset's pinned or primary
/// one; the values are `template_params`, held to a session's bounds. The
/// session is measured by the envelope check, whose report travels on the
/// result when it found anything — it warns, and never refuses the
/// prescription; a generated session past a session's bounds is refused
/// as an inline one would be. A template with no presets passes through
/// unchanged, and naming a preset or values for it is refused as input the
/// call cannot honour.
pub(super) fn family_session(
    resolved: ResolvedWorkout,
    args: &Value,
    anchors: Option<&AthleteAnchors>,
) -> AppResult<(WorkoutTemplate, Option<EnvelopeReport>)> {
    let ResolvedWorkout { template, request } = resolved;
    let preset_arg = args.get("preset").and_then(Value::as_str);
    let stated = match args.get("template_params").filter(|v| !v.is_null()) {
        Some(raw) => {
            let params = serde_json::from_value::<TemplateParams>(raw.clone())
                .map_err(|e| AppError::invalid_input(format!("`template_params`: {e}")))?;
            check_template_params("template_params", &params)?;
            Some(params)
        }
        None => None,
    };
    if !template.is_family() {
        if let Some(preset) = preset_arg {
            return Err(AppError::invalid_input(format!(
                "template '{}' has no presets, so preset '{preset}' cannot apply",
                template.slug
            )));
        }
        if stated.is_some() {
            return Err(AppError::invalid_input(format!(
                "template '{}' has no presets, so template_params cannot apply",
                template.slug
            )));
        }
        if let Some(sport) = args.get("sport").and_then(Value::as_str) {
            return Err(AppError::invalid_input(format!(
                "template '{}' has no presets, so sport '{sport}' cannot apply: it is \
                 written for {:?}",
                template.slug, template.sport
            )));
        }
        return Ok((template, None));
    }
    let instance = template.instantiate(
        &InstanceRequest {
            preset: preset_arg.map(str::to_owned).or(request.preset),
            sport: args.get("sport").and_then(Value::as_str).map(plan_sport),
            params: stated.map_or_else(PresetParams::default, |p| p.preset_params()),
        },
        anchors,
    );
    structure_minutes(
        &format!("the steps generated from {}", template.slug),
        &instance.generated.steps,
    )?;
    let report = instance.report;
    let session = session_view(&template, instance.generated);
    Ok((session, (!report.is_clear()).then_some(report)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::implementations::tests::{family, FAMILY_SLUG};
    use pierre_core::models::periodization::{
        ConstraintAction, EvidenceStatus, FindingCode, ReadinessLevel,
    };
    use serde_json::json;

    fn legacy(slug: &str, sport: SportType) -> ResolvedWorkout {
        ResolvedWorkout {
            template: family(),
            request: InstanceRequest {
                preset: Some(slug.to_owned()),
                sport: Some(sport),
                ..InstanceRequest::default()
            },
        }
    }

    fn as_written(template: WorkoutTemplate) -> ResolvedWorkout {
        ResolvedWorkout {
            template,
            request: InstanceRequest::default(),
        }
    }

    #[test]
    fn a_legacy_slug_prescribes_its_preset_named_once() {
        let args = json!({ "template_slug": "fixture_alias_30_30" });
        let (session, envelope) =
            family_session(legacy("fixture_alias_30_30", SportType::Run), &args, None).unwrap();
        assert_eq!(session.name, "VO2max — short intervals — 30/30 (Billat)");
        assert_eq!(session.sport, SportType::Run, "the alias pins the run form");
        assert_eq!(session.fit.readiness_min, ReadinessLevel::P3);
        assert_eq!(session.structure[1].target_zone, "100-105% vVO2max");
        assert_eq!(session.duration_minutes, 15 + 15 + 10);
        assert!(
            envelope.is_none(),
            "a default instance is clear: {envelope:?}"
        );
    }

    #[test]
    fn a_value_outside_the_preset_warns_and_still_prescribes() {
        let args = json!({
            "template_slug": FAMILY_SLUG,
            "preset": "fx_30_15",
            "sport": "ride",
            "template_params": { "reps": 20 },
        });
        let (session, envelope) = family_session(as_written(family()), &args, None).unwrap();
        assert_eq!(session.name, "VO2max — short intervals — 30/15 (Rønnestad)");
        let work = session.structure.iter().find(|s| s.repeat > 1).unwrap();
        assert_eq!(work.repeat, 20, "the value goes through as written");
        let finding = envelope
            .as_ref()
            .and_then(|r| {
                r.findings
                    .iter()
                    .find(|f| f.code == FindingCode::ParamOutOfRange)
            })
            .unwrap();
        assert_eq!(finding.action, ConstraintAction::Warn);
    }

    #[test]
    fn values_past_a_sessions_bounds_are_refused_before_anything_is_built() {
        let args = json!({
            "template_slug": FAMILY_SLUG,
            "template_params": { "sets": 4_000_000_000_u32 },
        });
        let refused = family_session(as_written(family()), &args, None).unwrap_err();
        assert!(
            refused.to_string().contains("template_params.sets"),
            "{refused}"
        );
    }

    #[test]
    fn a_preset_or_values_for_a_plain_template_are_refused() {
        let mut plain = family();
        plain.presets.clear();
        let preset = json!({ "template_slug": FAMILY_SLUG, "preset": "fx_30_15" });
        assert!(family_session(as_written(plain.clone()), &preset, None).is_err());
        let values = json!({ "template_slug": FAMILY_SLUG, "template_params": { "reps": 10 } });
        assert!(family_session(as_written(plain.clone()), &values, None).is_err());
        let (session, envelope) = family_session(
            as_written(plain.clone()),
            &json!({ "template_slug": FAMILY_SLUG }),
            None,
        )
        .unwrap();
        assert_eq!(session, plain, "a plain template passes through unchanged");
        assert!(envelope.is_none());
    }

    #[test]
    fn a_family_summary_names_each_preset_with_its_floor_and_badges() {
        let row = summary_row(&family()).unwrap();
        let keys: Vec<&str> = row.presets.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["fx_30_15", "fx_30_30"]);
        assert_eq!(row.presets[0].readiness_min, ReadinessLevel::P2);
        assert_eq!(row.presets[1].readiness_min, ReadinessLevel::P3);
        assert!(row.presets.iter().all(|p| p.counts_as_quality));
        assert_eq!(
            row.presets[1].badges.get("run"),
            Some(&EvidenceStatus::Trialled)
        );
        assert_eq!(
            row.presets[1].badges.get("ride"),
            Some(&EvidenceStatus::Transposed)
        );
        assert_eq!(
            row.aliases.get("fixture_alias_30_15").map(String::as_str),
            Some("fx_30_15")
        );
    }
}
