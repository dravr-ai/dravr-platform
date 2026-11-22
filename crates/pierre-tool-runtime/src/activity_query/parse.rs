// ABOUTME: The argument grammar of a count — window, filters, breakdown and metrics — and the echo of what was applied
// ABOUTME: An unknown sport, key or metric is refused by name, never ignored; athlete text is echoed only as a display line

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Parsing a count's arguments, and echoing them.

use dravr_tronc::dates::CivilDate;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{resolve_sport_type, ActivityTag, SportFamily, SportType};
use pierre_core::text_fold::fold_for_match;
use pierre_core::untrusted::display_line;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::accumulator::{GroupKey, QueryMetric, DEFAULT_METRICS};
use super::predicate::{ActivityPredicate, SegmentMatch};
use super::ActivityQuery;

/// Most keys a count is broken down by: two make a cross-tab.
const MAX_GROUP_KEYS: usize = 2;

/// Longest free-text filter accepted (`name_contains`, `has_segment`,
/// `gear`), and longest echoed text.
const MAX_TEXT_CHARS: usize = 60;

/// Longest unknown value a refusal echoes.
const MAX_REFUSED_ECHO_CHARS: usize = 40;

fn invalid(message: String) -> AppError {
    AppError::invalid_input(message)
}

/// A non-empty, trimmed string argument.
fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// A free-text filter: [`text_arg`], refused past [`MAX_TEXT_CHARS`].
fn capped_text_arg<'a>(args: &'a Value, key: &str) -> AppResult<Option<&'a str>> {
    match text_arg(args, key) {
        Some(raw) if raw.chars().count() > MAX_TEXT_CHARS => Err(invalid(format!(
            "{key} must hold 1 to {MAX_TEXT_CHARS} characters to match"
        ))),
        text => Ok(text),
    }
}

fn date_arg(args: &Value, key: &str) -> AppResult<Option<CivilDate>> {
    text_arg(args, key)
        .map(|raw| {
            CivilDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|e| invalid(format!("{key} must be YYYY-MM-DD: {e}")))
        })
        .transpose()
}

/// A list argument: an array of strings, or one string.
fn list_arg(args: &Value, key: &str) -> AppResult<Vec<String>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(one)) => Ok(vec![one.trim().to_owned()]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(|text| text.trim().to_owned())
                    .ok_or_else(|| invalid(format!("{key} must list strings")))
            })
            .collect(),
        Some(_) => Err(invalid(format!("{key} must be a list of strings"))),
    }
}

/// A list of one of the wire enums, each value named on refusal.
fn enum_list<T: for<'de> Deserialize<'de>>(args: &Value, key: &str) -> AppResult<Vec<T>> {
    list_arg(args, key)?
        .into_iter()
        .map(|raw| {
            serde_json::from_value(Value::String(raw.clone())).map_err(|_| {
                invalid(format!(
                    "{key}: unknown value '{}'",
                    display_line(&raw, MAX_REFUSED_ECHO_CHARS)
                ))
            })
        })
        .collect()
}

/// Every tag a filter accepts: the vocabulary minus `Unknown`, which only a
/// newer release's stored value reads as. The one list the refusal, the
/// tool description and the output schema all name.
const ASKABLE_TAGS: [ActivityTag; 9] = [
    ActivityTag::Race,
    ActivityTag::LongRun,
    ActivityTag::Workout,
    ActivityTag::Commute,
    ActivityTag::Recovery,
    ActivityTag::WithPet,
    ActivityTag::ForACause,
    ActivityTag::Competition,
    ActivityTag::WithKid,
];

/// The wire names of [`ASKABLE_TAGS`].
pub(crate) fn askable_tag_names() -> Vec<String> {
    ASKABLE_TAGS
        .iter()
        .filter_map(|tag| match serde_json::to_value(tag) {
            Ok(Value::String(name)) => Some(name),
            _ => None,
        })
        .collect()
}

/// The `workout_type` output schema: the accepted names, not the
/// vocabulary's rustdoc and its read-only `unknown`.
fn askable_tags_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "description": "Workout tags.",
        "type": "array",
        "items": { "type": "string", "enum": askable_tag_names() },
    })
}

