// ABOUTME: Social, groups, and notifications route group for the Pierre platform
// ABOUTME: Generic over SocialCtx + MiddlewareCtx so the crate stays decoupled from pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Social, Groups, and Notifications Routes
//!
//! Hosts the `/api/groups/*`, `/api/notifications/*`, and a tool-runtime-free
//! subset of `/api/social/*` (friends, feed, settings) REST endpoints.
//!
//! The full set of mounts covers group CRUD/membership/invites, device-token
//! registration / preferences / scheduled notifications, friend connection
//! management, the aggregated friend-insight feed, and user social settings.
//!
//! The route group is generic over [`pierre_runtime_context::SocialCtx`]
//! (for the notification service, group service, and admin-config reads)
//! and [`pierre_runtime_context::MiddlewareCtx`] (for repository access
//! and the `AuthenticatedUser` extractor); the composition root in
//! `pierre-server` implements both traits on its `ServerContext`.
//!
//! ## Migrated since the initial split
//!
//! - **Group analytics** (`/stats`, `/report`, `/health`) — moved into
//!   [`mod@group_analytics`], which builds member snapshots via the
//!   canonical [`pierre_tool_runtime::group_fitness::fetch_member_snapshots`]
//!   so REST analytics and the chat coach share one all-providers +
//!   deduplicated snapshot source. Routes are generic over
//!   `C: ToolRuntime + MiddlewareCtx + SocialCtx` so they can construct
//!   OAuth-authenticated fitness providers per member from the same
//!   `Arc<C>` that satisfies the social trait bounds. Mounted by the
//!   composition root next to [`groups::GroupRoutes::routes`] under the
//!   shared `/api/groups` prefix.
//!
//!
//! - **`/api/social/insights/*`** (and the reactions / adapted endpoints
//!   that hang off it) — insight generation/adaptation depends on the LLM
//!   provider, the provider registry, the tenant OAuth client, the
//!   notification service, and the two insight prompts. All of these
//!   surface as methods on [`pierre_tool_runtime::runtime::ToolRuntime`],
//!   so [`SocialRoutes`] is generic over
//!   `C: ToolRuntime + MiddlewareCtx + SocialCtx` and lives in
//!   [`mod@insights`] alongside the shared
//!   `fetch_activities_from_provider` helper.

#![warn(missing_docs)]

use std::sync::Arc;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use pierre_core::errors::AppError;
use pierre_database::repositories::SocialRepository;
use pierre_runtime_context::{MiddlewareCtx, SocialCtx};

/// Friend connection routes (list, request, accept, decline, unfriend, search).
pub mod friends;

/// Social feed route (aggregated friend insights).
pub mod feed;

/// User social settings routes (discoverability, default visibility, notifications).
pub mod settings;

/// Group analytics router (`/stats`, `/report`, `/health`).
///
/// Sits next to [`mod@groups`]; shares the
/// [`pierre_runtime_context::SocialCtx`] surface and the
/// `/api/groups/*` URL prefix.
pub mod group_analytics;

/// Background scheduler that pushes group weekly digests on a weekly cadence,
/// gated by the per-tenant `weekly_digest` tier flag.
pub mod group_digest_scheduler;

/// Group coaching endpoints (CRUD, membership, invites, analytics).
pub mod groups;

/// Push-notification endpoints (device tokens, preferences, feed, scheduling).
pub mod notifications;

/// Coach-mediated insight sharing, generation, reactions, and per-user adaptation.
pub mod insights;

pub use feed::{
    FeedAuthorResponse, FeedItemResponse, FeedQuery, FeedResponse, ReactionCountsResponse,
    SharedInsightResponse,
};
pub use friends::{
    FriendConnectionResponse, FriendWithInfoResponse, ListFriendsQuery, ListFriendsResponse,
    PendingRequestWithInfoResponse, PendingRequestsResponse, RespondFriendRequestBody,
    SearchUsersQuery, SearchUsersResponse, SendFriendRequestBody, UserProfileResponse,
};
pub use groups::{
    GroupMetadata, GroupRoutes, HealthFlagsResponse, StatsResponse, WeeklyReportResponse,
};
pub use insights::{
    AdaptInsightBody, AdaptInsightResultResponse, AdaptedInsightResponse, GenerateInsightBody,
    GenerateInsightResponse, InsightSuggestionResponse, ListAdaptedInsightsResponse,
    ListAdaptedQuery, ListInsightsQuery, ListInsightsResponse, ListReactionsResponse,
    ListSuggestionsResponse, ReactToInsightBody, ReactionResponse, ReactionSummaryResponse,
    ShareFromActivityBody, ShareInsightBody, SocialRoutes, SuggestionsQuery, UpdateHelpfulBody,
};
pub use notifications::NotificationRoutes;
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

/// Routes for the subset of `/api/social/*` endpoints that do not need
/// the LLM / provider / OAuth surface — friends, feed, and user social
/// settings.
///
/// Insights, reactions, and adapted-insight endpoints live alongside the
/// shared `fetch_activities_from_provider` helper in [`mod@insights`];
/// [`SocialRoutes::routes`] merges both routers so the public
/// `/api/social/*` surface stays whole.
pub struct SocialRestRoutes;

impl SocialRestRoutes {
    /// Build the friends + feed + settings router.
    pub fn routes<C: SocialCtx + MiddlewareCtx>(resources: Arc<C>) -> Router {
        Router::new()
            // Friend connections
            .route("/api/social/friends", get(Self::handle_list_friends::<C>))
            .route("/api/social/friends", post(Self::handle_send_request::<C>))
            .route(
                "/api/social/friends/pending",
                get(Self::handle_pending_requests::<C>),
            )
            .route(
                "/api/social/friends/{id}/accept",
                post(Self::handle_accept_request::<C>),
            )
            .route(
                "/api/social/friends/{id}/decline",
                post(Self::handle_decline_request::<C>),
            )
            .route(
                "/api/social/friends/{id}",
                delete(Self::handle_unfriend::<C>),
            )
            // Social settings
            .route("/api/social/settings", get(Self::handle_get_settings::<C>))
            .route(
                "/api/social/settings",
                put(Self::handle_update_settings::<C>),
            )
            // Feed
            .route("/api/social/feed", get(Self::handle_get_feed::<C>))
            // Discovery
            .route(
                "/api/social/users/search",
                get(Self::handle_search_users::<C>),
            )
            .with_state(resources)
    }

    /// Build metadata for responses.
    pub(crate) fn build_metadata() -> SocialMetadata {
        SocialMetadata {
            timestamp: Utc::now().to_rfc3339(),
            api_version: "1.0".to_owned(),
        }
    }

    /// Get the social repository from the registry.
    ///
    /// Trait is implemented for both `SQLite` (`Database`) and `PostgreSQL`
    /// (`PostgresDatabase`); the registry holds whichever the active
    /// backend produces. Returns the `Arc<dyn SocialRepository>` so
    /// handlers can call trait methods directly.
    pub(crate) fn get_social_manager<C: MiddlewareCtx>(
        resources: &Arc<C>,
    ) -> Result<Arc<dyn SocialRepository>, AppError> {
        resources
            .repos()
            .social
            .clone()
            .ok_or_else(|| AppError::internal("Social repository not configured"))
    }
}
