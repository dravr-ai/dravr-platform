// ABOUTME: Dispatch enum for social features that works with both SQLite and PostgreSQL backends
// ABOUTME: Provides a unified API wrapping SocialManager (SQLite) and PostgresSocialManager (PostgreSQL)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use uuid::Uuid;

use super::social::SocialManager;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    AdaptedInsight, FeedItem, FriendConnection, FriendInfo, FriendStatus, InsightReaction,
    InsightType, SharedInsight, UserSocialSettings,
};

#[cfg(feature = "postgresql")]
use crate::backends::social_postgres::PostgresSocialManager;

/// Backend-agnostic social manager that dispatches to either `SQLite` or `PostgreSQL`
///
/// All social route handlers use this enum so that social features work
/// regardless of which database backend is configured.
pub enum SocialManagerBackend {
    /// `SQLite` backend
    SQLite(SocialManager),
    /// `PostgreSQL` backend
    #[cfg(feature = "postgresql")]
    PostgreSQL(PostgresSocialManager),
}

impl SocialManagerBackend {
    // ========================================================================
    // Friend Connections
    // ========================================================================

    /// Create a new friend connection request
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn create_friend_connection(&self, connection: &FriendConnection) -> AppResult<Uuid> {
        match self {
            Self::SQLite(mgr) => mgr.create_friend_connection(connection).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.create_friend_connection(connection).await,
        }
    }

    /// Get a friend connection by ID
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_friend_connection(&self, id: Uuid) -> AppResult<Option<FriendConnection>> {
        match self {
            Self::SQLite(mgr) => mgr.get_friend_connection(id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_friend_connection(id).await,
        }
    }

    /// Get friend connection between two users (in either direction)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_friend_connection_between(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> AppResult<Option<FriendConnection>> {
        match self {
            Self::SQLite(mgr) => mgr.get_friend_connection_between(user_a, user_b).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_friend_connection_between(user_a, user_b).await,
        }
    }

    /// Update friend connection status
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn update_friend_connection_status(
        &self,
        id: Uuid,
        user_id: Uuid,
        status: FriendStatus,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(mgr) => {
                mgr.update_friend_connection_status(id, user_id, status)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => {
                mgr.update_friend_connection_status(id, user_id, status)
                    .await
            }
        }
    }

    /// Get all friends for a user (accepted connections only)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_friends(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>> {
        match self {
            Self::SQLite(mgr) => mgr.get_friends(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_friends(user_id).await,
        }
    }

