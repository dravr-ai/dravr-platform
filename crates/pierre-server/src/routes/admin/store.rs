// ABOUTME: Admin store review queue route handlers
// ABOUTME: Handles listing, approval, and rejection of coaches pending admin review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde_json::json;
use tracing::info;

use crate::{
    admin::models::{AdminPermission, ValidatedAdminToken},
    errors::{AppError, AppResult},
    models::TenantId,
};
use pierre_database::plugins::StoreListingsRepository;

use super::api_keys::json_response;
use super::types::{CoachReviewQuery, ListPendingCoachesQuery, RejectCoachRequest};
use super::AdminApiContext;

/// List coaches pending admin review
pub(super) async fn handle_list_pending_coaches(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<ListPendingCoachesQuery>,
) -> AppResult<impl IntoResponse> {
    if !admin_token.is_super_admin
        && !admin_token
            .permissions
            .has_permission(&AdminPermission::ManageUsers)
    {
        return Ok(json_response(
            json!({"error": "Insufficient permissions to view review queue"}),
            StatusCode::FORBIDDEN,
        ));
    }

    let repo: &dyn StoreListingsRepository = context.repos.store_listings.as_ref();
    let tenant_id: TenantId = query
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", query.tenant_id)))?;

    let coaches = repo
        .get_pending_review_coaches(tenant_id, query.limit, query.offset)
        .await?;

    info!(
        "Admin {} listed {} pending coaches for review in tenant {}",
        admin_token.service_name,
        coaches.len(),
        query.tenant_id
    );

    Ok(json_response(
        json!({
            "coaches": coaches,
            "count": coaches.len()
        }),
        StatusCode::OK,
    ))
}

/// Approve a coach for publishing to the Store
pub(super) async fn handle_approve_coach(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(coach_id): Path<String>,
    Query(query): Query<CoachReviewQuery>,
) -> AppResult<impl IntoResponse> {
    if !admin_token.is_super_admin
        && !admin_token
            .permissions
            .has_permission(&AdminPermission::ManageUsers)
    {
        return Ok(json_response(
            json!({"error": "Insufficient permissions to approve coaches"}),
            StatusCode::FORBIDDEN,
        ));
    }

    let repo: &dyn StoreListingsRepository = context.repos.store_listings.as_ref();
    let tenant_id: TenantId = query
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", query.tenant_id)))?;

    let coach = repo.approve_coach(&coach_id, tenant_id, None).await?;

    info!(
        "Admin {} approved coach {} for Store in tenant {}",
        admin_token.service_name, coach_id, query.tenant_id
    );

    Ok(json_response(
        json!({
            "message": "Coach approved and published to Store",
            "coach": coach
        }),
        StatusCode::OK,
    ))
}

/// Reject a coach with a reason
pub(super) async fn handle_reject_coach(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(coach_id): Path<String>,
    Query(query): Query<CoachReviewQuery>,
    Json(request): Json<RejectCoachRequest>,
) -> AppResult<impl IntoResponse> {
    if !admin_token.is_super_admin
        && !admin_token
            .permissions
            .has_permission(&AdminPermission::ManageUsers)
    {
        return Ok(json_response(
            json!({"error": "Insufficient permissions to reject coaches"}),
            StatusCode::FORBIDDEN,
        ));
    }

    if request.reason.trim().is_empty() {
        return Ok(json_response(
            json!({"error": "Rejection reason is required"}),
            StatusCode::BAD_REQUEST,
        ));
    }

    let repo: &dyn StoreListingsRepository = context.repos.store_listings.as_ref();
    let tenant_id: TenantId = query
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", query.tenant_id)))?;

    let coach = repo
        .reject_coach(&coach_id, tenant_id, None, &request.reason)
        .await?;

    info!(
        "Admin {} rejected coach {} in tenant {} with reason: {}",
        admin_token.service_name, coach_id, query.tenant_id, request.reason
    );

    Ok(json_response(
        json!({
            "message": "Coach rejected",
            "coach": coach
        }),
        StatusCode::OK,
    ))
}
