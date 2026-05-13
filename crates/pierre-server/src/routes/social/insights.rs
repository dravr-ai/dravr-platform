// ABOUTME: Insight sharing, reactions, and adaptation route handlers for the Social API
// ABOUTME: Handles coach-mediated sharing, insight generation, reactions, and adapted insights
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

#[cfg(feature = "client-notifications")]
use std::env;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;
use uuid::Uuid;

#[cfg(feature = "client-notifications")]
use pierre_notifications::{constants as notif_constants, triggers as notification_triggers};

use crate::{
    config::{environment::default_provider, social},
    errors::{AppError, ErrorCode},
    intelligence::{
        insight_adapter::UserTrainingContext,
        insight_validation::{validate_insight_with_policy, ValidationVerdict},
        social_insights::{
            InsightContextBuilder, InsightGenerationContext, InsightSuggestion,
            SharedInsightGenerator,
        },
    },
    llm::{ChatMessage, ChatRequest, LlmProvider},
    mcp::resources::ServerContext,
    middleware::AuthenticatedUser,
    models::{
        Activity, AdaptedInsight, InsightReaction, InsightType, ReactionType, ShareVisibility,
        SharedInsight, TenantId, TrainingPhase,
    },
    protocols::universal::auth_service::AuthService,
    services::social_insights,
};

use pierre_database::repositories::SocialRepository;

use super::{SocialMetadata, SocialRoutes};

// ============================================================================
// Response Types
// ============================================================================

/// Response for a shared insight
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

/// Response for listing insights
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListInsightsResponse {
    /// List of insights
    pub insights: Vec<SharedInsightResponse>,
    /// Total count
    pub total: usize,
    /// Cursor for next page (if any)
    pub next_cursor: Option<String>,
    /// Whether more items are available
    pub has_more: bool,
    /// Metadata
    pub metadata: SocialMetadata,
}

/// Response for a reaction
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReactionResponse {
    /// Reaction ID
    pub id: String,
    /// Insight ID
    pub insight_id: String,
    /// User who reacted
    pub user_id: String,
    /// Type of reaction
    pub reaction_type: String,
    /// When the reaction was created
    pub created_at: String,
}

impl From<InsightReaction> for ReactionResponse {
    fn from(reaction: InsightReaction) -> Self {
        Self {
            id: reaction.id.to_string(),
            insight_id: reaction.insight_id.to_string(),
            user_id: reaction.user_id.to_string(),
            reaction_type: reaction.reaction_type.as_str().to_owned(),
            created_at: reaction.created_at.to_rfc3339(),
        }
    }
}

/// Response for listing reactions
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListReactionsResponse {
    /// List of reactions
    pub reactions: Vec<ReactionResponse>,
    /// Summary by type
    pub summary: ReactionSummaryResponse,
}

/// Summary of reactions by type
#[derive(Debug, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReactionSummaryResponse {
    /// Number of likes
    pub like_count: i32,
    /// Number of celebrations
    pub celebrate_count: i32,
    /// Number of inspires
    pub inspire_count: i32,
    /// Number of supports
    pub support_count: i32,
    /// Total reactions
    pub total: i32,
}

/// Response for an adapted insight
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AdaptedInsightResponse {
    /// Unique identifier
    pub id: String,
    /// Original insight ID
    pub source_insight_id: String,
    /// User who requested the adaptation
    pub user_id: String,
    /// The personalized content
    pub adapted_content: String,
    /// Context used for adaptation
    pub adaptation_context: Option<String>,
    /// Whether user found this helpful
    pub was_helpful: Option<bool>,
    /// When the adaptation was created
    pub created_at: String,
}

impl From<AdaptedInsight> for AdaptedInsightResponse {
    fn from(insight: AdaptedInsight) -> Self {
        Self {
            id: insight.id.to_string(),
            source_insight_id: insight.source_insight_id.to_string(),
            user_id: insight.user_id.to_string(),
            adapted_content: insight.adapted_content,
            adaptation_context: insight.adaptation_context,
            was_helpful: insight.was_helpful,
            created_at: insight.created_at.to_rfc3339(),
        }
    }
}

/// Response for listing adapted insights
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListAdaptedInsightsResponse {
    /// List of adapted insights
    pub adapted_insights: Vec<AdaptedInsightResponse>,
    /// Total count
    pub total: usize,
    /// Cursor for next page (if any)
    pub next_cursor: Option<String>,
    /// Whether more items are available
    pub has_more: bool,
    /// Metadata
    pub metadata: SocialMetadata,
}

