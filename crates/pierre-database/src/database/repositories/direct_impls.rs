// ABOUTME: Direct repository trait implementations on Database for various domain traits
// ABOUTME: SocialRepository has inline SQL; others delegate to domain managers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::{
    CoachesRepository, MobilityRepository, RecipeRepository, SocialRepository,
    StoreListingsRepository,
};
use crate::database::coaches::{
    compute_content_hash, compute_request_hash, row_to_coach, row_to_coach_list_item,
    row_to_coach_version,
};
use crate::database::mobility::{
    row_to_activity_muscle_mapping, row_to_stretching_exercise, row_to_yoga_pose,
};
use crate::database::recipes::{
    get_ingredients_batch, get_recipe_ingredients, meal_timing_to_string, row_to_recipe,
    unit_to_string,
};
use crate::database::store_listings::{
    row_to_coach_with_listing, row_to_store_listing, CoachWithListing, StoreListing, COACH_COLUMNS,
    COACH_COLUMNS_ALIASED, LISTING_COLUMNS_ALIASED,
};
use crate::database::Database;
use crate::plugins::shared::transactions::SqliteTransactionGuard;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::intelligence::InsightSharingPolicy;
use pierre_core::models::coaches::{
    Coach, CoachAssignment, CoachCategory, CoachFieldOverlay, CoachListItem, CoachPrerequisites,
    CoachVersion, CoachVisibility, CreateCoachRequest, CreateSystemCoachRequest, ListCoachesFilter,
    PublishStatus, StoreAdminStats, UpdateCoachRequest,
};
use pierre_core::models::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};
use pierre_core::models::recipes::{MealTiming, Recipe, ValidatedNutrition};
use pierre_core::models::CoachRuntimeContext;
use pierre_core::models::TenantId;
use pierre_core::models::{
    AdaptedInsight, FriendConnection, FriendStatus, InsightReaction, NotificationPreferences,
    SharedInsight, TrainingPhase, UserSocialSettings,
};
use pierre_core::pagination::{Cursor, CursorPage, StoreCursor, StoreSortOrder};
use pierre_core::tokens::estimate_prompt_tokens;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::fmt::Write as _;
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

// ============================================================================
// CoachesRepository — helper functions
// ============================================================================

/// Ensure a `coach_assignments` row exists for a user+coach pair.
///
/// Uses `INSERT OR IGNORE` so it is safe to call multiple times.
/// Needed for operations like `toggle_favorite`, `record_usage`,
/// and `activate_coach` that need an assignment row to update.
async fn ensure_assignment_exists(
    pool: &SqlitePool,
    coach_id: &str,
    user_id: Uuid,
) -> AppResult<()> {
    let id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        r"
        INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
        VALUES ($1, $2, $3, $3, $4, 0, 0, 0, NULL)
        ",
    )
    .bind(id.to_string())
    .bind(coach_id)
    .bind(user_id.to_string())
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to ensure coach assignment: {e}")))?;

    Ok(())
}

