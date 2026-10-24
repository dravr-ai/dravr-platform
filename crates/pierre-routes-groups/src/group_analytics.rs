// ABOUTME: Group coaching analytics routes — /stats, /report, /health from member fitness data
// ABOUTME: Generic over ToolRuntime + GroupsCtx + MiddlewareCtx so the snapshot builder upcasts cleanly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Group coaching analytics routes
//!
//! Hosts the three analytics endpoints that compute aggregate stats,
//! weekly reports, and health flags from per-member fitness snapshots —
//! only ever the snapshots of the members whose training the caller may see
//! ([`GroupService::visible_member_ids`]), whatever their role.
//! Routes are generic over `C: ToolRuntime + MiddlewareCtx + GroupsCtx`
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
use pierre_groups::GroupService;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{GroupsCtx, MiddlewareCtx};
use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde::Deserialize;
use uuid::Uuid;

use crate::groups::{
    AnalyticsScope, GroupMetadata, HealthFlagsResponse, StatsResponse, WeeklyReportResponse,
};
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
    pub fn routes<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(resources: Arc<C>) -> Router {
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
            .map(TenantId::from_uuid)
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

    /// The fitness snapshots of the members whose training `viewer` may see
    /// in the given group, and the scope they cover.
    ///
    /// Reads the one visibility rule,
    /// [`GroupService::visible_member_ids`]: the group's human coach sees the
    /// members who share with the coach, anyone else — owner and admins
    /// included — the members who share with their peers, and everyone sees
    /// themself (carnet#1166). Every aggregate is computed over that set
    /// alone: a total over a member left out would give their week away to
    /// anyone who subtracts the sharers' figures the weekly digest posts into
    /// the group's chat.
    ///
    /// Delegates to the canonical
    /// [`fetch_member_snapshots`](pierre_tool_runtime::group_fitness::fetch_member_snapshots)
    /// builder so the REST analytics endpoints see the exact same
    /// all-providers + deduplicated snapshots the chat agent sees, and reads
    /// no other member's training.
    async fn visible_snapshots<C: ToolRuntime + MiddlewareCtx>(
        resources: &Arc<C>,
        group_id: &str,
        tenant_id: TenantId,
        viewer: Uuid,
    ) -> Result<(Vec<MemberFitnessSnapshot>, AnalyticsScope), AppError> {
        let repos = MiddlewareCtx::repos(resources.as_ref());
        // A group id with no row is a 404 before any snapshot is fetched.
        let group = repos
            .groups
            .get_group(group_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;
        let members = repos.groups.list_members(group_id).await?;
        let visible = GroupService::visible_member_ids(&group, &members, viewer);
        let user_ids: Vec<Uuid> = members
            .iter()
            .map(|m| m.user_id)
            .filter(|id| visible.contains(id))
            .collect();
        let scope = AnalyticsScope {
            visible_members: user_ids.len(),
            roster_members: members.len(),
        };
        let runtime = Self::as_runtime(resources);
        let snapshots = fetch_member_snapshots(&runtime, &user_ids, tenant_id).await;
        Ok((snapshots, scope))
    }

    /// GET `/api/groups/:group_id/stats` — Aggregate stats for a group.
    async fn handle_get_stats<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Query(_period): Query<PeriodQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        Self::require_member(&resources, &group_id, auth.user_id).await?;

        let (member_snapshots, scope) =
            Self::visible_snapshots(&resources, &group_id, tenant_id, auth.user_id).await?;

        let stats = resources
            .group_service()
            .compute_aggregate_stats(&member_snapshots);

        let response = StatsResponse {
            stats,
            scope,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/report` — Weekly report for a group.
    async fn handle_get_weekly_report<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
        Query(_period): Query<PeriodQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let (member_snapshots, scope) =
            Self::visible_snapshots(&resources, &group_id, tenant_id, auth.user_id).await?;

        let report = resources
            .group_service()
            .compute_weekly_report(&member_snapshots);

        let response = WeeklyReportResponse {
            report,
            scope,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// GET `/api/groups/:group_id/health` — Health flags for group members.
    async fn handle_get_health_flags<C: ToolRuntime + GroupsCtx + MiddlewareCtx>(
        State(resources): State<Arc<C>>,
        auth: AuthenticatedUser,
        Path(group_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        Self::require_admin(&resources, &group_id, auth.user_id).await?;

        let (member_snapshots, scope) =
            Self::visible_snapshots(&resources, &group_id, tenant_id, auth.user_id).await?;

        let flags = GroupService::compute_health_flags(&member_snapshots);

        let response = HealthFlagsResponse {
            total: flags.len(),
            flags,
            scope,
            metadata: Self::build_metadata(),
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }
}
