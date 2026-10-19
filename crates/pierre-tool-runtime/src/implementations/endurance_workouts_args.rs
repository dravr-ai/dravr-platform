// ABOUTME: Argument parsing for the endurance workout tools: prescription ids, list detail, vocabularies, sport
// ABOUTME: Also the summary row an agent picks a workout template by, without its steps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeSet;
use std::slice;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{SportType, WorkoutTemplate};
use serde_json::Value;
use uuid::Uuid;

use crate::implementations::endurance_workouts_output::{
    TemplateFit, TemplateProgression, WorkoutTemplateSummary,
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
    })
}
