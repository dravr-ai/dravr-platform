// ABOUTME: Admin analytics dashboard endpoints for overview, conversation, coach, and engagement metrics
// ABOUTME: Tenant-scoped read-only endpoints with cookie-based admin authentication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::{
    errors::AppError,
    llm::pricing::calculate_cost,
    mcp::resources::ServerResources,
    middleware::{extract_auth_from_headers, require_admin},
};
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{Duration, Utc};
use pierre_auth::auth::AuthResult;
use pierre_core::models::admin_analytics::{
    AnalyticsOverviewResponse, CoachAnalyticsResponse, ConversationAnalyticsResponse,
    EngagementAnalyticsResponse,
};
use pierre_core::models::TenantId;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{debug, info};

/// Minimum days for analytics window
const MIN_DAYS: u32 = 1;
/// Maximum days for analytics window
const MAX_DAYS: u32 = 365;
/// Default days for analytics window
const DEFAULT_DAYS: u32 = 7;
/// Maximum number of top coaches to return
const TOP_COACHES_LIMIT: i64 = 10;

/// Query parameters shared across analytics endpoints
#[derive(Debug, Deserialize)]
struct AnalyticsQuery {
    /// Number of days to look back (1-365, default 7)
    days: Option<u32>,
}

impl AnalyticsQuery {
    /// Return the clamped days value
    fn days(&self) -> u32 {
        self.days.unwrap_or(DEFAULT_DAYS).clamp(MIN_DAYS, MAX_DAYS)
    }
}

