// ABOUTME: Shared helpers for the tools that write to an athlete's provider calendar
// ABOUTME: The authed provider, the destructive annotation set, and the step contract (schema + bounds) every structured session shares
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{RelativeIntensity, TenantId, WorkoutStep};
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_providers::core::FitnessProvider;
use pierre_services::plan_calendar_push::CALENDAR_PROVIDER;
use uuid::Uuid;

use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;

/// Annotation set for a tool that deletes calendar entries on the provider:
/// destructive, but safe to repeat (a second call finds nothing to delete).
pub(super) fn destructive_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Build the athlete's authenticated calendar provider, or the refusal a
/// calendar-less athlete gets.
pub(super) async fn calendar_provider(
    context: &ToolExecutionContext,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Box<dyn FitnessProvider>> {
    let tenant_str = tenant_id.to_string();
    AuthService::new(context.resources.clone())
        .create_authenticated_provider(CALENDAR_PROVIDER, user_id, Some(tenant_str.as_str()))
        .await
        .map_err(|resp| {
            AppError::invalid_input(resp.error.unwrap_or_else(|| {
                "Connect an Intervals.icu account first — it is the only calendar this \
                 platform can write to"
                    .to_owned()
            }))
        })
}

/// Longest label, target, or other short text on a step or a plan day.
pub(super) const MAX_SHORT_TEXT_LEN: usize = 200;
/// Longest coaching cue on a step.
pub(super) const MAX_NOTE_LEN: usize = 1_000;
/// Upper bounds on one structured session. A session is serialized into the
/// ledger row and rendered into the calendar event body, so an unbounded
/// payload inflates both; these keep one degenerate call from doing that
/// while staying far above any real coached session.
pub(super) const MAX_SESSION_STEPS: usize = 50;
pub(super) const MAX_STEP_REPEAT: u32 = 100;
pub(super) const MAX_STEP_DURATION_SECONDS: u32 = 6 * 60 * 60;
pub(super) const MAX_STEP_DISTANCE_METERS: f64 = 1_000_000.0;

/// Reject a text field longer than `max` Unicode scalar values.
pub(super) fn bounded(field: &str, value: &str, max: usize) -> AppResult<()> {
    let len = value.chars().count();
    if len > max {
        return Err(AppError::invalid_input(format!(
            "{field} is too long ({len} chars; max {max})"
        )));
    }
    Ok(())
}

/// Reject a text field that is blank or over-long.
pub(super) fn required_text(field: &str, value: &str, max: usize) -> AppResult<()> {
    if value.trim().is_empty() {
        return Err(AppError::invalid_input(format!(
            "{field} must not be empty"
        )));
    }
    bounded(field, value, max)
}

/// What a step's `target_zone` must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetRule {
    /// Any non-empty label: the provider renders a target it cannot resolve
    /// as a timed step, and the label still reaches the athlete as the cue.
    /// `prescribe_workout`'s rule, and that tool ships with the data tools.
    #[cfg(feature = "tools-data")]
    AnyLabel,
    /// A label inside [`RelativeIntensity`]'s grammar, so the provider can
    /// compute a planned load for the step — the reason a plan day carries
    /// steps at all.
    Resolvable,
}

/// The vocabulary a resolvable target comes from, for the rejection.
const TARGET_VOCABULARY: &str = "a zone (Z1-Z7, or 'Z2 HR'), a named zone (recovery, endurance, \
                                 tempo, threshold, VO2max, anaerobic, sprint), 'sweet spot', or a \
                                 percent band ('75%', '88-93% FTP')";

/// Validate one step and return the seconds it contributes to its session.
///
/// `at` is the step's path in the payload (`session.structure[2]`), so a
/// model fixing a 12-step payload learns which one to change rather than
/// re-guessing.
///
/// # Errors
///
/// Returns the first bound the step breaks, naming the field.
pub(super) fn validate_step(at: &str, step: &WorkoutStep, rule: TargetRule) -> AppResult<u64> {
    let field = |name: &str| format!("{at}.{name}");
    required_text(&field("label"), &step.label, MAX_SHORT_TEXT_LEN)?;
    required_text(&field("target_zone"), &step.target_zone, MAX_SHORT_TEXT_LEN)?;
    if rule == TargetRule::Resolvable && RelativeIntensity::parse(&step.target_zone).is_none() {
        return Err(AppError::invalid_input(format!(
            "{} '{}' is not a target the calendar can resolve — use {TARGET_VOCABULARY}",
            field("target_zone"),
            step.target_zone
        )));
    }
    if let Some(note) = step.note.as_deref() {
        bounded(&field("note"), note, MAX_NOTE_LEN)?;
    }
    if !(1..=MAX_STEP_DURATION_SECONDS).contains(&step.duration_seconds) {
        return Err(AppError::invalid_input(format!(
            "{} must be between 1 and {MAX_STEP_DURATION_SECONDS}, got {}",
            field("duration_seconds"),
            step.duration_seconds
        )));
    }
    if !(1..=MAX_STEP_REPEAT).contains(&step.repeat) {
        return Err(AppError::invalid_input(format!(
            "{} must be between 1 and {MAX_STEP_REPEAT}, got {}",
            field("repeat"),
            step.repeat
        )));
    }
    if let Some(distance) = step.distance_meters {
        if !distance.is_finite() || distance <= 0.0 || distance > MAX_STEP_DISTANCE_METERS {
            return Err(AppError::invalid_input(format!(
                "{} must be between 0 and {MAX_STEP_DISTANCE_METERS}, got {distance}",
                field("distance_meters")
            )));
        }
    }
    Ok(u64::from(step.duration_seconds) * u64::from(step.repeat))
}

/// Schema for one step of a structured session — the shape
/// `prescribe_workout`'s `session.structure` and a plan day's `steps` share,
/// so the model learns one vocabulary and both calendars get the same DSL.
pub(super) fn step_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "label".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "What this step is (\"Warm-up\", \"Montées\", \"Interval\", \"Cool-down\")."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    p.insert(
        "duration_seconds".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some("How long ONE repetition of this step lasts, in seconds.".to_owned()),
            ..Default::default()
        },
    );
    p.insert(
        "target_zone".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Intensity RELATIVE to the athlete's thresholds (\"Z2\", \"Z2 HR\", \
                 \"Threshold\", \"sweet spot\", \"88-93% FTP\"). Never absolute watts."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    p.insert(
        "repeat".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some("Repetitions of this step; omit for a single block.".to_owned()),
            ..Default::default()
        },
    );
    p.insert(
        "distance_meters".to_owned(),
        PropertySchema {
            property_type: "number".to_owned(),
            description: Some("Distance in metres for a distance-based step.".to_owned()),
            ..Default::default()
        },
    );
    p.insert(
        "note".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Coaching cue for this step, in your own voice — it reaches the athlete's \
                 calendar entry."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    PropertySchema {
        property_type: "object".to_owned(),
        description: Some("One step of the session.".to_owned()),
        properties: Some(p),
        required: Some(vec![
            "label".to_owned(),
            "duration_seconds".to_owned(),
            "target_zone".to_owned(),
        ]),
        ..Default::default()
    }
}
