// ABOUTME: Direct RecipeRepository impl on Database (SQLite recipe persistence)
// ABOUTME: Split out of repositories/direct_impls.rs to mirror per-domain PG backend shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! #[async_trait] impl RecipeRepository for Database — recipe CRUD with transactional ingredient inserts.

use super::RecipeRepository;
use crate::backends::shared::transactions::SqliteTransactionGuard;
use crate::database::recipes::{
    get_ingredients_batch, get_recipe_ingredients, meal_timing_to_string, row_to_recipe,
    unit_to_string,
};
use crate::database::Database;
use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::recipes::{MealTiming, Recipe, ValidatedNutrition};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

#[async_trait]
impl RecipeRepository for Database {
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<String> {
        let now = Utc::now().to_rfc3339();
        let recipe_id = recipe.id.to_string();
        let instructions_json = serde_json::to_string(&recipe.instructions)?;
        let tags_json = serde_json::to_string(&recipe.tags)?;

        // Begin transaction for atomic recipe + ingredients insertion
        let tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("Failed to begin transaction: {e}")))?;
        let mut guard = SqliteTransactionGuard::new(tx);

        // Insert recipe within transaction
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
        .bind(user_id.to_string())
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
        .bind(
            recipe
                .nutrition
                .as_ref()
                .map(|n| n.validated_at.to_rfc3339()),
        )
        .bind(&now)
        .execute(guard.executor()?)
        .await
        .map_err(|e| AppError::database(format!("Failed to create recipe: {e}")))?;

        // Insert ingredients within same transaction
        for (idx, ingredient) in recipe.ingredients.iter().enumerate() {
            let ingredient_id = Uuid::new_v4().to_string();
            // Sort order is bounded by practical recipe ingredient count (< 100)
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

        // Commit transaction - if not reached, guard will auto-rollback on drop
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
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get recipe: {e}")))?;

        match row {
            Some(row) => {
                let ingredients = get_recipe_ingredients(self.pool(), recipe_id).await?;
                Ok(Some(row_to_recipe(&row, ingredients)?))
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
            .bind(user_id.to_string())
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
            .bind(user_id.to_string())
            .bind(tenant_id)
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list recipes: {e}")))?
        };

        // Batch fetch ingredients (2 queries instead of N+1)
        let recipe_ids: Vec<String> = rows.iter().map(|r| r.get("id")).collect();
        let mut ingredients_map = get_ingredients_batch(self.pool(), &recipe_ids).await?;

        let mut recipes = Vec::with_capacity(rows.len());
        for row in rows {
            let recipe_id: String = row.get("id");
            let ingredients = ingredients_map.remove(&recipe_id).unwrap_or_default();
            recipes.push(row_to_recipe(&row, ingredients)?);
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
        let now = Utc::now().to_rfc3339();
        let instructions_json = serde_json::to_string(&recipe.instructions)?;
        let tags_json = serde_json::to_string(&recipe.tags)?;

        // Begin transaction for atomic update + ingredients replacement
        let tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("Failed to begin transaction: {e}")))?;
        let mut guard = SqliteTransactionGuard::new(tx);

        // Update recipe within transaction
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
        .bind(
            recipe
                .nutrition
                .as_ref()
                .map(|n| n.validated_at.to_rfc3339()),
        )
        .bind(&now)
        .bind(recipe_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .execute(guard.executor()?)
        .await
        .map_err(|e| AppError::database(format!("Failed to update recipe: {e}")))?;

        if result.rows_affected() == 0 {
            // Recipe not found - transaction will auto-rollback on guard drop
            return Ok(false);
        }

        // Delete existing ingredients within same transaction
        sqlx::query("DELETE FROM recipe_ingredients WHERE recipe_id = $1")
            .bind(recipe_id)
            .execute(guard.executor()?)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete recipe ingredients: {e}")))?;

        // Insert updated ingredients within same transaction
        for (idx, ingredient) in recipe.ingredients.iter().enumerate() {
            let ingredient_id = Uuid::new_v4().to_string();
            // Sort order is bounded by practical recipe ingredient count (< 100)
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

        // Commit transaction - if not reached, guard will auto-rollback on drop
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
        .bind(user_id.to_string())
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
        .bind(nutrition.validated_at.to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .bind(recipe_id)
        .bind(user_id.to_string())
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
                name LIKE $3 OR tags LIKE $3 OR description LIKE $3
            )
            ORDER BY updated_at DESC
            LIMIT $4 OFFSET $5
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(&search_pattern)
        .bind(limit_val)
        .bind(offset_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to search recipes: {e}")))?;

        // Batch fetch ingredients (2 queries instead of N+1)
        let recipe_ids: Vec<String> = rows.iter().map(|r| r.get("id")).collect();
        let mut ingredients_map = get_ingredients_batch(self.pool(), &recipe_ids).await?;

        let mut recipes = Vec::with_capacity(rows.len());
        for row in rows {
            let recipe_id: String = row.get("id");
            let ingredients = ingredients_map.remove(&recipe_id).unwrap_or_default();
            recipes.push(row_to_recipe(&row, ingredients)?);
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
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to count recipes: {e}")))?;

        let count: i64 = row.get("count");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(count as u32)
    }
}
