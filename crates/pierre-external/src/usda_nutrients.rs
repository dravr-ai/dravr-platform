// ABOUTME: The one reader of a USDA food's per-100 g figures, keyed on nutrient ids, never on names
// ABOUTME: Also picks the generic (Foundation / SR Legacy) entry a plain ingredient name means

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Reading a USDA food
//!
//! A `FoodData` Central detail lists energy twice under one name: `Energy`
//! in kcal (id 1008) and `Energy` in kJ (id 1062). A reader that matched the
//! name summed both, so 120 g of banana (89 kcal and 371 kJ per 100 g) read
//! as 552 kcal. Names also drift: total sugars, once "Sugars, total including
//! NLEA", now ships as "Total Sugars" (id 2000 throughout), and every sugar
//! read 0. So [`Per100g::of`] keys every figure on the nutrient id and takes
//! one value per figure, in a stated order of preference, never a sum.
//!
//! Foundation foods often carry no id-1008 row: their energy comes as the
//! Atwater rows (2048 specific factors, 2047 general factors), in kcal. A kJ
//! row is read only when no kcal row exists, converted at 4.184 kJ/kcal.

use crate::usda_client::{FoodDetails, FoodNutrient, FoodSearchResult};

/// Energy, kcal (SR Legacy, Survey, Branded).
const ENERGY_KCAL: u32 = 1008;
/// Energy, Atwater specific factors, kcal (Foundation).
const ENERGY_ATWATER_SPECIFIC: u32 = 2048;
/// Energy, Atwater general factors, kcal (Foundation).
const ENERGY_ATWATER_GENERAL: u32 = 2047;
/// Energy, kJ.
const ENERGY_KJ: u32 = 1062;
/// kJ in one kcal (thermochemical calorie).
const KJ_PER_KCAL: f64 = 4.184;

/// Protein, g.
const PROTEIN: &[u32] = &[1003];
/// Total lipid (fat), then total fat (NLEA), g.
const FAT: &[u32] = &[1004, 1085];
/// Carbohydrate by difference, then by summation, g.
const CARBOHYDRATE: &[u32] = &[1005, 1050];
/// Fiber, total dietary, g.
const FIBER: &[u32] = &[1079];
/// Sodium, mg.
const SODIUM: &[u32] = &[1093];
/// Total sugars (2000, whatever its name that year), then Sugars, Total
/// (Foundation), g.
const SUGARS: &[u32] = &[2000, 1063];

/// What 100 g of a food holds, each figure read once.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Per100g {
    /// Energy, kcal.
    pub energy_kcal: f64,
    /// Protein, g.
    pub protein_g: f64,
    /// Carbohydrate, g.
    pub carbs_g: f64,
    /// Fat, g.
    pub fat_g: f64,
    /// Fiber, g.
    pub fiber_g: f64,
    /// Sodium, mg.
    pub sodium_mg: f64,
    /// Total sugars, g.
    pub sugar_g: f64,
}

impl Per100g {
    /// Read `food`'s per-100 g figures; a figure the food does not list is 0.
    #[must_use]
    pub fn of(food: &FoodDetails) -> Self {
        let nutrients = &food.food_nutrients;
        Self {
            energy_kcal: energy_kcal(nutrients),
            protein_g: first_of(nutrients, PROTEIN),
            carbs_g: first_of(nutrients, CARBOHYDRATE),
            fat_g: first_of(nutrients, FAT),
            fiber_g: first_of(nutrients, FIBER),
            sodium_mg: first_of(nutrients, SODIUM),
            sugar_g: first_of(nutrients, SUGARS),
        }
    }

    /// These figures for `grams` of the food.
    #[must_use]
    pub fn for_grams(&self, grams: f64) -> Self {
        let factor = grams / 100.0;
        Self {
            energy_kcal: self.energy_kcal * factor,
            protein_g: self.protein_g * factor,
            carbs_g: self.carbs_g * factor,
            fat_g: self.fat_g * factor,
            fiber_g: self.fiber_g * factor,
            sodium_mg: self.sodium_mg * factor,
            sugar_g: self.sugar_g * factor,
        }
    }