/// Response for adapting an insight (includes source insight for context)
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AdaptInsightResultResponse {
    /// The adapted insight
    pub adapted: AdaptedInsightResponse,
    /// The original insight that was adapted
    pub source_insight: SharedInsightResponse,
    /// Metadata
    pub metadata: SocialMetadata,
}

/// Response for a coach-generated insight suggestion
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct InsightSuggestionResponse {
    /// Type of insight
    pub insight_type: String,
    /// Suggested content (privacy-safe)
    pub suggested_content: String,
    /// Suggested title
    pub suggested_title: Option<String>,
    /// Relevance score (0-100)
    pub relevance_score: u8,
    /// Sport type context
    pub sport_type: Option<String>,
    /// Training phase context
    pub training_phase: Option<String>,
    /// Source activity ID (if suggestion is for specific activity)
    pub source_activity_id: Option<String>,
}

impl From<InsightSuggestion> for InsightSuggestionResponse {
    fn from(suggestion: InsightSuggestion) -> Self {
        Self {
            insight_type: suggestion.insight_type.as_str().to_owned(),
            suggested_content: suggestion.suggested_content,
            suggested_title: suggestion.suggested_title,
            relevance_score: suggestion.relevance_score,
            sport_type: suggestion.sport_type,
            training_phase: suggestion.training_phase.map(|p| p.as_str().to_owned()),
            source_activity_id: None,
        }
    }
}

/// Response for listing insight suggestions
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListSuggestionsResponse {
    /// List of suggestions
    pub suggestions: Vec<InsightSuggestionResponse>,
    /// Total count
    pub total: usize,
    /// Metadata
    pub metadata: SocialMetadata,
}

/// Response for generated insight
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GenerateInsightResponse {
    /// The generated shareable content
    pub content: String,
    /// Metadata
    pub metadata: SocialMetadata,
}

// ============================================================================
// Request Types
// ============================================================================

/// Request to share an insight
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ShareInsightBody {
    /// Type of insight
    pub insight_type: String,
    /// Content to share
    pub content: String,
    /// Optional title
    pub title: Option<String>,
    /// Visibility setting
    pub visibility: Option<String>,
    /// Sport type context
    pub sport_type: Option<String>,
    /// Training phase context
    pub training_phase: Option<String>,
}

/// Request to generate a shareable insight from analysis content
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GenerateInsightBody {
    /// The analysis content to transform into a shareable insight
    pub content: String,
}

/// Request to react to an insight
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReactToInsightBody {
    /// Type of reaction
    pub reaction_type: String,
}

/// Request to adapt an insight
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AdaptInsightBody {
    /// Optional context to include in adaptation
    pub context: Option<String>,
    /// Provider to fetch activities from (defaults to environment default)
    pub provider: Option<String>,
    /// Tenant ID for multi-tenant contexts
    pub tenant_id: Option<String>,
}

/// Request to update helpful status
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateHelpfulBody {
    /// Whether the adaptation was helpful
    pub was_helpful: bool,
}

/// Request to share an insight from an activity (coach-mediated)
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ShareFromActivityBody {
    /// Activity ID that generated the insight (optional for chat-based insights)
    pub activity_id: Option<String>,
    /// Insight type to share
    pub insight_type: String,
    /// User-edited content (optional, uses suggestion if not provided)
    pub content: Option<String>,
    /// Visibility setting
    pub visibility: Option<String>,
    /// Provider to fetch activities from (defaults to environment default)
    pub provider: Option<String>,
    /// Tenant ID for multi-tenant contexts
    pub tenant_id: Option<String>,
}

// ============================================================================
// Query Types
// ============================================================================

/// Query parameters for listing insights
#[derive(Debug, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListInsightsQuery {
    /// Filter by insight type
    pub insight_type: Option<String>,
    /// Maximum results
    pub limit: Option<i64>,
    /// Offset for pagination
    pub offset: Option<i64>,
}

/// Query parameters for insight suggestions
#[derive(Debug, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SuggestionsQuery {
    /// Specific activity ID to generate suggestions for
    pub activity_id: Option<String>,
    /// Maximum suggestions to return
    pub limit: Option<usize>,
    /// Provider to fetch activities from (defaults to environment default)
    pub provider: Option<String>,
    /// Tenant ID for multi-tenant contexts
    pub tenant_id: Option<String>,
    /// Maximum activities to fetch for context generation (capped by server config)
    pub activity_limit: Option<usize>,
}