/// Get the admin user's tenant scope for analytics queries.
///
/// Super-admins see all tenants (returns error for now since analytics
/// require a specific tenant). Regular admins are scoped to their
/// `active_tenant_id` from JWT claims.
async fn resolve_tenant(
    resources: &Arc<ServerResources>,
    auth: &AuthResult,
) -> Result<TenantId, AppError> {
    let user = resources
        .repos
        .users
        .get_global(auth.user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch admin user: {e}")))?
        .ok_or_else(|| AppError::not_found("Admin user not found"))?;

    // Super-admins must select a tenant context for analytics
    if user.role.is_super_admin() {
        return auth.active_tenant_id.map(TenantId::from).ok_or_else(|| {
            AppError::invalid_input("Super-admin must select a tenant for analytics queries")
        });
    }

    auth.active_tenant_id
        .map(TenantId::from)
        .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
}

/// Admin analytics routes
pub struct AdminAnalyticsRoutes;

impl AdminAnalyticsRoutes {
    /// Create all admin analytics routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route("/api/admin/analytics/overview", get(Self::handle_overview))
            .route(
                "/api/admin/analytics/conversations",
                get(Self::handle_conversations),
            )
            .route("/api/admin/analytics/coaches", get(Self::handle_coaches))
            .route(
                "/api/admin/analytics/engagement",
                get(Self::handle_engagement),
            )
            .with_state(resources)
    }

    /// Authenticate and verify admin privileges
    async fn authenticate_admin(
        headers: &HeaderMap,
        resources: &Arc<ServerResources>,
    ) -> Result<AuthResult, AppError> {
        let auth = extract_auth_from_headers(headers, resources).await?;
        require_admin(auth.user_id, &resources.repos.users).await?;
        Ok(auth)
    }

    /// Overview dashboard: key platform metrics at a glance
    async fn handle_overview(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(query): Query<AnalyticsQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        let tenant_id = resolve_tenant(&resources, &auth).await?;
        let days = query.days();
        let since = Utc::now() - Duration::days(i64::from(days));

        info!(
            user_id = %auth.user_id,
            tenant_id = %tenant_id,
            days = days,
            "Admin analytics: overview"
        );

        let repo = &resources.repos.admin_analytics;

        // Parallelize independent database queries
        let (
            total_users,
            total_conversations,
            total_messages,
            llm_rows,
            users_with_coach,
            connected_providers,
            active_groups,
        ) = tokio::join!(
            repo.count_users_for_tenant(tenant_id),
            repo.count_conversations_since(tenant_id, since),
            repo.count_messages_since(tenant_id, since),
            repo.get_llm_usage_for_cost(tenant_id, since),
            repo.count_users_with_coach_assignment(tenant_id),
            repo.count_connected_providers(tenant_id),
            repo.count_active_groups(tenant_id),
        );

        let total_users = total_users?;
        let total_conversations = total_conversations?;
        let total_messages = total_messages?;
        let llm_rows = llm_rows?;
        let users_with_coach = users_with_coach?;
        let connected_providers = connected_providers?;
        let active_groups = active_groups?;

        // Calculate total LLM cost from aggregated rows
        let total_llm_cost_usd: f64 = llm_rows
            .iter()
            .map(|r| calculate_cost(&r.provider, &r.model, r.prompt_tokens, r.completion_tokens))
            .sum();

        // Zero-division guard for coach adoption percentage
        let coach_adoption_pct = if total_users > 0 {
            #[allow(clippy::cast_precision_loss)]
            let pct = (users_with_coach as f64 / total_users.max(1) as f64) * 100.0;
            pct.clamp(0.0, 100.0)
        } else {
            0.0
        };

        debug!(
            total_users,
            total_conversations,
            total_messages,
            total_llm_cost_usd,
            coach_adoption_pct,
            "Analytics overview computed"
        );

        Ok(Json(AnalyticsOverviewResponse {
            total_users,
            total_conversations,
            total_messages,
            total_llm_cost_usd,
            coach_adoption_pct,
            connected_providers,
            active_groups,
            days,
        })
        .into_response())
    }

    /// Conversation analytics: time series, peak hours, message depth
    async fn handle_conversations(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(query): Query<AnalyticsQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        let tenant_id = resolve_tenant(&resources, &auth).await?;
        let days = query.days();
        let since = Utc::now() - Duration::days(i64::from(days));

        info!(
            user_id = %auth.user_id,
            tenant_id = %tenant_id,
            days = days,
            "Admin analytics: conversations"
        );

        let repo = &resources.repos.admin_analytics;

        let (daily_series, peak_hour, avg_messages) = tokio::join!(
            repo.get_daily_conversation_series(tenant_id, since),
            repo.get_peak_conversation_hour(tenant_id, since),
            repo.get_avg_messages_per_conversation(tenant_id, since),
        );

        Ok(Json(ConversationAnalyticsResponse {
            daily_series: daily_series?,
            peak_hour: peak_hour?,
            avg_messages_per_conversation: avg_messages?,
            days,
        })
        .into_response())
    }

    /// Coach analytics: rankings, category distribution, system vs user coaches
    async fn handle_coaches(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(query): Query<AnalyticsQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        let tenant_id = resolve_tenant(&resources, &auth).await?;
        let days = query.days();

        info!(
            user_id = %auth.user_id,
            tenant_id = %tenant_id,
            days = days,
            "Admin analytics: coaches"
        );

        let repo = &resources.repos.admin_analytics;

        let (top_coaches, category_distribution, system_user_counts, store_installations) = tokio::join!(
            repo.get_top_coaches(tenant_id, TOP_COACHES_LIMIT),
            repo.get_coach_category_distribution(tenant_id),
            repo.get_system_vs_user_coach_counts(tenant_id),
            repo.count_store_installations(tenant_id),
        );

        let (system_coach_count, user_coach_count) = system_user_counts?;

        Ok(Json(CoachAnalyticsResponse {
            top_coaches: top_coaches?,
            category_distribution: category_distribution?,
            system_coach_count,
            user_coach_count,
            store_installations: store_installations?,
            days,
        })
        .into_response())
    }

    /// Engagement analytics: DAU time series, tiers, providers, groups
    async fn handle_engagement(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(query): Query<AnalyticsQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        let tenant_id = resolve_tenant(&resources, &auth).await?;
        let days = query.days();
        let since = Utc::now() - Duration::days(i64::from(days));

        info!(
            user_id = %auth.user_id,
            tenant_id = %tenant_id,
            days = days,
            "Admin analytics: engagement"
        );

        let repo = &resources.repos.admin_analytics;

        let (daily_active_users, engagement_tiers, provider_distribution, group_stats) = tokio::join!(
            repo.get_daily_active_users_series(tenant_id, since),
            repo.get_engagement_tiers(tenant_id),
            repo.get_provider_distribution(tenant_id),
            repo.get_group_stats(tenant_id),
        );

        Ok(Json(EngagementAnalyticsResponse {
            daily_active_users: daily_active_users?,
            engagement_tiers: engagement_tiers?,
            provider_distribution: provider_distribution?,
            group_stats: group_stats?,
            days,
        })
        .into_response())
    }
}
