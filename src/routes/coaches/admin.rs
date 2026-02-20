// ABOUTME: Admin route handlers for system coach management and store moderation
// ABOUTME: Contains admin-only endpoints for CRUD on system coaches, assignments, and store review workflows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::{
    database::store_listings::{CoachWithListing, StoreListingsManager},
    errors::AppError,
    mcp::resources::ServerResources,
    middleware::require_admin,
    services::coaches as coaches_service,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pierre_core::models::coaches::UpdateCoachRequest;
use std::sync::Arc;

use super::types::{
    AdminCreateCoachBody, AssignCoachBody, AssignCoachResponse, CoachAssignment, CoachResponse,
    ListAssignmentsResponse, ListCoachesResponse, RejectCoachBody, StoreActionResponse,
    StoreAdminStatsResponse, StoreCoachResponse, StoreCoachesResponse, StoreListParams,
    UnassignCoachResponse, UpdateCoachBody,
};

/// Handle GET /admin/coaches - List all system coaches in tenant
pub(super) async fn handle_admin_list(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources)?;
    let coaches = manager.list_system_coaches(tenant_id).await?;

    let response = ListCoachesResponse {
        total: u32::try_from(coaches.len()).unwrap_or(0),
        coaches: coaches.into_iter().map(Into::into).collect(),
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /admin/coaches - Create a system coach
pub(super) async fn handle_admin_create(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(body): Json<AdminCreateCoachBody>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources)?;
    let coach = manager
        .create_system_coach(auth.user_id, tenant_id, &body.into())
        .await?;

    let response: CoachResponse = coach.into();
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Handle GET /admin/coaches/:id - Get a system coach
pub(super) async fn handle_admin_get(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources)?;
    let coach = manager
        .get_system_coach(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let response: CoachResponse = coach.into();
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle PUT /admin/coaches/:id - Update a system coach
pub(super) async fn handle_admin_update(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateCoachBody>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources)?;
    let request: UpdateCoachRequest = body.into();
    let coach = manager
        .update_system_coach(&id, tenant_id, &request)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let response: CoachResponse = coach.into();
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle DELETE /admin/coaches/:id - Delete a system coach
pub(super) async fn handle_admin_delete(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources)?;
    let deleted = manager.delete_system_coach(&id, tenant_id).await?;

    if !deleted {
        return Err(AppError::not_found(format!("System coach {id}")));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}

/// Handle POST /admin/coaches/:id/assign - Assign coach to users
///
/// Delegates tenant membership verification and bulk operations to
/// `services::coaches::bulk_assign_coach`.
pub(super) async fn handle_admin_assign(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<AssignCoachBody>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources)?;

    // Verify the coach exists and is a system coach
    manager
        .get_system_coach(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let result = coaches_service::bulk_assign_coach(
        &manager,
        resources.database.as_ref(),
        &id,
        tenant_id,
        auth.user_id,
        &body.user_ids,
    )
    .await?;

    let response = AssignCoachResponse {
        coach_id: id,
        assigned_count: result.affected_count,
        total_requested: result.total_requested,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle DELETE /admin/coaches/:id/assign - Remove coach assignment from users
///
/// Delegates tenant membership verification and bulk operations to
/// `services::coaches::bulk_unassign_coach`.
pub(super) async fn handle_admin_unassign(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<AssignCoachBody>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources)?;

    // Verify the coach exists
    manager
        .get_system_coach(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let result = coaches_service::bulk_unassign_coach(
        &manager,
        resources.database.as_ref(),
        &id,
        tenant_id,
        &body.user_ids,
    )
    .await?;

    let response = UnassignCoachResponse {
        coach_id: id,
        removed_count: result.affected_count,
        total_requested: result.total_requested,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /admin/coaches/:id/assignments - List users assigned to a coach
pub(super) async fn handle_admin_list_assignments(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources)?;

    // Verify the coach exists
    manager
        .get_system_coach(&id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {id}")))?;

    let db_assignments = manager.list_assignments_for_tenant(&id, tenant_id).await?;
    let assignments: Vec<CoachAssignment> = db_assignments.into_iter().map(Into::into).collect();

    let response = ListAssignmentsResponse {
        coach_id: id,
        assignments,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

// ============================================
// Admin Store Management Handlers
// ============================================

/// Handle GET /admin/store/stats - Get store statistics
pub(super) async fn handle_admin_store_stats(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&resources)?;
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

/// Handle GET /admin/store/review-queue - Get pending review coaches
pub(super) async fn handle_admin_review_queue(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Query(params): Query<StoreListParams>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&resources)?;
    let coaches = store_manager
        .get_pending_review_coaches(tenant_id, params.limit, params.offset)
        .await?;

    let coaches_with_email = enrich_coaches_with_email(&store_manager, coaches).await?;
    // Paginated results with limits - count never exceeds u32
    #[allow(clippy::cast_possible_truncation)]
    let total = coaches_with_email.len() as u32;

    let response = StoreCoachesResponse {
        coaches: coaches_with_email,
        total,
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /admin/store/published - Get published coaches
pub(super) async fn handle_admin_published(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Query(params): Query<StoreListParams>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;

    let store_manager = super::get_store_manager(&resources)?;
    let sort_by = params.sort_by.as_deref();
    let coaches = store_manager
        .get_published_coaches(None, sort_by, params.limit, params.offset)
        .await?;

    let coaches_with_email = enrich_coaches_with_email(&store_manager, coaches).await?;
    // Paginated results with limits - count never exceeds u32
    #[allow(clippy::cast_possible_truncation)]
    let total = coaches_with_email.len() as u32;

    let response = StoreCoachesResponse {
        coaches: coaches_with_email,
        total,
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /admin/store/rejected - Get rejected coaches
pub(super) async fn handle_admin_rejected(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Query(params): Query<StoreListParams>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&resources)?;
    let coaches = store_manager
        .get_rejected_coaches(tenant_id, params.limit, params.offset)
        .await?;

    let coaches_with_email = enrich_coaches_with_email(&store_manager, coaches).await?;
    // Paginated results with limits - count never exceeds u32
    #[allow(clippy::cast_possible_truncation)]
    let total = coaches_with_email.len() as u32;

    let response = StoreCoachesResponse {
        coaches: coaches_with_email,
        total,
        metadata: super::build_metadata(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /admin/store/coaches/:id/approve - Approve a coach
pub(super) async fn handle_admin_approve(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&resources)?;
    store_manager
        .approve_coach(&id, tenant_id, auth.user_id)
        .await?;

    let response = StoreActionResponse {
        success: true,
        message: "Coach approved and published".to_owned(),
        coach_id: id,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /admin/store/coaches/:id/reject - Reject a coach
pub(super) async fn handle_admin_reject(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RejectCoachBody>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let rejection_reason =
        coaches_service::format_rejection_reason(&body.reason, body.notes.as_deref());

    let store_manager = super::get_store_manager(&resources)?;
    store_manager
        .reject_coach(&id, tenant_id, auth.user_id, &rejection_reason)
        .await?;

    let response = StoreActionResponse {
        success: true,
        message: "Coach rejected".to_owned(),
        coach_id: id,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /admin/store/coaches/:id/unpublish - Unpublish a coach
pub(super) async fn handle_admin_unpublish(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::authenticate(&headers, &resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = super::get_user_tenant(&auth)?;

    let store_manager = super::get_store_manager(&resources)?;
    store_manager.unpublish_coach(&id, tenant_id).await?;

    let response = StoreActionResponse {
        success: true,
        message: "Coach unpublished".to_owned(),
        coach_id: id,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Enrich coaches with author email information
pub(super) async fn enrich_coaches_with_email(
    store_manager: &StoreListingsManager,
    coaches: Vec<CoachWithListing>,
) -> Result<Vec<StoreCoachResponse>, AppError> {
    let mut result = Vec::with_capacity(coaches.len());

    for cwl in coaches {
        let author_email = store_manager.get_author_email(cwl.coach.user_id).await?;
        result.push(StoreCoachResponse::from_coach_with_listing(
            cwl,
            author_email,
        ));
    }

    Ok(result)
}
