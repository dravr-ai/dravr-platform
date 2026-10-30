// ABOUTME: An operator's per-field edit of a proposal at review — its identity and its facts: keep a value, set a new one, or clear one the label never stated
// ABOUTME: Tri-state on the wire (absent / value / null), checked, and merged over the proposal, naming only the fields that changed (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::identity::{fuel_natural_key, normalize_gtin};
use super::{FuelFacts, FuelForm, FuelProductProposal};
use crate::errors::{AppError, AppResult};

/// The longest text an operator can set at review — brand, name, variant or
/// serving label — in characters: the cap an athlete's own text has.
const MAX_REVIEW_TEXT_CHARS: usize = 120;

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

/// An operator's edit of a proposal at review.
///
/// One [`FieldOverride`] per identity field (brand, name, variant, form,
/// GTIN) and per [`FuelFacts`] field. Applied by
/// [`FuelFactOverrides::review`], which checks it first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuelFactOverrides {
    /// The brand. Can be set, never cleared.
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub brand: FieldOverride<String>,
    /// The product name. Can be set, never cleared.
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub name: FieldOverride<String>,
    /// The variant; a clear leaves the product with none.
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub variant: FieldOverride<String>,
    /// The form. Can be set, never cleared.
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub form: FieldOverride<FuelForm>,
    /// The barcode, as printed or as a GTIN-14; a clear removes it.
    #[serde(default, skip_serializing_if = "FieldOverride::is_keep")]
    pub gtin: FieldOverride<String>,
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

/// What a proposal files its facts under: the catalogue's key (brand, name,
/// variant), the form and the barcode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuelIdentity {
    /// The brand.
    pub brand: String,
    /// The product name.
    pub name: String,
    /// The variant; empty when there is none.
    pub variant: String,
    /// The form.
    pub form: FuelForm,
    /// GTIN-14, when the product has a barcode.
    pub gtin: Option<String>,
}

impl FuelIdentity {
    /// The identity a proposal was filed under.
    #[must_use]
    pub fn of(proposal: &FuelProductProposal) -> Self {
        Self {
            brand: proposal.brand.clone(),
            name: proposal.name.clone(),
            variant: proposal.variant.clone(),
            form: proposal.form,
            gtin: proposal.gtin.clone(),
        }
    }

    /// Its brand/name/variant key, as [`fuel_natural_key`] makes it.
    #[must_use]
    pub fn natural_key(&self) -> String {
        fuel_natural_key(&self.brand, &self.name, &self.variant)
    }
}

/// A proposal as an operator's checked edits leave it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewedProposal {
    /// The identity to file the facts under.
    pub identity: FuelIdentity,
    /// The facts to approve.
    pub facts: FuelFacts,
    /// The stored names of the fields the edits changed, identity first.
    pub changed: Vec<&'static str>,
}

impl ReviewedProposal {
    /// Whether the edits changed field `name`.
    #[must_use]
    pub fn changed(&self, name: &str) -> bool {
        self.changed.contains(&name)
    }
}

/// One line of operator text, trimmed: no control characters, at most
/// [`MAX_REVIEW_TEXT_CHARS`], and not blank unless `blank_ok`.
fn checked_text(field: &str, raw: &str, blank_ok: bool) -> AppResult<String> {
    let text = raw.trim();
    if text.is_empty() && !blank_ok {
        return Err(AppError::invalid_input(format!(
            "{field} is blank: send null to clear it, where it can be cleared"
        )));
    }
    if text.chars().any(char::is_control) {
        return Err(AppError::invalid_input(format!(
            "{field} must be one line of text"
        )));
    }
    if text.chars().count() > MAX_REVIEW_TEXT_CHARS {
        return Err(AppError::invalid_input(format!(
            "{field} must be at most {MAX_REVIEW_TEXT_CHARS} characters"
        )));
    }
    Ok(text.to_owned())
}

