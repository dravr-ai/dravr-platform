// ABOUTME: Social settings route handlers for the Social API
// ABOUTME: Handles get/update social settings and notification preferences
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User social settings routes.
//!
//! Covers discoverability, default visibility, activity-type sharing,
//! and per-channel notification preferences. Generic over
//! [`pierre_runtime_context::SocialCtx`] +
//! [`pierre_runtime_context::MiddlewareCtx`]; mounted by
//! [`crate::SocialRestRoutes::routes`].

use std::str::FromStr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use pierre_core::errors::AppError;
use pierre_core::models::{ShareVisibility, UserSocialSettings};
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{MiddlewareCtx, SocialCtx};

use crate::SocialRestRoutes;

// ============================================================================
// Response Types
// ============================================================================

/// Response for user social settings
#[derive(Debug, Serialize, Deserialize)]
pub struct SocialSettingsResponse {
    /// Whether user can be found in search
    pub discoverable: bool,
    /// Default visibility for new insights
    pub default_visibility: String,
    /// Activity types to suggest for sharing
    pub share_activity_types: Vec<String>,
    /// Notification preferences
    pub notifications: NotificationPreferencesResponse,
    /// When settings were created
    pub created_at: String,
    /// When settings were last updated
    pub updated_at: String,
}

/// Notification preferences in response
#[derive(Debug, Serialize, Deserialize)]
pub struct NotificationPreferencesResponse {
    /// Receive notifications for friend requests
    pub friend_requests: bool,
    /// Receive notifications for reactions
    pub insight_reactions: bool,
    /// Receive notifications when insights are adapted
    pub adapted_insights: bool,
}

impl From<UserSocialSettings> for SocialSettingsResponse {
    fn from(settings: UserSocialSettings) -> Self {
        Self {
            discoverable: settings.discoverable,
            default_visibility: settings.default_visibility.as_str().to_owned(),
            share_activity_types: settings.share_activity_types,
            notifications: NotificationPreferencesResponse {
                friend_requests: settings.notifications.friend_requests,
                insight_reactions: settings.notifications.insight_reactions,
                adapted_insights: settings.notifications.adapted_insights,
            },
            created_at: settings.created_at.to_rfc3339(),
            updated_at: settings.updated_at.to_rfc3339(),
        }
    }
}

// ============================================================================
// Request Types
// ============================================================================

/// Request to update social settings
#[derive(Debug, Deserialize)]
pub struct UpdateSocialSettingsBody {
    /// Whether user can be found in search
    pub discoverable: Option<bool>,
    /// Default visibility for new insights
    pub default_visibility: Option<String>,
    /// Activity types to suggest for sharing
    pub share_activity_types: Option<Vec<String>>,
    /// Notification preferences
    pub notifications: Option<UpdateNotificationPreferencesBody>,
}

/// Request to update notification preferences
#[derive(Debug, Deserialize)]
pub struct UpdateNotificationPreferencesBody {
    /// Receive notifications for friend requests
    pub friend_requests: Option<bool>,
    /// Receive notifications for reactions
    pub insight_reactions: Option<bool>,
    /// Receive notifications when insights are adapted
    pub adapted_insights: Option<bool>,
}

// ============================================================================
// Handlers
// ============================================================================

impl SocialRestRoutes {
    /// Handle GET /api/social/settings - Get social settings
    pub(crate) async fn handle_get_settings<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let settings = social
            .get_social_settings(auth.user_id)
            .await?
            .unwrap_or_else(|| UserSocialSettings::default_for_user(auth.user_id));

        let response: SocialSettingsResponse = settings.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle PUT /api/social/settings - Update social settings
    pub(crate) async fn handle_update_settings<C: SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Json(body): Json<UpdateSocialSettingsBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        // Get existing settings or create defaults
        let mut settings = social
            .get_social_settings(auth.user_id)
            .await?
            .unwrap_or_else(|| UserSocialSettings::default_for_user(auth.user_id));

        // Apply updates
        if let Some(discoverable) = body.discoverable {
            settings.discoverable = discoverable;
        }
        if let Some(ref visibility) = body.default_visibility {
            settings.default_visibility = ShareVisibility::from_str(visibility)?;
        }
        if let Some(activity_types) = body.share_activity_types {
            settings.share_activity_types = activity_types;
        }
        if let Some(notifications) = body.notifications {
            if let Some(friend_requests) = notifications.friend_requests {
                settings.notifications.friend_requests = friend_requests;
            }
            if let Some(insight_reactions) = notifications.insight_reactions {
                settings.notifications.insight_reactions = insight_reactions;
            }
            if let Some(adapted_insights) = notifications.adapted_insights {
                settings.notifications.adapted_insights = adapted_insights;
            }
        }
        settings.updated_at = Utc::now();

        social.upsert_social_settings(&settings).await?;

        let response: SocialSettingsResponse = settings.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }
}
