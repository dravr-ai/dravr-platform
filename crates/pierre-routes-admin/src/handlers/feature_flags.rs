// ABOUTME: HTTP boundary for runtime feature flags — self read + admin tenant/user CRUD
// ABOUTME: Self-contained service layer operating on the repository registry directly
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
//! routes are mounted behind the cookie-admin middleware in
//! [`crate::AdminRoutes::cookie_admin_routes`]. Authorization uses the
//! canonical [`ValidatedAdminToken`] like every other admin endpoint:
//! `require_permission` gates reads/writes, and the tenant-scoped routes call
//! [`ValidatedAdminToken::require_tenant_access`] so a tenant-scoped token
//! cannot reach another tenant's defaults (super-admin tokens pass through).

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::feature_flags::FeatureKey;
use pierre_database::repositories::FeatureFlagRow;
use pierre_database::{AuthRepos, UsageRepos};
use pierre_middleware::extract_auth_from_headers;
use pierre_runtime_context::MiddlewareCtx;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::context::AdminApiContext;

// =========================================================================
// Response/request shapes
// =========================================================================

/// Effective flag map keyed by the storage string form of [`FeatureKey`].
///
/// Using [`BTreeMap`] (over [`std::collections::HashMap`]) keeps JSON output
/// deterministic for snapshot tests and admin UI rendering.
pub type EffectiveFlags = BTreeMap<String, bool>;

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
    pub flags: EffectiveFlags,
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

fn parse_feature_key(key: &str) -> Result<FeatureKey, AppError> {
    FeatureKey::from_str(key)
        .map_err(|e| AppError::invalid_input(format!("Unknown feature key: {e}")))
}

fn parse_uuid_path(value: &str, name: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value)
        .map_err(|e| AppError::invalid_input(format!("Invalid {name} format: {e}")))
}

/// Extract the admin user UUID from a cookie-auth-synthesized
/// `ValidatedAdminToken`. The cookie-admin middleware encodes the
/// originating user as `token_id = "cookie:<uuid>"`; programmatic admin
/// tokens use a different prefix and have no user id.
fn admin_user_id_from_token(admin_token: &ValidatedAdminToken) -> Result<Uuid, AppError> {
    let raw = admin_token
        .token_id
        .strip_prefix("cookie:")
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "Feature-flag admin endpoints require cookie authentication",
            )
        })?;
    Uuid::parse_str(raw).map_err(|e| {
        AppError::internal(format!(
            "Cookie admin token carried a non-UUID user id: {e}"
        ))
    })
}

// =========================================================================
// Self-read handler (any authenticated user) — uses MiddlewareCtx
// =========================================================================

/// `GET /api/me/features`
///
/// # Errors
///
/// Returns `AppError` when authentication fails or feature-flag resolution
/// errors against the database.
pub async fn handle_self_get<C: MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth_session = extract_auth_from_headers(&headers, &resources).await?;
    let auth_view = resources.repos().auth_repos();
    let usage_view = resources.repos().usage_repos();
    let flags = resolve_self_flags(&auth_view, &usage_view, auth_session.user_id).await?;
    Ok((
        StatusCode::OK,
        Json(MeFeaturesResponse {
            flags,
            known: known_flags(),
        }),
    )
        .into_response())
}

// =========================================================================
// Admin handlers — consume cookie-synthesized ValidatedAdminToken
// =========================================================================

