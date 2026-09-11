// ABOUTME: Admin route handlers for system agent management and store moderation
// ABOUTME: Contains admin-only endpoints for CRUD on system agents, assignments, and store review workflows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pierre_core::errors::AppError;
use pierre_core::models::agents::{Agent, UpdateAgentRequest};
use pierre_database::database::store_listings::AgentWithListing;
use pierre_middleware::{require_admin, AuthenticatedUser};
use pierre_runtime_context::{AgentsCtx, MiddlewareCtx};
use pierre_services::agent_package::review_package;
use pierre_services::agents as agents_service;
use pierre_tool_runtime::runtime::ToolRuntime;

#[cfg(feature = "client-notifications")]
use pierre_notifications::triggers as notification_triggers;

use super::types::{
    AdminCreateAgentBody, AgentAssignment, AgentResponse, AssignAgentBody, AssignAgentResponse,
    ListAgentsResponse, ListAssignmentsResponse, RejectAgentBody, StoreActionResponse,
    StoreAdminStatsResponse, StoreAgentResponse, StoreAgentsResponse, StoreListParams,
    UnassignAgentResponse, UpdateAgentBody,
};

/// Handle GET /admin/agents - List all system agents in tenant
pub(super) async fn handle_admin_list<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let agents = manager.list_system_agents(tenant_id).await?;

    let response = ListAgentsResponse {
        total: u32::try_from(agents.len()).unwrap_or(0),
        agents: agents.into_iter().map(Into::into).collect(),
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /admin/agents - Create a system agent
pub(super) async fn handle_admin_create<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Json(body): Json<AdminCreateAgentBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let agent = manager
        .create_system_agent(auth.user_id, tenant_id, &body.into())
        .await?;

    let response: AgentResponse = agent.into();
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle GET /admin/agents/:id - Get a system agent
pub(super) async fn handle_admin_get<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let agent = manager
        .get_system_agent(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let response: AgentResponse = agent.into();
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle PUT /admin/agents/:id - Update a system agent
pub(super) async fn handle_admin_update<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateAgentBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let request: UpdateAgentRequest = body.into();
    let agent = manager
        .update_system_agent(&id, tenant_id, &request)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    // Notify users assigned to this agent that their training plan was updated
    #[cfg(feature = "client-notifications")]
    if let Some(service) = ctx.notification_service() {
        let agent_name = agent.title.clone();
        if let Ok(assignments) = manager.list_assignments_for_tenant(&id, tenant_id).await {
            for assignment in assignments {
                if let Ok(user_uuid) = assignment.user_id.parse::<uuid::Uuid>() {
                    notification_triggers::trigger_plan_updated(
                        service,
                        user_uuid,
                        pierre_notifications::TenantId(tenant_id.as_uuid()),
                        &agent_name,
                    );
                }
            }
        }
    }

    let response: AgentResponse = agent.into();
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle DELETE /admin/agents/:id - Delete a system agent
pub(super) async fn handle_admin_delete<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);
    let deleted = manager.delete_system_agent(&id, tenant_id).await?;

    if !deleted {
        return Err(AppError::not_found(format!("System coach {id}")));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

/// Handle POST /admin/agents/:id/assign - Assign agent to users
///
/// Delegates tenant membership verification and bulk operations to
/// `services::agents::bulk_assign_agent`.
pub(super) async fn handle_admin_assign<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Json(body): Json<AssignAgentBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);

    // Verify the agent exists and is a system agent (also used for notification body)
    let agent: Agent = manager
        .get_system_agent(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let result = agents_service::bulk_assign_agent(
        manager,
        ctx.repos().tenants.as_ref(),
        &id,
        tenant_id,
        auth.user_id,
        &body.user_ids,
    )
    .await?;

    // Notify each assigned user about the new agent assignment
    #[cfg(feature = "client-notifications")]
    if let Some(service) = ctx.notification_service() {
        let agent_name = agent.title.clone();
        for user_id_str in &body.user_ids {
            if let Ok(user_uuid) = user_id_str.parse::<uuid::Uuid>() {
                notification_triggers::trigger_plan_updated(
                    service,
                    user_uuid,
                    pierre_notifications::TenantId(tenant_id.as_uuid()),
                    &agent_name,
                );
            }
        }
    }

    // When notifications are disabled the agent binding is unused — silence
    // the warning without dropping the lookup (it doubles as a 404 check).
    #[cfg(not(feature = "client-notifications"))]
    let _ = agent;

    let response = AssignAgentResponse {
        agent_id: id,
        assigned_count: result.affected_count,
        total_requested: result.total_requested,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle DELETE /admin/agents/:id/assign - Remove agent assignment from users
///
/// Delegates tenant membership verification and bulk operations to
/// `services::agents::bulk_unassign_agent`.
pub(super) async fn handle_admin_unassign<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Json(body): Json<AssignAgentBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);

    // Verify the agent exists
    manager
        .get_system_agent(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let result = agents_service::bulk_unassign_agent(
        manager,
        ctx.repos().tenants.as_ref(),
        &id,
        tenant_id,
        &body.user_ids,
    )
    .await?;

    let response = UnassignAgentResponse {
        agent_id: id,
        removed_count: result.affected_count,
        total_requested: result.total_requested,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /admin/agents/:id/assignments - List users assigned to an agent
pub(super) async fn handle_admin_list_assignments<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_agents_manager(&ctx);

    // Verify the agent exists
    manager
        .get_system_agent(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let db_assignments = manager.list_assignments_for_tenant(&id, tenant_id).await?;
    let assignments: Vec<AgentAssignment> = db_assignments.into_iter().map(Into::into).collect();

    let response = ListAssignmentsResponse {
        agent_id: id,
        assignments,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

// ============================================
// Admin Store Management Handlers
// ============================================

/// Handle GET /admin/store/stats - Get store statistics
pub(super) async fn handle_admin_store_stats<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&ctx);
    let stats = store_manager.get_store_admin_stats(tenant_id).await?;

    let response = StoreAdminStatsResponse {
        pending_count: stats.pending_count,
        published_count: stats.published_count,
        rejected_count: stats.rejected_count,
        total_installs: stats.total_installs,
        rejection_rate: stats.rejection_rate,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /admin/store/review-queue - Get pending review agents
pub(super) async fn handle_admin_review_queue<C: AgentsCtx + MiddlewareCtx + ToolRuntime>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(params): Query<StoreListParams>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &MiddlewareCtx::repos(ctx.as_ref()).users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&ctx);
    let agents = store_manager
        .get_pending_review_agents(tenant_id, params.limit, params.offset)
        .await?;

    let agents_with_email = enrich_store_agents(&ctx, agents).await?;
    // Paginated results with limits - count never exceeds u32
    #[allow(clippy::cast_possible_truncation)]
    let total = agents_with_email.len() as u32;

    let response = StoreAgentsResponse {
        agents: agents_with_email,
        total,
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /admin/store/published - Get published agents
pub(super) async fn handle_admin_published<C: AgentsCtx + MiddlewareCtx + ToolRuntime>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(params): Query<StoreListParams>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &MiddlewareCtx::repos(ctx.as_ref()).users).await?;

    let store_manager = super::get_store_manager(&ctx);
    let sort_by = params.sort_by.as_deref();
    let agents = store_manager
        .get_published_agents(None, sort_by, params.limit, params.offset)
        .await?;

    let agents_with_email = enrich_store_agents(&ctx, agents).await?;
    // Paginated results with limits - count never exceeds u32
    #[allow(clippy::cast_possible_truncation)]
    let total = agents_with_email.len() as u32;

    let response = StoreAgentsResponse {
        agents: agents_with_email,
        total,
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /admin/store/rejected - Get rejected agents
pub(super) async fn handle_admin_rejected<C: AgentsCtx + MiddlewareCtx + ToolRuntime>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(params): Query<StoreListParams>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &MiddlewareCtx::repos(ctx.as_ref()).users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&ctx);
    let agents = store_manager
        .get_rejected_agents(tenant_id, params.limit, params.offset)
        .await?;

    let agents_with_email = enrich_store_agents(&ctx, agents).await?;
    // Paginated results with limits - count never exceeds u32
    #[allow(clippy::cast_possible_truncation)]
    let total = agents_with_email.len() as u32;

    let response = StoreAgentsResponse {
        agents: agents_with_email,
        total,
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /admin/store/agents/:id/approve - Approve an agent
pub(super) async fn handle_admin_approve<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&ctx);
    store_manager
        .approve_agent(&id, tenant_id, Some(auth.user_id))
        .await?;

    let response = StoreActionResponse {
        success: true,
        message: "Agent approved and published".to_owned(),
        agent_id: id,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /admin/store/agents/:id/reject - Reject an agent
pub(super) async fn handle_admin_reject<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Json(body): Json<RejectAgentBody>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let rejection_reason =
        agents_service::format_rejection_reason(&body.reason, body.notes.as_deref());

    let store_manager = super::get_store_manager(&ctx);
    store_manager
        .reject_agent(&id, tenant_id, Some(auth.user_id), &rejection_reason)
        .await?;

    let response = StoreActionResponse {
        success: true,
        message: "Agent rejected".to_owned(),
        agent_id: id,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /admin/store/agents/:id/unpublish - Unpublish an agent
pub(super) async fn handle_admin_unpublish<C: AgentsCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    require_admin(auth.user_id, &ctx.repos().users).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&ctx);
    store_manager.unpublish_agent(&id, tenant_id).await?;

    let response = StoreActionResponse {
        success: true,
        message: "Agent unpublished".to_owned(),
        agent_id: id,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Enrich store agents with the author's email and the package review —
/// every artefact the agent ships, with its unresolved references checked
/// against the live catalogue and evidence corpus.
pub(super) async fn enrich_store_agents<C: AgentsCtx + ToolRuntime>(
    ctx: &Arc<C>,
    agents: Vec<AgentWithListing>,
) -> Result<Vec<StoreAgentResponse>, AppError> {
    let store_manager = super::get_store_manager(ctx);
    let mut result = Vec::with_capacity(agents.len());

    for cwl in agents {
        let author_email = store_manager.get_author_email(cwl.agent.user_id).await?;
        let rows = MiddlewareCtx::repos(ctx.as_ref())
            .agent_artefacts
            .list_agent_artefacts(&cwl.agent.tenant_id, &cwl.agent.id.to_string())
            .await?;
        let package = review_package(&rows, ctx.training_catalogue(), ctx.evidence_registry());
        result.push(StoreAgentResponse::from_agent_with_listing(
            cwl,
            author_email,
            package,
        ));
    }

    Ok(result)
}
