// ABOUTME: Repository trait definitions for the seed-only repository operations domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::mobility::{ActivityMuscleMapping, StretchingExercise, YogaPose};

use pierre_core::models::User;
use uuid::Uuid;

use crate::seed_models::SeedCoachTranslation;
use crate::seed_models::{
    SeedA2AClient, SeedA2AUsage, SeedAdaptedInsight, SeedApiKey, SeedApiKeyUsage, SeedCoach,
    SeedCoachAuthor, SeedCoachRelation, SeedDemoUser, SeedFriendConnection, SeedInsightReaction,
    SeedLlmUsageRecord, SeedProviderConnection, SeedSharedInsight, SeedSocialSettings,
    SeedStoreListing, SeedSyntheticActivity, SeedTenant,
};

/// Tables that seeders are allowed to reset (prevent arbitrary table access)
#[derive(Debug, Clone, Copy)]
pub enum SeedTable {
    /// `users` table
    Users,
    /// `api_keys` table
    ApiKeys,
    /// `a2a_clients` table
    A2AClients,
    /// `llm_usage` table
    LlmUsage,
    /// `api_key_usage` table
    ApiKeyUsage,
    /// `a2a_usage` table
    A2AUsage,
    /// `synthetic_activities` table
    SyntheticActivities,
    /// `friend_connections` table
    FriendConnections,
    /// `user_social_settings` table
    UserSocialSettings,
    /// `shared_insights` table
    SharedInsights,
    /// `insight_reactions` table
    InsightReactions,
    /// `adapted_insights` table
    AdaptedInsights,
    /// `stretching_exercises` table
    StretchingExercises,
    /// `yoga_poses` table
    YogaPoses,
    /// `activity_muscle_mapping` table
    ActivityMuscleMapping,
}

impl SeedTable {
    /// Get the SQL table name
    #[must_use]
    pub const fn table_name(&self) -> &'static str {
        match self {
            Self::Users => "users",
            Self::ApiKeys => "api_keys",
            Self::A2AClients => "a2a_clients",
            Self::LlmUsage => "llm_usage",
            Self::ApiKeyUsage => "api_key_usage",
            Self::A2AUsage => "a2a_usage",
            Self::SyntheticActivities => "synthetic_activities",
            Self::FriendConnections => "friend_connections",
            Self::UserSocialSettings => "user_social_settings",
            Self::SharedInsights => "shared_insights",
            Self::InsightReactions => "insight_reactions",
            Self::AdaptedInsights => "adapted_insights",
            Self::StretchingExercises => "stretching_exercises",
            Self::YogaPoses => "yoga_poses",
            Self::ActivityMuscleMapping => "activity_muscle_mapping",
        }
    }
}

/// Repository trait for seed-only database operations.
///
/// Used by seeder binaries to populate demo/test data.
/// Not used by the main server application. Provides write operations
/// for tables that only have read-only repository traits in the main app.
#[async_trait]
pub trait SeederRepository: Send + Sync {
    // ---- Generic operations ----

    /// Delete all rows from a seed table
    async fn seed_reset_table(&self, table: SeedTable) -> AppResult<u64>;

    /// Count rows in a seed table
    async fn seed_count_table(&self, table: SeedTable) -> AppResult<i64>;

    // ---- User lookup (shared across seeders) ----

    /// Get the first admin user (`super_admin` or admin role)
    async fn seed_get_admin_user(&self) -> AppResult<Option<User>>;

    /// Get the `tenant_id` for a user
    async fn seed_get_user_tenant(&self, user_id: Uuid) -> AppResult<Option<String>>;

    /// Find a user by email address (returns full User if found)
    async fn seed_find_user_by_email(&self, email: &str) -> AppResult<Option<User>>;

    /// Get IDs of all non-admin users ordered by creation date
    async fn seed_get_non_admin_user_ids(&self) -> AppResult<Vec<Uuid>>;

    /// Count non-admin users
    async fn seed_count_non_admin_users(&self) -> AppResult<i64>;

    // ---- Mobility seeder (stretching, yoga, activity mappings) ----

    /// Upsert a stretching exercise (insert or replace on conflict)
    async fn seed_upsert_stretching_exercise(&self, exercise: &StretchingExercise)
        -> AppResult<()>;

    /// Upsert a yoga pose (insert or replace on conflict)
    async fn seed_upsert_yoga_pose(&self, pose: &YogaPose) -> AppResult<()>;

    /// Upsert an activity-muscle mapping (insert or replace on conflict)
    async fn seed_upsert_activity_mapping(&self, mapping: &ActivityMuscleMapping) -> AppResult<()>;

    // ---- LLM usage seeder ----

    /// Delete LLM usage records for a specific tenant
    async fn seed_delete_llm_usage_by_tenant(&self, tenant_id: Uuid) -> AppResult<u64>;

    /// Insert a single LLM usage record
    async fn seed_insert_llm_usage(&self, record: &SeedLlmUsageRecord) -> AppResult<()>;

    // ---- Synthetic activities seeder ----

    /// Delete synthetic activities for a specific user
    async fn seed_delete_synthetic_by_user(&self, user_id: Uuid) -> AppResult<u64>;

    /// Insert a synthetic activity record
    async fn seed_insert_synthetic_activity(
        &self,
        activity: &SeedSyntheticActivity,
    ) -> AppResult<()>;

    /// Upsert a provider connection (insert or update on conflict)
    async fn seed_upsert_provider_connection(&self, conn: &SeedProviderConnection)
        -> AppResult<()>;

