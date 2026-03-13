// ABOUTME: Recipe repository dispatch for the database factory
// ABOUTME: Delegates RecipeRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::plugins::RecipeRepository;
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::recipes::{MealTiming, Recipe, ValidatedNutrition};
use pierre_core::models::TenantId;
use uuid::Uuid;

#[async_trait]
impl RecipeRepository for Database {
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.create(user_id, tenant_id, recipe).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create(user_id, tenant_id, recipe).await,
        }
    }

    async fn get_by_id(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Recipe>> {
        match self {
            Self::SQLite(db) => db.get_by_id(recipe_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_id(recipe_id, user_id, tenant_id).await,
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
        match self {
            Self::SQLite(db) => {
                db.list(user_id, tenant_id, meal_timing, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.list(user_id, tenant_id, meal_timing, limit, offset)
                    .await
            }
        }
    }

    async fn update(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.update(recipe_id, user_id, tenant_id, recipe).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update(recipe_id, user_id, tenant_id, recipe).await,
        }
    }

    async fn delete(&self, recipe_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.delete(recipe_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete(recipe_id, user_id, tenant_id).await,
        }
    }

    async fn update_nutrition_cache(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        nutrition: &ValidatedNutrition,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.update_nutrition_cache(recipe_id, user_id, tenant_id, nutrition)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_nutrition_cache(recipe_id, user_id, tenant_id, nutrition)
                    .await
            }
        }
    }

    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>> {
        match self {
            Self::SQLite(db) => db.search(user_id, tenant_id, query, limit, offset).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.search(user_id, tenant_id, query, limit, offset).await,
        }
    }

    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        match self {
            Self::SQLite(db) => db.count(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.count(user_id, tenant_id).await,
        }
    }
}
