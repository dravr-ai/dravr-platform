// ABOUTME: An operator's per-field edit of a proposal's facts at review: keep the value, set a new one, or clear one the label never stated
// ABOUTME: Tri-state on the wire (absent / value / null) and merged over the proposed facts, naming only the fields that changed (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::FuelFacts;

/// What an operator's edit does to one fact.
///
/// On the wire a field the edit leaves out keeps its value, a value sets it,
/// and an explicit `null` clears it — so an operator can remove a value OCR
/// read off something that was not the label. A struct field of this type
/// needs `#[serde(default, skip_serializing_if = "FieldOverride::is_keep")]`
/// for the absent case to round-trip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FieldOverride<T> {
    /// Leave the proposed value as it is.
    #[default]
    Keep,
    /// Replace it with this value.
    Set(T),
    /// Remove it: the label does not state it.
    Clear,
}

impl<T> FieldOverride<T> {
    /// Whether this leaves the value as it is.
    #[must_use]
    pub const fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FieldOverride<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Reached only for a key that is present: `#[serde(default)]` covers
        // an absent one, so `null` here is the operator clearing the field.
        Ok(Option::<T>::deserialize(deserializer)?.map_or(Self::Clear, Self::Set))
    }
}

impl<T: Serialize> Serialize for FieldOverride<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Set(value) => serializer.serialize_some(value),
            Self::Keep | Self::Clear => serializer.serialize_none(),
        }
    }
}

/// An operator's edit of every fact a proposal carries, one
/// [`FieldOverride`] per [`FuelFacts`] field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuelFactOverrides {
    /// See [`FuelFacts::serving_label`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub serving_label: FieldOverride<String>,
    /// See [`FuelFacts::serving_g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub serving_g: FieldOverride<f64>,
    /// See [`FuelFacts::energy_kcal`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub energy_kcal: FieldOverride<f64>,
    /// See [`FuelFacts::carbs_g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub carbs_g: FieldOverride<f64>,
    /// See [`FuelFacts::sugars_g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub sugars_g: FieldOverride<f64>,
    /// See [`FuelFacts::protein_g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub protein_g: FieldOverride<f64>,
    /// See [`FuelFacts::fat_g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub fat_g: FieldOverride<f64>,
    /// See [`FuelFacts::sodium_mg`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub sodium_mg: FieldOverride<f64>,
    /// See [`FuelFacts::potassium_mg`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub potassium_mg: FieldOverride<f64>,
    /// See [`FuelFacts::caffeine_mg`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub caffeine_mg: FieldOverride<f64>,
    /// See [`FuelFacts::fluid_ml`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub fluid_ml: FieldOverride<f64>,
    /// See [`FuelFacts::carbs_g_per_100g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub carbs_g_per_100g: FieldOverride<f64>,
    /// See [`FuelFacts::sugars_g_per_100g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub sugars_g_per_100g: FieldOverride<f64>,
    /// See [`FuelFacts::protein_g_per_100g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub protein_g_per_100g: FieldOverride<f64>,
    /// See [`FuelFacts::fat_g_per_100g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub fat_g_per_100g: FieldOverride<f64>,
    /// See [`FuelFacts::sodium_mg_per_100g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub sodium_mg_per_100g: FieldOverride<f64>,
    /// See [`FuelFacts::salt_g_per_100g`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub salt_g_per_100g: FieldOverride<f64>,
    /// See [`FuelFacts::glucose_fructose_ratio`].
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub glucose_fructose_ratio: FieldOverride<f64>,
}

/// Apply one numeric override, recording `name` when the value changed.
/// Compared bit for bit: an override is a typed value, not a computation.
fn apply_number(
    changed: &mut Vec<&'static str>,
    name: &'static str,
    slot: &mut Option<f64>,
    edit: &FieldOverride<f64>,
) {
    let next = match edit {
        FieldOverride::Keep => return,
        FieldOverride::Set(value) => Some(*value),
        FieldOverride::Clear => None,
    };
    if next.map(f64::to_bits) != slot.map(f64::to_bits) {
        *slot = next;
        changed.push(name);
    }
}

