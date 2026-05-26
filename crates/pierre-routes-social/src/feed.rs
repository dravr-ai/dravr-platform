// ABOUTME: Social feed route handler for the Social API
// ABOUTME: Handles the aggregated friend insights feed with reactions and author info
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Aggregated friend-insights feed route (`GET /api/social/feed`).
//!
//! Also hosts the [`SharedInsightResponse`] DTO that wraps
//! [`pierre_core::models::SharedInsight`]; the in-server insights router
//! (which stays in `pierre-server`) re-exports the type from here so a single
//! definition is shared by both the feed payload and the insights endpoints.
//!
//! Generic over [`pierre_runtime_context::SocialCtx`] +
//! [`pierre_runtime_context::MiddlewareCtx`]; mounted by
//! [`crate::SocialRestRoutes::routes`].

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use pierre_core::errors::AppError;
use pierre_core::models::SharedInsight;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{MiddlewareCtx, SocialCtx};

use crate::{SocialMetadata, SocialRestRoutes};

// ============================================================================
// Response Types
// ============================================================================

/// Response for a shared insight
///
/// Lives in this module (rather than alongside the insights router in
/// `pierre-server`) so the feed payload can construct it without crossing the
/// crate boundary; the in-server insights routes re-export it from here.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SharedInsightResponse {
    /// Unique identifier
    pub id: String,
    /// User who shared this insight
    pub user_id: String,
    /// Visibility setting
    pub visibility: String,
    /// Type of insight
    pub insight_type: String,
    /// Sport type context
    pub sport_type: Option<String>,
    /// The shareable content
    pub content: String,
    /// Optional title
    pub title: Option<String>,
    /// Training phase context
    pub training_phase: Option<String>,
    /// Number of reactions received
    pub reaction_count: i32,
    /// Number of times adapted by others
    pub adapt_count: i32,
    /// When the insight was shared
    pub created_at: String,
    /// When the insight was last updated
    pub updated_at: String,
    /// Optional expiry time
    pub expires_at: Option<String>,
    /// Source activity ID that generated this insight (for coach-mediated sharing)
    pub source_activity_id: Option<String>,
    /// Whether this insight was coach-generated (vs manual entry)
    pub coach_generated: bool,
}

impl From<SharedInsight> for SharedInsightResponse {
    fn from(insight: SharedInsight) -> Self {
        Self {
            id: insight.id.to_string(),
            user_id: insight.user_id.to_string(),
            visibility: insight.visibility.as_str().to_owned(),
            insight_type: insight.insight_type.as_str().to_owned(),
            sport_type: insight.sport_type,
            content: insight.content,
            title: insight.title,
            training_phase: insight.training_phase.map(|p| p.as_str().to_owned()),
            reaction_count: insight.reaction_count,
            adapt_count: insight.adapt_count,
            created_at: insight.created_at.to_rfc3339(),
            updated_at: insight.updated_at.to_rfc3339(),
            expires_at: insight.expires_at.map(|dt| dt.to_rfc3339()),
            source_activity_id: insight.source_activity_id,
            coach_generated: insight.coach_generated,
        }
    }
}

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

impl SocialRestRoutes {
    /// Handle GET /api/social/feed - Get social feed
    pub(crate) async fn handle_get_feed<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Query(query): Query<FeedQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
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