    // ---- Social data seeder ----

    /// Reset all social data tables (deletes in foreign key order)
    async fn seed_reset_social_data(&self) -> AppResult<()>;

    /// Insert social settings for a user if not already present, returns true if inserted
    async fn seed_upsert_social_settings(&self, settings: &SeedSocialSettings) -> AppResult<bool>;

    /// Insert a friend connection if one doesn't already exist between the users
    async fn seed_insert_friend_connection_if_absent(
        &self,
        conn: &SeedFriendConnection,
    ) -> AppResult<bool>;

    /// Insert a shared insight record
    async fn seed_insert_shared_insight(&self, insight: &SeedSharedInsight) -> AppResult<()>;

    /// Get all shared insight IDs
    async fn seed_get_shared_insight_ids(&self) -> AppResult<Vec<Uuid>>;

    /// Insert a reaction if one doesn't already exist for this user+insight
    async fn seed_insert_reaction_if_absent(
        &self,
        reaction: &SeedInsightReaction,
    ) -> AppResult<bool>;

    /// Get shared insights with their author IDs: `Vec<(insight_id, author_user_id)>`
    async fn seed_get_shared_insights_with_authors(&self) -> AppResult<Vec<(Uuid, Uuid)>>;

    /// Insert an adapted insight if one doesn't already exist for this user+source
    async fn seed_insert_adapted_insight_if_absent(
        &self,
        adapted: &SeedAdaptedInsight,
    ) -> AppResult<bool>;

    /// Get shared insight IDs not authored by the given user (limited)
    async fn seed_get_shared_insights_not_by_user(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<Uuid>>;

    // ---- Demo data seeder ----

    /// Check if a user exists by email, returning their ID if found
    async fn seed_check_user_exists(&self, email: &str) -> AppResult<Option<Uuid>>;

    /// Insert a demo user row
    async fn seed_insert_demo_user(&self, user: &SeedDemoUser) -> AppResult<()>;

    /// Insert a tenant row
    async fn seed_insert_tenant(&self, tenant: &SeedTenant) -> AppResult<()>;

    /// Insert a tenant-user junction row
    async fn seed_insert_tenant_user(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Update the `tenant_id` column on a user row
    async fn seed_update_user_tenant(&self, user_id: Uuid, tenant_id: Uuid) -> AppResult<()>;

    /// Check if an API key exists by name, returning its ID if found
    async fn seed_check_api_key_by_name(&self, name: &str) -> AppResult<Option<Uuid>>;

    /// Insert an API key
    async fn seed_insert_api_key(&self, key: &SeedApiKey) -> AppResult<()>;

    /// Check if an A2A client exists by name, returning its ID if found
    async fn seed_check_a2a_client_by_name(&self, name: &str) -> AppResult<Option<Uuid>>;

    /// Insert an A2A client
    async fn seed_insert_a2a_client(&self, client: &SeedA2AClient) -> AppResult<()>;

    /// Insert an API key usage record
    async fn seed_insert_api_key_usage(&self, usage: &SeedApiKeyUsage) -> AppResult<()>;

    /// Insert an A2A usage record
    async fn seed_insert_a2a_usage(&self, usage: &SeedA2AUsage) -> AppResult<()>;

    // ---- Coach seeder ----

    /// Find a coach by slug and tenant, returning `(id, content_hash)` if found
    async fn seed_find_coach_by_slug(
        &self,
        slug: &str,
        tenant_id: &str,
    ) -> AppResult<Option<(String, Option<String>)>>;

    /// Look up `(source, content_hash)` for a coach by slug, tenant-agnostic.
    ///
    /// Used by `pierre-cli check-drift coaches` (the daily contremaitre→DB
    /// drift gate); not used by the seed write path. Returns `None` when
    /// no row matches the slug.
    async fn seed_find_coach_drift_info(
        &self,
        slug: &str,
    ) -> AppResult<Option<(String, Option<String>)>>;

    /// Insert a coach record
    async fn seed_insert_coach(&self, coach: &SeedCoach) -> AppResult<()>;

    /// Update an existing coach record
    async fn seed_update_coach(&self, coach: &SeedCoach) -> AppResult<()>;

    /// Insert a coach relation if it doesn't already exist, returns true if inserted
    async fn seed_insert_coach_relation_if_absent(
        &self,
        relation: &SeedCoachRelation,
    ) -> AppResult<bool>;

    /// Upsert a coach author profile, returning the author ID
    ///
    /// Creates the `coach_authors` row if absent (idempotent by `user_id + tenant_id` unique).
    /// Returns the `coach_authors.id` for use as `store_listings.author_id`.
    async fn seed_upsert_coach_author(&self, author: &SeedCoachAuthor) -> AppResult<String>;

    /// Insert a store listing if it doesn't already exist, returns true if inserted
    async fn seed_insert_store_listing_if_absent(
        &self,
        listing: &SeedStoreListing,
    ) -> AppResult<bool>;

    /// Upsert a `coach_translations` row for `(coach_id, locale)`.
    ///
    /// Replaces existing translation content so re-running the seeder after a
    /// file edit brings the row back in sync. `source_sha` captures the first
    /// 16 hex chars of `sha256(en.md)` at translation time; the loader uses it
    /// later to detect drift when English content changes.
    async fn seed_upsert_coach_translation(
        &self,
        translation: &SeedCoachTranslation,
    ) -> AppResult<()>;
}