    /// The sum of two sets of figures.
    #[must_use]
    pub fn plus(&self, other: &Self) -> Self {
        Self {
            energy_kcal: self.energy_kcal + other.energy_kcal,
            protein_g: self.protein_g + other.protein_g,
            carbs_g: self.carbs_g + other.carbs_g,
            fat_g: self.fat_g + other.fat_g,
            fiber_g: self.fiber_g + other.fiber_g,
            sodium_mg: self.sodium_mg + other.sodium_mg,
            sugar_g: self.sugar_g + other.sugar_g,
        }
    }
}

/// The amount of the first id in `ids` the food lists.
fn first_of(nutrients: &[FoodNutrient], ids: &[u32]) -> f64 {
    ids.iter()
        .find_map(|id| nutrients.iter().find(|n| n.nutrient_id == *id))
        .map_or(0.0, |n| n.amount)
}

/// The food's energy in kcal: the first kcal row in preference order, else
/// its kJ row converted, else 0.
fn energy_kcal(nutrients: &[FoodNutrient]) -> f64 {
    let kcal = [ENERGY_KCAL, ENERGY_ATWATER_SPECIFIC, ENERGY_ATWATER_GENERAL]
        .iter()
        .find_map(|id| {
            nutrients
                .iter()
                .find(|n| n.nutrient_id == *id && n.unit_name.eq_ignore_ascii_case("kcal"))
        });
    if let Some(row) = kcal {
        return row.amount;
    }
    nutrients
        .iter()
        .find(|n| n.nutrient_id == ENERGY_KJ && n.unit_name.eq_ignore_ascii_case("kj"))
        .map_or(0.0, |row| row.amount / KJ_PER_KCAL)
}

/// The `dataType` filter that asks the search for generic foods only.
pub(crate) const GENERIC_DATA_TYPES: &str = "Foundation,SR Legacy";

/// The entry a plain ingredient name means among generic search results.
///
/// The search ranks poorly for a bare name ("banana" returns banana powder
/// before raw bananas), so among `results` the pick is the first entry whose
/// leading word is the name's own (plurals aside), preferring a raw one, then
/// the plainest (fewest comma-separated qualifiers); with no such entry, the
/// search's own first result. `None` only when `results` is empty.
#[must_use]
pub fn pick_generic_match<'a>(
    query: &str,
    results: &'a [FoodSearchResult],
) -> Option<&'a FoodSearchResult> {
    let head = query.split_whitespace().next().map(singular)?;
    results
        .iter()
        .enumerate()
        .filter(|(_, food)| leading_word(&food.description).is_some_and(|w| w == head))
        .min_by_key(|(rank, food)| {
            let segments: Vec<&str> = food.description.split(',').map(str::trim).collect();
            let raw = segments.iter().any(|s| s.eq_ignore_ascii_case("raw"));
            (!raw, segments.len(), *rank)
        })
        .map(|(_, food)| food)
        .or_else(|| results.first())
}

/// The first word of a description, lowercased and singular.
fn leading_word(description: &str) -> Option<String> {
    description
        .split(|c: char| c == ',' || c.is_whitespace())
        .find(|w| !w.is_empty())
        .map(singular)
}