/// Check if a coach can be hidden by a user.
///
/// A coach is hideable if it's a system coach or assigned to the user,
/// but NOT if it's a personal coach created by the user.
async fn is_coach_hideable(pool: &SqlitePool, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
    let is_system = sqlx::query("SELECT 1 FROM coaches WHERE id = $1 AND is_system = 1")
        .bind(coach_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check system coach: {e}")))?
        .is_some();

    if is_system {
        return Ok(true);
    }

    let is_assigned =
        sqlx::query("SELECT 1 FROM coach_assignments WHERE coach_id = $1 AND user_id = $2")
            .bind(coach_id)
            .bind(user_id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?
            .is_some();

    Ok(is_assigned)
}

// ============================================================================
// CoachesRepository
// ============================================================================

#[async_trait]
impl CoachesRepository for Database {
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateCoachRequest,
    ) -> AppResult<Coach> {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&request.tags)?;
        let sample_prompts_json = serde_json::to_string(&request.sample_prompts)?;

        let effective_system_prompt = request
            .instructions
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&request.system_prompt);

        let coach_for_tokens = Coach {
            id,
            user_id,
            tenant_id: tenant_id.to_string(),
            title: request.title.clone(),
            description: request.description.clone(),
            system_prompt: effective_system_prompt.to_owned(),
            category: request.category,
            tags: request.tags.clone(),
            sample_prompts: request.sample_prompts.clone(),
            token_count: 0,
            created_at: now,
            updated_at: now,
            is_system: false,
            visibility: CoachVisibility::Private,
            prerequisites: CoachPrerequisites::default(),
            forked_from: None,
            max_tool_iterations: None,
            temperature: None,
            startup_query: request.startup_query.clone(),
            data_requirements: request.data_requirements.clone(),
            purpose: request.purpose.clone(),
            when_to_use: request.when_to_use.clone(),
            instructions: request.instructions.clone(),
            example_inputs: request.example_inputs.clone(),
            example_outputs: request.example_outputs.clone(),
            success_criteria: request.success_criteria.clone(),
            source: "custom".to_owned(),
        };
        let token_count = coach_for_tokens.compute_token_count();

        let data_requirements_json = request
            .data_requirements
            .as_ref()
            .and_then(|dr| serde_json::to_string(dr).ok());
        let content_hash = compute_request_hash(request);

        sqlx::query(
            r"
            INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria,
                content_hash
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)
            ",
        )
        .bind(id.to_string()).bind(user_id.to_string()).bind(tenant_id)
        .bind(&request.title).bind(&request.description).bind(effective_system_prompt)
        .bind(request.category.as_str()).bind(&tags_json).bind(&sample_prompts_json)
        .bind(i64::from(token_count)).bind(now.to_rfc3339())
        .bind(0i64).bind(CoachVisibility::Private.as_str())
        .bind(Option::<String>::None).bind(Option::<String>::None).bind(Option::<i32>::None).bind(Option::<f32>::None)
        .bind(&request.startup_query).bind(&data_requirements_json)
        .bind(&request.purpose).bind(&request.when_to_use).bind(&request.instructions)
        .bind(&request.example_inputs).bind(&request.example_outputs)
        .bind(&request.success_criteria).bind(&content_hash)
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to create coach: {e}")))?;

        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, 0, NULL)
            ",
        )
        .bind(assignment_id.to_string()).bind(id.to_string())
        .bind(user_id.to_string()).bind(now.to_rfc3339())
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to create coach assignment: {e}")))?;

        Ok(Coach {
            id,
            user_id,
            tenant_id: tenant_id.to_string(),
            title: request.title.clone(),
            description: request.description.clone(),
            system_prompt: effective_system_prompt.to_owned(),
            category: request.category,
            tags: request.tags.clone(),
            sample_prompts: request.sample_prompts.clone(),
            token_count,
            created_at: now,
            updated_at: now,
            is_system: false,
            visibility: CoachVisibility::Private,
            prerequisites: CoachPrerequisites::default(),
            forked_from: None,
            max_tool_iterations: None,
            temperature: None,
            startup_query: request.startup_query.clone(),
            data_requirements: request.data_requirements.clone(),
            purpose: request.purpose.clone(),
            when_to_use: request.when_to_use.clone(),
            instructions: request.instructions.clone(),
            example_inputs: request.example_inputs.clone(),
            example_outputs: request.example_outputs.clone(),
            success_criteria: request.success_criteria.clone(),
            source: "custom".to_owned(),
        })
    }

    async fn get_by_id(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE id = $1 AND (
                (user_id = $2 AND tenant_id = $3)
                OR is_system = 1
                OR id IN (SELECT coach_id FROM coach_assignments WHERE user_id = $2)
            )",
        )
        .bind(coach_id).bind(user_id.to_string()).bind(tenant_id)
        .fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get coach: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListCoachesFilter,
    ) -> AppResult<Vec<CoachListItem>> {
        let limit_val = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);
        let user_id_str = user_id.to_string();
        let category_filter = filter
            .category
            .as_ref()
            .map(|c| format!("AND c.category = '{}'", c.as_str()))
            .unwrap_or_default();
        let favorites_filter = if filter.favorites_only {
            "AND ca.is_favorite = 1"
        } else {
            ""
        };
        let hidden_filter = if filter.include_hidden {
            ""
        } else {
            "AND c.id NOT IN (SELECT coach_id FROM user_coach_preferences WHERE user_id = $1 AND is_hidden = 1)"
        };
        let system_condition = if filter.include_system {
            "OR c.is_system = 1"
        } else {
            ""
        };

        let query = format!(
            r"SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.max_tool_iterations, c.temperature, c.startup_query, c.data_requirements,
                   c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria,
                   CASE WHEN ca.coach_id IS NOT NULL THEN 1 ELSE 0 END as is_assigned,
                   COALESCE(ca.is_favorite, 0) as is_favorite,
                   COALESCE(ca.is_active, 0) as is_active,
                   COALESCE(ca.use_count, 0) as use_count,
                   ca.last_used_at
            FROM coaches c
            LEFT JOIN coach_assignments ca ON c.id = ca.coach_id AND ca.user_id = $1
            WHERE (
                (c.user_id = $1 AND c.is_system = 0 AND c.tenant_id = $2)
                {system_condition}
                OR c.id IN (SELECT coach_id FROM coach_assignments WHERE user_id = $1)
            )
            {category_filter} {favorites_filter} {hidden_filter}
            ORDER BY c.updated_at DESC LIMIT $3 OFFSET $4"
        );
        let rows = sqlx::query(&query)
            .bind(&user_id_str)
            .bind(tenant_id)
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list coaches: {e}")))?;
        rows.iter().map(row_to_coach_list_item).collect()
    }

    async fn apply_translations(
        &self,
        coaches: &mut [CoachListItem],
        locale: &str,
    ) -> AppResult<()> {
        // English is canonical — skip the round-trip entirely.
        if locale == "en" || coaches.is_empty() {
            return Ok(());
        }

        // SQLite lacks `= ANY($2)`; build an IN list with one bind per coach.
        // Coach counts per turn are bounded by ListCoachesFilter::limit (default
        // 50) so the placeholder growth stays well under SQLite's 32766 cap.
        let mut query_str = String::from(
            "SELECT coach_id, title, description, purpose, instructions \
             FROM coach_translations WHERE locale = ?1 AND coach_id IN (",
        );
        for i in 0..coaches.len() {
            if i > 0 {
                query_str.push(',');
            }
            // SQLite positional binds are 1-indexed; reserve slot 1 for locale.
            let _ = write!(query_str, "?{}", i + 2);
        }
        query_str.push(')');

        let mut q = sqlx::query(&query_str).bind(locale);
        for item in coaches.iter() {
            q = q.bind(item.coach.id.to_string());
        }
        let rows = q
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to load coach translations: {e}")))?;

        if rows.is_empty() {
            return Ok(());
        }

        let mut overlays: HashMap<String, CoachFieldOverlay> = HashMap::with_capacity(rows.len());
        for row in &rows {
            let id: String = row.get("coach_id");
            overlays.insert(
                id,
                CoachFieldOverlay {
                    title: row.try_get("title").ok(),
                    description: row.try_get("description").ok(),
                    purpose: row.try_get("purpose").ok(),
                    instructions: row.try_get("instructions").ok(),
                },
            );
        }

        for item in coaches.iter_mut() {
            if let Some(ov) = overlays.get(&item.coach.id.to_string()) {
                ov.apply(&mut item.coach);
            }
        }
        Ok(())
    }

    async fn update(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        let existing = CoachesRepository::get_by_id(self, coach_id, user_id, tenant_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        self.create_version(coach_id, user_id, None).await?;

        let now = Utc::now();
        let title = request.title.as_ref().unwrap_or(&existing.title);
        let description = request.description.clone().or(existing.description);
        let system_prompt = request
            .system_prompt
            .as_ref()
            .unwrap_or(&existing.system_prompt);
        let category = request.category.unwrap_or(existing.category);
        let tags = request.tags.as_ref().unwrap_or(&existing.tags);
        let sample_prompts = request
            .sample_prompts
            .as_ref()
            .unwrap_or(&existing.sample_prompts);
        let tags_json = serde_json::to_string(tags)?;
        let sample_prompts_json = serde_json::to_string(sample_prompts)?;
        let token_count = estimate_prompt_tokens(system_prompt);

        let startup_query: Option<String> = if request.startup_query.is_some() {
            request
                .startup_query
                .as_ref()
                .filter(|q| !q.is_empty())
                .cloned()
        } else {
            let existing_row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT startup_query FROM coaches WHERE id = $1")
                    .bind(coach_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get startup_query: {e}")))?;
            existing_row.and_then(|(q,)| q)
        };
        let data_requirements_json: Option<String> = if request.data_requirements.is_some() {
            request
                .data_requirements
                .as_ref()
                .and_then(|dr| serde_json::to_string(dr).ok())
        } else {
            let existing_row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT data_requirements FROM coaches WHERE id = $1")
                    .bind(coach_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get data_requirements: {e}"))
                    })?;
            existing_row.and_then(|(dr,)| dr)
        };

        let purpose = request.purpose.clone().or(existing.purpose);
        let when_to_use = request.when_to_use.clone().or(existing.when_to_use);
        let instructions = request.instructions.clone().or(existing.instructions);
        let example_inputs = request.example_inputs.clone().or(existing.example_inputs);
        let example_outputs = request.example_outputs.clone().or(existing.example_outputs);
        let success_criteria = request
            .success_criteria
            .clone()
            .or(existing.success_criteria);
        let system_prompt = instructions
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(system_prompt);

        let result = sqlx::query(
            r"UPDATE coaches SET
                title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8,
                startup_query = $12, data_requirements = $13,
                purpose = $14, when_to_use = $15, instructions = $16,
                example_inputs = $17, example_outputs = $18, success_criteria = $19
            WHERE id = $9 AND user_id = $10 AND tenant_id = $11",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(token_count))
        .bind(now.to_rfc3339())
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(&startup_query)
        .bind(&data_requirements_json)
        .bind(&purpose)
        .bind(&when_to_use)
        .bind(&instructions)
        .bind(&example_inputs)
        .bind(&example_outputs)
        .bind(&success_criteria)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }
        CoachesRepository::get_by_id(self, coach_id, user_id, tenant_id).await
    }

    async fn delete(&self, coach_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        let result =
            sqlx::query("DELETE FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3")
                .bind(coach_id)
                .bind(user_id.to_string())
                .bind(tenant_id)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to delete coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn record_usage(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let exists = sqlx::query("SELECT 1 FROM coaches WHERE id = $1 AND tenant_id = $2")
            .bind(coach_id)
            .bind(tenant_id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
        if exists.is_none() {
            return Ok(false);
        }
        ensure_assignment_exists(self.pool(), coach_id, user_id).await?;
        let result = sqlx::query(
            "UPDATE coach_assignments SET use_count = use_count + 1, last_used_at = $1 WHERE coach_id = $2 AND user_id = $3",
        ).bind(&now).bind(coach_id).bind(user_id.to_string())
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to record coach usage: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn toggle_favorite(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<bool>> {
        let coach_exists = sqlx::query("SELECT 1 FROM coaches WHERE id = $1 AND tenant_id = $2")
            .bind(coach_id)
            .bind(tenant_id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
        if coach_exists.is_none() {
            return Ok(None);
        }
        ensure_assignment_exists(self.pool(), coach_id, user_id).await?;
        let row = sqlx::query("SELECT ca.is_favorite FROM coach_assignments ca WHERE ca.coach_id = $1 AND ca.user_id = $2")
            .bind(coach_id).bind(user_id.to_string()).fetch_optional(self.pool()).await
            .map_err(|e| AppError::database(format!("Failed to get favorite status: {e}")))?;
        let current: i64 = row.map_or(0, |r| r.get("is_favorite"));
        let new_value = i64::from(current != 1);
        sqlx::query(
            "UPDATE coach_assignments SET is_favorite = $1 WHERE coach_id = $2 AND user_id = $3",
        )
        .bind(new_value)
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to toggle favorite: {e}")))?;
        Ok(Some(new_value == 1))
    }

    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Coach>> {
        let limit_val = i32::try_from(limit.unwrap_or(20)).unwrap_or(20);
        let offset_val = i32::try_from(offset.unwrap_or(0)).unwrap_or(0);
        let search_pattern = format!("%{query}%");
        let rows = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches
            WHERE user_id = $1 AND tenant_id = $2 AND (title LIKE $3 OR description LIKE $3 OR tags LIKE $3)
            ORDER BY updated_at DESC LIMIT $4 OFFSET $5",
        ).bind(user_id.to_string()).bind(tenant_id).bind(&search_pattern)
        .bind(limit_val).bind(offset_val).fetch_all(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to search coaches: {e}")))?;
        rows.iter().map(row_to_coach).collect()
    }

    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        let row = sqlx::query(
            "SELECT COUNT(*) as count FROM coaches WHERE user_id = $1 AND tenant_id = $2",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to count coaches: {e}")))?;
        let count: i64 = row.get("count");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(count as u32)
    }

    // -- User methods --

    async fn fork_coach(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        let source = self
            .get_system_coach_any_tenant(source_coach_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("System coach {source_coach_id}")))?;
        if !source.is_system {
            return Err(AppError::invalid_input(
                "Only system coaches can be forked. Use duplicate for personal coaches.",
            ));
        }
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&source.tags)?;
        let sample_prompts_json = serde_json::to_string(&source.sample_prompts)?;
        let prerequisites_json = serde_json::to_string(&source.prerequisites)?;
        let source_data_requirements_json = source
            .data_requirements
            .as_ref()
            .and_then(|dr| serde_json::to_string(dr).ok());

        sqlx::query(
            r"INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25)",
        )
        .bind(id.to_string()).bind(user_id.to_string()).bind(tenant_id)
        .bind(&source.title).bind(&source.description).bind(&source.system_prompt)
        .bind(source.category.as_str()).bind(&tags_json).bind(&sample_prompts_json)
        .bind(i64::from(source.token_count)).bind(now.to_rfc3339())
        .bind(0i64).bind(CoachVisibility::Private.as_str())
        .bind(&prerequisites_json).bind(source_coach_id)
        .bind(source.max_tool_iterations).bind(source.temperature).bind(&source.startup_query)
        .bind(&source_data_requirements_json)
        .bind(&source.purpose).bind(&source.when_to_use).bind(&source.instructions)
        .bind(&source.example_inputs).bind(&source.example_outputs).bind(&source.success_criteria)
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to fork coach: {e}")))?;

        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"INSERT INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, 0, NULL)",
        )
        .bind(assignment_id.to_string()).bind(id.to_string())
        .bind(user_id.to_string()).bind(now.to_rfc3339())
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to create coach assignment: {e}")))?;

        Ok(Coach {
            id,
            user_id,
            tenant_id: tenant_id.to_string(),
            title: source.title,
            description: source.description,
            system_prompt: source.system_prompt,
            category: source.category,
            tags: source.tags,
            sample_prompts: source.sample_prompts,
            token_count: source.token_count,
            created_at: now,
            updated_at: now,
            is_system: false,
            visibility: CoachVisibility::Private,
            prerequisites: source.prerequisites,
            forked_from: Some(source_coach_id.to_owned()),
            max_tool_iterations: source.max_tool_iterations,
            temperature: source.temperature,
            startup_query: source.startup_query,
            data_requirements: source.data_requirements,
            purpose: source.purpose,
            when_to_use: source.when_to_use,
            instructions: source.instructions,
            example_inputs: source.example_inputs,
            example_outputs: source.example_outputs,
            success_criteria: source.success_criteria,
            source: "custom".to_owned(),
        })
    }

    async fn activate_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let coach_exists = sqlx::query("SELECT 1 FROM coaches WHERE id = $1 AND tenant_id = $2")
            .bind(coach_id)
            .bind(tenant_id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
        if coach_exists.is_none() {
            return Ok(None);
        }
        ensure_assignment_exists(self.pool(), coach_id, user_id).await?;
        sqlx::query("UPDATE coach_assignments SET is_active = 0 WHERE user_id = $1")
            .bind(user_id.to_string())
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to deactivate coaches: {e}")))?;
        sqlx::query(
            "UPDATE coach_assignments SET is_active = 1 WHERE coach_id = $1 AND user_id = $2",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to activate coach: {e}")))?;
        CoachesRepository::get_by_id(self, coach_id, user_id, tenant_id).await
    }

    async fn deactivate_coach(&self, user_id: Uuid, _tenant_id: TenantId) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE coach_assignments SET is_active = 0 WHERE user_id = $1 AND is_active = 1",
        )
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn get_active_coach(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.max_tool_iterations, c.temperature, c.startup_query, c.data_requirements,
                   c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria
            FROM coaches c
            JOIN coach_assignments ca ON c.id = ca.coach_id AND ca.user_id = $1
            WHERE ca.is_active = 1 AND c.tenant_id = $2",
        ).bind(user_id.to_string()).bind(tenant_id).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get active coach: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    async fn find_by_content_hash(
        &self,
        content_hash: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE content_hash = $1 AND user_id = $2 AND tenant_id = $3 LIMIT 1",
        ).bind(content_hash).bind(user_id.to_string()).bind(tenant_id)
        .fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to find coach by content hash: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    // -- Admin methods --

    async fn create_system_coach(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemCoachRequest,
    ) -> AppResult<Coach> {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&request.tags)?;
        let sample_prompts_json = serde_json::to_string(&request.sample_prompts)?;
        let token_count = estimate_prompt_tokens(&request.system_prompt);
        sqlx::query(
            r"INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, temperature, startup_query, data_requirements
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19)",
        )
        .bind(id.to_string()).bind(admin_user_id.to_string()).bind(tenant_id)
        .bind(&request.title).bind(&request.description).bind(&request.system_prompt)
        .bind(request.category.as_str()).bind(&tags_json).bind(&sample_prompts_json)
        .bind(i64::from(token_count)).bind(now.to_rfc3339())
        .bind(1i64).bind(request.visibility.as_str())
        .bind(Option::<String>::None).bind(Option::<String>::None)
        .bind(Option::<i32>::None).bind(Option::<f32>::None).bind(Option::<String>::None).bind(Option::<String>::None)
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to create system coach: {e}")))?;

        Ok(Coach {
            id,
            user_id: admin_user_id,
            tenant_id: tenant_id.to_string(),
            title: request.title.clone(),
            description: request.description.clone(),
            system_prompt: request.system_prompt.clone(),
            category: request.category,
            tags: request.tags.clone(),
            sample_prompts: request.sample_prompts.clone(),
            token_count,
            created_at: now,
            updated_at: now,
            is_system: true,
            visibility: request.visibility,
            prerequisites: CoachPrerequisites::default(),
            forked_from: None,
            max_tool_iterations: None,
            temperature: None,
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            source: "custom".to_owned(),
        })
    }

    async fn list_system_coaches(&self, tenant_id: TenantId) -> AppResult<Vec<Coach>> {
        let rows = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE tenant_id = $1 AND is_system = 1 ORDER BY created_at DESC",
        ).bind(tenant_id).fetch_all(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to list system coaches: {e}")))?;
        rows.iter().map(row_to_coach).collect()
    }

    async fn get_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE id = $1 AND tenant_id = $2 AND is_system = 1",
        ).bind(coach_id).bind(tenant_id).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get system coach: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    async fn get_system_coach_any_tenant(&self, coach_id: &str) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE id = $1 AND is_system = 1",
        ).bind(coach_id).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get system coach: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    async fn update_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        let existing = self.get_system_coach(coach_id, tenant_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        self.create_version(coach_id, existing.user_id, None)
            .await?;
        let now = Utc::now();
        let title = request.title.as_ref().unwrap_or(&existing.title);
        let description = request.description.clone().or(existing.description);
        let system_prompt = request
            .system_prompt
            .as_ref()
            .unwrap_or(&existing.system_prompt);
        let category = request.category.unwrap_or(existing.category);
        let tags = request.tags.as_ref().unwrap_or(&existing.tags);
        let sample_prompts = request
            .sample_prompts
            .as_ref()
            .unwrap_or(&existing.sample_prompts);
        let tags_json = serde_json::to_string(tags)?;
        let sample_prompts_json = serde_json::to_string(sample_prompts)?;
        let token_count = estimate_prompt_tokens(system_prompt);
        let result = sqlx::query(
            r"UPDATE coaches SET title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
            WHERE id = $9 AND tenant_id = $10 AND is_system = 1",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(token_count))
        .bind(now.to_rfc3339())
        .bind(coach_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update system coach: {e}")))?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_system_coach(coach_id, tenant_id).await
    }

    async fn delete_system_coach(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let result =
            sqlx::query("DELETE FROM coaches WHERE id = $1 AND tenant_id = $2 AND is_system = 1")
                .bind(coach_id)
                .bind(tenant_id)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to delete system coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    // -- Assignment methods --

    async fn get_user_preferences(
        &self,
        coach_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, bool, u32, Option<DateTime<Utc>>)> {
        let row = sqlx::query(
            "SELECT is_favorite, is_active, use_count, last_used_at FROM coach_assignments WHERE coach_id = $1 AND user_id = $2",
        ).bind(coach_id).bind(user_id.to_string()).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get user preferences: {e}")))?;
        row.map_or(Ok((false, false, 0, None)), |r| {
            let is_favorite: i64 = r.get("is_favorite");
            let is_active: i64 = r.get("is_active");
            let use_count: i64 = r.get("use_count");
            let last_used_at_str: Option<String> = r.get("last_used_at");
            let last_used_at = last_used_at_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok((
                is_favorite == 1,
                is_active == 1,
                use_count as u32,
                last_used_at,
            ))
        })
    }

    async fn assign_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $4, $5, 0, 0, 0, NULL)",
        ).bind(id.to_string()).bind(coach_id).bind(user_id.to_string())
        .bind(assigned_by.to_string()).bind(&now).execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to assign coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn unassign_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        let result =
            sqlx::query("DELETE FROM coach_assignments WHERE coach_id = $1 AND user_id = $2")
                .bind(coach_id)
                .bind(user_id.to_string())
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to unassign coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_assignments(&self, coach_id: &str) -> AppResult<Vec<CoachAssignment>> {
        let rows = sqlx::query(
            r"SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
            FROM coach_assignments ca LEFT JOIN users u ON ca.user_id = u.id
            WHERE ca.coach_id = $1 ORDER BY ca.created_at DESC",
        )
        .bind(coach_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;
        rows.iter()
            .map(|row| {
                Ok(CoachAssignment {
                    user_id: row.get("user_id"),
                    user_email: row.get("email"),
                    assigned_at: row.get("created_at"),
                    assigned_by: row.get("assigned_by"),
                })
            })
            .collect()
    }

    async fn list_assignments_for_tenant(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAssignment>> {
        let rows = sqlx::query(
            r"SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
            FROM coach_assignments ca LEFT JOIN users u ON ca.user_id = u.id
            INNER JOIN tenant_users tu ON ca.user_id = tu.user_id AND tu.tenant_id = $2
            WHERE ca.coach_id = $1 ORDER BY ca.created_at DESC",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;
        rows.iter()
            .map(|row| {
                Ok(CoachAssignment {
                    user_id: row.get("user_id"),
                    user_email: row.get("email"),
                    assigned_at: row.get("created_at"),
                    assigned_by: row.get("assigned_by"),
                })
            })
            .collect()
    }

    async fn hide_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        if !is_coach_hideable(self.pool(), coach_id, user_id).await? {
            return Err(AppError::invalid_input(
                "Only system or assigned coaches can be hidden",
            ));
        }
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r"INSERT INTO user_coach_preferences (id, user_id, coach_id, is_hidden, created_at)
            VALUES ($1, $2, $3, 1, $4) ON CONFLICT(user_id, coach_id) DO UPDATE SET is_hidden = 1",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(coach_id)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to hide coach: {e}")))?;
        Ok(true)
    }

    async fn show_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        let result = sqlx::query(
            "DELETE FROM user_coach_preferences WHERE coach_id = $1 AND user_id = $2 AND is_hidden = 1",
        ).bind(coach_id).bind(user_id.to_string()).execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to show coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_hidden_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        let rows = sqlx::query(
            r"SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.max_tool_iterations, c.temperature
            FROM coaches c
            INNER JOIN user_coach_preferences ucp ON c.id = ucp.coach_id
            WHERE ucp.user_id = $1 AND ucp.is_hidden = 1 AND c.tenant_id = $2
            ORDER BY c.title",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list hidden coaches: {e}")))?;
        rows.iter().map(row_to_coach).collect()
    }

    // -- Version methods --

    async fn create_version(
        &self,
        coach_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE id = $1",
        ).bind(coach_id).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get coach for versioning: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;
        let coach = row_to_coach(&row)?;

        let version_row = sqlx::query(
            "SELECT COALESCE(MAX(version), 0) as max_version FROM coach_versions WHERE coach_id = $1",
        ).bind(coach_id).fetch_one(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get max version: {e}")))?;
        let max_version: i32 = version_row.get("max_version");
        let new_version = max_version + 1;

        let content_snapshot = serde_json::json!({
            "title": coach.title, "description": coach.description,
            "system_prompt": coach.system_prompt, "category": coach.category.as_str(),
            "tags": coach.tags, "sample_prompts": coach.sample_prompts,
            "token_count": coach.token_count, "visibility": coach.visibility.as_str(),
            "prerequisites": coach.prerequisites,
        });
        let content_hash = compute_content_hash(&content_snapshot);
        let id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            r"INSERT INTO coach_versions (id, coach_id, version, content_hash, content_snapshot, change_summary, created_at, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        ).bind(id.to_string()).bind(coach_id).bind(new_version)
        .bind(&content_hash).bind(content_snapshot.to_string())
        .bind(change_summary).bind(now.to_rfc3339()).bind(user_id.to_string())
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to create version: {e}")))?;
        Ok(new_version)
    }

    async fn get_versions(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<CoachVersion>> {
        let exists = sqlx::query("SELECT 1 FROM coaches WHERE id = $1 AND tenant_id = $2")
            .bind(coach_id)
            .bind(tenant_id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
        if exists.is_none() {
            return Err(AppError::not_found(format!("Coach {coach_id}")));
        }
        let limit_val = i32::try_from(limit).unwrap_or(50);
        let rows = sqlx::query(
            r"SELECT cv.id, cv.coach_id, cv.version, cv.content_hash, cv.content_snapshot,
                   cv.change_summary, cv.created_at, cv.created_by
            FROM coach_versions cv WHERE cv.coach_id = $1 ORDER BY cv.version DESC LIMIT $2",
        )
        .bind(coach_id)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get versions: {e}")))?;
        rows.iter().map(row_to_coach_version).collect()
    }

    async fn get_version(
        &self,
        coach_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachVersion>> {
        let exists = sqlx::query("SELECT 1 FROM coaches WHERE id = $1 AND tenant_id = $2")
            .bind(coach_id)
            .bind(tenant_id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
        if exists.is_none() {
            return Err(AppError::not_found(format!("Coach {coach_id}")));
        }
        let row = sqlx::query(
            r"SELECT id, coach_id, version, content_hash, content_snapshot, change_summary, created_at, created_by
            FROM coach_versions WHERE coach_id = $1 AND version = $2",
        ).bind(coach_id).bind(version).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get version: {e}")))?;
        row.map(|r| row_to_coach_version(&r)).transpose()
    }

    async fn revert_to_version(
        &self,
        coach_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        let target_version = self
            .get_version(coach_id, version, tenant_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found(format!("Version {version} for coach {coach_id}"))
            })?;
        let snapshot = &target_version.content_snapshot;
        let title = snapshot
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::internal("Missing title in version snapshot"))?;
        let description = snapshot
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);
        let system_prompt = snapshot
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::internal("Missing system_prompt in version snapshot"))?;
        let category_str = snapshot
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("custom");
        let tags: Vec<String> = snapshot
            .get("tags")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let sample_prompts: Vec<String> = snapshot
            .get("sample_prompts")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let now = Utc::now();
        let tags_json = serde_json::to_string(&tags)?;
        let sample_prompts_json = serde_json::to_string(&sample_prompts)?;
        let token_count = estimate_prompt_tokens(system_prompt);

        let result = sqlx::query(
            r"UPDATE coaches SET title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
            WHERE id = $9 AND tenant_id = $10",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category_str)
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(token_count))
        .bind(now.to_rfc3339())
        .bind(coach_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to revert coach: {e}")))?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("Coach {coach_id}")));
        }

        let revert_summary = format!("Reverted to version {version}");
        self.create_version(coach_id, user_id, Some(&revert_summary))
            .await?;

        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE id = $1 AND tenant_id = $2",
        ).bind(coach_id).bind(tenant_id).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get reverted coach: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;
        row_to_coach(&row)
    }

    async fn get_current_version(&self, coach_id: &str) -> AppResult<i32> {
        let row = sqlx::query(
            "SELECT COALESCE(MAX(version), 0) as current_version FROM coach_versions WHERE coach_id = $1",
        ).bind(coach_id).fetch_one(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get current version: {e}")))?;
        Ok(row.get("current_version"))
    }

    async fn get_coach_runtime_context(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachRuntimeContext>> {
        type Row = (
            String,
            Option<String>,
            Option<String>,
            Option<i32>,
            Option<f32>,
            String,
        );
        let row: Option<Row> = sqlx::query_as(
            r"SELECT system_prompt, startup_query, data_requirements, max_tool_iterations, temperature, category
            FROM coaches WHERE id = $1 AND (tenant_id = $2 OR is_system = 1) LIMIT 1",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach runtime context: {e}")))?;
        Ok(row.map(
            |(
                system_prompt,
                startup_query,
                data_requirements,
                max_tool_iterations,
                temperature,
                category,
            )| {
                CoachRuntimeContext {
                    system_prompt,
                    startup_query,
                    data_requirements,
                    max_tool_iterations,
                    temperature,
                    category: CoachCategory::parse(&category),
                }
            },
        ))
    }
}