/// `GET /api/admin/tenants/{tenant_id}/features`
///
/// # Errors
///
/// `AppError` on auth failure, invalid UUID path param, or repository error.
pub async fn handle_admin_list_tenant_defaults(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(tenant_id): Path<String>,
) -> Result<Response, AppError> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_tenant_access(&tenant_id)?;
    let tenant_uuid = parse_uuid_path(&tenant_id, "tenant_id")?;
    let usage = context.repos.usage_repos();
    let rows = list_tenant_defaults(&usage, tenant_uuid).await?;
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
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path((tenant_id, key)): Path<(String, String)>,
    Json(body): Json<SetFeatureFlagRequest>,
) -> Result<Response, AppError> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    admin_token.require_tenant_access(&tenant_id)?;
    let admin_user_id = admin_user_id_from_token(&admin_token)?;
    let tenant_uuid = parse_uuid_path(&tenant_id, "tenant_id")?;
    let feature_key = parse_feature_key(&key)?;
    let usage = context.repos.usage_repos();
    set_tenant_default(
        &usage,
        admin_user_id,
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
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path((tenant_id, key)): Path<(String, String)>,
) -> Result<Response, AppError> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    admin_token.require_tenant_access(&tenant_id)?;
    let admin_user_id = admin_user_id_from_token(&admin_token)?;
    let tenant_uuid = parse_uuid_path(&tenant_id, "tenant_id")?;
    let feature_key = parse_feature_key(&key)?;
    let usage = context.repos.usage_repos();
    let removed = clear_tenant_default(&usage, admin_user_id, tenant_uuid, feature_key).await?;
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
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
) -> Result<Response, AppError> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    let user_uuid = parse_uuid_path(&user_id, "user_id")?;
    let usage = context.repos.usage_repos();
    let rows = list_user_overrides(&usage, user_uuid).await?;
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
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path((user_id, key)): Path<(String, String)>,
    Json(body): Json<SetFeatureFlagRequest>,
) -> Result<Response, AppError> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    let admin_user_id = admin_user_id_from_token(&admin_token)?;
    let user_uuid = parse_uuid_path(&user_id, "user_id")?;
    let feature_key = parse_feature_key(&key)?;
    let usage = context.repos.usage_repos();
    set_user_override(&usage, admin_user_id, user_uuid, feature_key, body.enabled).await?;
    Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
}

/// `DELETE /api/admin/users/{user_id}/features/{key}`
///
/// # Errors
///
/// `AppError` on auth failure, invalid UUID, unknown feature key, or repo error.
pub async fn handle_admin_clear_user_override(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path((user_id, key)): Path<(String, String)>,
) -> Result<Response, AppError> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    let admin_user_id = admin_user_id_from_token(&admin_token)?;
    let user_uuid = parse_uuid_path(&user_id, "user_id")?;
    let feature_key = parse_feature_key(&key)?;
    let usage = context.repos.usage_repos();
    let removed = clear_user_override(&usage, admin_user_id, user_uuid, feature_key).await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"success": true, "removed": removed})),
    )
        .into_response())
}

// =========================================================================
// Mount helper for the self-read endpoint
// =========================================================================

/// Mount-helper for the self-read endpoint. The admin routes are wired
/// directly inside [`crate::AdminRoutes::cookie_admin_routes`] to inherit
/// the cookie-admin middleware.
pub struct FeatureFlagsRoutes;

impl FeatureFlagsRoutes {
    /// User-facing routes (no admin gate).
    ///
    /// Generic over [`MiddlewareCtx`] so the crate stays decoupled from
    /// `pierre-server`'s `ServerContext`.
    pub fn routes<C>(resources: Arc<C>) -> Router
    where
        C: MiddlewareCtx,
    {
        Router::new()
            .route("/api/me/features", get(handle_self_get::<C>))
            .with_state(resources)
    }
}

// =========================================================================
// Inlined service layer — was crate::services::feature_flags_ops in pierre-server
// =========================================================================

