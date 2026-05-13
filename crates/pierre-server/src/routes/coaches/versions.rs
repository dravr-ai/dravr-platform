// ABOUTME: Version history route handlers for coach versioning endpoints
// ABOUTME: Provides listing, retrieval, revert, and diff operations on coach version snapshots
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::{
    errors::AppError, mcp::resources::ServerContext, middleware::AuthenticatedUser,
    services::recipes as recipes_service,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

use super::types::{
    CoachDiffResponse, CoachVersionResponse, FieldChange, ListVersionsQuery, ListVersionsResponse,
    RevertVersionResponse,
};

/// Handle GET /api/coaches/:id/versions - List version history
pub(super) async fn handle_list_versions(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Query(query): Query<ListVersionsQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let versions = manager.get_versions(&id, tenant_id, limit).await?;
    let current_version = manager.get_current_version(&id).await?;

    let version_responses: Vec<CoachVersionResponse> =
        versions.into_iter().map(Into::into).collect();

    let response = ListVersionsResponse {
        total: version_responses.len(),
        versions: version_responses,
        current_version,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/coaches/:id/versions/:version - Get a specific version
pub(super) async fn handle_get_version(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((id, version)): Path<(String, i32)>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let version_data = manager
        .get_version(&id, version, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Version {version} for coach {id}")))?;

    let response: CoachVersionResponse = version_data.into();
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/coaches/:id/versions/:version/revert - Revert to a version
pub(super) async fn handle_revert_version(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((id, version)): Path<(String, i32)>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);
    let coach = manager
        .revert_to_version(&id, version, auth.user_id, tenant_id)
        .await?;

    let new_version = manager.get_current_version(&id).await?;

    let response = RevertVersionResponse {
        coach: coach.into(),
        reverted_to_version: version,
        new_version,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle GET /api/coaches/:id/versions/:v1/diff/:v2 - Compare two versions
pub(super) async fn handle_diff_versions(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((id, v1, v2)): Path<(String, i32, i32)>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&resources);

    let version1 = manager
        .get_version(&id, v1, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Version {v1} for coach {id}")))?;

    let version2 = manager
        .get_version(&id, v2, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Version {v2} for coach {id}")))?;

    // Compare the content snapshots
    let changes = compute_diff(&version1.content_snapshot, &version2.content_snapshot);

    let response = CoachDiffResponse {
        from_version: v1,
        to_version: v2,
        changes,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Compute field-level differences between two JSON snapshots
///
/// Delegates to `services::recipes::compute_version_diff`.
fn compute_diff(from: &serde_json::Value, to: &serde_json::Value) -> Vec<FieldChange> {
    recipes_service::compute_version_diff(from, to)
        .into_iter()
        .map(|c| FieldChange {
            field: c.field,
            old_value: c.old_value,
            new_value: c.new_value,
        })
        .collect()
}
