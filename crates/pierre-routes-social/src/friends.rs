// ABOUTME: Friend management route handlers for the Social API
// ABOUTME: Handles friend requests, pending requests, unfriend, and user discovery
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Friend connection routes (list, request, accept, decline, unfriend, search).
//!
//! Generic over [`pierre_runtime_context::SocialCtx`] (for the optional
//! notification service) and [`pierre_runtime_context::MiddlewareCtx`] (for
//! the repository registry). Mounted by [`crate::SocialRestRoutes::routes`].

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;
use uuid::Uuid;

use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{FriendConnection, FriendStatus};
use pierre_middleware::AuthenticatedUser;
use pierre_notifications::triggers as notification_triggers;
use pierre_runtime_context::{MiddlewareCtx, SocialCtx};
use pierre_services::social_insights;

use crate::{SocialMetadata, SocialRestRoutes};

// ============================================================================
// Response Types
// ============================================================================

/// Response for a friend connection
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FriendConnectionResponse {
    /// Connection ID
    pub id: String,
    /// User who initiated the request
    pub initiator_id: String,
    /// User who received the request
    pub receiver_id: String,
    /// Current status
    pub status: String,
    /// When the request was created
    pub created_at: String,
    /// When the connection was last updated
    pub updated_at: String,
    /// When the request was accepted (if accepted)
    pub accepted_at: Option<String>,
}

impl From<FriendConnection> for FriendConnectionResponse {
    fn from(conn: FriendConnection) -> Self {
        Self {
            id: conn.id.to_string(),
            initiator_id: conn.initiator_id.to_string(),
            receiver_id: conn.receiver_id.to_string(),
            status: conn.status.as_str().to_owned(),
            created_at: conn.created_at.to_rfc3339(),
            updated_at: conn.updated_at.to_rfc3339(),
            accepted_at: conn.accepted_at.map(|dt| dt.to_rfc3339()),
        }
    }
}

/// Response for a friend connection with user info
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FriendWithInfoResponse {
    /// Connection ID
    pub id: String,
    /// User who initiated the request
    pub initiator_id: String,
    /// User who received the request
    pub receiver_id: String,
    /// Current status
    pub status: String,
    /// When the request was created
    pub created_at: String,
    /// When the connection was last updated
    pub updated_at: String,
    /// When the request was accepted (if accepted)
    pub accepted_at: Option<String>,
    /// Friend's display name
    pub friend_display_name: Option<String>,
    /// Friend's email
    pub friend_email: String,
    /// Friend's user ID
    pub friend_user_id: String,
}

/// Response for listing friends
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListFriendsResponse {
    /// List of friend connections with user info
    pub friends: Vec<FriendWithInfoResponse>,
    /// Total count
    pub total: usize,
    /// Cursor for next page (if any)
    pub next_cursor: Option<String>,
    /// Whether more items are available
    pub has_more: bool,
    /// Metadata
    pub metadata: SocialMetadata,
}

/// Response for a pending friend request with user info
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PendingRequestWithInfoResponse {
    /// Connection ID
    pub id: String,
    /// User who initiated the request
    pub initiator_id: String,
    /// User who received the request
    pub receiver_id: String,
    /// Current status
    pub status: String,
    /// When the request was created
    pub created_at: String,
    /// When the connection was last updated
    pub updated_at: String,
    /// When the request was accepted (if accepted)
    pub accepted_at: Option<String>,
    /// The other user's display name (initiator for received, receiver for sent)
    pub user_display_name: Option<String>,
    /// The other user's email. Omitted for pending requests: the counterpart is
    /// not yet an accepted friend, so exposing their email pre-acceptance would
    /// leak PII (cross-tenant in the global graph). Populated only once a
    /// friendship is accepted (which moves the row out of "pending").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    /// The other user's ID
    pub user_id: String,
}

/// Response for pending friend requests
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PendingRequestsResponse {
    /// Requests sent by the user (includes receiver's info)
    pub sent: Vec<PendingRequestWithInfoResponse>,
    /// Requests received by the user (includes initiator's info)
    pub received: Vec<PendingRequestWithInfoResponse>,
    /// Metadata
    pub metadata: SocialMetadata,
}

