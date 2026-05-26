// ABOUTME: Repository trait definitions for the social graph persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::{
    AdaptedInsight, FeedItem, FriendConnection, FriendInfo, FriendStatus, InsightReaction,
    InsightType, SharedInsight, UserSocialSettings,
};
use uuid::Uuid;

/// Social features repository for friend connections and shared insights
#[async_trait]
pub trait SocialRepository: Send + Sync {
    /// Create a new friend connection request
    async fn create_friend_connection(&self, connection: &FriendConnection) -> AppResult<Uuid>;
    /// Get a friend connection by ID
    async fn get_friend_connection(&self, id: Uuid) -> AppResult<Option<FriendConnection>>;
    /// Get the friend connection between two users (if any)
    async fn get_friend_connection_between(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> AppResult<Option<FriendConnection>>;
    /// Update friend connection status (accept, reject, block)
    async fn update_friend_connection_status(
        &self,
        id: Uuid,
        user_id: Uuid,
        status: FriendStatus,
    ) -> AppResult<()>;
    /// Get all accepted friends for a user
    async fn get_friends(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>>;
    /// Get pending incoming friend requests
    async fn get_pending_friend_requests(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>>;
    /// Get outgoing friend requests sent by the user
    async fn get_sent_friend_requests(&self, user_id: Uuid) -> AppResult<Vec<FriendConnection>>;
    /// Check whether two users are friends
    async fn are_friends(&self, user_a: Uuid, user_b: Uuid) -> AppResult<bool>;
    /// Delete a friend connection
    async fn delete_friend_connection(&self, id: Uuid, user_id: Uuid) -> AppResult<bool>;
    /// Get social settings for a user, creating defaults if not found
    async fn get_or_create_social_settings(&self, user_id: Uuid) -> AppResult<UserSocialSettings>;
    /// Get social settings for a user (returns None if not set)
    async fn get_social_settings(&self, user_id: Uuid) -> AppResult<Option<UserSocialSettings>>;
    /// Create or update social settings for a user
    async fn upsert_social_settings(&self, settings: &UserSocialSettings) -> AppResult<()>;
    /// Share an insight with friends
    async fn create_shared_insight(&self, insight: &SharedInsight) -> AppResult<Uuid>;
    /// Get a shared insight by ID, scoped to user
    async fn get_shared_insight(&self, id: Uuid, user_id: Uuid)
        -> AppResult<Option<SharedInsight>>;
    /// Get the friends feed (shared insights from friends)
    async fn get_friends_feed(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<SharedInsight>>;
    /// Get insights shared by a specific user, optionally filtered by type.
    async fn get_user_shared_insights(
        &self,
        user_id: Uuid,
        insight_type: Option<InsightType>,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<SharedInsight>>;
    /// Delete a shared insight
    async fn delete_shared_insight(&self, id: Uuid, user_id: Uuid) -> AppResult<bool>;
    /// Create or update a reaction to an insight
    async fn upsert_insight_reaction(&self, reaction: &InsightReaction) -> AppResult<()>;
    /// Get a specific user's reaction to an insight
    async fn get_insight_reaction(
        &self,
        insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<InsightReaction>>;
    /// Delete a reaction to an insight
    async fn delete_insight_reaction(&self, insight_id: Uuid, user_id: Uuid) -> AppResult<bool>;
    /// Get all reactions to an insight
    async fn get_insight_reactions(&self, insight_id: Uuid) -> AppResult<Vec<InsightReaction>>;
    /// Create an adapted version of a shared insight
    async fn create_adapted_insight(&self, insight: &AdaptedInsight) -> AppResult<Uuid>;
    /// Get an adapted insight by ID
    async fn get_adapted_insight(&self, id: Uuid) -> AppResult<Option<AdaptedInsight>>;
    /// Get a user's adaptation of a specific source insight
    async fn get_user_adaptation(
        &self,
        source_insight_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<Option<AdaptedInsight>>;
    /// Get all adapted insights for a user
    async fn get_user_adapted_insights(
        &self,
        user_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<AdaptedInsight>>;
    /// Update whether an adapted insight was helpful
    async fn update_adapted_insight_helpful(
        &self,
        id: Uuid,
        user_id: Uuid,
        was_helpful: bool,
    ) -> AppResult<bool>;
    /// Search for discoverable users by query
    async fn search_discoverable_users(
        &self,
        query: &str,
        exclude_user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<(Uuid, String, Option<String>)>>;
    /// Get total friend count for a user
    async fn get_friend_count(&self, user_id: Uuid) -> AppResult<i64>;

    /// Paginated friends list (i64 limit/offset for direct sqlx binding).
    ///
    /// Distinct from `get_friends` (which returns the entire accepted set)
    /// and from `get_friends_feed` (which returns shared insights). This
    /// returns `FriendConnection` rows for callers that need the connection
    /// metadata, paginated.
    async fn get_friends_paginated(
        &self,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<FriendConnection>>;

    /// Whether the user has already shared an insight for this activity.
    ///
    /// Used to gate the "share from activity" UX so a user doesn't accidentally
    /// re-share the same activity.
    async fn has_insight_for_activity(&self, user_id: Uuid, activity_id: &str) -> AppResult<bool>;

    /// Friends feed enriched with author info, reaction summary, and the
    /// caller's own reaction/adaptation state.
    ///
    /// Distinct from `get_friends_feed` (which returns raw `SharedInsight`
    /// rows). The "_full" variant performs N+1 lookups per insight to
    /// assemble the rich `FeedItem` shape consumed by the social feed UI.
    async fn get_friend_insights_feed_full(
        &self,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<FeedItem>>;

    /// Paginated list of insights the user has adapted.
    async fn get_user_adapted_insights_paginated(
        &self,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<AdaptedInsight>>;

    /// Lookup a user's profile (display name, email, account creation time).
    ///
    /// Used by the social feed to render author info next to each insight.
    /// Returns `NotFound` if the user does not exist.
    async fn get_user_profile(&self, user_id: Uuid) -> AppResult<FriendInfo>;
}
