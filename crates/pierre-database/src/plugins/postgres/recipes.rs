// ABOUTME: PostgreSQL implementation of RecipeRepository for recipe CRUD with nutrition caching
// ABOUTME: Uses PG-native types (TIMESTAMPTZ, UUID as TEXT) and ILIKE for case-insensitive search
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::RecipeRepository;
use super::PostgresDatabase;
use crate::plugins::shared::transactions::PostgresTransactionGuard;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::recipes::{
    IngredientUnit, MealTiming, Recipe, RecipeIngredient, ValidatedNutrition,
};
use pierre_core::models::TenantId;
use sqlx::postgres::PgRow;
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

// ============================================================================
// Helper Functions
// ============================================================================

const fn meal_timing_to_string(timing: MealTiming) -> &'static str {
    match timing {
        MealTiming::PreTraining => "pre_training",
        MealTiming::PostTraining => "post_training",
        MealTiming::RestDay => "rest_day",
        MealTiming::General => "general",
    }
}

fn string_to_meal_timing(s: &str) -> MealTiming {
    match s {
        "pre_training" => MealTiming::PreTraining,
        "post_training" => MealTiming::PostTraining,
        "rest_day" => MealTiming::RestDay,
        _ => MealTiming::General,
    }
}

const fn unit_to_string(unit: IngredientUnit) -> &'static str {
    match unit {
        IngredientUnit::Grams => "grams",
        IngredientUnit::Milliliters => "milliliters",
        IngredientUnit::Cups => "cups",
        IngredientUnit::Tablespoons => "tablespoons",
        IngredientUnit::Teaspoons => "teaspoons",
        IngredientUnit::Pieces => "pieces",
        IngredientUnit::Ounces => "ounces",
        IngredientUnit::Pounds => "pounds",
        IngredientUnit::Kilograms => "kilograms",
    }
}

fn string_to_unit(s: &str) -> IngredientUnit {
    match s {
        "milliliters" => IngredientUnit::Milliliters,
        "cups" => IngredientUnit::Cups,
        "tablespoons" => IngredientUnit::Tablespoons,
        "teaspoons" => IngredientUnit::Teaspoons,
        "pieces" => IngredientUnit::Pieces,
        "ounces" => IngredientUnit::Ounces,
        "pounds" => IngredientUnit::Pounds,
        "kilograms" => IngredientUnit::Kilograms,
        _ => IngredientUnit::Grams,
    }
}

// ============================================================================
// Row Conversion
// ============================================================================

/// Convert a `PostgreSQL` row to a `Recipe`
///
/// Reads PG-native types directly: `TIMESTAMPTZ` as `DateTime<Utc>`,
/// integers as `i32`. UUIDs stored as TEXT are parsed from String.
fn row_to_recipe_pg(row: &PgRow, ingredients: Vec<RecipeIngredient>) -> AppResult<Recipe> {
    let id_str: String = row.get("id");
    let user_id_str: String = row.get("user_id");
    let meal_timing_str: String = row.get("meal_timing");
    let instructions_json: String = row.get("instructions");
    let tags_json: String = row.get("tags");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");

    let instructions: Vec<String> = serde_json::from_str(&instructions_json)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json)?;

    // Nullable nutrition cache fields
    let nutrition = row
        .try_get::<Option<f64>, _>("cached_calories")
        .ok()
        .flatten()
        .map(|calories| {
            let nutrition_validated_at: Option<DateTime<Utc>> =
                row.try_get("nutrition_validated_at").ok().flatten();
            ValidatedNutrition {
                calories,
                protein_g: row
                    .try_get::<Option<f64>, _>("cached_protein_g")
                    .ok()
                    .flatten()
                    .unwrap_or(0.0),
                carbs_g: row
                    .try_get::<Option<f64>, _>("cached_carbs_g")
                    .ok()
                    .flatten()
                    .unwrap_or(0.0),
                fat_g: row
                    .try_get::<Option<f64>, _>("cached_fat_g")
                    .ok()
                    .flatten()
                    .unwrap_or(0.0),
                fiber_g: row.try_get("cached_fiber_g").ok().flatten(),
                sodium_mg: row.try_get("cached_sodium_mg").ok().flatten(),
                sugar_g: row.try_get("cached_sugar_g").ok().flatten(),
                validated_at: nutrition_validated_at.unwrap_or_else(Utc::now),
            }
        });

    let servings: i32 = row.get("servings");
    let prep_time: Option<i32> = row.get("prep_time_mins");
    let cook_time: Option<i32> = row.get("cook_time_mins");

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Recipe {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::internal(format!("Invalid UUID: {e}")))?,
        user_id: Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::internal(format!("Invalid UUID: {e}")))?,
        name: row.get("name"),
        description: row.get("description"),
        servings: servings as u8,
        prep_time_mins: prep_time.map(|v| v as u16),
        cook_time_mins: cook_time.map(|v| v as u16),
        ingredients,
        instructions,
        tags,
        nutrition,
        meal_timing: string_to_meal_timing(&meal_timing_str),
        created_at,
        updated_at,
    })
}

