// ABOUTME: HTTP boundary for runtime feature flags — self read + admin tenant/user CRUD
// ABOUTME: Logic lives in services::feature_flags_ops; this file shapes URLs + bodies
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes for runtime feature flags.
//!
//! - `GET /api/me/features` — the calling user's effective flag map
//!   (per-user override > tenant default > compile-time default).
//! - `GET /api/admin/tenants/{tenant_id}/features` — tenant defaults.
//! - `PUT /api/admin/tenants/{tenant_id}/features/{key}` — upsert a default.
//! - `DELETE /api/admin/tenants/{tenant_id}/features/{key}` — clear default.
//! - `GET /api/admin/users/{user_id}/features` — per-user overrides.
//! - `PUT /api/admin/users/{user_id}/features/{key}` — upsert an override.
//! - `DELETE /api/admin/users/{user_id}/features/{key}` — clear override.
//!
//! Authentication: `/api/me/features` accepts any logged-in user; the admin
//! routes go through the cookie-admin middleware mounted in `web_admin` and
//! the service layer additionally checks tenant membership / super-admin
//! before reading or writing.

use std::str::FromStr;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use pierre_auth::auth::AuthResult;
use pierre_core::feature_flags::FeatureKey;
use pierre_database::repositories::FeatureFlagRow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    errors::AppError,
    mcp::resources::ServerContext,
    middleware::{extract_auth_from_headers, require_admin},
    services::feature_flags_ops,
};

/// Convenience: list of every known [`FeatureKey`] with its description.
/// Surfaced to admin UIs so the toggle panel renders correctly even when
/// no rows are stored yet.
#[derive(Debug, Serialize)]
pub struct KnownFeatureFlag {
    /// Storage key (also the JSON key in `GET /api/me/features`).
    pub key: String,
    /// Human-readable description shown next to the toggle.
    pub description: String,
    /// Compile-time fallback value when neither tenant nor user has a row.
    pub default_enabled: bool,
}

/// Response body for `GET /api/me/features`.
#[derive(Debug, Serialize)]
pub struct MeFeaturesResponse {
    /// Effective flag map, keyed by storage string (`api_tokens`, …).
    pub flags: feature_flags_ops::EffectiveFlags,
    /// Registry metadata for callers that want to render all flags even
    /// when their value matches the default.
    pub known: Vec<KnownFeatureFlag>,
}

/// Row returned by the admin list endpoints. Mirrors [`FeatureFlagRow`]
/// with `feature_key` serialised as the storage string and timestamps as
/// RFC3339 strings.
#[derive(Debug, Serialize)]
pub struct AdminFeatureFlagRow {
    /// Storage key (e.g., `"api_tokens"`).
    pub feature_key: String,
    /// Stored value at this scope.
    pub enabled: bool,
    /// Last write timestamp (RFC3339).
    pub updated_at: String,
    /// Admin user who last wrote the row, or null when the audit FK was
    /// nulled by the admin's deletion.
    pub updated_by: Option<Uuid>,
}

impl From<FeatureFlagRow> for AdminFeatureFlagRow {
    fn from(row: FeatureFlagRow) -> Self {
        Self {
            feature_key: row.feature_key.as_str().to_owned(),
            enabled: row.enabled,
            updated_at: row.updated_at.to_rfc3339(),
            updated_by: row.updated_by,
        }
    }
}

/// Request body for the admin PUT endpoints.
#[derive(Debug, Deserialize)]
pub struct SetFeatureFlagRequest {
    /// New stored value at this scope.
    pub enabled: bool,
}

fn known_flags() -> Vec<KnownFeatureFlag> {
    FeatureKey::ALL
        .iter()
        .map(|k| KnownFeatureFlag {
            key: k.as_str().to_owned(),
            description: k.description().to_owned(),
            default_enabled: k.default_enabled(),
        })
        .collect()
}

async fn authenticated_user(
    headers: &HeaderMap,
    resources: &Arc<ServerContext>,
) -> Result<AuthResult, AppError> {
    extract_auth_from_headers(headers, resources).await
}

async fn authenticated_admin(
    headers: &HeaderMap,
    resources: &Arc<ServerContext>,
) -> Result<AuthResult, AppError> {
    let auth = extract_auth_from_headers(headers, resources).await?;
    require_admin(auth.user_id, &resources.repos.users).await?;
    Ok(auth)
}

fn parse_feature_key(key: &str) -> Result<FeatureKey, AppError> {
    FeatureKey::from_str(key)
        .map_err(|e| AppError::invalid_input(format!("Unknown feature key: {e}")))
}

fn parse_uuid_path(value: &str, name: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value)
        .map_err(|e| AppError::invalid_input(format!("Invalid {name} format: {e}")))
}

