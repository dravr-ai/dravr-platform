// ABOUTME: The fuel-product plausibility gate: whether a label's facts can be what it states, every violation named
// ABOUTME: It rejects and never clamps; sugars, serving mass, per-100 g sums, salt vs sodium and per-form bounds (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{FuelFacts, FuelForm};

/// The numeric fields of [`FuelFacts`], by their stored name.
type NumericField = (&'static str, fn(&FuelFacts) -> Option<f64>);

/// Every numeric field, in stored-name order of the version columns.
const NUMERIC_FIELDS: [NumericField; 17] = [
    ("serving_g", |f| f.serving_g),
    ("energy_kcal", |f| f.energy_kcal),
    ("carbs_g", |f| f.carbs_g),
    ("sugars_g", |f| f.sugars_g),
    ("protein_g", |f| f.protein_g),
    ("fat_g", |f| f.fat_g),
    ("sodium_mg", |f| f.sodium_mg),
    ("potassium_mg", |f| f.potassium_mg),
    ("caffeine_mg", |f| f.caffeine_mg),
    ("fluid_ml", |f| f.fluid_ml),
    ("carbs_g_per_100g", |f| f.carbs_g_per_100g),
    ("sugars_g_per_100g", |f| f.sugars_g_per_100g),
    ("protein_g_per_100g", |f| f.protein_g_per_100g),
    ("fat_g_per_100g", |f| f.fat_g_per_100g),
    ("sodium_mg_per_100g", |f| f.sodium_mg_per_100g),
    ("salt_g_per_100g", |f| f.salt_g_per_100g),
    ("glucose_fructose_ratio", |f| f.glucose_fructose_ratio),
];

/// Whether a fact is stated per serving or per 100 g.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactBasis {
    /// Per serving.
    PerServing,
    /// Per 100 g.
    Per100g,
}

/// One reason a fact set cannot be what a label states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum PlausibilityViolation {
    /// A value is NaN or infinite.
    NotFinite {
        /// The stored field name.
        field: String,
    },
    /// A value is negative.
    Negative {
        /// The stored field name.
        field: String,
        /// The value.
        value: f64,
    },
    /// A product carried for its carbohydrate states no carbs per serving.
    CarbsMissing,
    /// Sugars exceed the carbohydrate they are part of.
    SugarsExceedCarbs {
        /// Per serving or per 100 g.
        basis: FactBasis,
        /// The sugars.
        sugars_g: f64,
        /// The carbohydrate.
        carbs_g: f64,
    },
    /// One macronutrient per serving weighs more than the serving.
    ExceedsServingMass {
        /// The stored field name.
        field: String,
        /// Its grams.
        value_g: f64,
        /// The serving's grams.
        serving_g: f64,
    },
    /// The per-100 g masses add up to more than 100 g.
    Per100gOver100 {
        /// carbs + protein + fat + sodium, grams.
        total_g: f64,
    },
    /// Salt is not sodium × 2.5.
    SaltSodiumMismatch {
        /// Stated salt per 100 g, grams.
        salt_g_per_100g: f64,
        /// Salt the stated sodium implies, grams.
        expected_salt_g: f64,
    },
    /// A per-serving value does not match its per-100 g value at the stated
    /// serving mass.
    ServingMismatch {
        /// The per-serving field name.
        field: String,
        /// The stated per-serving value.
        per_serving: f64,
        /// What per-100 g × `serving_g` / 100 gives.
        expected: f64,
    },
    /// The serving mass is outside what the form allows.
    ServingMassOutOfRange {
        /// The stated mass, grams.
        serving_g: f64,
        /// The smallest the form allows.
        min_g: f64,
        /// The largest the form allows.
        max_g: f64,
    },
    /// More sodium per serving than any fuel product carries.
    SodiumTooHigh {
        /// Stated sodium per serving, mg.
        sodium_mg: f64,
    },
    /// More caffeine per serving than a single dose should carry.
    CaffeineTooHigh {
        /// Stated caffeine per serving, mg.
        caffeine_mg: f64,
    },
    /// The glucose:fructose ratio is outside 0..=1.5.
    RatioOutOfRange {
        /// The stated ratio.
        ratio: f64,
    },
}

