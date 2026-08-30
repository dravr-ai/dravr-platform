// ABOUTME: The schema save_training_plan advertises — outline, weeks, days, steps — and the rejection skeleton generated from it
// ABOUTME: One source for both: a payload that fails to deserialize is answered with the exact shape the deserializer accepts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Save-payload schema
//!
//! `save_training_plan` is the one tool in the registry with nested
//! parameters, and the 2026-07-27 outage was the model guessing that nesting
//! wrong 24 times out of 24. Everything here exists so the advertised shape
//! and the accepted shape cannot drift: the schema is built once from named
//! functions, the tool advertises it, and [`parse_payload_part`] answers a
//! mismatch with a skeleton rendered from that same schema.

use std::collections::HashMap;

use pierre_mcp_schema::PropertySchema;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::calendar::step_schema;

/// String schema property with a description.
pub(super) fn string_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// The `athlete` property both plan tools advertise: the coached athlete whose
/// plan the call acts on. One definition so the two tools cannot describe the
/// same argument two ways.
pub(super) fn athlete_prop() -> PropertySchema {
    string_prop(
        "Roster display name of the athlete whose plan this is. Only the group's human coach \
         (attached via a coach invite) may set it, for a consenting athlete in a group they \
         coach, and only from a direct chat — never in a room. Omit to act on your own plan.",
    )
}

/// Number schema property with a description.
fn number_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "number".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// Integer schema property with a description. Whole-valued floats are
/// still accepted at deserialization (LLM callers emit 60.0 for integers).
fn integer_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "integer".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// Object schema property with nested fields.
fn object_prop(
    description: &str,
    properties: HashMap<String, PropertySchema>,
    required: Vec<String>,
) -> PropertySchema {
    PropertySchema {
        property_type: "object".to_owned(),
        description: Some(description.to_owned()),
        properties: Some(properties),
        required: Some(required),
        ..Default::default()
    }
}

/// Array schema property with an item schema.
fn array_prop(description: &str, items: PropertySchema) -> PropertySchema {
    PropertySchema {
        property_type: "array".to_owned(),
        description: Some(description.to_owned()),
        items: Some(Box::new(items)),
        ..Default::default()
    }
}

/// Schema for one race entry (goal or secondary).
fn race_schema(description: &str) -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "name".to_owned(),
        string_prop("Race name as the athlete calls it."),
    );
    p.insert("date".to_owned(), string_prop("Race date, YYYY-MM-DD."));
    p.insert(
        "discipline".to_owned(),
        string_prop("Discipline: gravel, xco, road, trail run, …"),
    );
    p.insert(
        "priority".to_owned(),
        string_prop("A = goal race, B = tune-up, C = training race."),
    );
    object_prop(
        description,
        p,
        vec![
            "name".to_owned(),
            "date".to_owned(),
            "discipline".to_owned(),
            "priority".to_owned(),
        ],
    )
}

/// Schema for one outline block.
fn block_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "phase".to_owned(),
        string_prop("One of: rest | base | build | peak | taper."),
    );
    p.insert(
        "start".to_owned(),
        string_prop("Block start date, YYYY-MM-DD."),
    );
    p.insert("weeks".to_owned(), integer_prop("Block length in weeks."));
    p.insert(
        "intent".to_owned(),
        string_prop("What this block is for, in coach voice."),
    );
    p.insert(
        "target_hours".to_owned(),
        number_prop("Optional target weekly volume in hours."),
    );
    object_prop(
        "One training block (mesocycle).",
        p,
        vec![
            "phase".to_owned(),
            "start".to_owned(),
            "weeks".to_owned(),
            "intent".to_owned(),
        ],
    )
}

/// Schema for one planned day.
/// Schema for a day's fuelling prescription.
///
/// Mirrors `$defs.FuelingProtocol` in the structured-workout schema the
/// builder coaches already emit against, minus the required sodium: a coach
/// with no sweat estimate should omit it rather than invent one.
fn fueling_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "carbs_g_per_h".to_owned(),
        number_prop(
            "Carbohydrate target in grams per hour. Up to 60 g/h from any single source; above that requires multiple transportable carbohydrates, so state carb_source too.",
        ),
    );
    p.insert(
        "fluid_ml_per_h".to_owned(),
        number_prop("Fluid target in millilitres per hour."),
    );
    p.insert(
        "sodium_mg_per_h".to_owned(),
        number_prop(
            "ESTIMATED sodium LOSS in mg per hour, not a required intake. Give it only when the athlete has a sweat measurement behind it; omit it otherwise rather than guessing.",
        ),
    );
    p.insert(
        "carb_source".to_owned(),
        string_prop(
            "Carbohydrate source when the rate depends on it — 'glucose:fructose 1:0.8'. Required in practice for any rate above 60 g/h.",
        ),
    );
    object_prop(
        "What to take in during the session. Give it for any session long enough to need fuelling.",
        p,
        vec!["carbs_g_per_h".to_owned(), "fluid_ml_per_h".to_owned()],
    )
}

