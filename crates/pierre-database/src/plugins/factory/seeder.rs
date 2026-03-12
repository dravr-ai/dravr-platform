// ABOUTME: Seeder repository dispatch for the database factory
// ABOUTME: Delegates SeederRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::repositories::{SeedTable, SeederRepository};
use crate::seed_models::{
    SeedA2AClient, SeedA2AUsage, SeedAdaptedInsight, SeedApiKey, SeedApiKeyUsage, SeedCoach,
    SeedCoachRelation, SeedDemoUser, SeedFriendConnection, SeedInsightReaction, SeedLlmUsageRecord,
    SeedProviderConnection, SeedSharedInsight, SeedSocialSettings, SeedStoreListing,
    SeedSyntheticActivity, SeedTenant,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::mobility::{ActivityMuscleMapping, StretchingExercise, YogaPose};
use pierre_core::models::User;
use uuid::Uuid;

/// Macro to reduce boilerplate for factory dispatch methods.
/// Each method just delegates to the appropriate backend.
macro_rules! dispatch {
    ($self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            Self::SQLite(db) => db.$method($($arg),*).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.$method($($arg),*).await,
        }
    };
}

#[async_trait]
impl SeederRepository for Database {
    // ---- Generic operations ----

    async fn seed_reset_table(&self, table: SeedTable) -> AppResult<u64> {
        dispatch!(self, seed_reset_table(table))
    }

    async fn seed_count_table(&self, table: SeedTable) -> AppResult<i64> {
        dispatch!(self, seed_count_table(table))
    }

    // ---- User lookup ----

    async fn seed_get_admin_user(&self) -> AppResult<Option<User>> {
        dispatch!(self, seed_get_admin_user())
    }

    async fn seed_get_user_tenant(&self, user_id: Uuid) -> AppResult<Option<String>> {
        dispatch!(self, seed_get_user_tenant(user_id))
    }

    async fn seed_find_user_by_email(&self, email: &str) -> AppResult<Option<User>> {
        dispatch!(self, seed_find_user_by_email(email))
    }

    async fn seed_get_non_admin_user_ids(&self) -> AppResult<Vec<Uuid>> {
        dispatch!(self, seed_get_non_admin_user_ids())
    }

    async fn seed_count_non_admin_users(&self) -> AppResult<i64> {
        dispatch!(self, seed_count_non_admin_users())
    }

    // ---- Mobility seeder ----

    async fn seed_upsert_stretching_exercise(
        &self,
        exercise: &StretchingExercise,
    ) -> AppResult<()> {
        dispatch!(self, seed_upsert_stretching_exercise(exercise))
    }

    async fn seed_upsert_yoga_pose(&self, pose: &YogaPose) -> AppResult<()> {
        dispatch!(self, seed_upsert_yoga_pose(pose))
    }

    async fn seed_upsert_activity_mapping(&self, mapping: &ActivityMuscleMapping) -> AppResult<()> {
        dispatch!(self, seed_upsert_activity_mapping(mapping))
    }

    // ---- LLM usage seeder ----

    async fn seed_delete_llm_usage_by_tenant(&self, tenant_id: Uuid) -> AppResult<u64> {
        dispatch!(self, seed_delete_llm_usage_by_tenant(tenant_id))
    }

    async fn seed_insert_llm_usage(&self, record: &SeedLlmUsageRecord) -> AppResult<()> {
        dispatch!(self, seed_insert_llm_usage(record))
    }

    // ---- Synthetic activities seeder ----

    async fn seed_delete_synthetic_by_user(&self, user_id: Uuid) -> AppResult<u64> {
        dispatch!(self, seed_delete_synthetic_by_user(user_id))
    }

    async fn seed_insert_synthetic_activity(
        &self,
        activity: &SeedSyntheticActivity,
    ) -> AppResult<()> {
        dispatch!(self, seed_insert_synthetic_activity(activity))
    }

    async fn seed_upsert_provider_connection(
        &self,
        conn: &SeedProviderConnection,
    ) -> AppResult<()> {
        dispatch!(self, seed_upsert_provider_connection(conn))
    }

    // ---- Social data seeder ----

    async fn seed_reset_social_data(&self) -> AppResult<()> {
        dispatch!(self, seed_reset_social_data())
    }

    async fn seed_upsert_social_settings(&self, settings: &SeedSocialSettings) -> AppResult<bool> {
        dispatch!(self, seed_upsert_social_settings(settings))
    }