/// Query parameters for listing adapted insights
#[derive(Debug, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListAdaptedQuery {
    /// Maximum results
    pub limit: Option<i64>,
    /// Offset for pagination
    pub offset: Option<i64>,
}

// ============================================================================
// Handlers
// ============================================================================

impl SocialRoutes {
    /// Validate content for sharing based on user tier and sharing policy
    ///
    /// Returns the validated (and potentially improved/redacted) content, or an error if rejected.
    pub(crate) async fn validate_content_for_sharing(
        resources: &Arc<ServerContext>,
        social: &dyn SocialRepository,
        user_id: Uuid,
        content: &str,
        insight_type: InsightType,
    ) -> Result<String, AppError> {
        // SECURITY: Global lookup — sharing policy check on authenticated user
        let user = resources
            .repos
            .users
            .get_global(user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("User {user_id}")))?;

        // Get user's sharing policy from social settings
        let policy = social
            .get_social_settings(user_id)
            .await?
            .map(|s| s.insight_sharing_policy)
            .unwrap_or_default();

        // Get LLM provider from resources (injected for tests) or environment
        let llm_provider = Self::get_llm_provider(resources).await?;

        // Run validation
        let validation_prompt = resources.insight_validation_prompt();
        let result = validate_insight_with_policy(
            llm_provider.as_ref(),
            &validation_prompt,
            content,
            insight_type,
            &user.tier,
            &policy,
        )
        .await?;

