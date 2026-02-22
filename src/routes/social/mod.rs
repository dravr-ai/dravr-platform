// ABOUTME: Social features route module organizing friend, insight, settings, and feed handlers
// ABOUTME: Provides SocialRoutes with router wiring and shared utilities for sub-modules
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Social routes
//!
//! This module handles social feature endpoints for coach-mediated sharing.
//! All endpoints require JWT authentication to identify the user.

mod feed;
mod friends;
mod insights;
mod settings;

use axum::{
    http::HeaderMap,
    routing::{delete, get, post, put},
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::{
    auth::AuthResult, database::social_dispatch::SocialManagerBackend, errors::AppError,
    mcp::resources::ServerResources, middleware::extract_auth_from_headers,
};

// Re-export all public types for external consumers
pub use feed::{
    FeedAuthorResponse, FeedItemResponse, FeedQuery, FeedResponse, ReactionCountsResponse,
};
pub use friends::{
    FriendConnectionResponse, FriendWithInfoResponse, ListFriendsQuery, ListFriendsResponse,
    PendingRequestWithInfoResponse, PendingRequestsResponse, RespondFriendRequestBody,
    SearchUsersQuery, SearchUsersResponse, SendFriendRequestBody, UserProfileResponse,
};
pub use insights::{
    AdaptInsightBody, AdaptInsightResultResponse, AdaptedInsightResponse, GenerateInsightBody,
    GenerateInsightResponse, InsightSuggestionResponse, ListAdaptedInsightsResponse,
    ListAdaptedQuery, ListInsightsQuery, ListInsightsResponse, ListReactionsResponse,
    ListSuggestionsResponse, ReactToInsightBody, ReactionResponse, ReactionSummaryResponse,
    ShareFromActivityBody, ShareInsightBody, SharedInsightResponse, SuggestionsQuery,
    UpdateHelpfulBody,
};
pub use settings::{
    NotificationPreferencesResponse, SocialSettingsResponse, UpdateNotificationPreferencesBody,
    UpdateSocialSettingsBody,
};

// ============================================================================
// Shared Types
// ============================================================================

/// Metadata for social responses
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SocialMetadata {
    /// Response timestamp
    pub timestamp: String,
    /// API version
    pub api_version: String,
}

// ============================================================================
// Routes
// ============================================================================

/// Social routes handler
pub struct SocialRoutes;

impl SocialRoutes {
    /// Create all social routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            // Friend connections
            .route("/api/social/friends", get(Self::handle_list_friends))
            .route("/api/social/friends", post(Self::handle_send_request))
            .route(
                "/api/social/friends/pending",
                get(Self::handle_pending_requests),
            )
            .route(
                "/api/social/friends/:id/accept",
                post(Self::handle_accept_request),
            )
            .route(
                "/api/social/friends/:id/decline",
                post(Self::handle_decline_request),
            )
            .route("/api/social/friends/:id", delete(Self::handle_unfriend))
            // Social settings
            .route("/api/social/settings", get(Self::handle_get_settings))
            .route("/api/social/settings", put(Self::handle_update_settings))
            // Insights
            .route("/api/social/insights", get(Self::handle_list_insights))
            .route("/api/social/insights", post(Self::handle_share_insight))
            .route(
                "/api/social/insights/suggestions",
                get(Self::handle_get_suggestions),
            )
            .route(
                "/api/social/insights/from-activity",
                post(Self::handle_share_from_activity),
            )
            .route(
                "/api/social/insights/generate",
                post(Self::handle_generate_insight),
            )
            .route("/api/social/insights/:id", get(Self::handle_get_insight))
            .route(
                "/api/social/insights/:id",
                delete(Self::handle_delete_insight),
            )
            // Reactions
            .route(
                "/api/social/insights/:id/reactions",
                get(Self::handle_list_reactions),
            )
            .route(
                "/api/social/insights/:id/reactions",
                post(Self::handle_add_reaction),
            )
            .route(
                "/api/social/insights/:id/reactions/:reaction_type",
                delete(Self::handle_remove_reaction),
            )
            // Feed
            .route("/api/social/feed", get(Self::handle_get_feed))
            // Adapted insights
            .route(
                "/api/social/insights/:id/adapt",
                post(Self::handle_adapt_insight),
            )
            .route("/api/social/adapted", get(Self::handle_list_adapted))
            .route(
                "/api/social/adapted/:id/helpful",
                put(Self::handle_update_helpful),
            )
            // Discovery
            .route("/api/social/users/search", get(Self::handle_search_users))
            .with_state(resources)
    }

    /// Extract and authenticate user from authorization header or cookie
    pub(crate) async fn authenticate(
        headers: &HeaderMap,
        resources: &Arc<ServerResources>,
    ) -> Result<AuthResult, AppError> {
        extract_auth_from_headers(headers, resources).await
    }

    /// Build metadata for responses
    pub(crate) fn build_metadata() -> SocialMetadata {
        SocialMetadata {
            timestamp: Utc::now().to_rfc3339(),
            api_version: "1.0".to_owned(),
        }
    }

    /// Get social manager for the active database backend
    ///
    /// Returns a `SocialManagerBackend` that dispatches to either the `SQLite`
    /// or `PostgreSQL` social manager depending on which backend is configured.
    pub(crate) fn get_social_manager(
        resources: &Arc<ServerResources>,
    ) -> Result<SocialManagerBackend, AppError> {
        use crate::database::social::SocialManager;
        #[cfg(feature = "postgresql")]
        use crate::database_plugins::social_postgres::PostgresSocialManager;

        // Try SQLite first (most common in development)
        if let Some(pool) = resources.database.sqlite_pool() {
            return Ok(SocialManagerBackend::SQLite(SocialManager::new(
                pool.clone(),
            )));
        }

        // Try PostgreSQL
        #[cfg(feature = "postgresql")]
        if let Some(pool) = resources.database.postgres_pool() {
            return Ok(SocialManagerBackend::PostgreSQL(
                PostgresSocialManager::new(pool.clone()),
            ));
        }

        Err(AppError::internal(
            "No database backend available for social features",
        ))
    }
}
