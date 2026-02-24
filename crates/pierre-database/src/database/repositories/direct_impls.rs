// ABOUTME: Direct repository trait implementations on Database for domain-manager-backed traits
// ABOUTME: Bridges RecipeRepository, CoachesRepository, MobilityRepository, SocialRepository to their managers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::{CoachesRepository, MobilityRepository, RecipeRepository, SocialRepository};
use crate::database::coaches::CoachesManager;
use crate::database::mobility::MobilityManager;
use crate::database::recipes::RecipeManager;
use crate::database::social::SocialManager;
use crate::database::Database;
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::coaches::{
    Coach, CoachListItem, CreateCoachRequest, ListCoachesFilter, UpdateCoachRequest,
};
use pierre_core::models::mobility::{
    ActivityMuscleMapping, ListStretchingFilter, ListYogaFilter, StretchingExercise, YogaPose,
};
use pierre_core::models::recipes::{MealTiming, Recipe, ValidatedNutrition};
use pierre_core::models::TenantId;
use pierre_core::models::{
    AdaptedInsight, FriendConnection, FriendStatus, InsightReaction, SharedInsight,
    UserSocialSettings,
};
use uuid::Uuid;

// ============================================================================
// RecipeRepository
// ============================================================================

#[async_trait]
impl RecipeRepository for Database {
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<String> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.create_recipe(user_id, tenant_id, recipe).await
    }

    async fn get_by_id(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Recipe>> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.get_recipe(recipe_id, user_id, tenant_id).await
    }

    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        meal_timing: Option<MealTiming>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.list_recipes(user_id, tenant_id, meal_timing, limit, offset)
            .await
    }

    async fn update(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<bool> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.update_recipe(recipe_id, user_id, tenant_id, recipe)
            .await
    }

    async fn delete(&self, recipe_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.delete_recipe(recipe_id, user_id, tenant_id).await
    }

    async fn update_nutrition_cache(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        nutrition: &ValidatedNutrition,
    ) -> AppResult<bool> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.update_nutrition_cache(recipe_id, user_id, tenant_id, nutrition)
            .await
    }

    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<Recipe>> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.search_recipes(user_id, tenant_id, query, limit, None)
            .await
    }

    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.count_recipes(user_id, tenant_id).await
    }
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
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.create(user_id, tenant_id, request).await
    }

    async fn get_by_id(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get(coach_id, user_id, tenant_id).await
    }

    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListCoachesFilter,
    ) -> AppResult<Vec<CoachListItem>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.list(user_id, tenant_id, filter).await
    }

    async fn update(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.update(coach_id, user_id, tenant_id, request).await
    }

    async fn delete(&self, coach_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.delete(coach_id, user_id, tenant_id).await
    }

    async fn record_usage(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.record_usage(coach_id, user_id, tenant_id).await
    }

    async fn toggle_favorite(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<bool>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.toggle_favorite(coach_id, user_id, tenant_id).await
    }

    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.search(user_id, tenant_id, query, limit, offset).await
    }

    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.count(user_id, tenant_id).await
    }
}

// ============================================================================
// MobilityRepository
// ============================================================================

#[async_trait]
impl MobilityRepository for Database {
    async fn get_stretching_exercise(&self, id: &str) -> AppResult<Option<StretchingExercise>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.get_stretching_exercise(id).await
    }

    async fn list_stretching_exercises(
        &self,
        filter: &ListStretchingFilter,
    ) -> AppResult<Vec<StretchingExercise>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.list_stretching_exercises(filter).await
    }

    async fn search_stretching_exercises(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.search_stretching_exercises(query, limit).await
    }

    async fn get_stretches_for_activity(
        &self,
        activity_type: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<StretchingExercise>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.get_stretches_for_activity(activity_type, limit).await
    }

    async fn get_yoga_pose(&self, id: &str) -> AppResult<Option<YogaPose>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.get_yoga_pose(id).await
    }

    async fn list_yoga_poses(&self, filter: &ListYogaFilter) -> AppResult<Vec<YogaPose>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.list_yoga_poses(filter).await
    }

    async fn search_yoga_poses(&self, query: &str, limit: Option<u32>) -> AppResult<Vec<YogaPose>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.search_yoga_poses(query, limit).await
    }

    async fn get_poses_for_recovery(
        &self,
        recovery_context: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<YogaPose>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.get_poses_for_recovery(recovery_context, limit).await
    }

    async fn get_activity_muscle_mapping(
        &self,
        activity_type: &str,
    ) -> AppResult<Option<ActivityMuscleMapping>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.get_activity_muscle_mapping(activity_type).await
    }

    async fn list_activity_muscle_mappings(&self) -> AppResult<Vec<ActivityMuscleMapping>> {
        let mgr = MobilityManager::new(self.pool().clone());
        mgr.list_activity_muscle_mappings().await
    }
}