        // Handle validation result
        match result.verdict {
            ValidationVerdict::Valid => Ok(result.final_content),
            ValidationVerdict::Improved { .. } => {
                // Use the improved/redacted content
                Ok(result.final_content)
            }
            ValidationVerdict::Rejected { reason } => Err(AppError::new(
                ErrorCode::InvalidInput,
                format!("Content cannot be shared: {reason}"),
            )),
        }
    }

    /// Get LLM provider from resources or create from environment
    ///
    /// Uses injected provider if available (for testing), otherwise falls back to
    /// `create_chat_provider()` which reads API keys from environment variables.
    pub(crate) async fn get_llm_provider(
        resources: &Arc<ServerContext>,
    ) -> Result<Arc<dyn LlmProvider>, AppError> {
        match &resources.llm_provider {
            Some(provider) => Ok(provider.clone()),
            None => Ok(Arc::new(super::super::create_chat_provider().await?)),
        }
    }

    /// Handle GET /api/social/insights - List user's shared insights
    pub(crate) async fn handle_list_insights(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Query(query): Query<ListInsightsQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let insight_type = query
            .insight_type
            .map(|t| InsightType::from_str(&t))
            .transpose()?;

        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let offset = query.offset.unwrap_or(0).max(0);
        // Safe cast: limit clamped to [1, 100], offset to [0, _], both fit in u32.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let (limit_u32, offset_u32) = (limit as u32, offset as u32);

        let insights = social
            .get_user_shared_insights(auth.user_id, insight_type, limit_u32, offset_u32)
            .await?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        #[allow(clippy::cast_possible_truncation)] // limit is clamped to small values
        let limit_usize = limit as usize;
        let has_more = insights.len() >= limit_usize;
        let next_cursor = if has_more {
            Some((offset + limit).to_string())
        } else {
            None
        };

        let response = ListInsightsResponse {
            total: insights.len(),
            insights: insights.into_iter().map(Into::into).collect(),
            next_cursor,
            has_more,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle POST /api/social/insights - Share a new insight
    pub(crate) async fn handle_share_insight(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Json(body): Json<ShareInsightBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let insight_type = InsightType::from_str(&body.insight_type)?;
        let visibility = body
            .visibility
            .map(|v| ShareVisibility::from_str(&v))
            .transpose()?
            .unwrap_or_default();
        let training_phase = body
            .training_phase
            .map(|p| TrainingPhase::from_str(&p))
            .transpose()?;

        // Validate content before sharing
        let validated_content = Self::validate_content_for_sharing(
            &resources,
            &*social,
            auth.user_id,
            &body.content,
            insight_type,
        )
        .await?;

        let mut insight =
            SharedInsight::new(auth.user_id, insight_type, validated_content, visibility);
        insight.title = body.title;
        insight.sport_type = body.sport_type;
        insight.training_phase = training_phase;

        social.create_shared_insight(&insight).await?;

        // Notify connected friends about the shared insight (both FriendsOnly and Public)
        #[cfg(feature = "client-notifications")]
        {
            if let Some(service) = &resources.notification_service {
                if let Some(tenant_uuid) = auth.active_tenant_id {
                    let tenant_id = TenantId::from(tenant_uuid);
                    let sharer_user = resources.repos.users.get_global(auth.user_id).await?;
                    let sharer_name = sharer_user
                        .and_then(|u| u.display_name)
                        .unwrap_or_else(|| "Someone".to_owned());
                    let insight_id_str = insight.id.to_string();

                    // Fan out notifications to all friends, paginating through the full list
                    let page_size: i64 = env::var("NOTIFICATION_FAN_OUT_PAGE_SIZE")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(notif_constants::FAN_OUT_PAGE_SIZE);
                    let mut offset: i64 = 0;
                    loop {
                        let Ok(friends) = social
                            .get_friends_paginated(auth.user_id, page_size, offset)
                            .await
                        else {
                            break;
                        };
                        let batch_len = friends.len();
                        for conn in friends {
                            let friend_id = if conn.initiator_id == auth.user_id {
                                conn.receiver_id
                            } else {
                                conn.initiator_id
                            };
                            notification_triggers::trigger_insight_shared(
                                service,
                                friend_id,
                                pierre_notifications::TenantId(tenant_id.0),
                                &insight_id_str,
                                &sharer_name,
                            );
                        }
                        #[allow(clippy::cast_possible_wrap)]
                        if (batch_len as i64) < page_size {
                            break;
                        }
                        offset += page_size;
                    }
                }
            }
        }

        let response: SharedInsightResponse = insight.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// Handle GET /api/social/insights/:id - Get a specific insight
    pub(crate) async fn handle_get_insight(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let insight_id =
            Uuid::parse_str(&id).map_err(|_| AppError::invalid_input("Invalid insight ID"))?;

        let insight = social
            .get_shared_insight(insight_id, auth.user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Insight {id}")))?;

        // Check visibility
        let is_friend = if insight.user_id == auth.user_id {
            false
        } else {
            social
                .get_friend_connection_between(auth.user_id, insight.user_id)
                .await?
                .is_some_and(|conn| conn.status.is_connected())
        };

        if !insight.is_visible_to(auth.user_id, is_friend) {
            return Err(AppError::not_found(format!("Insight {id}")));
        }

        let response: SharedInsightResponse = insight.into();
        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle DELETE /api/social/insights/:id - Delete an insight
    pub(crate) async fn handle_delete_insight(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let insight_id =
            Uuid::parse_str(&id).map_err(|_| AppError::invalid_input("Invalid insight ID"))?;

        let insight = social
            .get_shared_insight(insight_id, auth.user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Insight {id}")))?;

        // Only owner can delete
        if insight.user_id != auth.user_id {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "You can only delete your own insights",
            ));
        }

        social
            .delete_shared_insight(insight_id, auth.user_id)
            .await?;

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    // ========================================================================
    // Coach-Mediated Sharing
    // ========================================================================

    /// Handle GET /api/social/insights/suggestions - Get coach suggestions
    ///
    /// Returns suggestions based on user's recent activities. If no activities
    /// can be fetched (e.g., no OAuth token connected), returns an empty list.
    pub(crate) async fn handle_get_suggestions(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Query(query): Query<SuggestionsQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        // Use provider from query or fall back to environment default
        let provider_name = query.provider.unwrap_or_else(default_provider);

        // Build insight generation context from user's activities
        // If we can't fetch activities (no OAuth token), return empty suggestions
        let suggestions = match Self::build_insight_context(
            &resources,
            auth.user_id,
            &provider_name,
            query.tenant_id.as_deref(),
            query.activity_limit,
        )
        .await
        {
            Ok(context) => {
                // Generate suggestions using the SharedInsightGenerator
                let generator = SharedInsightGenerator::new();
                let mut suggestions = generator.generate_suggestions(&context);

                // Limit results if requested
                let limit = query.limit.unwrap_or(5).clamp(1, 20);
                suggestions.truncate(limit);

                // Convert to response format, adding activity_id if provided
                suggestions
                    .into_iter()
                    .map(|s| {
                        let mut response: InsightSuggestionResponse = s.into();
                        response.source_activity_id.clone_from(&query.activity_id);
                        response
                    })
                    .collect()
            }
            Err(_) => {
                // No activities available (e.g., no OAuth token) - return empty list
                Vec::new()
            }
        };

        let response = ListSuggestionsResponse {
            total: suggestions.len(),
            suggestions,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle POST /api/social/insights/from-activity - Share coach-mediated insight
    pub(crate) async fn handle_share_from_activity(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Json(body): Json<ShareFromActivityBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        // Generate activity_id for chat-based insights that don't have one
        let activity_id = body
            .activity_id
            .clone()
            .unwrap_or_else(|| format!("chat-insight-{}", Uuid::new_v4()));

        // Check if user has already shared an insight from this activity (skip for generated IDs)
        if body.activity_id.is_some()
            && social
                .has_insight_for_activity(auth.user_id, &activity_id)
                .await?
        {
            return Err(AppError::already_exists(format!(
                "Insight from activity '{activity_id}'"
            )));
        }

        let insight_type = InsightType::from_str(&body.insight_type)?;
        let visibility = body
            .visibility
            .map(|v| ShareVisibility::from_str(&v))
            .transpose()?
            .unwrap_or_default();

        // Use provider from body or fall back to environment default
        let provider_name = body.provider.unwrap_or_else(default_provider);

        // Default message when we can't generate coach content
        let default_message = format!(
            "Sharing a {} from my recent training!",
            insight_type.description()
        );

        // Build content: use custom content if provided, otherwise generate coach content
        let content = if let Some(custom_content) = body.content.clone() {
            custom_content
        } else {
            // Try to generate coach content based on user's activities
            // Falls back to default message if context building fails (e.g., no OAuth token)
            let generated = Self::build_insight_context(
                &resources,
                auth.user_id,
                &provider_name,
                body.tenant_id.as_deref(),
                None, // Use default server limit for share operations
            )
            .await
            .map_or_else(
                |error| {
                    tracing::debug!("Could not build insight context for coach content: {error}");
                    default_message.clone()
                },
                |context| {
                    let generator = SharedInsightGenerator::new();
                    let suggestions = generator.generate_suggestions(&context);

                    // Find a matching suggestion for the insight type
                    suggestions
                        .into_iter()
                        .find(|s| s.insight_type == insight_type)
                        .map_or_else(|| default_message.clone(), |s| s.suggested_content)
                },
            );
            generated
        };

        // Validate user-provided content through LLM to ensure quality
        // Content from body.content may have been edited by the user in the share modal
        let validated_content = if body.content.is_some() {
            Self::validate_insight_content(&resources, &content).await?
        } else {
            // Auto-generated content doesn't need validation
            content
        };

        // Create the coach-generated insight
        let insight = SharedInsight::coach_generated(
            auth.user_id,
            insight_type,
            validated_content,
            visibility,
            activity_id,
        );

        social.create_shared_insight(&insight).await?;

        let response: SharedInsightResponse = insight.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// Handle POST /api/social/insights/generate - Generate shareable insight from analysis
    ///
    /// Transforms analysis content into a concise, inspiring social post format
    /// using the insight generation prompt. Returns only the shareable content
    /// without any preamble or explanatory text.
    pub(crate) async fn handle_generate_insight(
        State(resources): State<Arc<ServerContext>>,
        _: AuthenticatedUser,
        Json(body): Json<GenerateInsightBody>,
    ) -> Result<Response, AppError> {
        // Validate input
        if body.content.trim().is_empty() {
            return Err(AppError::invalid_input("Content cannot be empty"));
        }

        // Get LLM provider from resources (injected for tests) or environment
        let llm_provider = Self::get_llm_provider(&resources).await?;

        // Build the generation request using the insight generation prompt
        let system_prompt = resources.insight_generation_prompt();
        let user_message = body.content.clone();

        let messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(user_message),
        ];

        // Use low temperature for consistent output format
        let request = ChatRequest::new(messages).with_temperature(0.4);

        // Call LLM to generate the shareable insight
        let llm_response = llm_provider.complete(&request).await?;

        // Parse JSON response to extract content field
        let content = Self::parse_generation_json_response(&llm_response.content);

        let response = GenerateInsightResponse {
            content,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Validate user-provided insight content through LLM
    ///
    /// Uses the insight validation prompt to check content quality and improve if needed.
    /// Returns the validated (possibly improved) content.
    pub(crate) async fn validate_insight_content(
        resources: &Arc<ServerContext>,
        content: &str,
    ) -> Result<String, AppError> {
        let llm_provider = Self::get_llm_provider(resources).await?;

        let system_prompt = resources.insight_validation_prompt();
        let messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(content),
        ];

        let request = ChatRequest::new(messages).with_temperature(0.3);
        let llm_response = llm_provider.complete(&request).await?;

        // Parse the validation response JSON
        Self::parse_validation_json_response(&llm_response.content, content)
    }

    /// Parse JSON response from insight generation prompt
    ///
    /// Expected format: `{"content": "..."}`
    pub(crate) fn parse_generation_json_response(raw_content: &str) -> String {
        #[derive(Deserialize)]
        struct GenerationResponse {
            content: String,
        }

        // Try direct JSON parse
        if let Ok(response) = serde_json::from_str::<GenerationResponse>(raw_content) {
            return response.content;
        }

        // Try extracting JSON from markdown code blocks
        let trimmed = raw_content.trim();
        if let Some(json_start) = trimmed.find('{') {
            if let Some(json_end) = trimmed.rfind('}') {
                let json_str = &trimmed[json_start..=json_end];
                if let Ok(response) = serde_json::from_str::<GenerationResponse>(json_str) {
                    return response.content;
                }
            }
        }

        // Fallback to raw content
        tracing::warn!(
            "Failed to parse insight generation JSON, using raw content: {}...",
            &raw_content[..raw_content.len().min(50)]
        );
        raw_content.trim().to_owned()
    }

    /// Parse JSON response from insight validation prompt
    ///
    /// Expected format: `{"verdict": "valid|improved|rejected", "reason": "...", "improved_content": "..."}`
    pub(crate) fn parse_validation_json_response(
        raw_content: &str,
        original_content: &str,
    ) -> Result<String, AppError> {
        #[derive(Deserialize)]
        struct ValidationResponse {
            verdict: String,
            reason: Option<String>,
            improved_content: Option<String>,
        }

        let parse_json =
            |json_str: &str| -> Option<ValidationResponse> { serde_json::from_str(json_str).ok() };

        // Try direct JSON parse
        let response = parse_json(raw_content).or_else(|| {
            // Try extracting JSON from markdown code blocks
            let trimmed = raw_content.trim();
            if let Some(json_start) = trimmed.find('{') {
                if let Some(json_end) = trimmed.rfind('}') {
                    return parse_json(&trimmed[json_start..=json_end]);
                }
            }
            None
        });

        if let Some(v) = response {
            match v.verdict.as_str() {
                "valid" => Ok(original_content.to_owned()),
                "improved" => Ok(v
                    .improved_content
                    .unwrap_or_else(|| original_content.to_owned())),
                "rejected" => {
                    let detail = v
                        .reason
                        .as_deref()
                        .unwrap_or("Please include specific training data or achievements.");
                    Err(AppError::invalid_input(format!(
                        "Content does not meet quality standards for sharing. {detail}"
                    )))
                }
                _ => {
                    tracing::warn!(
                        verdict = %v.verdict,
                        reason = ?v.reason,
                        "Unknown validation verdict"
                    );
                    Ok(original_content.to_owned())
                }
            }
        } else {
            // If parsing fails, accept the content (don't block sharing)
            tracing::warn!(
                "Failed to parse validation JSON, accepting content: {}...",
                &raw_content[..raw_content.len().min(50)]
            );
            Ok(original_content.to_owned())
        }
    }

    /// Fetch activities from the user's connected provider
    ///
    /// Uses `AuthService` to get a valid OAuth token and creates a configured provider
    /// with tenant-scoped credential resolution to fetch recent activities.
    pub(crate) async fn fetch_activities_from_provider(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        provider_name: &str,
        tenant_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Activity>, AppError> {
        use crate::protocols::universal::handlers::provider_helpers::{
            create_configured_provider_with_tenant, TenantCredentialContext,
        };

        let auth_service = AuthService::new(resources.clone());

        // Get valid OAuth token
        let token_data = auth_service
            .get_valid_token(user_id, provider_name, tenant_id)
            .await
            .map_err(|e| AppError::internal(format!("OAuth error: {e}")))?
            .ok_or_else(|| {
                AppError::auth_invalid(format!(
                    "No valid token for provider '{provider_name}'. Please connect your account."
                ))
            })?;

        // Build tenant credential context for tenant-scoped OAuth resolution
        let tenant_ctx = tenant_id
            .and_then(|tid| tid.parse::<TenantId>().ok())
            .map(|tid| TenantCredentialContext {
                tenant_oauth_client: &resources.tenant_oauth_client,
                tenants: resources.repos.tenants.as_ref(),
                oauth_tokens: resources.repos.oauth_tokens.as_ref(),
                tenant_id: tid,
                user_id,
            });

        // Create configured provider with tenant-scoped credentials
        let provider = create_configured_provider_with_tenant(
            provider_name,
            &resources.provider_registry,
            &token_data,
            tenant_ctx,
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to configure provider: {e}")))?;

        // Fetch activities
        provider
            .get_activities(limit, None)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch activities: {e}")))
    }

    /// Build insight generation context from user's recent activities
    ///
    /// # Arguments
    /// * `activity_limit` - Optional client-requested limit (capped by server config)
    pub(crate) async fn build_insight_context(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        provider_name: &str,
        tenant_id: Option<&str>,
        activity_limit: Option<usize>,
    ) -> Result<InsightGenerationContext, AppError> {
        let config = social::global();

        // Use client limit if provided, but cap at server's max client limit
        let effective_limit = activity_limit.map_or(
            config.activity_fetch_limits.insight_context_limit,
            |client_limit| client_limit.min(config.activity_fetch_limits.max_client_limit),
        );

        let activities = Self::fetch_activities_from_provider(
            resources,
            user_id,
            provider_name,
            tenant_id,
            Some(effective_limit),
        )
        .await?;

        // Build context using InsightContextBuilder
        let context = InsightContextBuilder::new()
            .with_activities(activities)
            .build();

        Ok(context)
    }

    /// Build user training context for insight adaptation
    pub(crate) async fn build_user_training_context(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        provider_name: &str,
        tenant_id: Option<&str>,
    ) -> Result<UserTrainingContext, AppError> {
        let config = social::global();
        let activities = Self::fetch_activities_from_provider(
            resources,
            user_id,
            provider_name,
            tenant_id,
            Some(config.activity_fetch_limits.training_context_limit),
        )
        .await?;

        // Calculate fitness metrics from activities
        let recent_activity_count = u32::try_from(activities.len()).unwrap_or(u32::MAX);
        let days_since_last = activities.first().map_or(365, |a| {
            let diff = Utc::now() - a.start_date();
            u32::try_from(diff.num_days().max(0)).unwrap_or(u32::MAX)
        });

        // Calculate weekly volume (approximate) using duration_seconds
        // Safe: duration_seconds is bounded by u64, and we need f64 for hours calculation
        #[allow(clippy::cast_precision_loss)]
        let weekly_volume_hours = activities
            .iter()
            .filter(|a| {
                let age = Utc::now() - a.start_date();
                age.num_days() <= 7
            })
            .map(|a| a.duration_seconds() as f64)
            .sum::<f64>()
            / 3600.0;

        // Determine primary sport
        let primary_sport = activities
            .first()
            .map(|a| a.sport_type().display_name().to_owned());

        Ok(UserTrainingContext {
            fitness_score: None,
            training_phase: None,
            weekly_volume_hours: Some(weekly_volume_hours),
            primary_sport,
            training_goal: None,
            recent_activity_count,
            days_since_last_workout: days_since_last,
        })
    }

    // ========================================================================
    // Reactions
    // ========================================================================

    /// Handle GET /api/social/insights/:id/reactions - List reactions
    pub(crate) async fn handle_list_reactions(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
    ) -> Result<Response, AppError> {
        // Authenticate user (must be logged in to view reactions)
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let insight_id =
            Uuid::parse_str(&id).map_err(|_| AppError::invalid_input("Invalid insight ID"))?;

        // Verify insight exists and user can view it
        social
            .get_shared_insight(insight_id, auth.user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Insight {id}")))?;

        let reactions = social.get_insight_reactions(insight_id).await?;

        // Build summary
        let mut summary = ReactionSummaryResponse::default();
        for reaction in &reactions {
            match reaction.reaction_type {
                ReactionType::Like => summary.like_count += 1,
                ReactionType::Celebrate => summary.celebrate_count += 1,
                ReactionType::Inspire => summary.inspire_count += 1,
                ReactionType::Support => summary.support_count += 1,
            }
            summary.total += 1;
        }

        let response = ListReactionsResponse {
            reactions: reactions.into_iter().map(Into::into).collect(),
            summary,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle POST /api/social/insights/:id/reactions - Add reaction
    pub(crate) async fn handle_add_reaction(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
        Json(body): Json<ReactToInsightBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let insight_id =
            Uuid::parse_str(&id).map_err(|_| AppError::invalid_input("Invalid insight ID"))?;

        let reaction_type = ReactionType::from_str(&body.reaction_type)?;

        // Verify insight exists and capture owner for notification
        let insight = social
            .get_shared_insight(insight_id, auth.user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Insight {id}")))?;

        // Check if user already has a reaction
        let existing = social
            .get_insight_reaction(insight_id, auth.user_id)
            .await?;
        if let Some(existing_reaction) = existing {
            if existing_reaction.reaction_type == reaction_type {
                // Same reaction type - should use remove endpoint to toggle
                return Err(AppError::invalid_input(
                    "You have already reacted with this type. Use remove to toggle.",
                ));
            }
            // Different reaction type - update to new type (delete old, create new)
            social
                .delete_insight_reaction(insight_id, auth.user_id)
                .await?;
        }

        let reaction = InsightReaction::new(insight_id, auth.user_id, reaction_type);
        social.upsert_insight_reaction(&reaction).await?;

        // Fire-and-forget notification to the insight owner (skip self-reactions)
        #[cfg(feature = "client-notifications")]
        if insight.user_id != auth.user_id {
            if let Some(service) = &resources.notification_service {
                if let Some(tenant_uuid) = auth.active_tenant_id {
                    let reactor_user = resources.repos.users.get_global(auth.user_id).await?;
                    let reactor_name = reactor_user
                        .and_then(|u| u.display_name)
                        .unwrap_or_else(|| "Someone".to_owned());
                    let insight_type_str = insight.insight_type.description();
                    notification_triggers::trigger_activity_kudos(
                        service,
                        insight.user_id,
                        pierre_notifications::TenantId::from_uuid(tenant_uuid),
                        &insight_id.to_string(),
                        &reactor_name,
                        insight_type_str,
                    );
                }
            }
        }

        let response: ReactionResponse = reaction.into();
        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// Handle DELETE /api/social/insights/:id/reactions/:type - Remove reaction
    pub(crate) async fn handle_remove_reaction(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Path((id, reaction_type_str)): Path<(String, String)>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let insight_id =
            Uuid::parse_str(&id).map_err(|_| AppError::invalid_input("Invalid insight ID"))?;

        // Validate the reaction type from URL (even though delete removes any reaction by user)
        ReactionType::from_str(&reaction_type_str)?;

        social
            .delete_insight_reaction(insight_id, auth.user_id)
            .await?;

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    // ========================================================================
    // Adapted Insights
    // ========================================================================

    /// Handle POST /api/social/insights/:id/adapt - Adapt an insight
    pub(crate) async fn handle_adapt_insight(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
        Json(body): Json<AdaptInsightBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let insight_id =
            Uuid::parse_str(&id).map_err(|_| AppError::invalid_input("Invalid insight ID"))?;

        // Use provider from body or fall back to environment default
        let provider_name = body.provider.clone().unwrap_or_else(default_provider);

        // Build user training context from their activities
        // If we can't fetch activities (no OAuth token), use a default context
        let user_context = Self::build_user_training_context(
            &resources,
            auth.user_id,
            &provider_name,
            body.tenant_id.as_deref(),
        )
        .await
        .unwrap_or_else(|_| UserTrainingContext::default());

        let result = social_insights::adapt_insight_for_user(
            &*social,
            auth.user_id,
            insight_id,
            &user_context,
            body.context.as_deref(),
        )
        .await?;

        let status = if result.already_existed {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        };

        let response = AdaptInsightResultResponse {
            adapted: result.adapted.into(),
            source_insight: result.source_insight.into(),
            metadata: Self::build_metadata(),
        };
        Ok((status, Json(response)).into_response())
    }

    /// Handle GET /api/social/adapted - List user's adapted insights
    pub(crate) async fn handle_list_adapted(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Query(query): Query<ListAdaptedQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let offset = query.offset.unwrap_or(0).max(0);

        let adapted = social
            .get_user_adapted_insights_paginated(auth.user_id, limit, offset)
            .await?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        #[allow(clippy::cast_possible_truncation)] // limit is clamped to small values
        let limit_usize = limit as usize;
        let has_more = adapted.len() >= limit_usize;
        let next_cursor = if has_more {
            Some((offset + limit).to_string())
        } else {
            None
        };

        let response = ListAdaptedInsightsResponse {
            total: adapted.len(),
            adapted_insights: adapted.into_iter().map(Into::into).collect(),
            next_cursor,
            has_more,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle PUT /api/social/adapted/:id/helpful - Update helpful status
    pub(crate) async fn handle_update_helpful(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Path(id): Path<String>,
        Json(body): Json<UpdateHelpfulBody>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let social = Self::get_social_manager(&resources)?;

        let adapted_id = Uuid::parse_str(&id)
            .map_err(|_| AppError::invalid_input("Invalid adapted insight ID"))?;

        let updated = social
            .update_adapted_insight_helpful(adapted_id, auth.user_id, body.was_helpful)
            .await?;

        if !updated {
            return Err(AppError::not_found(format!("Adapted insight {id}")));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }
}