/// User profile for search results
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UserProfileResponse {
    /// User ID
    pub id: String,
    /// Display name
    pub display_name: Option<String>,
    /// Email (only visible to connected friends for privacy)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Whether the current user is friends with this user
    pub is_friend: bool,
    /// Whether there's a pending request
    pub has_pending_request: bool,
}

/// Response for user search
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SearchUsersResponse {
    /// List of users
    pub users: Vec<UserProfileResponse>,
    /// Total count
    pub total: usize,
}

// ============================================================================
// Request Types
// ============================================================================

/// Request to send a friend request
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SendFriendRequestBody {
    /// ID of the user to send request to
    pub receiver_id: String,
}

/// Request to respond to a friend request
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RespondFriendRequestBody {
    /// Whether to accept the request
    pub accept: bool,
}

// ============================================================================
// Query Types
// ============================================================================

/// Query parameters for listing friends
#[derive(Debug, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListFriendsQuery {
    /// Maximum results
    pub limit: Option<i64>,
    /// Offset for pagination
    pub offset: Option<i64>,
}

/// Query parameters for user search
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SearchUsersQuery {
    /// Search query string
    pub q: String,
    /// Maximum results
    pub limit: Option<i64>,
}

// ============================================================================
// Handlers
// ============================================================================

impl SocialRestRoutes {
    /// Handle GET /api/social/friends - List friends
    pub(crate) async fn handle_list_friends<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Query(query): Query<ListFriendsQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let offset = query.offset.unwrap_or(0).max(0);

        let friends = social
            .get_friends_paginated(auth.user_id, limit, offset)
            .await?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        #[allow(clippy::cast_possible_truncation)] // limit is clamped to small values
        let limit_usize = limit as usize;
        let has_more = friends.len() >= limit_usize;
        let next_cursor = if has_more {
            Some((offset + limit).to_string())
        } else {
            None
        };

        // Determine who the friend is in each connection (the other person).
        let friend_ids = friends
            .iter()
            .map(|conn| {
                if conn.initiator_id == auth.user_id {
                    conn.receiver_id
                } else {
                    conn.initiator_id
                }
            })
            .collect::<Vec<_>>();

        // Batch-fetch friend user info. The friend graph is intentionally
        // cross-tenant (a global consumer graph; see the social_schema migration
        // note), so this is a deliberate global lookup — NOT tenant-scoped.
        // Email here is appropriate: every listed connection is an accepted
        // friend (`get_friends_paginated` returns only `status='accepted'`).
        let friend_users = resources.repos().users.get_global_many(&friend_ids).await?;

        // Build response with friend user info
        let mut friends_with_info = Vec::with_capacity(friends.len());
        for conn in friends {
            let friend_id = if conn.initiator_id == auth.user_id {
                conn.receiver_id
            } else {
                conn.initiator_id
            };

            let (friend_display_name, friend_email) = friend_users.get(&friend_id).map_or_else(
                || (None, format!("user-{friend_id}")),
                |user| (user.display_name.clone(), user.email.clone()),
            );

            friends_with_info.push(FriendWithInfoResponse {
                id: conn.id.to_string(),
                initiator_id: conn.initiator_id.to_string(),
                receiver_id: conn.receiver_id.to_string(),
                status: conn.status.as_str().to_owned(),
                created_at: conn.created_at.to_rfc3339(),
                updated_at: conn.updated_at.to_rfc3339(),
                accepted_at: conn.accepted_at.map(|dt| dt.to_rfc3339()),
                friend_display_name,
                friend_email,
                friend_user_id: friend_id.to_string(),
            });
        }

