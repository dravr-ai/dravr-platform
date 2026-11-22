// ABOUTME: Pins the JSON Schema each date type declares, which tool output schemas and the SDK carry
// ABOUTME: Recorded from the date engine in use on 2026-11-17: instant, optional instant, date, time, weekday
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Schema goldens for dates (carnet#1620, the schema contract of gate 6).
//!
//! A tool's declared `outputSchema` and the generated SDK types say what a
//! date field looks like: a string, with a `format` for the three families
//! that have one. Clients validate against that text, so it is a contract of
//! its own, separate from the bytes the serializer writes. Schemas are
//! compared key by key and value by value, not as text: the order of a
//! schema's keys follows how the JSON library was built.
//!
//! The first test asks the schema generator about each bare type. The second
//! asks production models that derive their schema, and compares only the
//! date field's own schema, so an unrelated field can change without touching
//! this file.
//!
//! Everything that names the date engine lives in [`engine`].

use pierre_core::models::measurement::AnchorStamp;
use pierre_core::models::refresh::ProviderFreshness;
use pierre_core::models::training_history::DailyTrainingState;
use schemars::{schema_for, JsonSchema};
use serde_json::Value;

/// Everything in this file that names the date engine.
mod engine {
    use dravr_tronc::dates::{CivilDate, CivilDateTime, CivilTime, Timestamp, Weekday};
    use schemars::schema_for;

    /// The schema of each bare date type, as a JSON value, by family name.
    pub(super) fn schemas() -> Vec<(&'static str, serde_json::Value)> {
        vec![
            ("instant", schema_for!(Timestamp)),
            ("optional instant", schema_for!(Option<Timestamp>)),
            ("calendar date", schema_for!(CivilDate)),
            ("optional calendar date", schema_for!(Option<CivilDate>)),
            ("time of day", schema_for!(CivilTime)),
            ("zoneless date-time", schema_for!(CivilDateTime)),
            ("weekday", schema_for!(Weekday)),
        ]
        .into_iter()
        .map(|(family, schema)| (family, serde_json::to_value(&schema).unwrap()))
        .collect()
    }
}

#[test]
fn each_date_type_declares_this_schema() {
    let expected = [
        (
            "instant",
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Timestamp","type":"string","format":"date-time"}"#,
        ),
        (
            "optional instant",
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Nullable_Timestamp","type":["string","null"],"format":"date-time"}"#,
        ),
        (
            "calendar date",
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"CivilDate","type":"string","format":"date"}"#,
        ),
        (
            "optional calendar date",
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Nullable_CivilDate","type":["string","null"],"format":"date"}"#,
        ),
        (
            "time of day",
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"CivilTime","type":"string","format":"partial-time"}"#,
        ),
        (
            "zoneless date-time",
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"CivilDateTime","type":"string","format":"partial-date-time"}"#,
        ),
        (
            "weekday",
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Weekday","type":"string","enum":["Mon","Tue","Wed","Thu","Fri","Sat","Sun"]}"#,
        ),
    ];
    assert_eq!(
        engine::schemas(),
        as_values(&expected),
        "the schema each bare date type declares"
    );
}

/// The expected schemas as JSON values: every key and every value is
/// compared, the order of the keys is not, because it follows how the JSON
/// library was built.
fn as_values<'a>(expected: &[(&'a str, &str)]) -> Vec<(&'a str, Value)> {
    expected
        .iter()
        .map(|(name, schema)| (*name, serde_json::from_str(schema).unwrap()))
        .collect()
}

/// The schema one field of a production model declares, without its prose
/// description.
fn field_schema<T: JsonSchema>(field: &str) -> Value {
    let mut schema = serde_json::to_value(schema_for!(T)).unwrap();
    let declared = schema["properties"][field].as_object_mut().unwrap();
    declared.remove("description");
    Value::Object(declared.clone())
}

#[test]
fn production_models_declare_their_date_fields_the_same_way() {
    let expected = [
        (
            "ProviderFreshness.last_sync_at",
            r#"{"format":"date-time","type":["string","null"]}"#,
        ),
        (
            "AnchorStamp.as_of",
            r#"{"format":"date","type":["string","null"]}"#,
        ),
        (
            "DailyTrainingState.date",
            r#"{"format":"date","type":"string"}"#,
        ),
    ];
    let actual = vec![
        (
            "ProviderFreshness.last_sync_at",
            field_schema::<ProviderFreshness>("last_sync_at"),
        ),
        ("AnchorStamp.as_of", field_schema::<AnchorStamp>("as_of")),
        (
            "DailyTrainingState.date",
            field_schema::<DailyTrainingState>("date"),
        ),
    ];
    assert_eq!(
        actual,
        as_values(&expected),
        "the schema of each production date field"
    );
}