/// The `workout_type` tags asked. cageux's `ActivityTag` reads a value it
/// cannot name as `Unknown` so a stored activity never fails to load; a
/// filter must refuse it instead of silently matching nothing.
fn tag_list(args: &Value) -> AppResult<Vec<ActivityTag>> {
    list_arg(args, "workout_type")?
        .into_iter()
        .map(
            |raw| match serde_json::from_value::<ActivityTag>(Value::String(raw.clone())) {
                Ok(tag) if tag != ActivityTag::Unknown => Ok(tag),
                _ => Err(invalid(format!(
                    "workout_type: unknown value '{}'",
                    display_line(&raw, MAX_REFUSED_ECHO_CHARS)
                ))),
            },
        )
        .collect()
}

fn distance_arg(args: &Value, key: &str) -> AppResult<Option<f64>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|km| km.is_finite() && *km >= 0.0)
            .map(|km| Some(km * 1000.0))
            .ok_or_else(|| invalid(format!("{key} must be a number of km, 0 or more"))),
    }
}

/// A segment asked by id (digits) or by part of its name.
fn segment_arg(args: &Value) -> AppResult<Option<SegmentMatch>> {
    if let Some(id) = args.get("has_segment").and_then(Value::as_u64) {
        return Ok(Some(SegmentMatch::Id(id.to_string())));
    }
    let Some(raw) = capped_text_arg(args, "has_segment")? else {
        return Ok(None);
    };
    if raw.chars().all(|c| c.is_ascii_digit()) {
        return Ok(Some(SegmentMatch::Id(raw.to_owned())));
    }
    let folded = fold_for_match(raw);
    if folded.is_empty() {
        return Err(invalid(
            "has_segment holds no letters to match; pass a segment id or name".to_owned(),
        ));
    }
    Ok(Some(SegmentMatch::Name(folded)))
}

fn sport_types(args: &Value) -> AppResult<Vec<SportType>> {
    list_arg(args, "sport_type")?
        .into_iter()
        .map(|raw| {
            resolve_sport_type(&raw).ok_or_else(|| {
                invalid(format!(
                    "sport_type: unknown sport '{}'",
                    display_line(&raw, MAX_REFUSED_ECHO_CHARS)
                ))
            })
        })
        .collect()
}

fn name_arg(args: &Value) -> AppResult<Option<String>> {
    let Some(raw) = capped_text_arg(args, "name_contains")? else {
        return Ok(None);
    };
    if fold_for_match(raw).is_empty() {
        return Err(invalid(format!(
            "name_contains must hold 1 to {MAX_TEXT_CHARS} characters to match"
        )));
    }
    Ok(Some(raw.to_owned()))
}

fn group_by(args: &Value) -> AppResult<Vec<GroupKey>> {
    let keys: Vec<GroupKey> = enum_list(args, "group_by")?;
    if keys.len() > MAX_GROUP_KEYS {
        return Err(invalid(format!(
            "group_by takes at most {MAX_GROUP_KEYS} keys"
        )));
    }
    if keys.len() == MAX_GROUP_KEYS && keys[0] == keys[1] {
        return Err(invalid("group_by names one key twice".to_owned()));
    }
    Ok(keys)
}

fn trainer_arg(args: &Value) -> AppResult<Option<bool>> {
    match args.get("trainer") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(on)) => Ok(Some(*on)),
        Some(_) => Err(invalid("trainer must be true or false".to_owned())),
    }
}

/// Parse a count's arguments, every one optional.
///
/// The window (`to` defaults to the athlete's `today`; no `from` reads the whole stored
/// history), the filters, `provider`, `group_by` (two keys at most) and
/// `metrics` (the default four when none). `format` is the tool's own.
///
/// # Errors
///
/// Refuses an argument it cannot read, naming it: a malformed date, `from`
/// after `to`, a least distance above the most, an unknown sport, family,
/// tag, key or metric, more than two keys, an empty name to match, or a
/// name, segment or gear text longer than 60 characters.
pub fn parse_query(args: &Value, today: CivilDate) -> AppResult<ActivityQuery> {
    let to = date_arg(args, "to")?.unwrap_or(today);
    let from = date_arg(args, "from")?;
    if from.is_some_and(|from| from > to) {
        return Err(invalid("from must not be after to".to_owned()));
    }
    let min_distance_m = distance_arg(args, "min_distance_km")?;
    let max_distance_m = distance_arg(args, "max_distance_km")?;
    // An inverted range matches nothing and would answer a false zero.
    if let (Some(least), Some(most)) = (min_distance_m, max_distance_m) {
        if least > most {
            return Err(invalid(
                "min_distance_km must not be above max_distance_km".to_owned(),
            ));
        }
    }
    let metrics: Vec<QueryMetric> = enum_list(args, "metrics")?;
    Ok(ActivityQuery {
        from,
        to,
        filter: ActivityPredicate {
            sport_types: sport_types(args)?,
            sport_families: enum_list(args, "sport_family")?,
            place_ids: list_arg(args, "place_id")?,
            name_contains: name_arg(args)?,
            min_distance_m,
            max_distance_m,
            has_segment: segment_arg(args)?,
            gear: capped_text_arg(args, "gear")?.map(str::to_owned),
            workout_type: tag_list(args)?,
            trainer: trainer_arg(args)?,
        },
        group_by: group_by(args)?,
        metrics: if metrics.is_empty() {
            DEFAULT_METRICS.to_vec()
        } else {
            metrics
        },
        provider: text_arg(args, "provider").map(str::to_ascii_lowercase),
    })
}

