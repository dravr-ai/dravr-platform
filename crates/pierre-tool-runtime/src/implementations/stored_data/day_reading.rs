// ABOUTME: A stored day as the recovery and health tools answer it: every field but `recorded_at`
// ABOUTME: The store keeps no reading time for a day, only when it first wrote the row (carnet#1629)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A stored day, without the time it does not have
//!
//! dravr-equilibre's `StoredRecoveryMetrics`, `StoredHealthMetrics` and
//! `StoredDailyActivity` each carry a required `recorded_at`. The store keys
//! those rows by day and keeps no reading time: what it hands back as
//! `recorded_at` is the row's first write (`created_at`), so a day first
//! synced at 18:21 would read as "recorded at 18:21" and keep that stamp
//! however often it is updated. [`DayReading`] answers the day with that
//! field left out, in both its serialized form and its declared schema; the
//! `date` and the tool's `last_synced_at` are what a reader goes by.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Serialize, Serializer};
use serde_json::Value;

/// The field a stored day carries and a tool does not show.
const RECORDED_AT: &str = "recorded_at";

/// One stored day (merged across sources) as a tool answers it: every
/// field of `T` but `recorded_at`.
#[derive(Debug, Clone)]
pub struct DayReading<T>(pub T);

impl<T: Serialize> Serialize for DayReading<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error as _;

        let mut day = serde_json::to_value(&self.0).map_err(S::Error::custom)?;
        if let Value::Object(fields) = &mut day {
            fields.shift_remove(RECORDED_AT);
        }
        day.serialize(serializer)
    }
}

impl<T: JsonSchema> JsonSchema for DayReading<T> {
    fn schema_name() -> Cow<'static, str> {
        T::schema_name()
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Owned(format!(
            "{}::DayReading<{}>",
            module_path!(),
            T::schema_id()
        ))
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = T::json_schema(generator);
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            properties.shift_remove(RECORDED_AT);
        }
        if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|field| field.as_str() != Some(RECORDED_AT));
        }
        schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversions;
    use dravr_tronc::dates::{CivilDate, Timestamp};
    use pierre_core::models::StoredDailyActivity;
    use serde_json::json;

    fn day() -> StoredDailyActivity {
        StoredDailyActivity {
            id: "day-1".to_owned(),
            user_id: "user-1".to_owned(),
            data_source_id: "ds-1".to_owned(),
            date: CivilDate::from_ymd(2026, 8, 28).ok().unwrap_or_default(),
            steps: Some(11_284),
            active_kcal: None,
            total_kcal: None,
            intensity_minutes: None,
            source_name: "garmin".to_owned(),
            recorded_at: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_day_is_answered_without_recorded_at_and_keeps_its_date() {
        let stored = serde_json::to_value(day()).unwrap_or_default();
        assert!(
            stored.get(RECORDED_AT).is_some(),
            "fixture bug: the stored day must carry recorded_at to be left out: {stored}"
        );

        let shown = serde_json::to_value(DayReading(day())).unwrap_or_default();
        assert_eq!(
            shown.get(RECORDED_AT),
            None,
            "bug: a day is answered with recorded_at: {shown}"
        );
        assert_eq!(
            shown["date"],
            json!("2026-08-28"),
            "bug: the day lost its date: {shown}"
        );
        assert_eq!(
            shown["steps"],
            json!(11_284),
            "bug: the day lost its readings: {shown}"
        );
        let mut expected = stored;
        if let Value::Object(fields) = &mut expected {
            fields.shift_remove(RECORDED_AT);
        }
        assert_eq!(
            shown, expected,
            "bug: a field other than recorded_at was left out"
        );
    }

    #[test]
    fn the_declared_schema_neither_names_nor_requires_recorded_at() {
        let stored = conversions::output_schema_for::<StoredDailyActivity>();
        assert!(
            stored["properties"].get(RECORDED_AT).is_some(),
            "fixture bug: the stored schema must declare recorded_at to be left out: {stored}"
        );

        let shown = conversions::output_schema_for::<DayReading<StoredDailyActivity>>();
        assert_eq!(
            shown["properties"].get(RECORDED_AT),
            None,
            "bug: the schema declares recorded_at: {shown}"
        );
        assert!(
            shown["properties"].get("date").is_some(),
            "bug: the schema lost date: {shown}"
        );
        let required = shown["required"].as_array().cloned().unwrap_or_default();
        assert!(
            !required.contains(&json!(RECORDED_AT)),
            "bug: the schema requires recorded_at: {shown}"
        );
        assert!(
            required.contains(&json!("date")),
            "bug: the schema no longer requires date: {shown}"
        );
    }
}