fn day_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert("date".to_owned(), string_prop("Day date, YYYY-MM-DD."));
    p.insert(
        "sport".to_owned(),
        string_prop("Sport (mtb, gravel, run, …) or 'rest'."),
    );
    p.insert(
        "workout".to_owned(),
        string_prop("What to do, in coach voice."),
    );
    p.insert(
        "duration_min".to_owned(),
        integer_prop(
            "Planned duration in minutes; omit for rest days, and omit when steps are given — it is summed from them.",
        ),
    );
    p.insert(
        "intensity".to_owned(),
        string_prop(
            "Intensity RELATIVE to thresholds ('Z2', 'tempo', '88-93% FTP'). Never absolute watts. With steps, the day's summary label.",
        ),
    );
    p.insert(
        "steps".to_owned(),
        array_prop(
            "The session's steps, in order — the same shape as prescribe_workout's session.structure. Give them for any day with interval structure (warm-up, work, recovery, cool-down; repeat on the work and recovery steps): that is what puts workout-builder steps and a planned load on the athlete's calendar. Prose alone reaches the calendar as a timed entry. Omit for a steady or unstructured day.",
            step_schema(),
        ),
    );
    p.insert("fueling".to_owned(), fueling_schema());
    object_prop(
        "One prescribed day.",
        p,
        vec!["date".to_owned(), "sport".to_owned(), "workout".to_owned()],
    )
}

/// Schema for the outline half of the save payload.
///
/// A named function rather than an inline block so the rejection skeleton in
/// [`shape_hint`] is generated from the very schema the tool advertises. A
/// hand-written copy of the expected shape would be free to drift from what
/// the deserializer actually accepts, which is the whole defect being fixed.
pub(super) fn outline_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "goal_race".to_owned(),
        race_schema("The goal (A) race this plan builds toward."),
    );
    p.insert(
        "races".to_owned(),
        array_prop(
            "Other races on the calendar (B/C priorities).",
            race_schema("A secondary race."),
        ),
    );
    p.insert(
        "strategy".to_owned(),
        string_prop(
            "The coach's strategy in prose — what the athlete sees as the long-term direction.",
        ),
    );
    p.insert(
        "blocks".to_owned(),
        array_prop(
            "Ordered training blocks from now to the goal race. Omit for a short plan that has no mesocycle structure.",
            block_schema(),
        ),
    );
    // `blocks` is deliberately absent from the required list: a two-week
    // "hold form then taper" plan has no mesocycle structure, and demanding
    // one forced the coach to invent phase/start/weeks/intent before any plan
    // could be saved at all. The outline still needs a race to aim at and a
    // stated strategy.
    object_prop(
        "The plan outline (goal race + strategy, optionally blocks). Required when creating a plan; omit to adjust weeks of the existing active plan. Re-sending an outline supersedes the athlete's current plan.",
        p,
        vec!["goal_race".to_owned(), "strategy".to_owned()],
    )
}

/// Schema for the `weeks` half of the save payload.
pub(super) fn weeks_schema() -> PropertySchema {
    array_prop(
        "Day-by-day weeks to save. Send the full multi-week detail when the athlete asks to see the whole plan; send a single adjusted week for 'move Tuesday to Wednesday' changes.",
        week_schema(),
    )
}

/// A shape-correct JSON skeleton for `prop`, each leaf carrying its own
/// description as the placeholder value.
///
/// Handed back when a payload fails to deserialize so the model's next
/// iteration can see the exact field names it got wrong — the tool loop runs
/// another LLM turn after a tool error, so an actionable rejection can convert
/// into a successful save within the same turn. Using the description as the
/// placeholder keeps the format hints ("Race date, YYYY-MM-DD.") in the hint
/// itself rather than stripping them to an empty string.
fn shape_hint(prop: &PropertySchema) -> Value {
    match prop.property_type.as_str() {
        "object" => {
            let mut map = serde_json::Map::new();
            if let Some(props) = prop.properties.as_ref() {
                for (name, child) in props {
                    map.insert(name.clone(), shape_hint(child));
                }
            }
            Value::Object(map)
        }
        "array" => prop
            .items
            .as_ref()
            .map_or_else(|| json!([]), |item| json!([shape_hint(item)])),
        "integer" => json!(0),
        "number" => json!(0.0),
        "boolean" => json!(false),
        _ => Value::String(prop.description.clone().unwrap_or_default()),
    }
}

/// Deserialize one half of the save payload, turning a shape mismatch into a
/// message that carries the schema the tool actually accepts.
///
/// `Ok(None)` when the field was simply not sent — both halves are optional on
/// their own, and "nothing to save" is caught separately.
pub(super) fn parse_payload_part<T: DeserializeOwned>(
    raw: Option<&Value>,
    field: &str,
    schema: &PropertySchema,
) -> Result<Option<T>, String> {
    let Some(value) = raw.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|e| {
            format!(
                "`{field}` does not match the schema: {e}. Send exactly this shape \
                 (each value below describes the field): {}",
                shape_hint(schema)
            )
        })
}

/// Schema for one week entry in the save payload.
fn week_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "week_start".to_owned(),
        string_prop("Date of the week's first day, YYYY-MM-DD."),
    );
    p.insert(
        "focus".to_owned(),
        string_prop("The week's intent in one line."),
    );
    p.insert(
        "days".to_owned(),
        array_prop("The day rows, in date order (max 7).", day_schema()),
    );
    p.insert(
        "adjustment_reason".to_owned(),
        string_prop("Why this week is being re-saved; omit on first save."),
    );
    object_prop(
        "One week of day-by-day prescriptions. Re-saving a week_start supersedes the previous version (prospective adjustment).",
        p,
        vec!["week_start".to_owned(), "days".to_owned()],
    )
}