impl FuelFacts {
    /// These facts with `overrides` applied, and the stored names of the
    /// fields it changed — an operator's edit before an approval. A field the
    /// edit keeps is left alone, and one it sets to the value already there,
    /// or clears when it is already empty, is not a change: a review form that
    /// sends every field back must not relabel the label's own values as
    /// operator edits.
    #[must_use]
    pub fn merged_with(&self, overrides: &FuelFactOverrides) -> (Self, Vec<&'static str>) {
        let mut merged = self.clone();
        let mut changed = Vec::new();
        let numbers: [(&'static str, &mut Option<f64>, &FieldOverride<f64>); 17] = [
            ("serving_g", &mut merged.serving_g, &overrides.serving_g),
            (
                "energy_kcal",
                &mut merged.energy_kcal,
                &overrides.energy_kcal,
            ),
            ("carbs_g", &mut merged.carbs_g, &overrides.carbs_g),
            ("sugars_g", &mut merged.sugars_g, &overrides.sugars_g),
            ("protein_g", &mut merged.protein_g, &overrides.protein_g),
            ("fat_g", &mut merged.fat_g, &overrides.fat_g),
            ("sodium_mg", &mut merged.sodium_mg, &overrides.sodium_mg),
            (
                "potassium_mg",
                &mut merged.potassium_mg,
                &overrides.potassium_mg,
            ),
            (
                "caffeine_mg",
                &mut merged.caffeine_mg,
                &overrides.caffeine_mg,
            ),
            ("fluid_ml", &mut merged.fluid_ml, &overrides.fluid_ml),
            (
                "carbs_g_per_100g",
                &mut merged.carbs_g_per_100g,
                &overrides.carbs_g_per_100g,
            ),
            (
                "sugars_g_per_100g",
                &mut merged.sugars_g_per_100g,
                &overrides.sugars_g_per_100g,
            ),
            (
                "protein_g_per_100g",
                &mut merged.protein_g_per_100g,
                &overrides.protein_g_per_100g,
            ),
            (
                "fat_g_per_100g",
                &mut merged.fat_g_per_100g,
                &overrides.fat_g_per_100g,
            ),
            (
                "sodium_mg_per_100g",
                &mut merged.sodium_mg_per_100g,
                &overrides.sodium_mg_per_100g,
            ),
            (
                "salt_g_per_100g",
                &mut merged.salt_g_per_100g,
                &overrides.salt_g_per_100g,
            ),
            (
                "glucose_fructose_ratio",
                &mut merged.glucose_fructose_ratio,
                &overrides.glucose_fructose_ratio,
            ),
        ];
        for (name, slot, edit) in numbers {
            apply_number(&mut changed, name, slot, edit);
        }
        let label = match &overrides.serving_label {
            FieldOverride::Keep => None,
            FieldOverride::Set(text) => Some(Some(text.clone())),
            FieldOverride::Clear => Some(None),
        };
        if let Some(label) = label {
            if label != merged.serving_label {
                merged.serving_label = label;
                changed.push("serving_label");
            }
        }
        (merged, changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gel() -> FuelFacts {
        FuelFacts {
            serving_g: Some(40.0),
            carbs_g: Some(25.0),
            sugars_g: Some(12.0),
            ..FuelFacts::default()
        }
    }

    #[test]
    fn overrides_replace_only_what_they_state() {
        let overrides = FuelFactOverrides {
            carbs_g: FieldOverride::Set(26.0),
            serving_label: FieldOverride::Set("1 sachet".to_owned()),
            ..FuelFactOverrides::default()
        };
        let (merged, changed) = gel().merged_with(&overrides);
        assert_eq!(merged.carbs_g, Some(26.0));
        assert_eq!(merged.sugars_g, Some(12.0));
        assert_eq!(merged.serving_label.as_deref(), Some("1 sachet"));
        assert_eq!(changed, vec!["carbs_g", "serving_label"]);
    }

    #[test]
    fn an_override_restating_a_value_is_not_an_edit() {
        let restated = FuelFactOverrides {
            serving_g: FieldOverride::Set(40.0),
            carbs_g: FieldOverride::Set(25.0),
            sugars_g: FieldOverride::Set(11.0),
            ..FuelFactOverrides::default()
        };
        let (merged, changed) = gel().merged_with(&restated);
        assert_eq!(merged.sugars_g, Some(11.0));
        assert_eq!(changed, vec!["sugars_g"]);
    }

    #[test]
    fn a_clear_removes_a_value_and_clearing_nothing_is_not_an_edit() {
        let overrides = FuelFactOverrides {
            sugars_g: FieldOverride::Clear,
            caffeine_mg: FieldOverride::Clear,
            ..FuelFactOverrides::default()
        };
        let (merged, changed) = gel().merged_with(&overrides);
        assert_eq!(merged.sugars_g, None);
        assert_eq!(merged.carbs_g, Some(25.0));
        assert_eq!(changed, vec!["sugars_g"]);
    }

    #[test]
    fn the_wire_reads_absent_as_keep_null_as_clear_and_a_value_as_set(
    ) -> Result<(), serde_json::Error> {
        let parsed: FuelFactOverrides = serde_json::from_str(
            r#"{"carbs_g": 30.5, "sugars_g": null, "serving_label": "1 gel"}"#,
        )?;
        assert_eq!(parsed.carbs_g, FieldOverride::Set(30.5));
        assert_eq!(parsed.sugars_g, FieldOverride::Clear);
        assert_eq!(parsed.serving_label, FieldOverride::Set("1 gel".to_owned()));
        assert_eq!(parsed.fat_g, FieldOverride::Keep);

        let written = serde_json::to_value(&parsed)?;
        assert_eq!(
            written,
            serde_json::json!({"carbs_g": 30.5, "sugars_g": null, "serving_label": "1 gel"})
        );
        Ok(())
    }

    #[test]
    fn an_unknown_field_is_refused() {
        assert!(serde_json::from_str::<FuelFactOverrides>(r#"{"carb_g": 1.0}"#).is_err());
    }
}