    /// Get all accepted friends for a user with pagination
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_friends_paginated(
        &self,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<FriendConnection>> {
        match self {
            Self::SQLite(mgr) => mgr.get_friends_paginated(user_id, limit, offset).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_friends_paginated(user_id, limit, offset).await,
        }
    }

    /// Get pending friend requests for a user (both sent and received)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_pending_friend_requests(
        &self,
        user_id: Uuid,
    ) -> AppResult<Vec<FriendConnection>> {
        match self {
            Self::SQLite(mgr) => mgr.get_pending_friend_requests(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_pending_friend_requests(user_id).await,
        }
    }

    /// Delete a friend connection
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn delete_friend_connection(&self, id: Uuid, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(mgr) => mgr.delete_friend_connection(id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.delete_friend_connection(id, user_id).await,
        }
    }

    // ========================================================================
    // User Social Settings
    // ========================================================================

    /// Get social settings for a user
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_user_social_settings(
        &self,
        user_id: Uuid,
    ) -> AppResult<Option<UserSocialSettings>> {
        match self {
            Self::SQLite(mgr) => mgr.get_user_social_settings(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_user_social_settings(user_id).await,
        }
    }

    /// Create or update social settings
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn upsert_user_social_settings(
        &self,
        settings: &UserSocialSettings,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(mgr) => mgr.upsert_user_social_settings(settings).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.upsert_user_social_settings(settings).await,
        }
    }

    // ========================================================================
    // Shared Insights
    // ========================================================================

    /// Create a shared insight
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn create_shared_insight(&self, insight: &SharedInsight) -> AppResult<Uuid> {
        match self {
            Self::SQLite(mgr) => mgr.create_shared_insight(insight).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.create_shared_insight(insight).await,
        }
    }

    /// Get a shared insight by ID
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_shared_insight(
        &self,
        id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<SharedInsight>> {
        match self {
            Self::SQLite(mgr) => mgr.get_shared_insight(id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_shared_insight(id, user_id).await,
        }
    }

    /// Check if a user has already shared an insight from a specific activity
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn has_insight_for_activity(
        &self,
        user_id: Uuid,
        activity_id: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(mgr) => mgr.has_insight_for_activity(user_id, activity_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.has_insight_for_activity(user_id, activity_id).await,
        }
    }

    /// Get user's shared insights with optional filtering
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_user_shared_insights(
        &self,
        user_id: Uuid,
        insight_type: Option<InsightType>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<SharedInsight>> {
        match self {
            Self::SQLite(mgr) => {
                mgr.get_user_shared_insights(user_id, insight_type, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => {
                mgr.get_user_shared_insights(user_id, insight_type, limit, offset)
                    .await
            }
        }
    }

    /// Get friend's insights feed
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_friend_insights_feed(
        &self,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<SharedInsight>> {
        match self {
            Self::SQLite(mgr) => mgr.get_friend_insights_feed(user_id, limit, offset).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_friend_insights_feed(user_id, limit, offset).await,
        }
    }

    /// Get friend's insights feed with full item data
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_friend_insights_feed_full(
        &self,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<FeedItem>> {
        match self {
            Self::SQLite(mgr) => {
                mgr.get_friend_insights_feed_full(user_id, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => {
                mgr.get_friend_insights_feed_full(user_id, limit, offset)
                    .await
            }
        }
    }

    /// Delete a shared insight
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn delete_shared_insight(&self, id: Uuid, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(mgr) => mgr.delete_shared_insight(id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.delete_shared_insight(id, user_id).await,
        }
    }

    // ========================================================================
    // Insight Reactions
    // ========================================================================

    /// Create a reaction to an insight
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn create_insight_reaction(&self, reaction: &InsightReaction) -> AppResult<()> {
        match self {
            Self::SQLite(mgr) => mgr.create_insight_reaction(reaction).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.create_insight_reaction(reaction).await,
        }
    }

    /// Get a user's reaction to an insight
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_user_reaction(
        &self,
        insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<InsightReaction>> {
        match self {
            Self::SQLite(mgr) => mgr.get_user_reaction(insight_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_user_reaction(insight_id, user_id).await,
        }
    }

    /// Delete a reaction
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn delete_insight_reaction(
        &self,
        insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(mgr) => mgr.delete_insight_reaction(insight_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.delete_insight_reaction(insight_id, user_id).await,
        }
    }

    /// Get all reactions for an insight
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_insight_reactions(&self, insight_id: Uuid) -> AppResult<Vec<InsightReaction>> {
        match self {
            Self::SQLite(mgr) => mgr.get_insight_reactions(insight_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_insight_reactions(insight_id).await,
        }
    }

    // ========================================================================
    // Adapted Insights
    // ========================================================================

    /// Create an adapted insight
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn create_adapted_insight(&self, insight: &AdaptedInsight) -> AppResult<Uuid> {
        match self {
            Self::SQLite(mgr) => mgr.create_adapted_insight(insight).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.create_adapted_insight(insight).await,
        }
    }

    /// Get an adapted insight by source insight ID and user ID
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_adapted_insight_by_source(
        &self,
        source_insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<AdaptedInsight>> {
        match self {
            Self::SQLite(mgr) => {
                mgr.get_adapted_insight_by_source(source_insight_id, user_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => {
                mgr.get_adapted_insight_by_source(source_insight_id, user_id)
                    .await
            }
        }
    }

    /// Get user's adapted insights
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_user_adapted_insights(&self, user_id: Uuid) -> AppResult<Vec<AdaptedInsight>> {
        match self {
            Self::SQLite(mgr) => mgr.get_user_adapted_insights(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_user_adapted_insights(user_id).await,
        }
    }

    /// Get adapted insights for a user with pagination
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_user_adapted_insights_paginated(
        &self,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<AdaptedInsight>> {
        match self {
            Self::SQLite(mgr) => {
                mgr.get_user_adapted_insights_paginated(user_id, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => {
                mgr.get_user_adapted_insights_paginated(user_id, limit, offset)
                    .await
            }
        }
    }

    /// Update the `was_helpful` field for an adapted insight
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn update_adapted_insight_helpful(
        &self,
        id: Uuid,
        user_id: Uuid,
        was_helpful: bool,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(mgr) => {
                mgr.update_adapted_insight_helpful(id, user_id, was_helpful)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => {
                mgr.update_adapted_insight_helpful(id, user_id, was_helpful)
                    .await
            }
        }
    }

    /// Get an adapted insight by its primary key
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_adapted_insight(&self, id: Uuid) -> AppResult<Option<AdaptedInsight>> {
        match self {
            Self::SQLite(mgr) => mgr.get_adapted_insight(id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_adapted_insight(id).await,
        }
    }

    // ========================================================================
    // Search and Discovery
    // ========================================================================

    /// Search for users by email or display name (for friend discovery)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn search_discoverable_users(
        &self,
        query: &str,
        exclude_user_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<(Uuid, String, Option<String>)>> {
        match self {
            Self::SQLite(mgr) => {
                mgr.search_discoverable_users(query, exclude_user_id, limit)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => {
                mgr.search_discoverable_users(query, exclude_user_id, limit)
                    .await
            }
        }
    }

    // ========================================================================
    // Additional methods
    // ========================================================================

    /// Get outgoing friend requests sent by the user
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_sent_friend_requests(
        &self,
        user_id: Uuid,
    ) -> AppResult<Vec<FriendConnection>> {
        match self {
            Self::SQLite(mgr) => mgr.get_sent_friend_requests(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_sent_friend_requests(user_id).await,
        }
    }

    /// Check whether two users are friends (have an accepted connection)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn are_friends(&self, user_a: Uuid, user_b: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(mgr) => mgr.are_friends(user_a, user_b).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.are_friends(user_a, user_b).await,
        }
    }

    /// Get social settings for a user, creating defaults if not found
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_or_create_social_settings(
        &self,
        user_id: Uuid,
    ) -> AppResult<UserSocialSettings> {
        match self {
            Self::SQLite(mgr) => mgr.get_or_create_social_settings(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_or_create_social_settings(user_id).await,
        }
    }

    /// Get basic user profile for feed author display
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or user not found
    pub async fn get_user_profile(&self, user_id: Uuid) -> AppResult<FriendInfo> {
        match self {
            Self::SQLite(mgr) => mgr.get_user_profile(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_user_profile(user_id).await,
        }
    }

    /// Get total friend count for a user (accepted connections only)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_friend_count(&self, user_id: Uuid) -> AppResult<i64> {
        match self {
            Self::SQLite(mgr) => mgr.get_friend_count(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(mgr) => mgr.get_friend_count(user_id).await,
        }
    }
}
