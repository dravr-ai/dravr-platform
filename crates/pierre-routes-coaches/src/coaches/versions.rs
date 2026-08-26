// ABOUTME: Version history route handlers for coach versioning endpoints
// ABOUTME: Provides listing, retrieval, revert, and diff operations on coach version snapshots
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{CoachesCtx, MiddlewareCtx};
use pierre_services::recipes as recipes_service;
use uuid::Uuid;

use super::types::{
    CoachDiffResponse, CoachVersionResponse, FieldChange, ListVersionsQuery, ListVersionsResponse,
    RevertVersionResponse,
};

/// Handle GET /api/coaches/:id/versions - List version history
pub(super) async fn handle_list_versions<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
    Query(query): Query<ListVersionsQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&ctx);
    // Authorization: reading version history exposes the coach's private content
    // (title, system_prompt, tags). Mirror the coach-detail access rule — the
    // caller may read versions only if they can access the coach itself
    // (owner, assigned, or system). get_by_id enforces exactly that predicate;
    // deny (404) when it returns nothing so a non-owner cannot enumerate another
    // user's private coach via the version endpoints.
    if manager
        .get_by_id(&id, auth.user_id, tenant_id)
        .await?
        .is_none()
    {
        return Err(AppError::not_found(format!("Coach {id}")));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let versions = manager.get_versions(&id, tenant_id, limit).await?;
    let current_version = manager.get_current_version(&id).await?;

    let mut creator_names: HashMap<Uuid, Option<String>> = HashMap::new();
    let mut version_responses: Vec<CoachVersionResponse> = Vec::with_capacity(versions.len());
    for version in versions {
        let created_by = version.created_by;
        let mut response: CoachVersionResponse = version.into();
        response.created_by_name =
            resolve_creator_name(&ctx, tenant_id, created_by, &mut creator_names).await;
        version_responses.push(response);
    }

    let response = ListVersionsResponse {
        total: version_responses.len(),
        versions: version_responses,
        current_version,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle POST /api/coaches/:id/versions/:version/revert - Revert to a version
pub(super) async fn handle_revert_version<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path((id, version)): Path<(String, i32)>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&ctx);
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
pub(super) async fn handle_diff_versions<C: CoachesCtx + MiddlewareCtx>(
    State(ctx): State<Arc<C>>,
    auth: AuthenticatedUser,
    Path((id, v1, v2)): Path<(String, i32, i32)>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = super::get_user_tenant(&auth)?;

    let manager = super::get_coaches_manager(&ctx);

    // Authorization: mirror the coach-detail access rule (owner, assigned, or
    // system) before exposing version snapshot content through the diff.
    if manager
        .get_by_id(&id, auth.user_id, tenant_id)
        .await?
        .is_none()
    {
        return Err(AppError::not_found(format!("Coach {id}")));
    }

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

/// Resolve a version author's `Uuid` to a human-readable name.
///
/// Prefers the user's `display_name`, falls back to email, and returns `None`
/// when the version has no recorded author or the user can no longer be found.
/// Results are memoized in `cache` so listing many versions by the same author
/// performs a single database lookup.
async fn resolve_creator_name<C: CoachesCtx + MiddlewareCtx>(
    ctx: &Arc<C>,
    tenant_id: TenantId,
    created_by: Option<Uuid>,
    cache: &mut HashMap<Uuid, Option<String>>,
) -> Option<String> {
    let user_id = created_by?;
    if let Some(name) = cache.get(&user_id) {
        return name.clone();
    }
    let name = ctx
        .database()
        .repositories()
        .users
        .get(user_id, tenant_id)
        .await
        .ok()
        .flatten()
        .map(|user| user.display_name.unwrap_or(user.email));
    cache.insert(user_id, name.clone());
    name
}

/// Compute field-level differences between two JSON snapshots
///
/// Delegates to `services::recipes::compute_version_diff`. The version-diff
/// machinery lives in the `recipes` service module and is deliberately reused
/// for the coaches domain: coach version snapshots and recipe snapshots share
/// the same JSON shape, so the diff helper is domain-agnostic despite the
/// module name.
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