/// `GET /api/me/features`
///
/// # Errors
///
/// Returns `AppError` when authentication fails or feature-flag resolution
/// errors against the database.
pub async fn handle_self_get(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = authenticated_user(&headers, &resources).await?;
    let flags = feature_flags_ops::resolve_self_flags(&resources.data(), auth.user_id).await?;
    Ok((
        StatusCode::OK,
        Json(MeFeaturesResponse {
            flags,
            known: known_flags(),
        }),
    )
        .into_response())
}

/// `GET /api/admin/tenants/{tenant_id}/features`
///
/// # Errors
///
/// `AppError` on auth failure, invalid UUID path param, or repository error.
pub async fn handle_admin_list_tenant_defaults(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(tenant_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = authenticated_admin(&headers, &resources).await?;
    let tenant_uuid = parse_uuid_path(&tenant_id, "tenant_id")?;
    let rows =
        feature_flags_ops::list_tenant_defaults(&resources.data(), auth.user_id, tenant_uuid)
            .await?;
    let body: Vec<AdminFeatureFlagRow> = rows.into_iter().map(AdminFeatureFlagRow::from).collect();
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"rows": body, "known": known_flags()})),
    )
        .into_response())
}

/// `PUT /api/admin/tenants/{tenant_id}/features/{key}`
///
/// # Errors
///
/// `AppError` on auth failure, invalid UUID, unknown feature key, or repo error.
pub async fn handle_admin_set_tenant_default(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path((tenant_id, key)): Path<(String, String)>,
    Json(body): Json<SetFeatureFlagRequest>,
) -> Result<Response, AppError> {
    let auth = authenticated_admin(&headers, &resources).await?;
    let tenant_uuid = parse_uuid_path(&tenant_id, "tenant_id")?;
    let feature_key = parse_feature_key(&key)?;
    feature_flags_ops::set_tenant_default(
        &resources.data(),
        auth.user_id,
        tenant_uuid,
        feature_key,
        body.enabled,
    )
    .await?;
    Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
}

/// `DELETE /api/admin/tenants/{tenant_id}/features/{key}`
///
/// # Errors
///
/// `AppError` on auth failure, invalid UUID, unknown feature key, or repo error.
pub async fn handle_admin_clear_tenant_default(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path((tenant_id, key)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let auth = authenticated_admin(&headers, &resources).await?;
    let tenant_uuid = parse_uuid_path(&tenant_id, "tenant_id")?;
    let feature_key = parse_feature_key(&key)?;
    let removed = feature_flags_ops::clear_tenant_default(
        &resources.data(),
        auth.user_id,
        tenant_uuid,
        feature_key,
    )
    .await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"success": true, "removed": removed})),
    )
        .into_response())
}

/// `GET /api/admin/users/{user_id}/features`
///
/// # Errors
///
/// `AppError` on auth failure, invalid UUID, or repository error.
pub async fn handle_admin_list_user_overrides(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = authenticated_admin(&headers, &resources).await?;
    let user_uuid = parse_uuid_path(&user_id, "user_id")?;
    let rows =
        feature_flags_ops::list_user_overrides(&resources.data(), auth.user_id, user_uuid).await?;
    let body: Vec<AdminFeatureFlagRow> = rows.into_iter().map(AdminFeatureFlagRow::from).collect();
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"rows": body, "known": known_flags()})),
    )
        .into_response())
}

/// `PUT /api/admin/users/{user_id}/features/{key}`
///
/// # Errors
///
/// `AppError` on auth failure, invalid UUID, unknown feature key, or repo error.
pub async fn handle_admin_set_user_override(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path((user_id, key)): Path<(String, String)>,
    Json(body): Json<SetFeatureFlagRequest>,
) -> Result<Response, AppError> {
    let auth = authenticated_admin(&headers, &resources).await?;
    let user_uuid = parse_uuid_path(&user_id, "user_id")?;
    let feature_key = parse_feature_key(&key)?;
    feature_flags_ops::set_user_override(
        &resources.data(),
        auth.user_id,
        user_uuid,
        feature_key,
        body.enabled,
    )
    .await?;
    Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
}

/// `DELETE /api/admin/users/{user_id}/features/{key}`
///
/// # Errors
///
/// `AppError` on auth failure, invalid UUID, unknown feature key, or repo error.
pub async fn handle_admin_clear_user_override(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path((user_id, key)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let auth = authenticated_admin(&headers, &resources).await?;
    let user_uuid = parse_uuid_path(&user_id, "user_id")?;
    let feature_key = parse_feature_key(&key)?;
    let removed = feature_flags_ops::clear_user_override(
        &resources.data(),
        auth.user_id,
        user_uuid,
        feature_key,
    )
    .await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"success": true, "removed": removed})),
    )
        .into_response())
}

/// Mount-helper for the self-read endpoint. The admin routes are wired
/// directly inside `web_admin::WebAdminRouter::router` to inherit the
/// cookie-admin middleware.
pub struct FeatureFlagsRoutes;

impl FeatureFlagsRoutes {
    /// User-facing routes (no admin gate).
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/features", get(handle_self_get))
            .with_state(resources)
    }
}
