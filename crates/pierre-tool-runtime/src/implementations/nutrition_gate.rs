// ABOUTME: The one code-level gate on prescriptive nutrition figures: a medical/PAR-Q flag withholds them
// ABOUTME: A flagged athlete gets qualitative guidance plus a machine-readable note that a clinician sets the amounts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Nutrition figures gate
//!
//! The agent prompt tells the model to withhold nutrition amounts when the
//! dossier carries a medical flag (`okf::render_fact` injects the redacted
//! flag). That covers one path. A direct MCP client, an A2A caller and any
//! agent built without that prompt call the same tools and used to get the
//! same grams, so the gate lives here, in code, where every transport passes.
//!
//! **What is gated** — the tools that *prescribe* an amount for this athlete:
//!
//! | Tool | Figures it would prescribe |
//! |---|---|
//! | `calculate_daily_nutrition` | kcal/day, protein/carbohydrate/fat g/day |
//! | `get_nutrient_timing` | pre/post-session carbohydrate and protein g, protein g per meal |
//! | `get_recipe_constraints` | a meal's kcal and macro g targets |
//! | `generate_recommendations` (`nutrition`) | recovery protein g, refuelling carbohydrate g/h, rehydration L/kg, meal gram counts |
//!
//! **What is not** — tools that *describe* food or a meal the athlete names:
//! `analyze_meal_nutrition`, `search_food`, `get_food_details`,
//! `validate_recipe`, `get_recipe`, `list_recipes`, `search_recipes`. What a
//! plate or a recipe contains is a property of the food, the same fact a label
//! prints; an athlete whose clinician sets their targets still needs to know
//! what they ate to stay inside them. None of these sets a target.
//!
//! Plan fuelling (`save_training_plan` day `fueling`) is gated on the same
//! flag: a save carrying rates is refused and every surface that shows a
//! stored protocol withholds it — see `pierre_services::plan_fueling`.
//!
//! The flag read and the [`FiguresWithheld`] statement live in
//! `pierre_services::medical_flag`, the one reader both gates share; they are
//! re-exported here as the tool runtime's vocabulary for the gate.

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
pub use pierre_services::medical_flag::{
    athlete_tenants, medical_flag_on_file, nutrition_figures_gate, FigureAuthority,
    FiguresWithheld, WithheldReason,
};
use serde::{Deserialize, Serialize};

use crate::context::ToolExecutionContext;

/// A prescriptive nutrition answer without its figures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WithheldFigures {
    /// Why the amounts are absent and who sets them.
    pub figures_withheld: FiguresWithheld,
    /// What to do instead, qualitatively — foods and timing, no amounts.
    pub guidance: Vec<String>,
}

/// What a gated tool answers with: its figures, or guidance without them.
///
/// Untagged, so an athlete without a flag gets the tool's own shape byte for
/// byte. A client tells the arms apart by `figures_withheld`, which only the
/// withheld arm carries.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum NutritionAnswer<T> {
    /// The figures, for an athlete with no medical flag on file.
    Figures(T),
    /// Qualitative guidance only: a clinician sets this athlete's amounts.
    Withheld(WithheldFigures),
}

impl<T> NutritionAnswer<T> {
    /// The withheld arm for a medical flag, carrying `guidance`.
    #[must_use]
    pub fn withheld(figures_withheld: FiguresWithheld, guidance: &[&str]) -> Self {
        Self::Withheld(WithheldFigures {
            figures_withheld,
            guidance: guidance.iter().map(|line| (*line).to_owned()).collect(),
        })
    }
}

/// [`nutrition_figures_gate`] for the athlete a tool call is made for.
///
/// # Errors
///
/// Returns the repository error when the tenants or the flag cannot be read.
pub async fn gate_for_call(context: &ToolExecutionContext) -> AppResult<Option<FiguresWithheld>> {
    nutrition_figures_gate(
        context.resources.repos(),
        context.tenant_id.map(TenantId::from_uuid),
        context.user_id,
    )
    .await
}