// ============================================================================
// MobilityRepository
// ============================================================================

#[async_trait]
impl MobilityRepository for Database {
    async fn get_stretching_exercise(&self, id: &str) -> AppResult<Option<StretchingExercise>> {
        let row = sqlx::query(
            r"
            SELECT id, name, description, category, difficulty,
                   primary_muscles, secondary_muscles, duration_seconds,
                   repetitions, sets, recommended_for_activities, contraindications,
                   instructions, cues, image_url, video_url, created_at, updated_at
            FROM stretching_exercises
            WHERE id = $1
            ",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get stretching exercise: {e}")))?;

        row.map(|r| row_to_stretching_exercise(&r)).transpose()
    }

    async fn list_stretching_exercises(
        &self,
        filter: &ListStretchingFilter,
    ) -> AppResult<Vec<StretchingExercise>> {
        let limit_val = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);

        // Build dynamic query with parameterized conditions to prevent SQL injection
        let mut conditions = Vec::new();
        let mut bind_values: Vec<String> = Vec::new();

        if let Some(ref cat) = filter.category {
            conditions.push("category = ?".to_owned());
            bind_values.push(cat.as_str().to_owned());
        }
        if let Some(ref diff) = filter.difficulty {
            conditions.push("difficulty = ?".to_owned());
            bind_values.push(diff.as_str().to_owned());
        }
        if let Some(ref muscle) = filter.muscle_group {
            conditions.push("(primary_muscles LIKE ? OR secondary_muscles LIKE ?)".to_owned());
            let pattern = format!("%\"{muscle}\"");
            bind_values.push(pattern.clone());
            bind_values.push(pattern);
        }
        if let Some(ref activity) = filter.activity_type {
            conditions.push("recommended_for_activities LIKE ?".to_owned());
            bind_values.push(format!("%\"{activity}\""));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r"
            SELECT id, name, description, category, difficulty,
                   primary_muscles, secondary_muscles, duration_seconds,
                   repetitions, sets, recommended_for_activities, contraindications,
                   instructions, cues, image_url, video_url, created_at, updated_at
            FROM stretching_exercises
            {where_clause}
            ORDER BY name ASC
            LIMIT ? OFFSET ?
            "
        );

        let mut sql_query = sqlx::query(&query);
        for value in &bind_values {
            sql_query = sql_query.bind(value);
        }
        sql_query = sql_query.bind(limit_val).bind(offset_val);

        let rows = sql_query
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list stretching exercises: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    async fn search_stretching_exercises(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>> {
        let limit_val = i32::try_from(limit.unwrap_or(20)).unwrap_or(20);
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(
            r"
            SELECT id, name, description, category, difficulty,
                   primary_muscles, secondary_muscles, duration_seconds,
                   repetitions, sets, recommended_for_activities, contraindications,
                   instructions, cues, image_url, video_url, created_at, updated_at
            FROM stretching_exercises
            WHERE name LIKE $1 OR description LIKE $1
            ORDER BY name ASC
            LIMIT $2
            ",
        )
        .bind(&search_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to search stretching exercises: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    async fn get_stretches_for_activity(
        &self,
        activity_type: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>> {
        let limit_val = i32::try_from(limit.unwrap_or(10)).unwrap_or(10);
        let activity_pattern = format!("%\"{activity_type}\"%");

        let rows = sqlx::query(
            r"
            SELECT id, name, description, category, difficulty,
                   primary_muscles, secondary_muscles, duration_seconds,
                   repetitions, sets, recommended_for_activities, contraindications,
                   instructions, cues, image_url, video_url, created_at, updated_at
            FROM stretching_exercises
            WHERE recommended_for_activities LIKE $1
            ORDER BY category, name ASC
            LIMIT $2
            ",
        )
        .bind(&activity_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get stretches for activity: {e}")))?;

        rows.iter().map(row_to_stretching_exercise).collect()
    }

    async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>> {
        let row = sqlx::query(
            r"
            SELECT id, english_name, sanskrit_name, description, benefits,
                   category, difficulty, pose_type, primary_muscles, secondary_muscles,
                   chakras, hold_duration_seconds, breath_guidance,
                   recommended_for_activities, recommended_for_recovery, contraindications,
                   instructions, modifications, progressions, cues,
                   warmup_poses, followup_poses, image_url, video_url,
                   created_at, updated_at
            FROM yoga_poses
            WHERE id = $1
            ",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get yoga pose: {e}")))?;

        row.map(|r| row_to_yoga_pose(&r)).transpose()
    }

    async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>> {
        let limit_val = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);

        // Build dynamic query with parameterized conditions to prevent SQL injection
        let mut conditions = Vec::new();
        let mut bind_values: Vec<String> = Vec::new();

        if let Some(ref cat) = filter.category {
            conditions.push("category = ?".to_owned());
            bind_values.push(cat.as_str().to_owned());
        }
        if let Some(ref diff) = filter.difficulty {
            conditions.push("difficulty = ?".to_owned());
            bind_values.push(diff.as_str().to_owned());
        }
        if let Some(ref pt) = filter.pose_type {
            conditions.push("pose_type = ?".to_owned());
            bind_values.push(pt.as_str().to_owned());
        }
        if let Some(ref muscle) = filter.muscle_group {
            conditions.push("(primary_muscles LIKE ? OR secondary_muscles LIKE ?)".to_owned());
            let pattern = format!("%\"{muscle}\"");
            bind_values.push(pattern.clone());
            bind_values.push(pattern);
        }
        if let Some(ref activity) = filter.activity_type {
            conditions.push("recommended_for_activities LIKE ?".to_owned());
            bind_values.push(format!("%\"{activity}\""));
        }
        if let Some(ref recovery) = filter.recovery_context {
            conditions.push("recommended_for_recovery LIKE ?".to_owned());
            bind_values.push(format!("%\"{recovery}\""));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r"
            SELECT id, english_name, sanskrit_name, description, benefits,
                   category, difficulty, pose_type, primary_muscles, secondary_muscles,
                   chakras, hold_duration_seconds, breath_guidance,
                   recommended_for_activities, recommended_for_recovery, contraindications,
                   instructions, modifications, progressions, cues,
                   warmup_poses, followup_poses, image_url, video_url,
                   created_at, updated_at
            FROM yoga_poses
            {where_clause}
            ORDER BY english_name ASC
            LIMIT ? OFFSET ?
            "
        );

        let mut sql_query = sqlx::query(&query);
        for value in &bind_values {
            sql_query = sql_query.bind(value);
        }
        sql_query = sql_query.bind(limit_val).bind(offset_val);

        let rows = sql_query
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list yoga poses: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    async fn search_yoga_poses(&self, query: &str, limit: Option<u32>) -> AppResult<Vec<YogaPose>> {
        let limit_val = i32::try_from(limit.unwrap_or(20)).unwrap_or(20);
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(
            r"
            SELECT id, english_name, sanskrit_name, description, benefits,
                   category, difficulty, pose_type, primary_muscles, secondary_muscles,
                   chakras, hold_duration_seconds, breath_guidance,
                   recommended_for_activities, recommended_for_recovery, contraindications,
                   instructions, modifications, progressions, cues,
                   warmup_poses, followup_poses, image_url, video_url,
                   created_at, updated_at
            FROM yoga_poses
            WHERE english_name LIKE $1 OR sanskrit_name LIKE $1 OR description LIKE $1
            ORDER BY english_name ASC
            LIMIT $2
            ",
        )
        .bind(&search_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to search yoga poses: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    async fn get_poses_for_recovery(
        &self,
        recovery_context: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<YogaPose>> {
        let limit_val = i32::try_from(limit.unwrap_or(10)).unwrap_or(10);
        let recovery_pattern = format!("%\"{recovery_context}\"%");

        let rows = sqlx::query(
            r"
            SELECT id, english_name, sanskrit_name, description, benefits,
                   category, difficulty, pose_type, primary_muscles, secondary_muscles,
                   chakras, hold_duration_seconds, breath_guidance,
                   recommended_for_activities, recommended_for_recovery, contraindications,
                   instructions, modifications, progressions, cues,
                   warmup_poses, followup_poses, image_url, video_url,
                   created_at, updated_at
            FROM yoga_poses
            WHERE recommended_for_recovery LIKE $1
            ORDER BY category, english_name ASC
            LIMIT $2
            ",
        )
        .bind(&recovery_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get poses for recovery: {e}")))?;

        rows.iter().map(row_to_yoga_pose).collect()
    }

    async fn get_activity_muscle_mapping(
        &self,
        activity_type: &str,
    ) -> AppResult<Option<ActivityMuscleMapping>> {
        let row = sqlx::query(
            r"
            SELECT id, activity_type, primary_muscles, secondary_muscles,
                   recommended_stretch_categories, recommended_yoga_categories,
                   created_at, updated_at
            FROM activity_muscle_mapping
            WHERE activity_type = $1
            ",
        )
        .bind(activity_type)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get activity muscle mapping: {e}")))?;

        row.map(|r| row_to_activity_muscle_mapping(&r)).transpose()
    }

    async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>> {
        let rows = sqlx::query(
            r"
            SELECT id, activity_type, primary_muscles, secondary_muscles,
                   recommended_stretch_categories, recommended_yoga_categories,
                   created_at, updated_at
            FROM activity_muscle_mapping
            ORDER BY activity_type ASC
            ",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list activity muscle mappings: {e}")))?;

        rows.iter().map(row_to_activity_muscle_mapping).collect()
    }
}

// ============================================================================
// SocialRepository
// ============================================================================

#[async_trait]
impl SocialRepository for Database {
    async fn create_friend_connection(&self, connection: &FriendConnection) -> AppResult<Uuid> {
        sqlx::query(
            r"
            INSERT INTO friend_connections (id, initiator_id, receiver_id, status, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ",
        )
        .bind(connection.id.to_string())
        .bind(connection.initiator_id.to_string())
        .bind(connection.receiver_id.to_string())
        .bind(connection.status.as_str())
        .bind(connection.created_at.to_rfc3339())
        .bind(connection.updated_at.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create friend connection: {e}")))?;

        Ok(connection.id)
    }

    async fn get_friend_connection(&self, id: Uuid) -> AppResult<Option<FriendConnection>> {
        let row = sqlx::query(
            r"
            SELECT id, initiator_id, receiver_id, status, created_at, updated_at, accepted_at
            FROM friend_connections
            WHERE id = $1
            ",
        )
        .bind(id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get friend connection: {e}")))?;

        row.map(|r| row_to_friend_connection(&r)).transpose()
    }

    async fn get_friend_connection_between(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> AppResult<Option<FriendConnection>> {
        let row = sqlx::query(
            r"
            SELECT id, initiator_id, receiver_id, status, created_at, updated_at, accepted_at
            FROM friend_connections
            WHERE (initiator_id = $1 AND receiver_id = $2)
               OR (initiator_id = $2 AND receiver_id = $1)
            ",
        )
        .bind(user_a.to_string())
        .bind(user_b.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get friend connection: {e}")))?;

        row.map(|r| row_to_friend_connection(&r)).transpose()
    }

    async fn update_friend_connection_status(
        &self,
        id: Uuid,
        user_id: Uuid,
        status: FriendStatus,
    ) -> AppResult<()> {
        let now = Utc::now();
        let accepted_at = if status == FriendStatus::Accepted {
            Some(now.to_rfc3339())
        } else {
            None
        };

        // Only the receiver of a friend request can accept/reject it
        sqlx::query(
            r"
            UPDATE friend_connections
            SET status = $1, updated_at = $2, accepted_at = $3
            WHERE id = $4 AND receiver_id = $5
            ",
        )
        .bind(status.as_str())
        .bind(now.to_rfc3339())
        .bind(accepted_at)
        .bind(id.to_string())
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update friend connection: {e}")))?;

        Ok(())
    }

    async fn get_friends(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>> {
        let rows = sqlx::query(
            r"
            SELECT id, initiator_id, receiver_id, status, created_at, updated_at, accepted_at
            FROM friend_connections
            WHERE (initiator_id = $1 OR receiver_id = $1)
              AND status = 'accepted'
            ORDER BY accepted_at DESC
            ",
        )
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get friends: {e}")))?;

        rows.iter().map(row_to_friend_connection).collect()
    }

    async fn get_pending_friend_requests(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>> {
        let rows = sqlx::query(
            r"
            SELECT id, initiator_id, receiver_id, status, created_at, updated_at, accepted_at
            FROM friend_connections
            WHERE (initiator_id = $1 OR receiver_id = $1) AND status = 'pending'
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get pending requests: {e}")))?;

        rows.iter().map(row_to_friend_connection).collect()
    }

    async fn get_sent_friend_requests(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>> {
        let rows = sqlx::query(
            r"
            SELECT id, initiator_id, receiver_id, status, created_at, updated_at, accepted_at
            FROM friend_connections
            WHERE initiator_id = $1 AND status = 'pending'
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get sent friend requests: {e}")))?;

        rows.iter().map(row_to_friend_connection).collect()
    }

    async fn are_friends(&self, user_a: Uuid, user_b: Uuid) -> AppResult<bool> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as cnt FROM friend_connections
            WHERE ((initiator_id = $1 AND receiver_id = $2)
                OR (initiator_id = $2 AND receiver_id = $1))
              AND status = 'accepted'
            ",
        )
        .bind(user_a.to_string())
        .bind(user_b.to_string())
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to check friendship: {e}")))?;

        let count: i64 = row.get("cnt");
        Ok(count > 0)
    }

    async fn delete_friend_connection(&self, id: Uuid, user_id: Uuid) -> AppResult<bool> {
        // Only the two parties involved (initiator or receiver) can delete the connection
        let result = sqlx::query(
            "DELETE FROM friend_connections WHERE id = $1 AND (initiator_id = $2 OR receiver_id = $2)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to delete friend connection: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_or_create_social_settings(&self, user_id: Uuid) -> AppResult<UserSocialSettings> {
        if let Some(settings) = self.get_social_settings(user_id).await? {
            return Ok(settings);
        }

        let defaults = UserSocialSettings::default_for_user(user_id);
        self.upsert_social_settings(&defaults).await?;
        Ok(defaults)
    }

    async fn get_social_settings(&self, user_id: Uuid) -> AppResult<Option<UserSocialSettings>> {
        let row = sqlx::query(
            r"
            SELECT user_id, discoverable, default_visibility, share_activity_types,
                   notify_friend_requests, notify_insight_reactions, notify_adapted_insights,
                   insight_sharing_policy, created_at, updated_at
            FROM user_social_settings
            WHERE user_id = $1
            ",
        )
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get social settings: {e}")))?;

        row.map(|r| row_to_social_settings(&r)).transpose()
    }

    async fn upsert_social_settings(&self, settings: &UserSocialSettings) -> AppResult<()> {
        let activity_types_json =
            serde_json::to_string(&settings.share_activity_types).unwrap_or_else(|_| "[]".into());

        sqlx::query(
            r"
            INSERT INTO user_social_settings (
                user_id, discoverable, default_visibility, share_activity_types,
                notify_friend_requests, notify_insight_reactions, notify_adapted_insights,
                insight_sharing_policy, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT(user_id) DO UPDATE SET
                discoverable = excluded.discoverable,
                default_visibility = excluded.default_visibility,
                share_activity_types = excluded.share_activity_types,
                notify_friend_requests = excluded.notify_friend_requests,
                notify_insight_reactions = excluded.notify_insight_reactions,
                notify_adapted_insights = excluded.notify_adapted_insights,
                insight_sharing_policy = excluded.insight_sharing_policy,
                updated_at = excluded.updated_at
            ",
        )
        .bind(settings.user_id.to_string())
        .bind(i32::from(settings.discoverable))
        .bind(settings.default_visibility.as_str())
        .bind(activity_types_json)
        .bind(i32::from(settings.notifications.friend_requests))
        .bind(i32::from(settings.notifications.insight_reactions))
        .bind(i32::from(settings.notifications.adapted_insights))
        .bind(settings.insight_sharing_policy.as_str())
        .bind(settings.created_at.to_rfc3339())
        .bind(settings.updated_at.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert social settings: {e}")))?;

        Ok(())
    }

    async fn create_shared_insight(&self, insight: &SharedInsight) -> AppResult<Uuid> {
        let training_phase_str = insight.training_phase.as_ref().map(TrainingPhase::as_str);
        let expires_at_str = insight.expires_at.map(|dt| dt.to_rfc3339());

        sqlx::query(
            r"
            INSERT INTO shared_insights (
                id, user_id, visibility, insight_type, sport_type, content, title,
                training_phase, reaction_count, adapt_count, created_at, updated_at, expires_at,
                source_activity_id, coach_generated
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ",
        )
        .bind(insight.id.to_string())
        .bind(insight.user_id.to_string())
        .bind(insight.visibility.as_str())
        .bind(insight.insight_type.as_str())
        .bind(&insight.sport_type)
        .bind(&insight.content)
        .bind(&insight.title)
        .bind(training_phase_str)
        .bind(insight.reaction_count)
        .bind(insight.adapt_count)
        .bind(insight.created_at.to_rfc3339())
        .bind(insight.updated_at.to_rfc3339())
        .bind(expires_at_str)
        .bind(&insight.source_activity_id)
        .bind(i32::from(insight.coach_generated))
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create shared insight: {e}")))?;

        Ok(insight.id)
    }

    async fn get_shared_insight(
        &self,
        id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<SharedInsight>> {
        // Scope visibility: the insight creator or friends who can see it via visibility rules
        // For now, we check the creator OR friends (via friend_connections)
        let row = sqlx::query(
            r"
            SELECT si.id, si.user_id, si.visibility, si.insight_type, si.sport_type, si.content,
                   si.title, si.training_phase, si.reaction_count, si.adapt_count, si.created_at,
                   si.updated_at, si.expires_at, si.source_activity_id, si.coach_generated
            FROM shared_insights si
            WHERE si.id = $1
              AND (
                si.user_id = $2
                OR si.visibility = 'public'
                OR (si.visibility = 'friends_only' AND EXISTS (
                    SELECT 1 FROM friend_connections fc
                    WHERE fc.status = 'accepted'
                      AND ((fc.initiator_id = $2 AND fc.receiver_id = si.user_id)
                        OR (fc.receiver_id = $2 AND fc.initiator_id = si.user_id))
                ))
              )
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get shared insight: {e}")))?;

        row.map(|r| row_to_shared_insight(&r)).transpose()
    }

    async fn get_friends_feed(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<SharedInsight>> {
        let rows = sqlx::query(
            r"
            SELECT si.id, si.user_id, si.visibility, si.insight_type, si.sport_type,
                   si.content, si.title, si.training_phase, si.reaction_count, si.adapt_count,
                   si.created_at, si.updated_at, si.expires_at, si.source_activity_id, si.coach_generated
            FROM shared_insights si
            WHERE (
                si.user_id = $1
                OR si.user_id IN (
                    SELECT CASE
                        WHEN fc.initiator_id = $1 THEN fc.receiver_id
                        ELSE fc.initiator_id
                    END
                    FROM friend_connections fc
                    WHERE (fc.initiator_id = $1 OR fc.receiver_id = $1)
                      AND fc.status = 'accepted'
                )
            )
            AND (si.expires_at IS NULL OR si.expires_at > datetime('now'))
            ORDER BY si.created_at DESC
            LIMIT $2 OFFSET $3
            ",
        )
        .bind(user_id.to_string())
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get friends feed: {e}")))?;

        rows.iter().map(row_to_shared_insight).collect()
    }

    async fn get_user_shared_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<SharedInsight>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, visibility, insight_type, sport_type, content, title,
                   training_phase, reaction_count, adapt_count, created_at, updated_at, expires_at,
                   source_activity_id, coach_generated
            FROM shared_insights
            WHERE user_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            ",
        )
        .bind(user_id.to_string())
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get user insights: {e}")))?;

        rows.iter().map(row_to_shared_insight).collect()
    }

    async fn delete_shared_insight(&self, id: Uuid, user_id: Uuid) -> AppResult<bool> {
        // Only the creator of the insight can delete it
        let result = sqlx::query("DELETE FROM shared_insights WHERE id = $1 AND user_id = $2")
            .bind(id.to_string())
            .bind(user_id.to_string())
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to delete shared insight: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn upsert_insight_reaction(&self, reaction: &InsightReaction) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO insight_reactions (id, insight_id, user_id, reaction_type, created_at)
            VALUES ($1, $2, $3, $4, $5)
            ",
        )
        .bind(reaction.id.to_string())
        .bind(reaction.insight_id.to_string())
        .bind(reaction.user_id.to_string())
        .bind(reaction.reaction_type.as_str())
        .bind(reaction.created_at.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create reaction: {e}")))?;

        Ok(())
    }

    async fn get_insight_reaction(
        &self,
        insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<InsightReaction>> {
        let row = sqlx::query(
            r"
            SELECT id, insight_id, user_id, reaction_type, created_at
            FROM insight_reactions
            WHERE insight_id = $1 AND user_id = $2
            ",
        )
        .bind(insight_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get reaction: {e}")))?;

        row.map(|r| row_to_insight_reaction(&r)).transpose()
    }

    async fn delete_insight_reaction(&self, insight_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        let result =
            sqlx::query("DELETE FROM insight_reactions WHERE insight_id = $1 AND user_id = $2")
                .bind(insight_id.to_string())
                .bind(user_id.to_string())
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to delete reaction: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_insight_reactions(&self, insight_id: Uuid) -> AppResult<Vec<InsightReaction>> {
        let rows = sqlx::query(
            r"
            SELECT id, insight_id, user_id, reaction_type, created_at
            FROM insight_reactions
            WHERE insight_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(insight_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get reactions: {e}")))?;

        rows.iter().map(row_to_insight_reaction).collect()
    }

    async fn create_adapted_insight(&self, insight: &AdaptedInsight) -> AppResult<Uuid> {
        sqlx::query(
            r"
            INSERT INTO adapted_insights (
                id, source_insight_id, user_id, adapted_content, adaptation_context,
                was_helpful, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
        )
        .bind(insight.id.to_string())
        .bind(insight.source_insight_id.to_string())
        .bind(insight.user_id.to_string())
        .bind(&insight.adapted_content)
        .bind(&insight.adaptation_context)
        .bind(insight.was_helpful.map(i32::from))
        .bind(insight.created_at.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create adapted insight: {e}")))?;

        Ok(insight.id)
    }

    async fn get_adapted_insight(&self, id: Uuid) -> AppResult<Option<AdaptedInsight>> {
        let row = sqlx::query(
            r"
            SELECT id, source_insight_id, user_id, adapted_content, adaptation_context,
                   was_helpful, created_at
            FROM adapted_insights
            WHERE id = $1
            ",
        )
        .bind(id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get adapted insight: {e}")))?;

        row.as_ref().map(row_to_adapted_insight).transpose()
    }

    async fn get_user_adaptation(
        &self,
        source_insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<AdaptedInsight>> {
        let row = sqlx::query(
            r"
            SELECT id, source_insight_id, user_id, adapted_content, adaptation_context,
                   was_helpful, created_at
            FROM adapted_insights
            WHERE source_insight_id = $1 AND user_id = $2
            ",
        )
        .bind(source_insight_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get adapted insight: {e}")))?;

        row.as_ref().map(row_to_adapted_insight).transpose()
    }

    async fn get_user_adapted_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<AdaptedInsight>> {
        let rows = sqlx::query(
            r"
            SELECT id, source_insight_id, user_id, adapted_content, adaptation_context,
                   was_helpful, created_at
            FROM adapted_insights
            WHERE user_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            ",
        )
        .bind(user_id.to_string())
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get adapted insights: {e}")))?;

        rows.iter().map(row_to_adapted_insight).collect()
    }

    async fn update_adapted_insight_helpful(
        &self,
        id: Uuid,
        user_id: Uuid,
        was_helpful: bool,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            "UPDATE adapted_insights SET was_helpful = $1 WHERE id = $2 AND user_id = $3",
        )
        .bind(i32::from(was_helpful))
        .bind(id.to_string())
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update adapted insight: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn search_discoverable_users(
        &self,
        query: &str,
        exclude_user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<(Uuid, String, Option<String>)>> {
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(
            r"
            SELECT u.id, u.email, u.display_name
            FROM users u
            LEFT JOIN user_social_settings uss ON u.id = uss.user_id
            WHERE (uss.discoverable = 1 OR uss.discoverable IS NULL)
              AND u.user_status = 'active'
              AND u.id != $1
              AND (u.email LIKE $2 OR u.display_name LIKE $2)
            ORDER BY u.display_name, u.email
            LIMIT $3
            ",
        )
        .bind(exclude_user_id.to_string())
        .bind(&search_pattern)
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to search users: {e}")))?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let id_str: String = row.get("id");
            let email: String = row.get("email");
            let display_name: Option<String> = row.get("display_name");

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?;

            results.push((id, email, display_name));
        }

        Ok(results)
    }

    async fn get_friend_count(&self, user_id: Uuid) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as cnt FROM friend_connections
            WHERE (initiator_id = $1 OR receiver_id = $1) AND status = 'accepted'
            ",
        )
        .bind(user_id.to_string())
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to count friends: {e}")))?;

        Ok(row.get("cnt"))
    }
}

// ============================================================================
// Social row-to-model helper functions
// ============================================================================

fn row_to_friend_connection(row: &SqliteRow) -> AppResult<FriendConnection> {
    let id_str: String = row.get("id");
    let initiator_id_str: String = row.get("initiator_id");
    let receiver_id_str: String = row.get("receiver_id");
    let status_str: String = row.get("status");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");
    let accepted_at_str: Option<String> = row.get("accepted_at");

    Ok(FriendConnection {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        initiator_id: Uuid::parse_str(&initiator_id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        receiver_id: Uuid::parse_str(&receiver_id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        status: status_str
            .parse()
            .map_err(|e: String| AppError::database(e))?,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::database(format!("Invalid date: {e}")))?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| AppError::database(format!("Invalid date: {e}")))?
            .with_timezone(&Utc),
        accepted_at: accepted_at_str
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| AppError::database(format!("Invalid date: {e}")))
            })
            .transpose()?,
    })
}

fn row_to_social_settings(row: &SqliteRow) -> AppResult<UserSocialSettings> {
    let user_id_str: String = row.get("user_id");
    let discoverable: i32 = row.get("discoverable");
    let default_visibility_str: String = row.get("default_visibility");
    let activity_types_json: String = row.get("share_activity_types");
    let notify_friend_requests: i32 = row.get("notify_friend_requests");
    let notify_insight_reactions: i32 = row.get("notify_insight_reactions");
    let notify_adapted_insights: i32 = row.get("notify_adapted_insights");
    let insight_sharing_policy_str: String = row.get("insight_sharing_policy");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");

    let share_activity_types: Vec<String> =
        serde_json::from_str(&activity_types_json).unwrap_or_default();

    let insight_sharing_policy =
        InsightSharingPolicy::parse(&insight_sharing_policy_str).unwrap_or_default();

    Ok(UserSocialSettings {
        user_id: Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        discoverable: discoverable != 0,
        default_visibility: default_visibility_str
            .parse()
            .map_err(|e: String| AppError::database(e))?,
        share_activity_types,
        notifications: NotificationPreferences {
            friend_requests: notify_friend_requests != 0,
            insight_reactions: notify_insight_reactions != 0,
            adapted_insights: notify_adapted_insights != 0,
        },
        insight_sharing_policy,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::database(format!("Invalid date: {e}")))?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| AppError::database(format!("Invalid date: {e}")))?
            .with_timezone(&Utc),
    })
}

fn row_to_shared_insight(row: &SqliteRow) -> AppResult<SharedInsight> {
    let id_str: String = row.get("id");
    let user_id_str: String = row.get("user_id");
    let visibility_str: String = row.get("visibility");
    let insight_type_str: String = row.get("insight_type");
    let sport_type: Option<String> = row.get("sport_type");
    let content: String = row.get("content");
    let title: Option<String> = row.get("title");
    let training_phase_str: Option<String> = row.get("training_phase");
    let reaction_count: i32 = row.get("reaction_count");
    let adapt_count: i32 = row.get("adapt_count");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");
    let expires_at_str: Option<String> = row.get("expires_at");
    let source_activity_id: Option<String> = row.get("source_activity_id");
    let coach_generated: i32 = row.get("coach_generated");

    Ok(SharedInsight {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        user_id: Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        visibility: visibility_str
            .parse()
            .map_err(|e: String| AppError::database(e))?,
        insight_type: insight_type_str
            .parse()
            .map_err(|e: String| AppError::database(e))?,
        sport_type,
        content,
        title,
        training_phase: training_phase_str
            .map(|s| s.parse())
            .transpose()
            .map_err(|e: String| AppError::database(e))?,
        reaction_count,
        adapt_count,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::database(format!("Invalid date: {e}")))?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| AppError::database(format!("Invalid date: {e}")))?
            .with_timezone(&Utc),
        expires_at: expires_at_str
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| AppError::database(format!("Invalid date: {e}")))
            })
            .transpose()?,
        source_activity_id,
        coach_generated: coach_generated != 0,
    })
}

fn row_to_insight_reaction(row: &SqliteRow) -> AppResult<InsightReaction> {
    let id_str: String = row.get("id");
    let insight_id_str: String = row.get("insight_id");
    let user_id_str: String = row.get("user_id");
    let reaction_type_str: String = row.get("reaction_type");
    let created_at_str: String = row.get("created_at");

    Ok(InsightReaction {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        insight_id: Uuid::parse_str(&insight_id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        user_id: Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        reaction_type: reaction_type_str
            .parse()
            .map_err(|e: String| AppError::database(e))?,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::database(format!("Invalid date: {e}")))?
            .with_timezone(&Utc),
    })
}

fn row_to_adapted_insight(row: &SqliteRow) -> AppResult<AdaptedInsight> {
    let id_str: String = row.get("id");
    let source_insight_id_str: String = row.get("source_insight_id");
    let user_id_str: String = row.get("user_id");
    let adapted_content: String = row.get("adapted_content");
    let adaptation_context: Option<String> = row.get("adaptation_context");
    let was_helpful: Option<i32> = row.get("was_helpful");
    let created_at_str: String = row.get("created_at");

    Ok(AdaptedInsight {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        source_insight_id: Uuid::parse_str(&source_insight_id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        user_id: Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::database(format!("Invalid UUID: {e}")))?,
        adapted_content,
        adaptation_context,
        was_helpful: was_helpful.map(|v| v != 0),
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::database(format!("Invalid date: {e}")))?
            .with_timezone(&Utc),
    })
}

// ============================================================================
// StoreListingsRepository
// ============================================================================

// ============================================================================
// StoreListingsRepository — private helpers on Database
// ============================================================================

/// Private helper methods for store listing queries, used by the trait impl below.
impl Database {
    /// Get a coach with its listing by `coach_id` and `tenant_id`
    async fn store_get_coach_with_listing(
        &self,
        coach_id: &str,
        tenant_id: &TenantId,
    ) -> AppResult<CoachWithListing> {
        let row = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE c.id = $1 AND sl.tenant_id = $2
            "
        ))
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach with listing: {e}")))?;

        row_to_coach_with_listing(&row)
    }

    /// Query for newest sort order (`published_at` DESC, id DESC)
    async fn store_query_newest_sort(
        &self,
        category_filter: &str,
        cursor: Option<&StoreCursor>,
        fetch_limit: i64,
    ) -> AppResult<Vec<SqliteRow>> {
        if let Some(c) = cursor {
            let ts = c.published_at.map_or(0, |dt| dt.timestamp_millis());
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                  AND (
                    (CAST(strftime('%s', sl.published_at) AS INTEGER) * 1000 +
                     CAST(strftime('%f', sl.published_at) * 1000 AS INTEGER) % 1000) < $1
                    OR (
                      (CAST(strftime('%s', sl.published_at) AS INTEGER) * 1000 +
                       CAST(strftime('%f', sl.published_at) * 1000 AS INTEGER) % 1000) = $1
                      AND c.id < $2
                    )
                  )
                ORDER BY sl.published_at DESC, c.id DESC
                LIMIT $3
                "
            );
            sqlx::query(&query)
                .bind(ts)
                .bind(&c.id)
                .bind(fetch_limit)
                .fetch_all(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to query coaches (newest): {e}")))
        } else {
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                ORDER BY sl.published_at DESC, c.id DESC
                LIMIT $1
                "
            );
            sqlx::query(&query)
                .bind(fetch_limit)
                .fetch_all(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to query coaches (newest first): {e}"))
                })
        }
    }

    /// Query for popular sort order (`install_count` DESC, `published_at` DESC, id DESC)
    async fn store_query_popular_sort(
        &self,
        category_filter: &str,
        cursor: Option<&StoreCursor>,
        fetch_limit: i64,
    ) -> AppResult<Vec<SqliteRow>> {
        if let Some(c) = cursor {
            let count = c.install_count.unwrap_or(0);
            let ts = c.published_at.map_or(0, |dt| dt.timestamp_millis());
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                  AND (
                    sl.install_count < $1
                    OR (
                      sl.install_count = $1
                      AND (CAST(strftime('%s', sl.published_at) AS INTEGER) * 1000 +
                           CAST(strftime('%f', sl.published_at) * 1000 AS INTEGER) % 1000) < $2
                    )
                    OR (
                      sl.install_count = $1
                      AND (CAST(strftime('%s', sl.published_at) AS INTEGER) * 1000 +
                           CAST(strftime('%f', sl.published_at) * 1000 AS INTEGER) % 1000) = $2
                      AND c.id < $3
                    )
                  )
                ORDER BY sl.install_count DESC, sl.published_at DESC, c.id DESC
                LIMIT $4
                "
            );
            sqlx::query(&query)
                .bind(count)
                .bind(ts)
                .bind(&c.id)
                .bind(fetch_limit)
                .fetch_all(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to query coaches (popular): {e}")))
        } else {
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                ORDER BY sl.install_count DESC, sl.published_at DESC, c.id DESC
                LIMIT $1
                "
            );
            sqlx::query(&query)
                .bind(fetch_limit)
                .fetch_all(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to query coaches (popular first): {e}"))
                })
        }
    }

    /// Query for title sort order (title ASC, id ASC)
    async fn store_query_title_sort(
        &self,
        category_filter: &str,
        cursor: Option<&StoreCursor>,
        fetch_limit: i64,
    ) -> AppResult<Vec<SqliteRow>> {
        if let Some(c) = cursor {
            let title = c.title.as_deref().unwrap_or("");
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                  AND (
                    c.title > $1
                    OR (c.title = $1 AND c.id > $2)
                  )
                ORDER BY c.title ASC, c.id ASC
                LIMIT $3
                "
            );
            sqlx::query(&query)
                .bind(title)
                .bind(&c.id)
                .bind(fetch_limit)
                .fetch_all(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to query coaches (title): {e}")))
        } else {
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                ORDER BY c.title ASC, c.id ASC
                LIMIT $1
                "
            );
            sqlx::query(&query)
                .bind(fetch_limit)
                .fetch_all(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to query coaches (title first): {e}"))
                })
        }
    }
}

// ============================================================================
// StoreListingsRepository
// ============================================================================

#[async_trait]
impl StoreListingsRepository for Database {
    async fn submit_for_review(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<StoreListing> {
        let now = Utc::now();

        // Verify the coach exists and belongs to the user
        let coach_row = sqlx::query(
            "SELECT id, tenant_id FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to check coach ownership: {e}")))?;

        if coach_row.is_none() {
            return Err(AppError::invalid_input(
                "Coach not found, not owned by you, or not in your tenant",
            ));
        }

        // Check if listing already exists
        let existing =
            sqlx::query("SELECT id, publish_status FROM store_listings WHERE coach_id = $1")
                .bind(coach_id)
                .fetch_optional(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to check existing listing: {e}"))
                })?;

        if let Some(row) = existing {
            let status: String = row.get("publish_status");
            if status != "draft" {
                return Err(AppError::invalid_input(
                    "Coach is not in draft status — cannot submit for review",
                ));
            }
            let listing_id: String = row.get("id");

            // Update existing draft listing to pending_review
            sqlx::query(
                r"
                UPDATE store_listings SET
                    publish_status = $1,
                    review_submitted_at = $2,
                    updated_at = $2
                WHERE id = $3
                ",
            )
            .bind(PublishStatus::PendingReview.as_str())
            .bind(now.to_rfc3339())
            .bind(&listing_id)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to submit for review: {e}")))?;

            // Also update coaches.updated_at to reflect the change
            sqlx::query("UPDATE coaches SET updated_at = $1 WHERE id = $2")
                .bind(now.to_rfc3339())
                .bind(coach_id)
                .execute(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to update coach timestamp: {e}"))
                })?;

            return self
                .get_listing(coach_id)
                .await?
                .ok_or_else(|| AppError::internal("Failed to fetch updated listing"));
        }

        // Create new listing with pending_review status
        let listing_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO store_listings (
                id, coach_id, tenant_id, publish_status, review_submitted_at,
                install_count, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, 0, $5, $5)
            ",
        )
        .bind(listing_id.to_string())
        .bind(coach_id)
        .bind(tenant_id)
        .bind(PublishStatus::PendingReview.as_str())
        .bind(now.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create store listing: {e}")))?;

        // Also update coaches.updated_at
        sqlx::query("UPDATE coaches SET updated_at = $1 WHERE id = $2")
            .bind(now.to_rfc3339())
            .bind(coach_id)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to update coach timestamp: {e}")))?;

        self.get_listing(coach_id)
            .await?
            .ok_or_else(|| AppError::internal("Failed to fetch created listing"))
    }

    async fn get_listing(&self, coach_id: &str) -> AppResult<Option<StoreListing>> {
        let row = sqlx::query(
            r"
            SELECT id, coach_id, tenant_id, publish_status, published_at,
                   review_submitted_at, review_decision_at, review_decision_by,
                   rejection_reason, install_count, icon_url, author_id,
                   created_at, updated_at
            FROM store_listings
            WHERE coach_id = $1
            ",
        )
        .bind(coach_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get store listing: {e}")))?;

        row.map(|r| row_to_store_listing(&r)).transpose()
    }

    async fn approve_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
    ) -> AppResult<CoachWithListing> {
        let now = Utc::now();

        let result = sqlx::query(
            r"
            UPDATE store_listings SET
                publish_status = $1,
                published_at = $2,
                review_decision_at = $2,
                review_decision_by = $3,
                rejection_reason = NULL,
                updated_at = $2
            WHERE coach_id = $4 AND tenant_id = $5 AND publish_status = 'pending_review'
            ",
        )
        .bind(PublishStatus::Published.as_str())
        .bind(now.to_rfc3339())
        .bind(admin_user_id.map(|id| id.to_string()))
        .bind(coach_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to approve coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::invalid_input(
                "Coach not found or not pending review",
            ));
        }

        self.store_get_coach_with_listing(coach_id, &tenant_id)
            .await
    }

    async fn reject_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
        reason: &str,
    ) -> AppResult<CoachWithListing> {
        let now = Utc::now();

        let result = sqlx::query(
            r"
            UPDATE store_listings SET
                publish_status = $1,
                review_decision_at = $2,
                review_decision_by = $3,
                rejection_reason = $4,
                updated_at = $2
            WHERE coach_id = $5 AND tenant_id = $6 AND publish_status = 'pending_review'
            ",
        )
        .bind(PublishStatus::Rejected.as_str())
        .bind(now.to_rfc3339())
        .bind(admin_user_id.map(|id| id.to_string()))
        .bind(reason)
        .bind(coach_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to reject coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::invalid_input(
                "Coach not found or not pending review",
            ));
        }

        self.store_get_coach_with_listing(coach_id, &tenant_id)
            .await
    }

    async fn unpublish_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<CoachWithListing> {
        let now = Utc::now();

        let result = sqlx::query(
            r"
            UPDATE store_listings SET
                publish_status = $1,
                published_at = NULL,
                updated_at = $2
            WHERE coach_id = $3 AND tenant_id = $4 AND publish_status = 'published'
            ",
        )
        .bind(PublishStatus::Draft.as_str())
        .bind(now.to_rfc3339())
        .bind(coach_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to unpublish coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::invalid_input("Coach not found or not published"));
        }

        self.store_get_coach_with_listing(coach_id, &tenant_id)
            .await
    }

    async fn get_pending_review_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let limit_val = i64::from(limit.unwrap_or(50).min(100));
        let offset_val = i64::from(offset.unwrap_or(0));

        let rows = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.tenant_id = $1 AND sl.publish_status = 'pending_review'
            ORDER BY sl.review_submitted_at ASC
            LIMIT $2 OFFSET $3
            "
        ))
        .bind(tenant_id)
        .bind(limit_val)
        .bind(offset_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get pending review coaches: {e}")))?;

        rows.iter().map(row_to_coach_with_listing).collect()
    }

    async fn get_rejected_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let limit_val = i64::from(limit.unwrap_or(50).min(100));
        let offset_val = i64::from(offset.unwrap_or(0));

        let rows = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.tenant_id = $1 AND sl.publish_status = 'rejected'
            ORDER BY sl.review_decision_at DESC
            LIMIT $2 OFFSET $3
            "
        ))
        .bind(tenant_id)
        .bind(limit_val)
        .bind(offset_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get rejected coaches: {e}")))?;

        rows.iter().map(row_to_coach_with_listing).collect()
    }

    async fn get_store_admin_stats(&self, tenant_id: TenantId) -> AppResult<StoreAdminStats> {
        let row = sqlx::query(
            r"
            SELECT
                COUNT(CASE WHEN publish_status = 'pending_review' THEN 1 END) as pending_count,
                COUNT(CASE WHEN publish_status = 'published' THEN 1 END) as published_count,
                COUNT(CASE WHEN publish_status = 'rejected' THEN 1 END) as rejected_count,
                COALESCE(SUM(CASE WHEN publish_status = 'published' THEN install_count ELSE 0 END), 0) as total_installs
            FROM store_listings
            WHERE tenant_id = $1
            ",
        )
        .bind(tenant_id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get store stats: {e}")))?;

        let pending_count: i64 = row.get("pending_count");
        let published_count: i64 = row.get("published_count");
        let rejected_count: i64 = row.get("rejected_count");
        let total_installs: i64 = row.get("total_installs");

        // Calculate rejection rate
        let total_decided = published_count + rejected_count;
        #[allow(clippy::cast_precision_loss)]
        let rejection_rate = if total_decided > 0 {
            (rejected_count as f64 / total_decided as f64) * 100.0
        } else {
            0.0
        };

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(StoreAdminStats {
            pending_count: pending_count as u32,
            published_count: published_count as u32,
            rejected_count: rejected_count as u32,
            total_installs: total_installs as u32,
            rejection_rate,
        })
    }

    async fn get_author_email(&self, user_id: Uuid) -> AppResult<Option<String>> {
        let row = sqlx::query("SELECT email FROM users WHERE id = $1")
            .bind(user_id.to_string())
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to get author email: {e}")))?;

        Ok(row.map(|r| r.get("email")))
    }

    async fn get_published_coaches(
        &self,
        category: Option<CoachCategory>,
        sort_by: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let limit_val = i64::from(limit.unwrap_or(50).min(100));
        let offset_val = i64::from(offset.unwrap_or(0));

        let order_clause = match sort_by {
            Some("popular") => "sl.install_count DESC, sl.published_at DESC",
            Some("title") => "c.title ASC",
            _ => "sl.published_at DESC",
        };

        let category_filter = category.map_or_else(String::new, |cat| {
            format!("AND c.category = '{}'", cat.as_str())
        });

        let query = format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.publish_status = 'published' {category_filter}
            ORDER BY {order_clause}
            LIMIT $1 OFFSET $2
            "
        );

        let rows = sqlx::query(&query)
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to get published coaches: {e}")))?;

        rows.iter().map(row_to_coach_with_listing).collect()
    }

    async fn get_published_coaches_cursor(
        &self,
        category: Option<CoachCategory>,
        sort_by: StoreSortOrder,
        limit: u32,
        cursor: Option<&str>,
    ) -> AppResult<CursorPage<CoachWithListing>> {
        let limit_val = limit.min(100);
        let fetch_limit = i64::from(limit_val) + 1;

        let decoded_cursor = if let Some(cursor_str) = cursor {
            let cursor_obj = Cursor::from_string(cursor_str.to_owned());
            let decoded = StoreCursor::decode(&cursor_obj, sort_by)
                .ok_or_else(|| AppError::invalid_input("Invalid cursor for current sort order"))?;
            Some(decoded)
        } else {
            None
        };

        let category_filter = category.map_or_else(String::new, |cat| {
            format!("AND c.category = '{}'", cat.as_str())
        });

        let rows = match sort_by {
            StoreSortOrder::Newest => {
                self.store_query_newest_sort(&category_filter, decoded_cursor.as_ref(), fetch_limit)
                    .await?
            }
            StoreSortOrder::Popular => {
                self.store_query_popular_sort(
                    &category_filter,
                    decoded_cursor.as_ref(),
                    fetch_limit,
                )
                .await?
            }
            StoreSortOrder::Title => {
                self.store_query_title_sort(&category_filter, decoded_cursor.as_ref(), fetch_limit)
                    .await?
            }
        };

        let mut all_items: Vec<CoachWithListing> = Vec::new();
        for row in rows {
            all_items.push(row_to_coach_with_listing(&row)?);
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let has_more = all_items.len() > limit_val as usize;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let items: Vec<CoachWithListing> = all_items.into_iter().take(limit_val as usize).collect();

        let next_cursor = if has_more {
            items.last().map(|cwl| {
                let store_cursor = match sort_by {
                    StoreSortOrder::Newest => {
                        StoreCursor::newest(cwl.coach.id.to_string(), cwl.listing.published_at)
                    }
                    StoreSortOrder::Popular => StoreCursor::popular(
                        cwl.coach.id.to_string(),
                        cwl.listing.install_count,
                        cwl.listing.published_at,
                    ),
                    StoreSortOrder::Title => {
                        StoreCursor::title(cwl.coach.id.to_string(), cwl.coach.title.clone())
                    }
                };
                store_cursor.encode()
            })
        } else {
            None
        };

        Ok(CursorPage::new(items, next_cursor, None, has_more))
    }

    async fn search_published_coaches(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let limit_val = i64::from(limit.unwrap_or(20).min(100));
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.publish_status = 'published'
              AND (c.title LIKE $1 OR c.description LIKE $1 OR c.tags LIKE $1)
            ORDER BY sl.install_count DESC, sl.published_at DESC
            LIMIT $2
            "
        ))
        .bind(&search_pattern)
        .bind(limit_val)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to search published coaches: {e}")))?;

        rows.iter().map(row_to_coach_with_listing).collect()
    }

    async fn get_published_coach(&self, coach_id: &str) -> AppResult<Option<CoachWithListing>> {
        let row = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE c.id = $1 AND sl.publish_status = 'published'
            "
        ))
        .bind(coach_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get published coach: {e}")))?;

        row.map(|r| row_to_coach_with_listing(&r)).transpose()
    }

    async fn get_category_counts(&self) -> AppResult<HashMap<CoachCategory, i64>> {
        let rows = sqlx::query(
            r"
            SELECT c.category, COUNT(*) as count
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.publish_status = 'published'
            GROUP BY c.category
            ",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get category counts: {e}")))?;

        let mut counts = HashMap::new();
        for row in &rows {
            let cat_str: String = row.get("category");
            let count: i64 = row.get("count");
            counts.insert(CoachCategory::parse(&cat_str), count);
        }
        Ok(counts)
    }

    async fn increment_install_count(&self, coach_id: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE store_listings
            SET install_count = install_count + 1, updated_at = $1
            WHERE coach_id = $2 AND publish_status = 'published'
            ",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(coach_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to increment install count: {e}")))?;

        Ok(())
    }

    async fn decrement_install_count(&self, coach_id: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE store_listings
            SET install_count = MAX(install_count - 1, 0), updated_at = $1
            WHERE coach_id = $2
            ",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(coach_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to decrement install count: {e}")))?;

        Ok(())
    }

    async fn install_from_store(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        // Get the source coach (must be published, cross-tenant lookup)
        let source = self
            .get_published_coach(source_coach_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Published coach {source_coach_id}")))?;

        // Check if user already has this coach installed
        let existing = sqlx::query(
            "SELECT id FROM coaches WHERE user_id = $1 AND tenant_id = $2 AND forked_from = $3",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(source_coach_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to check existing installation: {e}")))?;

        if existing.is_some() {
            return Err(AppError::invalid_input(format!(
                "Coach {} is already installed",
                source.coach.title
            )));
        }

        // Create the user's copy (without store fields — it's a personal coach)
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&source.coach.tags)?;
        let sample_prompts_json = serde_json::to_string(&source.coach.sample_prompts)?;
        let prerequisites_json = serde_json::to_string(&source.coach.prerequisites)?;

        sqlx::query(
            r"
            INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt, category, tags,
                sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites, forked_from
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, 0, $12, $13, $14)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(&source.coach.title)
        .bind(&source.coach.description)
        .bind(&source.coach.system_prompt)
        .bind(source.coach.category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(source.coach.token_count))
        .bind(now.to_rfc3339())
        .bind(CoachVisibility::Private.as_str())
        .bind(&prerequisites_json)
        .bind(source_coach_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to install coach: {e}")))?;

        // Create self-assignment row for the installed coach
        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, 0, NULL)
            ",
        )
        .bind(assignment_id.to_string())
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(now.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create coach assignment: {e}")))?;

        // Increment install count on the source coach's listing
        self.increment_install_count(source_coach_id).await?;

        // Fetch and return the created coach
        let row = sqlx::query(&format!(
            "SELECT {COACH_COLUMNS} FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3"
        ))
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch installed coach: {e}")))?;

        row_to_coach(&row)
    }

    async fn uninstall_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<String> {
        // Get the coach to verify ownership and get forked_from
        let row = sqlx::query(
            "SELECT id, forked_from FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

        let source_id: Option<String> = row.get("forked_from");
        let source_id = source_id.ok_or_else(|| {
            AppError::invalid_input("This coach was not installed from the Store")
        })?;

        // Delete the user's copy
        sqlx::query("DELETE FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3")
            .bind(coach_id)
            .bind(user_id.to_string())
            .bind(tenant_id)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to uninstall coach: {e}")))?;

        // Decrement install count on the source coach's listing
        self.decrement_install_count(&source_id).await?;

        Ok(source_id)
    }

    async fn get_installed_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        let rows = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS}
            FROM coaches
            WHERE user_id = $1 AND tenant_id = $2 AND forked_from IS NOT NULL
            ORDER BY created_at DESC
            "
        ))
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get installed coaches: {e}")))?;

        rows.iter().map(row_to_coach).collect()
    }

    async fn ensure_listing(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<StoreListing> {
        if let Some(listing) = self.get_listing(coach_id).await? {
            return Ok(listing);
        }

        let now = Utc::now();
        let listing_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO store_listings (
                id, coach_id, tenant_id, publish_status, install_count, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, 0, $5, $5)
            ",
        )
        .bind(listing_id.to_string())
        .bind(coach_id)
        .bind(tenant_id)
        .bind(PublishStatus::Draft.as_str())
        .bind(now.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to create store listing: {e}")))?;

        self.get_listing(coach_id)
            .await?
            .ok_or_else(|| AppError::internal("Failed to fetch created listing"))
    }
}