impl PlausibilityViolation {
    /// The stable code a proposal's gate flags and the review console carry.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFinite { .. } => "not_finite",
            Self::Negative { .. } => "negative",
            Self::CarbsMissing => "carbs_missing",
            Self::SugarsExceedCarbs { .. } => "sugars_exceed_carbs",
            Self::ExceedsServingMass { .. } => "exceeds_serving_mass",
            Self::Per100gOver100 { .. } => "per_100g_over_100",
            Self::SaltSodiumMismatch { .. } => "salt_sodium_mismatch",
            Self::ServingMismatch { .. } => "serving_mismatch",
            Self::ServingMassOutOfRange { .. } => "serving_mass_out_of_range",
            Self::SodiumTooHigh { .. } => "sodium_too_high",
            Self::CaffeineTooHigh { .. } => "caffeine_too_high",
            Self::RatioOutOfRange { .. } => "ratio_out_of_range",
        }
    }
}

impl fmt::Display for PlausibilityViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFinite { field } => write!(f, "{field} is not a finite number"),
            Self::Negative { field, value } => write!(f, "{field} is negative ({value})"),
            Self::CarbsMissing => f.write_str("carbs per serving are required for this form"),
            Self::SugarsExceedCarbs {
                basis,
                sugars_g,
                carbs_g,
            } => write!(
                f,
                "sugars {sugars_g} g exceed carbs {carbs_g} g ({})",
                basis_label(*basis)
            ),
            Self::ExceedsServingMass {
                field,
                value_g,
                serving_g,
            } => write!(f, "{field} {value_g} g exceeds the {serving_g} g serving"),
            Self::Per100gOver100 { total_g } => {
                write!(f, "per-100 g masses add up to {total_g} g")
            }
            Self::SaltSodiumMismatch {
                salt_g_per_100g,
                expected_salt_g,
            } => write!(
                f,
                "salt {salt_g_per_100g} g/100 g does not match sodium (expected {expected_salt_g:.3} g)"
            ),
            Self::ServingMismatch {
                field,
                per_serving,
                expected,
            } => write!(
                f,
                "{field} {per_serving} per serving does not match its per-100 g value (expected {expected:.2})"
            ),
            Self::ServingMassOutOfRange {
                serving_g,
                min_g,
                max_g,
            } => write!(
                f,
                "serving {serving_g} g is outside {min_g}..={max_g} g for this form"
            ),
            Self::SodiumTooHigh { sodium_mg } => {
                write!(f, "sodium {sodium_mg} mg per serving exceeds {MAX_SODIUM_MG_PER_SERVING} mg")
            }
            Self::CaffeineTooHigh { caffeine_mg } => write!(
                f,
                "caffeine {caffeine_mg} mg per serving exceeds {MAX_CAFFEINE_MG_PER_SERVING} mg"
            ),
            Self::RatioOutOfRange { ratio } => write!(
                f,
                "glucose:fructose ratio {ratio} is outside 0..={MAX_GLUCOSE_FRUCTOSE_RATIO}"
            ),
        }
    }
}

const fn basis_label(basis: FactBasis) -> &'static str {
    match basis {
        FactBasis::PerServing => "per serving",
        FactBasis::Per100g => "per 100 g",
    }
}

/// The most sodium one serving of a fuel product carries, mg.
pub const MAX_SODIUM_MG_PER_SERVING: f64 = 2000.0;

/// The most caffeine one serving carries, mg: the single-day ceiling
/// EFSA judges of no concern for a healthy adult.
pub const MAX_CAFFEINE_MG_PER_SERVING: f64 = 400.0;

/// The largest glucose:fructose ratio a label can state (fructose per gram of
/// glucose source).
pub const MAX_GLUCOSE_FRUCTOSE_RATIO: f64 = 1.5;

/// Relative tolerance between two statements of one fact on a label.
const LABEL_TOLERANCE: f64 = 0.05;

/// Absolute floors under [`LABEL_TOLERANCE`]: labels round grams to 0.1 g
/// below 10 g and to 1 g above, sodium to the milligram and salt to 0.01 g,
/// so near zero a relative 5 % is narrower than the rounding itself.
const GRAMS_ROUNDING_FLOOR: f64 = 0.5;
const SODIUM_MG_ROUNDING_FLOOR: f64 = 5.0;
const SALT_G_ROUNDING_FLOOR: f64 = 0.0125;

/// Rounding headroom on the 100 g mass sum: a pure dextrose label states
/// 100 g carbs and 0.01 g salt.
const PER_100G_SUM_ROUNDING_G: f64 = 1.0;

/// Grams of salt per gram of sodium (`NaCl` / `Na`, as EU labelling uses).
const SALT_PER_SODIUM: f64 = 2.5;

fn within(stated: f64, expected: f64, floor: f64) -> bool {
    (stated - expected).abs() <= (expected.abs() * LABEL_TOLERANCE).max(floor)
}