// ============================================================================
// SocialRepository
// ============================================================================

#[async_trait]
impl SocialRepository for Database {
    async fn create_friend_connection(&self, connection: &FriendConnection) -> AppResult<Uuid> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.create_friend_connection(connection).await
    }

    async fn get_friend_connection(&self, id: Uuid) -> AppResult<Option<FriendConnection>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_friend_connection(id).await
    }

    async fn get_friend_connection_between(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> AppResult<Option<FriendConnection>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_friend_connection_between(user_a, user_b).await
    }

    async fn update_friend_connection_status(
        &self,
        id: Uuid,
        user_id: Uuid,
        status: FriendStatus,
    ) -> AppResult<()> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.update_friend_connection_status(id, user_id, status)
            .await
    }

    async fn get_friends(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_friends(user_id).await
    }

    async fn get_pending_friend_requests(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_pending_friend_requests(user_id).await
    }

    async fn get_sent_friend_requests(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_sent_friend_requests(user_id).await
    }

    async fn are_friends(&self, user_a: Uuid, user_b: Uuid) -> AppResult<bool> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.are_friends(user_a, user_b).await
    }

    async fn delete_friend_connection(&self, id: Uuid, user_id: Uuid) -> AppResult<bool> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.delete_friend_connection(id, user_id).await
    }

    async fn get_or_create_social_settings(&self, user_id: Uuid) -> AppResult<UserSocialSettings> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_or_create_social_settings(user_id).await
    }

    async fn get_social_settings(&self, user_id: Uuid) -> AppResult<Option<UserSocialSettings>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_user_social_settings(user_id).await
    }

    async fn upsert_social_settings(&self, settings: &UserSocialSettings) -> AppResult<()> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.upsert_user_social_settings(settings).await
    }

    async fn create_shared_insight(&self, insight: &SharedInsight) -> AppResult<Uuid> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.create_shared_insight(insight).await
    }

    async fn get_shared_insight(
        &self,
        id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<SharedInsight>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_shared_insight(id, user_id).await
    }

    async fn get_friends_feed(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<SharedInsight>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_friend_insights_feed(user_id, i64::from(limit), i64::from(offset))
            .await
    }

    async fn get_user_shared_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<SharedInsight>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_user_shared_insights(user_id, None, i64::from(limit), i64::from(offset))
            .await
    }

    async fn delete_shared_insight(&self, id: Uuid, user_id: Uuid) -> AppResult<bool> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.delete_shared_insight(id, user_id).await
    }

    async fn upsert_insight_reaction(&self, reaction: &InsightReaction) -> AppResult<()> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.create_insight_reaction(reaction).await
    }

    async fn get_insight_reaction(
        &self,
        insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<InsightReaction>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_user_reaction(insight_id, user_id).await
    }

    async fn delete_insight_reaction(&self, insight_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.delete_insight_reaction(insight_id, user_id).await
    }

    async fn get_insight_reactions(&self, insight_id: Uuid) -> AppResult<Vec<InsightReaction>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_insight_reactions(insight_id).await
    }

    async fn create_adapted_insight(&self, insight: &AdaptedInsight) -> AppResult<Uuid> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.create_adapted_insight(insight).await
    }

    async fn get_adapted_insight(&self, id: Uuid) -> AppResult<Option<AdaptedInsight>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_adapted_insight(id).await
    }

    async fn get_user_adaptation(
        &self,
        source_insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<AdaptedInsight>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_adapted_insight_by_source(source_insight_id, user_id)
            .await
    }

    async fn get_user_adapted_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<AdaptedInsight>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_user_adapted_insights_paginated(user_id, i64::from(limit), i64::from(offset))
            .await
    }

    async fn update_adapted_insight_helpful(
        &self,
        id: Uuid,
        user_id: Uuid,
        was_helpful: bool,
    ) -> AppResult<bool> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.update_adapted_insight_helpful(id, user_id, was_helpful)
            .await
    }

    async fn search_discoverable_users(
        &self,
        query: &str,
        exclude_user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<(Uuid, String, Option<String>)>> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.search_discoverable_users(query, exclude_user_id, i64::from(limit))
            .await
    }

    async fn get_friend_count(&self, user_id: Uuid) -> AppResult<i64> {
        let mgr = SocialManager::new(self.pool().clone());
        mgr.get_friend_count(user_id).await
    }
}