/// Resolve the effective feature-flag map for a caller (no admin gate).
///
/// Backs `GET /api/me/features`. When the user has no tenant rows the map
/// still contains every known key with its compile-time default.
async fn resolve_self_flags(
    auth: &AuthRepos,
    usage: &UsageRepos,
    user_id: Uuid,
) -> Result<EffectiveFlags, AppError> {
    let tenant_uuid = primary_tenant_for_user(auth, user_id).await?;

    let resolved = match tenant_uuid {
        Some(tid) => usage
            .feature_flags
            .resolve_for_user(tid, user_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to resolve feature flags: {e}")))?,
        None => FeatureKey::ALL
            .iter()
            .map(|k| (*k, k.default_enabled()))
            .collect(),
    };

    Ok(resolved
        .into_iter()
        .map(|(k, v)| (k.as_str().to_owned(), v))
        .collect())
}

/// Lookup the user's primary tenant for flag resolution.
///
/// Multi-tenant memberships exist in this codebase but a user has one
/// primary working tenant from the admin's perspective. We pick the first
/// row from `TenantRepository::list_for_user` (stable order); when the user
/// is not yet tied to any tenant we return `None` and the caller falls back
/// to compile-time defaults.
async fn primary_tenant_for_user(
    auth: &AuthRepos,
    user_id: Uuid,
) -> Result<Option<Uuid>, AppError> {
    let tenants = auth
        .tenants
        .list_for_user(user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to look up user tenants: {e}")))?;
    Ok(tenants.first().map(|t| t.id.as_uuid()))
}

/// Admin: list stored tenant defaults. Empty vec when nothing is configured.
///
/// Authorization is enforced at the HTTP boundary via
/// [`ValidatedAdminToken::require_tenant_access`].
async fn list_tenant_defaults(
    usage: &UsageRepos,
    tenant_id: Uuid,
) -> Result<Vec<FeatureFlagRow>, AppError> {
    usage
        .feature_flags
        .list_tenant_defaults(tenant_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to list tenant defaults: {e}")))
}

/// Admin: insert-or-update a tenant default.
async fn set_tenant_default(
    usage: &UsageRepos,
    admin_user_id: Uuid,
    tenant_id: Uuid,
    feature_key: FeatureKey,
    enabled: bool,
) -> Result<(), AppError> {
    usage
        .feature_flags
        .set_tenant_default(tenant_id, feature_key, enabled, Some(admin_user_id))
        .await
        .map_err(|e| AppError::internal(format!("Failed to set tenant default: {e}")))?;
    info!(
        admin_id = %admin_user_id,
        tenant_id = %tenant_id,
        feature_key = feature_key.as_str(),
        enabled,
        "Tenant feature default set"
    );
    Ok(())
}

/// Admin: clear a tenant default. Returns `true` when a row was removed.
async fn clear_tenant_default(
    usage: &UsageRepos,
    admin_user_id: Uuid,
    tenant_id: Uuid,
    feature_key: FeatureKey,
) -> Result<bool, AppError> {
    let removed = usage
        .feature_flags
        .clear_tenant_default(tenant_id, feature_key)
        .await
        .map_err(|e| AppError::internal(format!("Failed to clear tenant default: {e}")))?;
    info!(
        admin_id = %admin_user_id,
        tenant_id = %tenant_id,
        feature_key = feature_key.as_str(),
        removed,
        "Tenant feature default cleared"
    );
    Ok(removed)
}

/// Admin: list stored per-user overrides for `target_user_id`.
///
/// Like the sibling user-management endpoints (`/api/admin/users/...`), the
/// admin view is global; authorization is the `ManageConfiguration` /
/// `ViewConfiguration` permission carried by the [`ValidatedAdminToken`].
async fn list_user_overrides(
    usage: &UsageRepos,
    target_user_id: Uuid,
) -> Result<Vec<FeatureFlagRow>, AppError> {
    usage
        .feature_flags
        .list_user_overrides(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to list user overrides: {e}")))
}

/// Admin: insert-or-update a per-user override.
async fn set_user_override(
    usage: &UsageRepos,
    admin_user_id: Uuid,
    target_user_id: Uuid,
    feature_key: FeatureKey,
    enabled: bool,
) -> Result<(), AppError> {
    usage
        .feature_flags
        .set_user_override(target_user_id, feature_key, enabled, Some(admin_user_id))
        .await
        .map_err(|e| AppError::internal(format!("Failed to set user override: {e}")))?;
    info!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        feature_key = feature_key.as_str(),
        enabled,
        "Per-user feature override set"
    );
    Ok(())
}

/// Admin: clear a per-user override so the user inherits the tenant default.
async fn clear_user_override(
    usage: &UsageRepos,
    admin_user_id: Uuid,
    target_user_id: Uuid,
    feature_key: FeatureKey,
) -> Result<bool, AppError> {
    let removed = usage
        .feature_flags
        .clear_user_override(target_user_id, feature_key)
        .await
        .map_err(|e| AppError::internal(format!("Failed to clear user override: {e}")))?;
    info!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        feature_key = feature_key.as_str(),
        removed,
        "Per-user feature override cleared"
    );
    Ok(removed)
}
