// ABOUTME: Social feed route handler for the Social API
// ABOUTME: Handles the aggregated friend insights feed with reactions and author info
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::{errors::AppError, mcp::resources::ServerContext};

use super::{insights::SharedInsightResponse, SocialMetadata, SocialRoutes};

// ============================================================================
// Response Types
// ============================================================================

/// Author information for feed display
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FeedAuthorResponse {
    /// User ID
    pub user_id: String,
    /// Display name
    pub display_name: Option<String>,
    /// Email
    pub email: String,
}

/// Reaction counts by type
#[derive(Debug, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReactionCountsResponse {
    /// Number of likes
    pub like: i32,
    /// Number of celebrations
    pub celebrate: i32,
    /// Number of inspires
    pub inspire: i32,
    /// Number of supports
    pub support: i32,
    /// Total reactions
    pub total: i32,
}

/// A feed item with full metadata
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FeedItemResponse {
    /// The shared insight
    pub insight: SharedInsightResponse,
    /// Author information
    pub author: FeedAuthorResponse,
    /// Reaction counts
    pub reactions: ReactionCountsResponse,
    /// Current user's reaction type (if any)
    pub user_reaction: Option<String>,
    /// Whether current user has adapted this insight
    pub user_has_adapted: bool,
}

/// Response for social feed
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FeedResponse {
    /// Feed items with full metadata
    pub items: Vec<FeedItemResponse>,
    /// Cursor for next page (if any)
    pub next_cursor: Option<String>,
    /// Whether more items are available
    pub has_more: bool,
    /// Metadata
    pub metadata: SocialMetadata,
}

// ============================================================================
// Query Types
// ============================================================================

/// Query parameters for feed
#[derive(Debug, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FeedQuery {
    /// Maximum results
    pub limit: Option<i64>,
    /// Offset for pagination
    pub offset: Option<i64>,
}

// ============================================================================
// Handlers
// ============================================================================

impl SocialRoutes {
    /// Handle GET /api/social/feed - Get social feed
    pub(crate) async fn handle_get_feed(
        State(resources): State<Arc<ServerContext>>,
        headers: HeaderMap,
        Query(query): Query<FeedQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let social = Self::get_social_manager(&resources)?;

        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let offset = query.offset.unwrap_or(0).max(0);

        // Get full feed items with author, reactions, and user-specific state
        let feed_items = social
            .get_friend_insights_feed_full(auth.user_id, limit, offset)
            .await?;

        // Convert to response format
        let items: Vec<FeedItemResponse> = feed_items
            .into_iter()
            .map(|item| FeedItemResponse {
                insight: item.insight.into(),
                author: FeedAuthorResponse {
                    user_id: item.author.user_id.to_string(),
                    display_name: item.author.display_name,
                    email: item.author.email,
                },
                reactions: ReactionCountsResponse {
                    like: item.reactions.like_count,
                    celebrate: item.reactions.celebrate_count,
                    inspire: item.reactions.inspire_count,
                    support: item.reactions.support_count,
                    total: item.reactions.total,
                },
                user_reaction: item.user_reaction.map(|r| r.as_str().to_owned()),
                user_has_adapted: item.user_has_adapted,
            })
            .collect();

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        #[allow(clippy::cast_possible_truncation)] // limit is clamped to small values
        let limit_usize = limit as usize;
        let has_more = items.len() >= limit_usize;
        let next_cursor = if has_more {
            Some((offset + limit).to_string())
        } else {
            None
        };

        let response = FeedResponse {
            items,
            next_cursor,
            has_more,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }
}
