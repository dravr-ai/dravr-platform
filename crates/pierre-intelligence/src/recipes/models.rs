// ABOUTME: Re-exports recipe data models from pierre-core and extends MacroTargets with config-aware methods
// ABOUTME: Provides MacroTargetsExt trait for IntelligenceConfig-dependent calorie/timing calculations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::IntelligenceConfig;

// Re-export all recipe model types from pierre-core (canonical definitions)
pub use pierre_core::models::recipes::{
    DietaryRestriction, IngredientUnit, MacroTargets, MealTiming, Recipe, RecipeConstraints,
    RecipeIngredient, SkillLevel, ValidatedNutrition,
};

/// Extension trait for `MacroTargets` that adds config-aware construction methods.
///
/// This trait exists because `from_calories_and_timing` requires access to
/// `IntelligenceConfig` (which lives in `pierre-intelligence`), while the
/// `MacroTargets` type itself is defined in `pierre-core` to avoid a circular
/// dependency between `pierre-database` and `pierre-intelligence`.
pub trait MacroTargetsExt {
    /// Create targets from calorie goal and meal timing
    ///
    /// Uses configurable macro distribution percentages from the global intelligence config.
    /// Defaults are based on ISSN sports nutrition position stands.
    fn from_calories_and_timing(calories: f64, timing: MealTiming) -> MacroTargets;
}

impl MacroTargetsExt for MacroTargets {
    fn from_calories_and_timing(calories: f64, timing: MealTiming) -> MacroTargets {
        // Use configurable macro distribution from global config
        let config = IntelligenceConfig::global();
        let (protein_pct, carbs_pct, fat_pct) =
            config.nutrition.meal_timing_macros.get_distribution(timing);

        Self::from_calories_and_distribution(calories, protein_pct, carbs_pct, fat_pct)
    }
}
