// ABOUTME: The nutrition admin parameters — the athlete protein target, its evidence range and the kernel default it overrides
// ABOUTME: Registers nutrition.protein_athlete_g_per_kg and reads a resolved override back as a validated g/kg/day
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Nutrition as admin configuration.
//!
//! `calculate_daily_nutrition` sets the protein target of a very or extra
//! active athlete on a maintenance or weight-loss goal at dravr-cageux's
//! `MacronutrientConfig::protein_athlete_g_per_kg` per kilogram of body mass;
//! the endurance and muscle-gain goals and the lower activity levels read
//! their own factors.
//! [`PROTEIN_ATHLETE_G_PER_KG_KEY`] lets an operator move that value
//! system-wide, for one tenant or for one user, inside the 1.2-2.0 g/kg/day
//! the joint ACSM / Academy of Nutrition and Dietetics / Dietitians of Canada
//! position statement gives for athletes (Thomas, Erdman & Burke 2016).
//!
//! The default is never retyped: [`register_nutrition`] reads it from the
//! kernel's compiled [`IntelligenceConfig`] default. The calculator applies
//! only an explicit override; with none, it keeps the kernel's configured
//! value, so the catalogue and the kernel cannot disagree about what the
//! default is.

use std::collections::HashMap;
use std::hash::BuildHasher;

use pierre_core::errors::{AppError, AppResult};
use pierre_intelligence::IntelligenceConfig;
use serde_json::Value;

use crate::admin_definitions::ParameterDefinition;
use crate::admin_types::{ConfigDataType, ParameterRange};

/// Catalog category the nutrition parameters are grouped under.
pub const CATEGORY: &str = "nutrition";

/// The athlete protein target, g/kg/day.
pub const PROTEIN_ATHLETE_G_PER_KG_KEY: &str = "nutrition.protein_athlete_g_per_kg";

/// Lowest athlete protein target the position statement supports, g/kg/day.
pub const PROTEIN_ATHLETE_MIN_G_PER_KG: f64 = 1.2;

/// Highest athlete protein target the position statement supports, g/kg/day.
pub const PROTEIN_ATHLETE_MAX_G_PER_KG: f64 = 2.0;

/// Register the `Nutrition` catalog entries, the default read from the
/// kernel's compiled `MacronutrientConfig`.
pub fn register_nutrition<S: BuildHasher>(defs: &mut HashMap<String, ParameterDefinition, S>) {
    let kernel_default = IntelligenceConfig::<true>::default()
        .nutrition
        .macronutrients
        .protein_athlete_g_per_kg;
    let def = ParameterDefinition {
        key: PROTEIN_ATHLETE_G_PER_KG_KEY.to_owned(),
        display_name: "Athlete Protein Target".to_owned(),
        description: "Daily protein intake per kg body weight that calculate_daily_nutrition \
                      prescribes a very or extra active athlete on a maintenance or \
                      weight-loss goal"
            .to_owned(),
        category: CATEGORY.to_owned(),
        data_type: ConfigDataType::Float,
        default_value: serde_json::json!(kernel_default),
        valid_range: Some(ParameterRange {
            min: serde_json::json!(PROTEIN_ATHLETE_MIN_G_PER_KG),
            max: serde_json::json!(PROTEIN_ATHLETE_MAX_G_PER_KG),
            step: Some(0.1),
        }),
        enum_options: None,
        units: Some("g/kg/day".to_owned()),
        scientific_basis: Some(
            "Thomas, Erdman & Burke 2016, ACSM / Academy of Nutrition and Dietetics / \
             Dietitians of Canada joint position statement: 1.2-2.0 g/kg/day"
                .to_owned(),
        ),
        env: None,
        is_runtime_configurable: true,
        requires_restart: false,
    };
    defs.insert(def.key.clone(), def);
}

/// Read a resolved [`PROTEIN_ATHLETE_G_PER_KG_KEY`] value as g/kg/day.
///
/// Every write path bounds the value to the catalogue's range, so a stored
/// override outside it is a row edited in the database. It is refused rather
/// than clamped: prescribing an amount the operator did not choose is worse
/// than an error that names the key.
///
/// # Errors
///
/// Returns [`AppError::config`] when the value is not a number or lies
/// outside `1.2..=2.0`.
pub fn athlete_protein_g_per_kg(value: &Value) -> AppResult<f64> {
    let g_per_kg = value.as_f64().ok_or_else(|| {
        AppError::config(format!(
            "{PROTEIN_ATHLETE_G_PER_KG_KEY} holds {value}, which is not a number"
        ))
    })?;
    if (PROTEIN_ATHLETE_MIN_G_PER_KG..=PROTEIN_ATHLETE_MAX_G_PER_KG).contains(&g_per_kg) {
        Ok(g_per_kg)
    } else {
        Err(AppError::config(format!(
            "{PROTEIN_ATHLETE_G_PER_KG_KEY} holds {g_per_kg} g/kg/day, outside \
             {PROTEIN_ATHLETE_MIN_G_PER_KG}-{PROTEIN_ATHLETE_MAX_G_PER_KG}"
        )))
    }
}