/// Parse a PG ingredient row to `RecipeIngredient`
fn row_to_ingredient_pg(row: &PgRow) -> RecipeIngredient {
    let unit_str: String = row.get("unit");
    RecipeIngredient {
        fdc_id: row.get("fdc_id"),
        name: row.get("name"),
        amount: row.get("amount"),
        unit: string_to_unit(&unit_str),
        grams: row.get("grams"),
        preparation: row.get("preparation"),
    }
}

// ============================================================================
// Private Helpers
// ============================================================================

impl PostgresDatabase {
    /// Get ingredients for a single recipe
    async fn get_recipe_ingredients_pg(&self, recipe_id: &str) -> AppResult<Vec<RecipeIngredient>> {
        let rows = sqlx::query(
            r"
            SELECT id, recipe_id, fdc_id, name, amount, unit, grams, preparation, sort_order
            FROM recipe_ingredients
            WHERE recipe_id = $1
            ORDER BY sort_order
            ",
        )
        .bind(recipe_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get recipe ingredients: {e}")))?;

        Ok(rows.iter().map(row_to_ingredient_pg).collect())
    }

    /// Batch fetch ingredients for multiple recipes in a single query
    async fn get_ingredients_batch_pg(
        &self,
        recipe_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<RecipeIngredient>>> {
        if recipe_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // Build parameterized IN clause
        let placeholders: Vec<String> = (1..=recipe_ids.len()).map(|i| format!("${i}")).collect();
        let in_clause = placeholders.join(", ");

        let query = format!(
            r"
            SELECT id, recipe_id, fdc_id, name, amount, unit, grams, preparation, sort_order
            FROM recipe_ingredients
            WHERE recipe_id IN ({in_clause})
            ORDER BY recipe_id, sort_order
            "
        );

        let mut query_builder = sqlx::query(&query);
        for recipe_id in recipe_ids {
            query_builder = query_builder.bind(recipe_id);
        }

        let rows = query_builder
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to batch fetch ingredients: {e}")))?;

        let mut ingredients_map: HashMap<String, Vec<RecipeIngredient>> =
            HashMap::with_capacity(recipe_ids.len());

        for row in &rows {
            let recipe_id: String = row.get("recipe_id");
            ingredients_map
                .entry(recipe_id)
                .or_default()
                .push(row_to_ingredient_pg(row));
        }

        Ok(ingredients_map)
    }
}

// ============================================================================
// RecipeRepository Implementation
// ============================================================================

#[async_trait]
impl RecipeRepository for PostgresDatabase {
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<String> {
        let now = Utc::now();
        let recipe_id = recipe.id.to_string();
        let instructions_json = serde_json::to_string(&recipe.instructions)?;
        let tags_json = serde_json::to_string(&recipe.tags)?;

        // Begin transaction for atomic recipe + ingredients insertion
        let tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("Failed to begin transaction: {e}")))?;
        let mut guard = PostgresTransactionGuard::new(tx);

        // Insert recipe
        sqlx::query(
            r"
            INSERT INTO recipes (
                id, user_id, tenant_id, name, description, servings,
                prep_time_mins, cook_time_mins, instructions, tags, meal_timing,
                cached_calories, cached_protein_g, cached_carbs_g, cached_fat_g,
                cached_fiber_g, cached_sodium_mg, cached_sugar_g, nutrition_validated_at,
                created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                $12, $13, $14, $15, $16, $17, $18, $19, $20, $20
            )
            ",
        )
        .bind(&recipe_id)
        .bind(user_id)
        .bind(tenant_id)
        .bind(&recipe.name)
        .bind(&recipe.description)
        .bind(i32::from(recipe.servings))
        .bind(recipe.prep_time_mins.map(i32::from))
        .bind(recipe.cook_time_mins.map(i32::from))
        .bind(&instructions_json)
        .bind(&tags_json)
        .bind(meal_timing_to_string(recipe.meal_timing))
        .bind(recipe.nutrition.as_ref().map(|n| n.calories))
        .bind(recipe.nutrition.as_ref().map(|n| n.protein_g))
        .bind(recipe.nutrition.as_ref().map(|n| n.carbs_g))
        .bind(recipe.nutrition.as_ref().map(|n| n.fat_g))
        .bind(recipe.nutrition.as_ref().and_then(|n| n.fiber_g))
        .bind(recipe.nutrition.as_ref().and_then(|n| n.sodium_mg))
        .bind(recipe.nutrition.as_ref().and_then(|n| n.sugar_g))
        .bind(recipe.nutrition.as_ref().map(|n| n.validated_at))
        .bind(now)
        .execute(guard.executor()?)
        .await
        .map_err(|e| AppError::database(format!("Failed to create recipe: {e}")))?;

        // Insert ingredients within same transaction
        for (idx, ingredient) in recipe.ingredients.iter().enumerate() {
            let ingredient_id = Uuid::new_v4().to_string();
            // Sort order bounded by practical ingredient count (< 100)
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let sort_order = idx as i32;
            sqlx::query(
                r"
                INSERT INTO recipe_ingredients (
                    id, recipe_id, fdc_id, name, amount, unit, grams, preparation, sort_order
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ",
            )
            .bind(&ingredient_id)
            .bind(&recipe_id)
            .bind(ingredient.fdc_id)
            .bind(&ingredient.name)
            .bind(ingredient.amount)
            .bind(unit_to_string(ingredient.unit))
            .bind(ingredient.grams)
            .bind(&ingredient.preparation)
            .bind(sort_order)
            .execute(guard.executor()?)
            .await
            .map_err(|e| AppError::database(format!("Failed to create recipe ingredient: {e}")))?;
        }

        guard.commit().await?;

        Ok(recipe_id)
    }

    async fn get_by_id(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Recipe>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, name, description, servings,
                   prep_time_mins, cook_time_mins, instructions, tags, meal_timing,
                   cached_calories, cached_protein_g, cached_carbs_g, cached_fat_g,
                   cached_fiber_g, cached_sodium_mg, cached_sugar_g, nutrition_validated_at,
                   created_at, updated_at
            FROM recipes
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(recipe_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get recipe: {e}")))?;

        match row {
            Some(row) => {
                let ingredients = self.get_recipe_ingredients_pg(recipe_id).await?;
                Ok(Some(row_to_recipe_pg(&row, ingredients)?))
            }
            None => Ok(None),
        }
    }

    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        meal_timing: Option<MealTiming>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>> {
        let limit_val = i32::try_from(limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(offset.unwrap_or(0)).unwrap_or(0);

        let rows = if let Some(timing) = meal_timing {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, name, description, servings,
                       prep_time_mins, cook_time_mins, instructions, tags, meal_timing,
                       cached_calories, cached_protein_g, cached_carbs_g, cached_fat_g,
                       cached_fiber_g, cached_sodium_mg, cached_sugar_g, nutrition_validated_at,
                       created_at, updated_at
                FROM recipes
                WHERE user_id = $1 AND tenant_id = $2 AND meal_timing = $3
                ORDER BY updated_at DESC
                LIMIT $4 OFFSET $5
                ",
            )
            .bind(user_id)
            .bind(tenant_id)
            .bind(meal_timing_to_string(timing))
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list recipes: {e}")))?
        } else {
            sqlx::query(
                r"
                SELECT id, user_id, tenant_id, name, description, servings,
                       prep_time_mins, cook_time_mins, instructions, tags, meal_timing,
                       cached_calories, cached_protein_g, cached_carbs_g, cached_fat_g,
                       cached_fiber_g, cached_sodium_mg, cached_sugar_g, nutrition_validated_at,
                       created_at, updated_at
                FROM recipes
                WHERE user_id = $1 AND tenant_id = $2
                ORDER BY updated_at DESC
                LIMIT $3 OFFSET $4
                ",
            )
            .bind(user_id)
            .bind(tenant_id)
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list recipes: {e}")))?
        };

        // Batch fetch ingredients (2 queries instead of N+1)
        let recipe_ids: Vec<String> = rows.iter().map(|r| r.get("id")).collect();
        let mut ingredients_map = self.get_ingredients_batch_pg(&recipe_ids).await?;

        let mut recipes = Vec::with_capacity(rows.len());
        for row in &rows {
            let recipe_id: String = row.get("id");
            let ingredients = ingredients_map.remove(&recipe_id).unwrap_or_default();
            recipes.push(row_to_recipe_pg(row, ingredients)?);
        }

        Ok(recipes)
    }

    async fn update(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let instructions_json = serde_json::to_string(&recipe.instructions)?;
        let tags_json = serde_json::to_string(&recipe.tags)?;

        // Begin transaction for atomic update + ingredients replacement
        let tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("Failed to begin transaction: {e}")))?;
        let mut guard = PostgresTransactionGuard::new(tx);

        // Update recipe
        let result = sqlx::query(
            r"
            UPDATE recipes SET
                name = $1, description = $2, servings = $3,
                prep_time_mins = $4, cook_time_mins = $5,
                instructions = $6, tags = $7, meal_timing = $8,
                cached_calories = $9, cached_protein_g = $10, cached_carbs_g = $11,
                cached_fat_g = $12, cached_fiber_g = $13, cached_sodium_mg = $14,
                cached_sugar_g = $15, nutrition_validated_at = $16,
                updated_at = $17
            WHERE id = $18 AND user_id = $19 AND tenant_id = $20
            ",
        )
        .bind(&recipe.name)
        .bind(&recipe.description)
        .bind(i32::from(recipe.servings))
        .bind(recipe.prep_time_mins.map(i32::from))
        .bind(recipe.cook_time_mins.map(i32::from))
        .bind(&instructions_json)
        .bind(&tags_json)
        .bind(meal_timing_to_string(recipe.meal_timing))
        .bind(recipe.nutrition.as_ref().map(|n| n.calories))
        .bind(recipe.nutrition.as_ref().map(|n| n.protein_g))
        .bind(recipe.nutrition.as_ref().map(|n| n.carbs_g))
        .bind(recipe.nutrition.as_ref().map(|n| n.fat_g))
        .bind(recipe.nutrition.as_ref().and_then(|n| n.fiber_g))
        .bind(recipe.nutrition.as_ref().and_then(|n| n.sodium_mg))
        .bind(recipe.nutrition.as_ref().and_then(|n| n.sugar_g))
        .bind(recipe.nutrition.as_ref().map(|n| n.validated_at))
        .bind(now)
        .bind(recipe_id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(guard.executor()?)
        .await
        .map_err(|e| AppError::database(format!("Failed to update recipe: {e}")))?;

        if result.rows_affected() == 0 {
            return Ok(false);
        }

        // Delete existing ingredients
        sqlx::query("DELETE FROM recipe_ingredients WHERE recipe_id = $1")
            .bind(recipe_id)
            .execute(guard.executor()?)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete recipe ingredients: {e}")))?;

        // Insert updated ingredients
        for (idx, ingredient) in recipe.ingredients.iter().enumerate() {
            let ingredient_id = Uuid::new_v4().to_string();
            // Sort order bounded by practical ingredient count (< 100)
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let sort_order = idx as i32;
            sqlx::query(
                r"
                INSERT INTO recipe_ingredients (
                    id, recipe_id, fdc_id, name, amount, unit, grams, preparation, sort_order
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ",
            )
            .bind(&ingredient_id)
            .bind(recipe_id)
            .bind(ingredient.fdc_id)
            .bind(&ingredient.name)
            .bind(ingredient.amount)
            .bind(unit_to_string(ingredient.unit))
            .bind(ingredient.grams)
            .bind(&ingredient.preparation)
            .bind(sort_order)
            .execute(guard.executor()?)
            .await
            .map_err(|e| AppError::database(format!("Failed to update recipe ingredient: {e}")))?;
        }

        guard.commit().await?;

        Ok(true)
    }

    async fn delete(&self, recipe_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        // Ingredients are deleted via CASCADE
        let result = sqlx::query(
            r"
            DELETE FROM recipes
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(recipe_id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to delete recipe: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn update_nutrition_cache(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        nutrition: &ValidatedNutrition,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE recipes SET
                cached_calories = $1, cached_protein_g = $2, cached_carbs_g = $3,
                cached_fat_g = $4, cached_fiber_g = $5, cached_sodium_mg = $6,
                cached_sugar_g = $7, nutrition_validated_at = $8, updated_at = $9
            WHERE id = $10 AND user_id = $11 AND tenant_id = $12
            ",
        )
        .bind(nutrition.calories)
        .bind(nutrition.protein_g)
        .bind(nutrition.carbs_g)
        .bind(nutrition.fat_g)
        .bind(nutrition.fiber_g)
        .bind(nutrition.sodium_mg)
        .bind(nutrition.sugar_g)
        .bind(nutrition.validated_at)
        .bind(Utc::now())
        .bind(recipe_id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update nutrition cache: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>> {
        let limit_val = i32::try_from(limit.unwrap_or(20)).unwrap_or(20);
        let offset_val = i32::try_from(offset.unwrap_or(0)).unwrap_or(0);
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, name, description, servings,
                   prep_time_mins, cook_time_mins, instructions, tags, meal_timing,
                   cached_calories, cached_protein_g, cached_carbs_g, cached_fat_g,
                   cached_fiber_g, cached_sodium_mg, cached_sugar_g, nutrition_validated_at,
                   created_at, updated_at
            FROM recipes
            WHERE user_id = $1 AND tenant_id = $2 AND (
                name ILIKE $3 OR tags ILIKE $3 OR description ILIKE $3
            )
            ORDER BY updated_at DESC
            LIMIT $4 OFFSET $5
            ",
        )
        .bind(user_id)
        .bind(tenant_id)
        .bind(&search_pattern)
        .bind(limit_val)
        .bind(offset_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to search recipes: {e}")))?;

        // Batch fetch ingredients (2 queries instead of N+1)
        let recipe_ids: Vec<String> = rows.iter().map(|r| r.get("id")).collect();
        let mut ingredients_map = self.get_ingredients_batch_pg(&recipe_ids).await?;

        let mut recipes = Vec::with_capacity(rows.len());
        for row in &rows {
            let recipe_id: String = row.get("id");
            let ingredients = ingredients_map.remove(&recipe_id).unwrap_or_default();
            recipes.push(row_to_recipe_pg(row, ingredients)?);
        }

        Ok(recipes)
    }

    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as count FROM recipes
            WHERE user_id = $1 AND tenant_id = $2
            ",
        )
        .bind(user_id)
        .bind(tenant_id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to count recipes: {e}")))?;

        let count: i64 = row.get("count");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(count as u32)
    }
}
