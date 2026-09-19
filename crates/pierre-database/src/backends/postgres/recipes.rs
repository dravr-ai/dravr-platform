// ABOUTME: PostgreSQL-backed RecipeRepository, emitted from the shared implementation in repositories/recipes.rs
// ABOUTME: user_id is a native uuid here, so the shell passes the native uuid codec; search matches with ILIKE
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::recipes::{MealTiming, Recipe, RecipeIngredient, ValidatedNutrition};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::shared::transactions::TransactionGuard;
use crate::repositories::recipes::{
    contains_pattern, fdc_id_column, impl_recipe_repository, ingredient_from_row,
    ingredients_batch_sql, ingredients_by_recipe, insert_recipe_ingredients, meal_timing_to_string,
    page_bound, recipe_columns, recipe_from_row, recipes_with_ingredients, search_recipes_sql,
    sort_order_column, unit_to_string, RecipeRepository, COUNT_RECIPES_SQL, DELETE_INGREDIENTS_SQL,
    DELETE_RECIPE_SQL, GET_INGREDIENTS_SQL, GET_RECIPE_SQL, INSERT_INGREDIENT_SQL,
    INSERT_RECIPE_SQL, LIST_RECIPES_BY_TIMING_SQL, LIST_RECIPES_SQL, UPDATE_NUTRITION_CACHE_SQL,
    UPDATE_RECIPE_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

use super::PostgresDatabase;

impl_recipe_repository!(PostgresDatabase, NativeUuid, "ILIKE");