        let response = ListFriendsResponse {
            total: friends_with_info.len(),
            friends: friends_with_info,
            next_cursor,
            has_more,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle POST /api/social/friends - Send friend request
    pub(crate) async fn handle_send_request<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Json(body): Json<SendFriendRequestBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let receiver_id = Uuid::parse_str(&body.receiver_id)
            .map_err(|_| AppError::invalid_input("Invalid receiver_id format"))?;

        let result =
            social_insights::create_friend_request(&*social, auth.user_id, receiver_id).await?;

        // Fire-and-forget notification to the receiver
        if let Some(service) = resources.notification_service() {
            if let Some(tenant_uuid) = auth.active_tenant_id {
                let sender_user = resources.repos().users.get_global(auth.user_id).await?;
                let sender_name = sender_user
                    .and_then(|u| u.display_name)
                    .unwrap_or_else(|| "Someone".to_owned());
                notification_triggers::trigger_friend_request_received(
                    service,
                    receiver_id,
                    pierre_notifications::TenantId::from_uuid(tenant_uuid),
                    &result.connection.id.to_string(),
                    &sender_name,
                );
            }
        }

        let response: FriendConnectionResponse = result.connection.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// Handle GET /api/social/friends/pending - Get pending requests
    pub(crate) async fn handle_pending_requests<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let pending = social.get_pending_friend_requests(auth.user_id).await?;

        let (sent_conns, received_conns): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|conn| conn.initiator_id == auth.user_id);

        // Batch-fetch the counterpart user for every pending request in one go:
        // sent requests need the receiver, received requests need the initiator.
        let counterpart_ids = sent_conns
            .iter()
            .map(|conn| conn.receiver_id)
            .chain(received_conns.iter().map(|conn| conn.initiator_id))
            .collect::<Vec<_>>();
        let counterpart_users = resources
            .repos()
            .users
            .get_global_many(&counterpart_ids)
            .await?;

        // Build sent requests with receiver's user info. Email is withheld:
        // the receiver has not accepted, so they are not yet a friend.
        let mut sent = Vec::with_capacity(sent_conns.len());
        for conn in sent_conns {
            let user_display_name = counterpart_users
                .get(&conn.receiver_id)
                .and_then(|user| user.display_name.clone());

            sent.push(PendingRequestWithInfoResponse {
                id: conn.id.to_string(),
                initiator_id: conn.initiator_id.to_string(),
                receiver_id: conn.receiver_id.to_string(),
                status: conn.status.as_str().to_owned(),
                created_at: conn.created_at.to_rfc3339(),
                updated_at: conn.updated_at.to_rfc3339(),
                accepted_at: conn.accepted_at.map(|dt| dt.to_rfc3339()),
                user_display_name,
                user_email: None,
                user_id: conn.receiver_id.to_string(),
            });
        }

        // Build received requests with initiator's user info. Email is withheld:
        // the request is unaccepted, so the initiator is a stranger until the
        // recipient accepts — exposing their email here is the pre-acceptance
        // PII leak this gate closes.
        let mut received = Vec::with_capacity(received_conns.len());
        for conn in received_conns {
            let user_display_name = counterpart_users
                .get(&conn.initiator_id)
                .and_then(|user| user.display_name.clone());

            received.push(PendingRequestWithInfoResponse {
                id: conn.id.to_string(),
                initiator_id: conn.initiator_id.to_string(),
                receiver_id: conn.receiver_id.to_string(),
                status: conn.status.as_str().to_owned(),
                created_at: conn.created_at.to_rfc3339(),
                updated_at: conn.updated_at.to_rfc3339(),
                accepted_at: conn.accepted_at.map(|dt| dt.to_rfc3339()),
                user_display_name,
                user_email: None,
                user_id: conn.initiator_id.to_string(),
            });
        }

        let response = PendingRequestsResponse {
            sent,
            received,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle POST /api/social/friends/:id/accept - Accept friend request
    pub(crate) async fn handle_accept_request<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let connection_id = Uuid::parse_str(&id)
            .map_err(|_| AppError::invalid_input("Invalid connection ID format"))?;

        // Get the connection and verify user can accept it
        let connection = social
            .get_friend_connection(connection_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Friend request {id}")))?;

        // Only receiver can accept
        if connection.receiver_id != auth.user_id {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only the receiver can accept a friend request",
            ));
        }