/// Whether `facts` can be what a label of `form` states.
///
/// Every check runs only when the values it compares are present, and every
/// violation is reported — the gate never adjusts a value to make it pass.
///
/// # Errors
/// Returns every [`PlausibilityViolation`] the facts trip.
pub fn check_plausibility(
    form: FuelForm,
    facts: &FuelFacts,
) -> Result<(), Vec<PlausibilityViolation>> {
    let mut violations = Vec::new();
    for (field, read) in NUMERIC_FIELDS {
        if let Some(value) = read(facts) {
            if !value.is_finite() {
                violations.push(PlausibilityViolation::NotFinite {
                    field: field.to_owned(),
                });
            } else if value < 0.0 {
                violations.push(PlausibilityViolation::Negative {
                    field: field.to_owned(),
                    value,
                });
            }
        }
    }
    // Comparisons over NaN or negative values would only restate the above.
    if !violations.is_empty() {
        return Err(violations);
    }
    if form.requires_carbs() && facts.carbs_g.is_none() {
        violations.push(PlausibilityViolation::CarbsMissing);
    }
    check_sugars(facts, &mut violations);
    check_serving(form, facts, &mut violations);
    check_per_100g(facts, &mut violations);
    check_bounds(facts, &mut violations);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn check_sugars(facts: &FuelFacts, violations: &mut Vec<PlausibilityViolation>) {
    let pairs = [
        (FactBasis::PerServing, facts.sugars_g, facts.carbs_g),
        (
            FactBasis::Per100g,
            facts.sugars_g_per_100g,
            facts.carbs_g_per_100g,
        ),
    ];
    for (basis, sugars, carbs) in pairs {
        if let (Some(sugars_g), Some(carbs_g)) = (sugars, carbs) {
            if sugars_g > carbs_g {
                violations.push(PlausibilityViolation::SugarsExceedCarbs {
                    basis,
                    sugars_g,
                    carbs_g,
                });
            }
        }
    }
}

fn check_serving(form: FuelForm, facts: &FuelFacts, violations: &mut Vec<PlausibilityViolation>) {
    let Some(serving_g) = facts.serving_g else {
        return;
    };
    let (min_g, max_g) = form.serving_g_bounds();
    if !(min_g..=max_g).contains(&serving_g) {
        violations.push(PlausibilityViolation::ServingMassOutOfRange {
            serving_g,
            min_g,
            max_g,
        });
    }
    let masses = [
        ("carbs_g", facts.carbs_g),
        ("sugars_g", facts.sugars_g),
        ("protein_g", facts.protein_g),
        ("fat_g", facts.fat_g),
    ];
    for (field, value) in masses {
        if let Some(value_g) = value {
            if value_g > serving_g {
                violations.push(PlausibilityViolation::ExceedsServingMass {
                    field: field.to_owned(),
                    value_g,
                    serving_g,
                });
            }
        }
    }
    let pairs = [
        (
            "carbs_g",
            facts.carbs_g,
            facts.carbs_g_per_100g,
            GRAMS_ROUNDING_FLOOR,
        ),
        (
            "sugars_g",
            facts.sugars_g,
            facts.sugars_g_per_100g,
            GRAMS_ROUNDING_FLOOR,
        ),
        (
            "protein_g",
            facts.protein_g,
            facts.protein_g_per_100g,
            GRAMS_ROUNDING_FLOOR,
        ),
        (
            "fat_g",
            facts.fat_g,
            facts.fat_g_per_100g,
            GRAMS_ROUNDING_FLOOR,
        ),
        (
            "sodium_mg",
            facts.sodium_mg,
            facts.sodium_mg_per_100g,
            SODIUM_MG_ROUNDING_FLOOR,
        ),
    ];
    for (field, per_serving, per_100g, floor) in pairs {
        if let (Some(per_serving), Some(per_100g)) = (per_serving, per_100g) {
            let expected = per_100g * serving_g / 100.0;
            if !within(per_serving, expected, floor) {
                violations.push(PlausibilityViolation::ServingMismatch {
                    field: field.to_owned(),
                    per_serving,
                    expected,
                });
            }
        }
    }
}

fn check_per_100g(facts: &FuelFacts, violations: &mut Vec<PlausibilityViolation>) {
    let total_g = facts.carbs_g_per_100g.unwrap_or(0.0)
        + facts.protein_g_per_100g.unwrap_or(0.0)
        + facts.fat_g_per_100g.unwrap_or(0.0)
        + facts.sodium_mg_per_100g.unwrap_or(0.0) / 1000.0;
    if total_g > 100.0 + PER_100G_SUM_ROUNDING_G {
        violations.push(PlausibilityViolation::Per100gOver100 { total_g });
    }
    if let (Some(salt_g_per_100g), Some(sodium_mg)) =
        (facts.salt_g_per_100g, facts.sodium_mg_per_100g)
    {
        let expected_salt_g = sodium_mg * SALT_PER_SODIUM / 1000.0;
        if !within(salt_g_per_100g, expected_salt_g, SALT_G_ROUNDING_FLOOR) {
            violations.push(PlausibilityViolation::SaltSodiumMismatch {
                salt_g_per_100g,
                expected_salt_g,
            });
        }
    }
}

fn check_bounds(facts: &FuelFacts, violations: &mut Vec<PlausibilityViolation>) {
    if let Some(sodium_mg) = facts.sodium_mg.filter(|v| *v > MAX_SODIUM_MG_PER_SERVING) {
        violations.push(PlausibilityViolation::SodiumTooHigh { sodium_mg });
    }
    if let Some(caffeine_mg) = facts
        .caffeine_mg
        .filter(|v| *v > MAX_CAFFEINE_MG_PER_SERVING)
    {
        violations.push(PlausibilityViolation::CaffeineTooHigh { caffeine_mg });
    }
    if let Some(ratio) = facts
        .glucose_fructose_ratio
        .filter(|v| *v > MAX_GLUCOSE_FRUCTOSE_RATIO)
    {
        violations.push(PlausibilityViolation::RatioOutOfRange { ratio });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Maurten-like Gel 100: 40 g serving, 25 g carbs.
    fn gel() -> FuelFacts {
        FuelFacts {
            serving_label: Some("1 gel (40 g)".to_owned()),
            serving_g: Some(40.0),
            energy_kcal: Some(100.0),
            carbs_g: Some(25.0),
            sugars_g: Some(12.0),
            protein_g: Some(0.0),
            fat_g: Some(0.0),
            sodium_mg: Some(34.0),
            carbs_g_per_100g: Some(62.5),
            sugars_g_per_100g: Some(30.0),
            protein_g_per_100g: Some(0.0),
            fat_g_per_100g: Some(0.0),
            sodium_mg_per_100g: Some(85.0),
            salt_g_per_100g: Some(0.21),
            glucose_fructose_ratio: Some(0.8),
            ..FuelFacts::default()
        }
    }

    fn codes(form: FuelForm, facts: &FuelFacts) -> Vec<&'static str> {
        match check_plausibility(form, facts) {
            Ok(()) => Vec::new(),
            Err(v) => v.iter().map(PlausibilityViolation::code).collect(),
        }
    }

    #[test]
    fn a_consistent_label_passes() {
        assert_eq!(codes(FuelForm::Gel, &gel()), Vec::<&str>::new());
    }

    #[test]
    fn sugars_above_carbs_is_rejected_on_either_basis() {
        let mut facts = gel();
        facts.sugars_g = Some(26.0);
        assert!(codes(FuelForm::Gel, &facts).contains(&"sugars_exceed_carbs"));
        let mut facts = gel();
        facts.sugars_g_per_100g = Some(70.0);
        assert!(codes(FuelForm::Gel, &facts).contains(&"sugars_exceed_carbs"));
    }

    #[test]
    fn a_macro_heavier_than_the_serving_is_rejected() {
        let mut facts = gel();
        facts.carbs_g = Some(45.0);
        facts.carbs_g_per_100g = None;
        assert_eq!(codes(FuelForm::Gel, &facts), vec!["exceeds_serving_mass"]);
    }

    #[test]
    fn per_100g_masses_over_100_are_rejected() {
        let facts = FuelFacts {
            carbs_g: Some(10.0),
            carbs_g_per_100g: Some(90.0),
            protein_g_per_100g: Some(15.0),
            ..FuelFacts::default()
        };
        assert_eq!(codes(FuelForm::Bar, &facts), vec!["per_100g_over_100"]);
    }

    #[test]
    fn pure_dextrose_rounding_is_not_over_100() {
        let facts = FuelFacts {
            carbs_g: Some(20.0),
            carbs_g_per_100g: Some(100.0),
            sodium_mg_per_100g: Some(4.0),
            salt_g_per_100g: Some(0.01),
            ..FuelFacts::default()
        };
        assert_eq!(codes(FuelForm::DrinkMix, &facts), Vec::<&str>::new());
    }

    #[test]
    fn salt_must_be_sodium_times_two_and_a_half() {
        let mut facts = gel();
        facts.salt_g_per_100g = Some(0.5);
        assert_eq!(codes(FuelForm::Gel, &facts), vec!["salt_sodium_mismatch"]);
        // Within 5 %: 85 mg sodium is 0.2125 g salt; 0.22 is 3.5 % off.
        facts.salt_g_per_100g = Some(0.22);
        assert_eq!(codes(FuelForm::Gel, &facts), Vec::<&str>::new());
    }

    #[test]
    fn per_serving_must_match_per_100g_at_the_serving_mass() {
        let mut facts = gel();
        facts.carbs_g = Some(30.0);
        assert_eq!(codes(FuelForm::Gel, &facts), vec!["serving_mismatch"]);
        let mut facts = gel();
        facts.sodium_mg = Some(60.0);
        assert_eq!(codes(FuelForm::Gel, &facts), vec!["serving_mismatch"]);
    }

    #[test]
    fn serving_mass_is_bounded_by_form() {
        let tablet = FuelFacts {
            serving_g: Some(4.5),
            sodium_mg: Some(300.0),
            ..FuelFacts::default()
        };
        assert_eq!(codes(FuelForm::Tablet, &tablet), Vec::<&str>::new());
        let heavy = FuelFacts {
            serving_g: Some(6.0),
            ..tablet
        };
        assert_eq!(
            codes(FuelForm::Capsule, &heavy),
            vec!["serving_mass_out_of_range"]
        );
        let mut facts = gel();
        facts.serving_g = Some(250.0);
        facts.carbs_g_per_100g = None;
        facts.sugars_g_per_100g = None;
        facts.protein_g_per_100g = None;
        facts.fat_g_per_100g = None;
        facts.sodium_mg_per_100g = None;
        assert_eq!(
            codes(FuelForm::Gel, &facts),
            vec!["serving_mass_out_of_range"]
        );
    }

    #[test]
    fn sodium_caffeine_and_ratio_are_bounded() {
        let facts = FuelFacts {
            carbs_g: Some(25.0),
            sodium_mg: Some(2500.0),
            caffeine_mg: Some(500.0),
            glucose_fructose_ratio: Some(2.0),
            ..FuelFacts::default()
        };
        assert_eq!(
            codes(FuelForm::Gel, &facts),
            vec!["sodium_too_high", "caffeine_too_high", "ratio_out_of_range"]
        );
    }

    #[test]
    fn negatives_and_non_finite_values_are_rejected() {
        let facts = FuelFacts {
            carbs_g: Some(-1.0),
            fat_g: Some(f64::NAN),
            ..FuelFacts::default()
        };
        assert_eq!(codes(FuelForm::Gel, &facts), vec!["negative", "not_finite"]);
    }

    #[test]
    fn carbs_are_required_only_for_carbohydrate_forms() {
        let none = FuelFacts::default();
        for form in [
            FuelForm::Gel,
            FuelForm::DrinkMix,
            FuelForm::Chew,
            FuelForm::Bar,
            FuelForm::Puree,
            FuelForm::Waffle,
        ] {
            assert_eq!(codes(form, &none), vec!["carbs_missing"], "{form:?}");
        }
        for form in [FuelForm::Capsule, FuelForm::Tablet, FuelForm::Other] {
            assert_eq!(codes(form, &none), Vec::<&str>::new(), "{form:?}");
        }
    }

    #[test]
    fn a_puree_is_bounded_like_a_gel_and_a_waffle_like_a_bar() {
        assert_eq!(
            FuelForm::Puree.serving_g_bounds(),
            FuelForm::Gel.serving_g_bounds()
        );
        assert_eq!(
            FuelForm::Waffle.serving_g_bounds(),
            FuelForm::Bar.serving_g_bounds()
        );
        // A 100 g pouch and a 30 g waffle pass; a 2 g "waffle" is a misread.
        let pouch = FuelFacts {
            serving_g: Some(100.0),
            carbs_g: Some(20.0),
            ..FuelFacts::default()
        };
        assert_eq!(codes(FuelForm::Puree, &pouch), Vec::<&str>::new());
        let waffle = FuelFacts {
            serving_g: Some(30.0),
            carbs_g: Some(21.0),
            ..FuelFacts::default()
        };
        assert_eq!(codes(FuelForm::Waffle, &waffle), Vec::<&str>::new());
        let crumb = FuelFacts {
            serving_g: Some(2.0),
            carbs_g: Some(1.0),
            ..FuelFacts::default()
        };
        assert_eq!(
            codes(FuelForm::Waffle, &crumb),
            vec!["serving_mass_out_of_range"]
        );
    }

    #[test]
    fn the_gate_never_changes_the_facts() {
        let mut facts = gel();
        facts.sugars_g = Some(26.0);
        let before = facts.clone();
        assert!(check_plausibility(FuelForm::Gel, &facts).is_err());
        assert_eq!(facts, before);
    }
}
