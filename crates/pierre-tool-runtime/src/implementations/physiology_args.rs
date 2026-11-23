// ABOUTME: Argument parsing for set_physiology: numbers, labels, past days, the stated VDOT and a measurement's provenance
// ABOUTME: Each reader refuses a value it cannot read rather than dropping it, so no measurement is silently lost
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{MeasurementKind, MetricProvenance, PhysiologicalSex};
use serde_json::Value;

use crate::implementations::physiology::MEASUREMENT_SOURCE_MAX_CHARS;

/// Read an optional number, rejecting a non-numeric value rather than
/// silently ignoring it — a dropped measurement is exactly the failure this
/// tool exists to end.
pub(crate) fn optional_number(args: &Value, key: &str) -> AppResult<Option<f64>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_f64()
            .ok_or_else(|| AppError::invalid_input(format!("'{key}' must be a number")))
            .map(Some),
    }
}

/// Read an optional whole number, tolerating the `285.0` an LLM emits where
/// the schema says integer — the same leniency `commitment_create` needed
/// after strict rejection killed live calls.
pub(crate) fn optional_whole_number(args: &Value, key: &str) -> AppResult<Option<u64>> {
    let Some(n) = optional_number(args, key)? else {
        return Ok(None);
    };
    if n.fract() != 0.0 || !(0.0..=1_000_000.0).contains(&n) {
        return Err(AppError::invalid_input(format!(
            "'{key}' must be a whole number, got {n}"
        )));
    }
    // Guarded above: non-negative, integral, and far inside u64.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Some(n as u64))
}

/// Parse a fitness level from the athlete-facing label, case-insensitively.
///
/// Spelled out rather than deferring to serde: the enum derives `Deserialize`
/// with no rename rule, so serde would accept only `"Recreational"` and
/// reject the `"recreational"` an LLM writes.
pub(super) fn parse_fitness_level(raw: &str) -> AppResult<FitnessLevel> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "beginner" => Ok(FitnessLevel::Beginner),
        "recreational" => Ok(FitnessLevel::Recreational),
        "intermediate" => Ok(FitnessLevel::Intermediate),
        "advanced" => Ok(FitnessLevel::Advanced),
        "elite" => Ok(FitnessLevel::Elite),
        "professional" => Ok(FitnessLevel::Professional),
        other => Err(AppError::invalid_input(format!(
            "fitness_level must be one of beginner, recreational, intermediate, advanced, elite, professional; got '{other}'"
        ))),
    }
}

/// Parse `sex`. Strict: only the two values the published equations take.
pub(super) fn parse_sex(raw: &str) -> AppResult<PhysiologicalSex> {
    PhysiologicalSex::parse(raw.trim().to_ascii_lowercase().as_str()).ok_or_else(|| {
        AppError::invalid_input(format!("sex must be one of male, female; got '{raw}'"))
    })
}

/// Parse `measurement_kind`. Strict: a kind the tool cannot read must not be
/// guessed into a measurement.
fn parse_measurement_kind(raw: &str) -> AppResult<MeasurementKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "measured" => Ok(MeasurementKind::Measured),
        "estimated" => Ok(MeasurementKind::Estimated),
        other => Err(AppError::invalid_input(format!(
            "measurement_kind must be measured or estimated; got '{other}'"
        ))),
    }
}

/// Read an optional free-text label (`measurement_source`, `vo2_max_method`,
/// `vdot_method`): trimmed, `None` when blank, refused past
/// [`MEASUREMENT_SOURCE_MAX_CHARS`].
pub(super) fn optional_label(args: &Value, key: &str) -> AppResult<Option<String>> {
    let label = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if label
        .as_ref()
        .is_some_and(|l| l.chars().count() > MEASUREMENT_SOURCE_MAX_CHARS)
    {
        return Err(AppError::invalid_input(format!(
            "{key} must be at most {MEASUREMENT_SOURCE_MAX_CHARS} characters"
        )));
    }
    Ok(label)
}

/// Read an optional `YYYY-MM-DD` day no later than today (`measured_on`,
/// `vdot_as_of`).
fn optional_past_day(args: &Value, key: &str) -> AppResult<Option<CivilDate>> {
    let Some(raw) = args.get(key).and_then(Value::as_str) else {
        return Ok(None);
    };
    let day = CivilDate::parse_from_str(raw.trim(), "%Y-%m-%d").map_err(|_| {
        AppError::invalid_input(format!("{key} must be a date as YYYY-MM-DD; got '{raw}'"))
    })?;
    if day > Timestamp::now().utc_date() {
        return Err(AppError::invalid_input(format!(
            "{key} cannot be in the future; got {day}"
        )));
    }
    Ok(Some(day))
}

/// A stated VDOT's day and method, which describe the call's `vdot`.
pub(super) struct VdotStatement {
    /// The day it was true; the athlete's today when the call gives none.
    pub(super) as_of: Option<CivilDate>,
    /// How it was found.
    pub(super) method: Option<String>,
}

impl VdotStatement {
    /// Parse `vdot_as_of` and `vdot_method`, refusing either without `vdot`,
    /// so no caller believes it dated or labelled a VDOT it did not save.
    pub(super) fn from_args(args: &Value, has_vdot: bool) -> AppResult<Self> {
        let statement = Self {
            as_of: optional_past_day(args, "vdot_as_of")?,
            method: optional_label(args, "vdot_method")?,
        };
        if !has_vdot && (statement.as_of.is_some() || statement.method.is_some()) {
            return Err(AppError::invalid_input(
                "vdot_as_of and vdot_method describe a vdot; pass vdot too",
            ));
        }
        Ok(statement)
    }
}

/// Parse the provenance one call gives its critical-power family values:
/// `None` when the call names none of `measurement_kind`,
/// `measurement_source` or `measured_on`.
///
/// # Errors
/// Returns an invalid-input error when `measurement_source` or `measured_on`
/// arrives without `measurement_kind`, the source is too long, or the date is
/// not a `YYYY-MM-DD` day up to today.
pub(super) fn provenance_from_args(args: &Value) -> AppResult<Option<MetricProvenance>> {
    let kind = args
        .get("measurement_kind")
        .and_then(Value::as_str)
        .map(parse_measurement_kind)
        .transpose()?;
    let origin = optional_label(args, "measurement_source")?;
    let as_of = optional_past_day(args, "measured_on")?;
    match kind {
        Some(kind) => Ok(Some(MetricProvenance { kind, origin, as_of })),
        None if origin.is_some() || as_of.is_some() => Err(AppError::invalid_input(
            "measurement_source and measured_on describe a measurement_kind; pass measurement_kind too",
        )),
        None => Ok(None),
    }
}