        if connection.status != FriendStatus::Pending {
            return Err(AppError::invalid_input(format!(
                "Cannot accept request with status: {}",
                connection.status
            )));
        }

        social
            .update_friend_connection_status(connection_id, auth.user_id, FriendStatus::Accepted)
            .await?;

        // Fire-and-forget notification to the original requester
        if let Some(service) = resources.notification_service() {
            if let Some(tenant_uuid) = auth.active_tenant_id {
                let accepter_user = resources.repos().users.get_global(auth.user_id).await?;
                let accepter_name = accepter_user
                    .and_then(|u| u.display_name)
                    .unwrap_or_else(|| "Someone".to_owned());
                notification_triggers::trigger_friend_request_accepted(
                    service,
                    connection.initiator_id,
                    pierre_notifications::TenantId::from_uuid(tenant_uuid),
                    &accepter_name,
                );
            }
        }

        let updated = social
            .get_friend_connection(connection_id)
            .await?
            .ok_or_else(|| AppError::internal("Failed to fetch updated connection"))?;

        let response: FriendConnectionResponse = updated.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle POST /api/social/friends/:id/decline - Decline friend request
    pub(crate) async fn handle_decline_request<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let connection_id = Uuid::parse_str(&id)
            .map_err(|_| AppError::invalid_input("Invalid connection ID format"))?;

        let connection = social
            .get_friend_connection(connection_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Friend request {id}")))?;

        // Only receiver can decline
        if connection.receiver_id != auth.user_id {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only the receiver can decline a friend request",
            ));
        }

        if connection.status != FriendStatus::Pending {
            return Err(AppError::invalid_input(format!(
                "Cannot decline request with status: {}",
                connection.status
            )));
        }

        social
            .update_friend_connection_status(connection_id, auth.user_id, FriendStatus::Declined)
            .await?;

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    /// Handle DELETE /api/social/friends/:id - Remove friend
    pub(crate) async fn handle_unfriend<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let connection_id = Uuid::parse_str(&id)
            .map_err(|_| AppError::invalid_input("Invalid connection ID format"))?;

        let connection = social
            .get_friend_connection(connection_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Friend connection {id}")))?;

        // Either party can unfriend
        if !connection.involves_user(auth.user_id) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "You are not part of this connection",
            ));
        }

        social
            .delete_friend_connection(connection_id, auth.user_id)
            .await?;

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    /// Handle POST /api/social/friends/:id/block - Block the other party
    ///
    /// Either participant may block the other from any connection state. The
    /// connection moves to `Blocked`, which the accepted-friends queries already
    /// exclude, so the two users drop out of each other's lists and feeds.
    pub(crate) async fn handle_block_user<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let connection_id = Uuid::parse_str(&id)
            .map_err(|_| AppError::invalid_input("Invalid connection ID format"))?;

        let connection = social
            .get_friend_connection(connection_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Friend connection {id}")))?;

        // Either party can block the other.
        if !connection.involves_user(auth.user_id) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "You are not part of this connection",
            ));
        }

        social
            .update_friend_connection_status(connection_id, auth.user_id, FriendStatus::Blocked)
            .await?;

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    /// Handle GET /api/social/users/search - Search for users
    pub(crate) async fn handle_search_users<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Query(query): Query<SearchUsersQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        // Safe cast: limit is clamped to [1, 50] which fits in u32
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let limit = query.limit.unwrap_or(20).clamp(1, 50) as u32;
        let enriched =
            social_insights::search_users_with_status(&*social, auth.user_id, &query.q, limit)
                .await?;

        let results: Vec<UserProfileResponse> = enriched
            .into_iter()
            .map(|u| UserProfileResponse {
                id: u.user_id.to_string(),
                display_name: u.display_name,
                email: u.visible_email,
                is_friend: u.is_friend,
                has_pending_request: u.has_pending_request,
            })
            .collect();

        let response = SearchUsersResponse {
            total: results.len(),
            users: results,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }
}
