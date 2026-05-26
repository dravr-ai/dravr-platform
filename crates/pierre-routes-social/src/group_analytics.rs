// ABOUTME: Group coaching analytics routes — /stats, /report, /health from member fitness data
// ABOUTME: Generic over ToolRuntime + SocialCtx + MiddlewareCtx so the snapshot builder upcasts cleanly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Group coaching analytics routes
//!
//! Hosts the three analytics endpoints that compute aggregate stats,
//! weekly reports, and health flags from per-member fitness snapshots.
//! Routes are generic over `C: ToolRuntime + MiddlewareCtx + SocialCtx`
//! so the snapshot builder can be handed an `Arc<dyn ToolRuntime>` upcast
//! from the same `Arc<C>` that satisfies the social trait bounds.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::groups::{GroupMember, MemberFitnessSnapshot};
use pierre_core::models::TenantId;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{MiddlewareCtx, SocialCtx};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde::Deserialize;
use uuid::Uuid;

use crate::group_snapshots;
use crate::groups::{GroupMetadata, HealthFlagsResponse, StatsResponse, WeeklyReportResponse};
use pierre_auth::auth::AuthResult;

/// Optional query parameters for stats/report endpoints (currently unused
/// but reserved for `from` / `to` filtering once historical aggregates land).
#[derive(Debug, Deserialize, Default)]
pub struct PeriodQuery {
    /// Period start (ISO 8601). Defaults to 7 days ago.
    pub from: Option<DateTime<Utc>>,
    /// Period end (ISO 8601). Defaults to now.
    pub to: Option<DateTime<Utc>>,
}

/// Group analytics route registry.
pub struct GroupAnalyticsRoutes;

impl GroupAnalyticsRoutes {
    /// Mount the three analytics endpoints alongside
    /// [`crate::groups::GroupRoutes::routes`] in the composition root.
    pub fn routes<C: ToolRuntime + SocialCtx + MiddlewareCtx>(resources: Arc<C>) -> Router {
        Router::new()
            .route(
                "/api/groups/{group_id}/stats",
                get(Self::handle_get_stats::<C>),
            )
            .route(
                "/api/groups/{group_id}/report",
                get(Self::handle_get_weekly_report::<C>),
            )
            .route(
                "/api/groups/{group_id}/health",
                get(Self::handle_get_health_flags::<C>),
            )
            .with_state(resources)
    }

    /// Build response metadata.
    fn build_metadata() -> GroupMetadata {
        GroupMetadata {
            timestamp: Utc::now().to_rfc3339(),
            api_version: "1.0".to_owned(),
        }
    }

    /// Extract tenant ID from auth claims.
    fn get_tenant_id(auth: &AuthResult) -> Result<TenantId, AppError> {
        auth.active_tenant_id
            .map(TenantId::from)
            .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
    }

    /// Upcast a concrete state `Arc<C>` to a trait-object
    /// `Arc<dyn ToolRuntime>` for the snapshot helper.
    fn as_runtime<C: ToolRuntime>(resources: &Arc<C>) -> Arc<dyn ToolRuntime> {
        let cloned: Arc<C> = Arc::clone(resources);
        cloned
    }

    /// Verify the caller is a member of the given group.
    async fn require_member<C: ToolRuntime + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        MiddlewareCtx::repos(resources.as_ref())
            .groups
            .get_member(group_id, user_id)
            .await?
            .ok_or_else(|| AppError::not_found("You are not a member of this group"))
    }

    /// Verify the caller is an admin or owner of the given group.
    async fn require_admin<C: ToolRuntime + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        user_id: Uuid,
    ) -> Result<GroupMember, AppError> {
        let member = MiddlewareCtx::repos(resources.as_ref())
            .groups
            .get_member(group_id, user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;

        if !member.role.can_manage_members() {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Admin or owner role required for this operation",
            ));
        }

        Ok(member)
    }

    /// Fetch members and build fitness snapshots for the given group.
    async fn fetch_snapshots<C: ToolRuntime + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        tenant_id: &str,
    ) -> Result<Vec<MemberFitnessSnapshot>, AppError> {
        let members = MiddlewareCtx::repos(resources.as_ref())
            .groups
            .list_members(group_id)
            .await?;
        let runtime = Self::as_runtime(resources);
        Ok(group_snapshots::build_member_snapshots(&runtime, &members, tenant_id).await)
    }

    /// GET `/api/groups/:group_id/stats` — Aggregate stats for a group.
    async fn handle_get_stats<C: ToolRuntime + SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Query(_period): Query<PeriodQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let member_snapshots =
            Self::fetch_snapshots(&resources, &group_id, &tenant_id.to_string()).await?;

        let stats = resources
            .group_service()
            .compute_aggregate_stats(&member_snapshots);

        let response = StatsResponse {
            stats,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/report` — Weekly report for a group.
    async fn handle_get_weekly_report<C: ToolRuntime + SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Query(_period): Query<PeriodQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let group = MiddlewareCtx::repos(resources.as_ref())
            .groups
            .get_group(&group_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let member_snapshots =
            Self::fetch_snapshots(&resources, &group_id, &tenant_id.to_string()).await?;

        let report = resources
            .group_service()
            .compute_weekly_report(&member_snapshots, &group.name);

        let response = WeeklyReportResponse {
            report,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/health` — Health flags for group members.
    async fn handle_get_health_flags<C: ToolRuntime + SocialCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let member_snapshots =
            Self::fetch_snapshots(&resources, &group_id, &tenant_id.to_string()).await?;

        let flags = resources
            .group_service()
            .compute_health_flags(&member_snapshots);

        let response = HealthFlagsResponse {
            total: flags.len(),
            flags,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }
}