    async fn seed_insert_friend_connection_if_absent(
        &self,
        conn: &SeedFriendConnection,
    ) -> AppResult<bool> {
        dispatch!(self, seed_insert_friend_connection_if_absent(conn))
    }

    async fn seed_insert_shared_insight(&self, insight: &SeedSharedInsight) -> AppResult<()> {
        dispatch!(self, seed_insert_shared_insight(insight))
    }

    async fn seed_get_shared_insight_ids(&self) -> AppResult<Vec<Uuid>> {
        dispatch!(self, seed_get_shared_insight_ids())
    }

    async fn seed_insert_reaction_if_absent(
        &self,
        reaction: &SeedInsightReaction,
    ) -> AppResult<bool> {
        dispatch!(self, seed_insert_reaction_if_absent(reaction))
    }

    async fn seed_get_shared_insights_with_authors(&self) -> AppResult<Vec<(Uuid, Uuid)>> {
        dispatch!(self, seed_get_shared_insights_with_authors())
    }

    async fn seed_insert_adapted_insight_if_absent(
        &self,
        adapted: &SeedAdaptedInsight,
    ) -> AppResult<bool> {
        dispatch!(self, seed_insert_adapted_insight_if_absent(adapted))
    }

    async fn seed_get_shared_insights_not_by_user(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<Uuid>> {
        dispatch!(self, seed_get_shared_insights_not_by_user(user_id, limit))
    }

    // ---- Demo data seeder ----

    async fn seed_check_user_exists(&self, email: &str) -> AppResult<Option<Uuid>> {
        dispatch!(self, seed_check_user_exists(email))
    }

    async fn seed_insert_demo_user(&self, user: &SeedDemoUser) -> AppResult<()> {
        dispatch!(self, seed_insert_demo_user(user))
    }

    async fn seed_insert_tenant(&self, tenant: &SeedTenant) -> AppResult<()> {
        dispatch!(self, seed_insert_tenant(tenant))
    }

    async fn seed_insert_tenant_user(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        dispatch!(self, seed_insert_tenant_user(id, tenant_id, user_id, now))
    }

    async fn seed_update_user_tenant(&self, user_id: Uuid, tenant_id: Uuid) -> AppResult<()> {
        dispatch!(self, seed_update_user_tenant(user_id, tenant_id))
    }

    async fn seed_check_api_key_by_name(&self, name: &str) -> AppResult<Option<Uuid>> {
        dispatch!(self, seed_check_api_key_by_name(name))
    }

    async fn seed_insert_api_key(&self, key: &SeedApiKey) -> AppResult<()> {
        dispatch!(self, seed_insert_api_key(key))
    }

    async fn seed_check_a2a_client_by_name(&self, name: &str) -> AppResult<Option<Uuid>> {
        dispatch!(self, seed_check_a2a_client_by_name(name))
    }

    async fn seed_insert_a2a_client(&self, client: &SeedA2AClient) -> AppResult<()> {
        dispatch!(self, seed_insert_a2a_client(client))
    }

    async fn seed_insert_api_key_usage(&self, usage: &SeedApiKeyUsage) -> AppResult<()> {
        dispatch!(self, seed_insert_api_key_usage(usage))
    }

    async fn seed_insert_a2a_usage(&self, usage: &SeedA2AUsage) -> AppResult<()> {
        dispatch!(self, seed_insert_a2a_usage(usage))
    }

    // ---- Coach seeder ----

    async fn seed_find_coach_by_slug(
        &self,
        slug: &str,
        tenant_id: &str,
    ) -> AppResult<Option<(String, Option<String>)>> {
        dispatch!(self, seed_find_coach_by_slug(slug, tenant_id))
    }

    async fn seed_insert_coach(&self, coach: &SeedCoach) -> AppResult<()> {
        dispatch!(self, seed_insert_coach(coach))
    }

    async fn seed_update_coach(&self, coach: &SeedCoach) -> AppResult<()> {
        dispatch!(self, seed_update_coach(coach))
    }

    async fn seed_insert_coach_relation_if_absent(
        &self,
        relation: &SeedCoachRelation,
    ) -> AppResult<bool> {
        dispatch!(self, seed_insert_coach_relation_if_absent(relation))
    }

    async fn seed_insert_store_listing_if_absent(
        &self,
        listing: &SeedStoreListing,
    ) -> AppResult<bool> {
        dispatch!(self, seed_insert_store_listing_if_absent(listing))
    }
}