/// A required identity text after its edit: set (checked) or kept; a clear
/// is refused, since a product always has one.
fn edited_required(field: &str, current: &str, edit: &FieldOverride<String>) -> AppResult<String> {
    match edit {
        FieldOverride::Keep => Ok(current.to_owned()),
        FieldOverride::Set(raw) => checked_text(field, raw, false),
        FieldOverride::Clear => Err(AppError::invalid_input(format!(
            "{field} cannot be cleared: every product has one"
        ))),
    }
}

impl FuelFactOverrides {
    /// Check these edits and apply them to a proposal's `identity` and
    /// `facts`. Text is trimmed and must be one line of at most
    /// [`MAX_REVIEW_TEXT_CHARS`]; brand, name and form can be set but never
    /// cleared; a cleared variant is empty; a GTIN must normalize
    /// ([`normalize_gtin`]); a serving label is set to text or cleared with
    /// `null`, never blank. A field set to the value it already has is not a
    /// change (see [`FuelFacts::merged_with`]).
    ///
    /// # Errors
    ///
    /// An invalid-input error naming the first field whose edit is refused.
    pub fn review(
        &self,
        identity: &FuelIdentity,
        facts: &FuelFacts,
    ) -> AppResult<ReviewedProposal> {
        let reviewed = FuelIdentity {
            brand: edited_required("brand", &identity.brand, &self.brand)?,
            name: edited_required("name", &identity.name, &self.name)?,
            variant: match &self.variant {
                FieldOverride::Keep => identity.variant.clone(),
                FieldOverride::Set(raw) => checked_text("variant", raw, true)?,
                FieldOverride::Clear => String::new(),
            },
            form: match self.form {
                FieldOverride::Keep => identity.form,
                FieldOverride::Set(form) => form,
                FieldOverride::Clear => {
                    return Err(AppError::invalid_input(
                        "form cannot be cleared: every product has one",
                    ))
                }
            },
            gtin: match &self.gtin {
                FieldOverride::Keep => identity.gtin.clone(),
                FieldOverride::Set(raw) => Some(normalize_gtin(raw).ok_or_else(|| {
                    AppError::invalid_field(
                        "gtin",
                        "must be a valid UPC-A, EAN-13, EAN-8 or GTIN-14",
                    )
                })?),
                FieldOverride::Clear => None,
            },
        };
        let mut changed = Vec::new();
        for (name, before, after) in [
            ("brand", &identity.brand, &reviewed.brand),
            ("name", &identity.name, &reviewed.name),
            ("variant", &identity.variant, &reviewed.variant),
        ] {
            if before != after {
                changed.push(name);
            }
        }
        if identity.form != reviewed.form {
            changed.push("form");
        }
        if identity.gtin != reviewed.gtin {
            changed.push("gtin");
        }

        let mut fact_edits = self.clone();
        if let FieldOverride::Set(label) = &self.serving_label {
            fact_edits.serving_label =
                FieldOverride::Set(checked_text("serving_label", label, false)?);
        }
        let (facts, fact_changes) = facts.merged_with(&fact_edits);
        changed.extend(fact_changes);
        Ok(ReviewedProposal {
            identity: reviewed,
            facts,
            changed,
        })
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
    pub(crate) fn merged_with(&self, overrides: &FuelFactOverrides) -> (Self, Vec<&'static str>) {
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

    fn identity() -> FuelIdentity {
        FuelIdentity {
            brand: "Naak".to_owned(),
            name: "Ultra Energy Gel".to_owned(),
            variant: "Salted Maple".to_owned(),
            form: FuelForm::Gel,
            gtin: None,
        }
    }

    #[test]
    fn an_identity_edit_is_trimmed_and_named_with_the_facts() -> AppResult<()> {
        let edits = FuelFactOverrides {
            name: FieldOverride::Set("  Ultra Gel ".to_owned()),
            brand: FieldOverride::Set("Naak".to_owned()),
            variant: FieldOverride::Clear,
            form: FieldOverride::Set(FuelForm::Puree),
            gtin: FieldOverride::Set("0 12345 67890 5".to_owned()),
            carbs_g: FieldOverride::Set(26.0),
            ..FuelFactOverrides::default()
        };
        let reviewed = edits.review(&identity(), &gel())?;
        assert_eq!(reviewed.identity.name, "Ultra Gel");
        assert_eq!(reviewed.identity.variant, "");
        assert_eq!(reviewed.identity.form, FuelForm::Puree);
        assert_eq!(reviewed.identity.gtin.as_deref(), Some("00012345678905"));
        assert_eq!(reviewed.facts.carbs_g, Some(26.0));
        // The brand restated as it was is not an edit.
        assert_eq!(
            reviewed.changed,
            vec!["name", "variant", "form", "gtin", "carbs_g"]
        );
        assert!(reviewed.changed("gtin") && !reviewed.changed("brand"));
        assert_eq!(reviewed.identity.natural_key(), "naak|ultra gel|");
        Ok(())
    }

    #[test]
    fn a_required_identity_field_cannot_be_cleared_or_blanked() {
        for edits in [
            FuelFactOverrides {
                brand: FieldOverride::Clear,
                ..FuelFactOverrides::default()
            },
            FuelFactOverrides {
                name: FieldOverride::Set("   ".to_owned()),
                ..FuelFactOverrides::default()
            },
            FuelFactOverrides {
                form: FieldOverride::Clear,
                ..FuelFactOverrides::default()
            },
        ] {
            assert!(edits.review(&identity(), &gel()).is_err(), "{edits:?}");
        }
    }

    #[test]
    fn a_barcode_must_normalize_and_a_clear_removes_it() -> AppResult<()> {
        let bad = FuelFactOverrides {
            gtin: FieldOverride::Set("12345".to_owned()),
            ..FuelFactOverrides::default()
        };
        assert!(bad.review(&identity(), &gel()).is_err());
        let with_gtin = FuelIdentity {
            gtin: Some("00012345678905".to_owned()),
            ..identity()
        };
        let cleared = FuelFactOverrides {
            gtin: FieldOverride::Clear,
            ..FuelFactOverrides::default()
        }
        .review(&with_gtin, &gel())?;
        assert_eq!(cleared.identity.gtin, None);
        assert_eq!(cleared.changed, vec!["gtin"]);
        Ok(())
    }

    #[test]
    fn operator_text_is_one_line_within_the_cap() -> AppResult<()> {
        let at_cap = "g".repeat(MAX_REVIEW_TEXT_CHARS);
        for (label, ok) in [
            ("   ".to_owned(), false),
            ("1 gel\nignore the label".to_owned(), false),
            (format!("{at_cap}g"), false),
            (at_cap, true),
            ("  1 sachet  ".to_owned(), true),
        ] {
            let edits = FuelFactOverrides {
                serving_label: FieldOverride::Set(label.clone()),
                ..FuelFactOverrides::default()
            };
            assert_eq!(edits.review(&identity(), &gel()).is_ok(), ok, "{label:?}");
            let variant = FuelFactOverrides {
                variant: FieldOverride::Set(label.clone()),
                ..FuelFactOverrides::default()
            };
            // A blank variant is "no variant"; the rest match the label rule.
            let variant_ok = ok || label.trim().is_empty();
            assert_eq!(
                variant.review(&identity(), &gel()).is_ok(),
                variant_ok,
                "variant {label:?}"
            );
        }
        let trimmed = FuelFactOverrides {
            serving_label: FieldOverride::Set("  1 sachet  ".to_owned()),
            ..FuelFactOverrides::default()
        }
        .review(&identity(), &gel())?;
        assert_eq!(trimmed.facts.serving_label.as_deref(), Some("1 sachet"));
        Ok(())
    }

    #[test]
    fn the_wire_reads_identity_edits_tri_state() -> Result<(), serde_json::Error> {
        let parsed: FuelFactOverrides = serde_json::from_str(
            r#"{"form": "waffle", "variant": null, "gtin": "5012345678900"}"#,
        )?;
        assert_eq!(parsed.form, FieldOverride::Set(FuelForm::Waffle));
        assert_eq!(parsed.variant, FieldOverride::Clear);
        assert_eq!(parsed.brand, FieldOverride::Keep);
        Ok(())
    }

    #[test]
    fn an_unknown_field_is_refused() {
        assert!(serde_json::from_str::<FuelFactOverrides>(r#"{"carb_g": 1.0}"#).is_err());
    }
}