/// `word` lowercased, without a plural `s` ("Bananas" → "banana", "oats" →
/// "oat"); a word ending in "ss" keeps it.
fn singular(word: &str) -> String {
    let lower = word.to_lowercase();
    match lower.strip_suffix('s') {
        Some(stem) if !stem.is_empty() && !stem.ends_with('s') => stem.to_owned(),
        _ => lower,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nutrient(id: u32, name: &str, unit: &str, amount: f64) -> FoodNutrient {
        FoodNutrient {
            nutrient_id: id,
            nutrient_name: name.to_owned(),
            unit_name: unit.to_owned(),
            amount,
        }
    }

    fn food(nutrients: Vec<FoodNutrient>) -> FoodDetails {
        FoodDetails {
            fdc_id: 173_944,
            description: "Bananas, raw".to_owned(),
            data_type: "SR Legacy".to_owned(),
            food_nutrients: nutrients,
            serving_size: None,
            serving_size_unit: None,
        }
    }

    /// fdc 173944 as `format=full` serves it (2026-11-26): energy in kcal
    /// and in kJ under one name, sugars named "Total Sugars".
    fn banana() -> FoodDetails {
        food(vec![
            nutrient(1008, "Energy", "kcal", 89.0),
            nutrient(1062, "Energy", "kJ", 371.0),
            nutrient(1003, "Protein", "g", 1.09),
            nutrient(1004, "Total lipid (fat)", "g", 0.33),
            nutrient(1005, "Carbohydrate, by difference", "g", 22.84),
            nutrient(1079, "Fiber, total dietary", "g", 2.6),
            nutrient(2000, "Total Sugars", "g", 12.23),
            nutrient(1093, "Sodium, Na", "mg", 1.0),
        ])
    }

    #[test]
    fn energy_is_read_once_in_kcal_never_summed_with_kj() {
        let per_120g = Per100g::of(&banana()).for_grams(120.0);
        assert!(
            (per_120g.energy_kcal - 106.8).abs() < 1e-9,
            "bug: 120 g of banana is 106.8 kcal, read {}",
            per_120g.energy_kcal
        );
    }

    #[test]
    fn sugars_are_read_by_id_whatever_their_name() {
        let sugar_g = Per100g::of(&banana()).sugar_g;
        assert!(
            (sugar_g - 12.23).abs() < 1e-9,
            "bug: banana's id-2000 \"Total Sugars\" is 12.23 g, read {sugar_g}"
        );
    }

    #[test]
    fn a_foundation_food_without_the_kcal_row_reads_its_atwater_energy() {
        let oats = food(vec![
            nutrient(1062, "Energy", "kJ", 1600.0),
            nutrient(2047, "Energy (Atwater General Factors)", "KCAL", 379.0),
            nutrient(2048, "Energy (Atwater Specific Factors)", "KCAL", 375.0),
        ]);
        let kcal = Per100g::of(&oats).energy_kcal;
        assert!(
            (kcal - 375.0).abs() < 1e-9,
            "bug: the Atwater specific-factor row (375 kcal) is preferred, read {kcal}"
        );
    }

    #[test]
    fn a_food_listing_only_kj_reads_it_converted() {
        let only_kj = food(vec![nutrient(1062, "Energy", "kJ", 418.4)]);
        let kcal = Per100g::of(&only_kj).energy_kcal;
        assert!(
            (kcal - 100.0).abs() < 1e-9,
            "bug: 418.4 kJ is 100 kcal, read {kcal}"
        );
    }

    fn result(fdc_id: u64, data_type: &str, description: &str) -> FoodSearchResult {
        serde_json::from_value(serde_json::json!({
            "fdcId": fdc_id,
            "description": description,
            "dataType": data_type,
        }))
        .unwrap()
    }

    #[test]
    fn a_plain_name_picks_the_raw_generic_food_over_a_processed_one() {
        // The generic search for "banana", in the order USDA ranked it.
        let results = [
            result(
                173_945,
                "SR Legacy",
                "Bananas, dehydrated, or banana powder",
            ),
            result(173_944, "SR Legacy", "Bananas, raw"),
            result(1_105_073, "Foundation", "Bananas, overripe, raw"),
            result(167_629, "SR Legacy", "Melon, banana (Navajo)"),
        ];
        assert_eq!(
            pick_generic_match("banana", &results).map(|f| f.fdc_id),
            Some(173_944),
            "bug: \"banana\" means raw bananas, not the powder USDA ranks first"
        );
    }

    #[test]
    fn a_name_no_entry_leads_with_falls_back_to_the_search_order() {
        let results = [
            result(173_161, "SR Legacy", "Rice crackers"),
            result(167_967, "SR Legacy", "Snacks, rice cakes, brown rice"),
        ];
        assert_eq!(
            pick_generic_match("basmati", &results).map(|f| f.fdc_id),
            Some(173_161),
            "bug: with no entry leading with the name, the search's first result is kept"
        );
        assert!(
            pick_generic_match("banana", &[]).is_none(),
            "bug: no result means no match"
        );
    }
}