/// The filters, breakdown and metrics a count ran, after aliases and
/// defaults: what a corrected call is compared against. Athlete text is
/// echoed as one display line.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AppliedQuery {
    /// First day, `YYYY-MM-DD`; absent for the whole stored history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<CivilDate>,
    /// Last day, `YYYY-MM-DD`, included.
    pub to: CivilDate,
    /// Sports, resolved from their aliases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sport_type: Vec<SportType>,
    /// Sport families.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sport_family: Vec<SportFamily>,
    /// Place ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub place_id: Vec<String>,
    /// Text the activity name holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_contains: Option<String>,
    /// Least distance, km.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_distance_km: Option<f64>,
    /// Most distance, km.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_distance_km: Option<f64>,
    /// The segment: its id, or the folded text its name holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_segment: Option<String>,
    /// Gear id or text its name holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<String>,
    /// Workout tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(schema_with = "askable_tags_schema")]
    pub workout_type: Vec<ActivityTag>,
    /// The trainer flag asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trainer: Option<bool>,
    /// The one source counted; `null` for every source.
    pub provider: Option<String>,
    /// What the count is broken down by.
    pub group_by: Vec<GroupKey>,
    /// What it sums.
    pub metrics: Vec<QueryMetric>,
}

impl AppliedQuery {
    /// The echo of `query`.
    #[must_use]
    pub fn of(query: &ActivityQuery) -> Self {
        let echo = |text: &str| display_line(text, MAX_TEXT_CHARS);
        let filter = &query.filter;
        Self {
            from: query.from,
            to: query.to,
            sport_type: filter.sport_types.clone(),
            sport_family: filter.sport_families.clone(),
            place_id: filter.place_ids.iter().map(|id| echo(id)).collect(),
            name_contains: filter.name_contains.as_deref().map(echo),
            min_distance_km: filter.min_distance_m.map(|m| m / 1000.0),
            max_distance_km: filter.max_distance_m.map(|m| m / 1000.0),
            has_segment: filter.has_segment.as_ref().map(|segment| match segment {
                SegmentMatch::Id(id) | SegmentMatch::Name(id) => echo(id),
            }),
            gear: filter.gear.as_deref().map(echo),
            workout_type: filter.workout_type.clone(),
            trainer: filter.trainer,
            provider: query.provider.as_deref().map(echo),
            group_by: query.group_by.clone(),
            metrics: query.metrics.clone(),
        }
    }
}

#[cfg(test)]
mod tag_tests {
    use super::*;

    #[test]
    fn the_askable_tags_are_the_vocabulary_minus_unknown() {
        let schema = schemars::schema_for!(ActivityTag);
        let mut vocabulary: Vec<String> = schema
            .get("oneOf")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|variant| variant.get("const").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        vocabulary.retain(|name| name != "unknown");
        let mut askable = askable_tag_names();
        vocabulary.sort();
        askable.sort();
        assert_eq!(
            askable, vocabulary,
            "bug: ASKABLE_TAGS drifted from ActivityTag's variants"
        );
    }

    #[test]
    fn a_refused_tag_is_echoed() {
        let refused = tag_list(&serde_json::json!({ "workout_type": ["tempo"] }));
        let message = refused.err().map_or_default(|e| e.to_string());
        assert!(
            message.contains("'tempo'"),
            "bug: the refusal does not name the value: {message}"
        );
    }
}
