// ABOUTME: Direct repository trait implementations on Database for domain-manager-backed traits
// ABOUTME: Bridges RecipeRepository, CoachesRepository, MobilityRepository, SocialRepository to their managers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::{
    CoachesRepository, MobilityRepository, RecipeRepository, SocialRepository,
    StoreListingsRepository,
};
use crate::database::coaches::CoachesManager;
use crate::database::mobility::MobilityManager;
use crate::database::recipes::RecipeManager;
use crate::database::social::SocialManager;
use crate::database::store_listings::{CoachWithListing, StoreListing, StoreListingsManager};
use crate::database::Database;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::coaches::{
    Coach, CoachAssignment, CoachCategory, CoachListItem, CoachVersion, CreateCoachRequest,
    CreateSystemCoachRequest, ListCoachesFilter, StoreAdminStats, UpdateCoachRequest,
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
use pierre_core::pagination::{CursorPage, StoreSortOrder};
use std::collections::HashMap;
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
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>> {
        let mgr = RecipeManager::new(self.pool().clone());
        mgr.search_recipes(user_id, tenant_id, query, limit, offset)
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

    // -- User methods --

    async fn fork_coach(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.fork_coach(source_coach_id, user_id, tenant_id).await
    }

    async fn activate_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.activate_coach(coach_id, user_id, tenant_id).await
    }

    async fn deactivate_coach(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.deactivate_coach(user_id, tenant_id).await
    }

    async fn get_active_coach(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_active_coach(user_id, tenant_id).await
    }

    // -- Admin methods --

    async fn create_system_coach(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemCoachRequest,
    ) -> AppResult<Coach> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.create_system_coach(admin_user_id, tenant_id, request)
            .await
    }

    async fn list_system_coaches(&self, tenant_id: TenantId) -> AppResult<Vec<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.list_system_coaches(tenant_id).await
    }

    async fn get_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_system_coach(coach_id, tenant_id).await
    }

    async fn get_system_coach_any_tenant(&self, coach_id: &str) -> AppResult<Option<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_system_coach_any_tenant(coach_id).await
    }

    async fn update_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.update_system_coach(coach_id, tenant_id, request).await
    }

    async fn delete_system_coach(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.delete_system_coach(coach_id, tenant_id).await
    }

    // -- Assignment methods --

    async fn get_user_preferences(
        &self,
        coach_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, bool, u32, Option<DateTime<Utc>>)> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_user_preferences(coach_id, user_id).await
    }

    async fn assign_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.assign_coach(coach_id, user_id, assigned_by).await
    }

    async fn unassign_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.unassign_coach(coach_id, user_id).await
    }

    async fn list_assignments(&self, coach_id: &str) -> AppResult<Vec<CoachAssignment>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.list_assignments(coach_id).await
    }

    async fn list_assignments_for_tenant(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAssignment>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.list_assignments_for_tenant(coach_id, tenant_id).await
    }

    async fn hide_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.hide_coach(coach_id, user_id).await
    }

    async fn show_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.show_coach(coach_id, user_id).await
    }

    async fn list_hidden_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.list_hidden_coaches(user_id, tenant_id).await
    }

    // -- Version methods --

    async fn create_version(
        &self,
        coach_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.create_version(coach_id, user_id, change_summary).await
    }

    async fn get_versions(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<CoachVersion>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_versions(coach_id, tenant_id, limit).await
    }

    async fn get_version(
        &self,
        coach_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachVersion>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_version(coach_id, version, tenant_id).await
    }

    async fn revert_to_version(
        &self,
        coach_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.revert_to_version(coach_id, version, user_id, tenant_id)
            .await
    }

    async fn get_current_version(&self, coach_id: &str) -> AppResult<i32> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_current_version(coach_id).await
    }

    async fn get_startup_query_by_system_prompt(
        &self,
        system_prompt: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<String>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_startup_query_by_system_prompt(system_prompt, tenant_id)
            .await
    }

    async fn get_max_tool_iterations_by_system_prompt(
        &self,
        system_prompt: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<i32>> {
        let mgr = CoachesManager::new(self.pool().clone());
        mgr.get_max_tool_iterations_by_system_prompt(system_prompt, tenant_id)
            .await
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
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.submit_for_review(coach_id, user_id, tenant_id).await
    }

    async fn get_listing(&self, coach_id: &str) -> AppResult<Option<StoreListing>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_listing(coach_id).await
    }

    async fn approve_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
    ) -> AppResult<CoachWithListing> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.approve_coach(coach_id, tenant_id, admin_user_id).await
    }

    async fn reject_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
        reason: &str,
    ) -> AppResult<CoachWithListing> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.reject_coach(coach_id, tenant_id, admin_user_id, reason)
            .await
    }

    async fn unpublish_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<CoachWithListing> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.unpublish_coach(coach_id, tenant_id).await
    }

    async fn get_pending_review_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_pending_review_coaches(tenant_id, limit, offset)
            .await
    }

    async fn get_rejected_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_rejected_coaches(tenant_id, limit, offset).await
    }

    async fn get_store_admin_stats(&self, tenant_id: TenantId) -> AppResult<StoreAdminStats> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_store_admin_stats(tenant_id).await
    }

    async fn get_author_email(&self, user_id: Uuid) -> AppResult<Option<String>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_author_email(user_id).await
    }

    async fn get_published_coaches(
        &self,
        category: Option<CoachCategory>,
        sort_by: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_published_coaches(category, sort_by, limit, offset)
            .await
    }

    async fn get_published_coaches_cursor(
        &self,
        category: Option<CoachCategory>,
        sort_by: StoreSortOrder,
        limit: u32,
        cursor: Option<&str>,
    ) -> AppResult<CursorPage<CoachWithListing>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_published_coaches_cursor(category, sort_by, limit, cursor)
            .await
    }

    async fn search_published_coaches(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.search_published_coaches(query, limit).await
    }

    async fn get_published_coach(&self, coach_id: &str) -> AppResult<Option<CoachWithListing>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_published_coach(coach_id).await
    }

    async fn get_category_counts(&self) -> AppResult<HashMap<CoachCategory, i64>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_category_counts().await
    }

    async fn increment_install_count(&self, coach_id: &str) -> AppResult<()> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.increment_install_count(coach_id).await
    }

    async fn decrement_install_count(&self, coach_id: &str) -> AppResult<()> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.decrement_install_count(coach_id).await
    }

    async fn install_from_store(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.install_from_store(source_coach_id, user_id, tenant_id)
            .await
    }

    async fn uninstall_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<String> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.uninstall_coach(coach_id, user_id, tenant_id).await
    }

    async fn get_installed_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.get_installed_coaches(user_id, tenant_id).await
    }

    async fn ensure_listing(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<StoreListing> {
        let mgr = StoreListingsManager::new(self.pool().clone());
        mgr.ensure_listing(coach_id, tenant_id).await
    }
}
