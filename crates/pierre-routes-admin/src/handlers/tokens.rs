// ABOUTME: Admin token management route handlers
// ABOUTME: Create, list, get, revoke and rotate admin service tokens for the CLI and the console
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One handler per operation, for both admin mounts.
//!
//! `pierre-cli` reaches them under `/admin/tokens` and the console under
//! `/api/admin/tokens`.
//!
//! `ManageAdminTokens` opens the surface; a super-admin token stays out of
//! reach of every caller that is not itself super-admin — it is not listed to
//! them, cannot be read, revoked or rotated by them, and cannot be minted by
//! them — and a caller that is not super-admin grants only permissions it holds.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, to_value};
use tracing::{error, info};

use pierre_core::admin::models::{
    AdminPermission, AdminPermission as AdminPerm, AdminToken, AdminTokenSummary,
    CreateAdminTokenRequest, ValidatedAdminToken,
};
use pierre_core::errors::{AppError, AppResult};

use super::api_keys::json_response;
use super::types::AdminResponse;
use crate::context::AdminApiContext;

/// Lifetime of a rotated token when the request names none.
const DEFAULT_ROTATION_DAYS: u64 = 365;

/// Body of `POST /admin/tokens`.
#[derive(Debug, Deserialize)]
pub(crate) struct CreateAdminTokenBody {
    /// Name of the service the token is for
    service_name: String,
    /// What the service is
    service_description: Option<String>,
    /// Permission names; the role default when absent
    permissions: Option<Vec<String>>,
    /// Mint a super-admin token (super-admin callers only)
    #[serde(default)]
    is_super_admin: bool,
    /// Lifetime in days; no expiry when absent
    expires_in_days: Option<u64>,
}

/// Query of `GET /admin/tokens`.
#[derive(Debug, Deserialize)]
pub(crate) struct ListAdminTokensQuery {
    /// Return revoked tokens alongside live ones. The console asks for them
    /// to render its Inactive badge; a bare listing is live tokens only.
    #[serde(default)]
    include_inactive: bool,
}

/// Body of `POST /admin/tokens/{id}/rotate`; every field is optional, so a
/// bare `{}` rotates on the default lifetime.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RotateAdminTokenBody {
    /// Lifetime of the replacement in days
    expires_in_days: Option<u64>,
}

/// A 403 in the admin response envelope.
fn forbidden(message: &str) -> Response {
    json_response(
        AdminResponse {
            success: false,
            message: message.to_owned(),
            data: None,
        },
        StatusCode::FORBIDDEN,
    )
    .into_response()
}

/// Deny a request whose token lacks `ManageAdminTokens`.
fn deny_without_manage_admin_tokens(token: &ValidatedAdminToken) -> Option<Response> {
    (!token.is_super_admin
        && !token
            .permissions
            .has_permission(&AdminPerm::ManageAdminTokens))
    .then(|| forbidden("Permission denied: ManageAdminTokens required"))
}

/// Load a token by id, answering 404 for an unknown one. `deactivate_token` is
/// an unconditional UPDATE, so without this a miss would report a revocation
/// that touched no row.
async fn load_token(ctx: &AdminApiContext, token_id: &str) -> AppResult<AdminToken> {
    ctx.repos
        .admin
        .get_token_by_id(token_id)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get admin token");
            AppError::internal(format!("Failed to get admin token: {e}"))
        })?
        .ok_or_else(|| AppError::not_found(format!("Admin token {token_id}")))
}

/// Handle admin token creation
pub(crate) async fn handle_create_admin_token(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<CreateAdminTokenBody>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_admin_tokens(&admin_token) {
        return Ok(denied);
    }

    info!("Creating admin token by token: {}", admin_token.token_id);

    if request.is_super_admin && !admin_token.is_super_admin {
        return Ok(forbidden(
            "Only super-admin tokens can create super-admin tokens",
        ));
    }

    let permissions = match request.permissions {
        Some(names) => {
            let mut parsed = Vec::with_capacity(names.len());
            for name in &names {
                let Ok(permission) = name.parse::<AdminPermission>() else {
                    return Ok(json_response(
                        AdminResponse {
                            success: false,
                            message: format!("Invalid permission: {name}"),
                            data: None,
                        },
                        StatusCode::BAD_REQUEST,
                    )
                    .into_response());
                };
                // A caller that is not super-admin cannot hand out authority it
                // does not hold, or a console session could mint its way to
                // configuration access.
                if !admin_token.is_super_admin
                    && !admin_token.permissions.has_permission(&permission)
                {
                    return Ok(forbidden(&format!(
                        "Cannot grant {permission}: the caller does not hold it"
                    )));
                }
                parsed.push(permission);
            }
            Some(parsed)
        }
        None => None,
    };

    let token_request = CreateAdminTokenRequest {
        service_name: request.service_name,
        service_description: request.service_description,
        permissions,
        expires_in_days: request.expires_in_days,
        is_super_admin: request.is_super_admin,
        tenant_id: None,
        operator_user_id: None,
    }
    .for_creator()?;

    let ctx = context.as_ref();
    let generated_token = ctx
        .repos
        .admin
        .create_token(&token_request, &ctx.admin_jwt_secret, &ctx.jwks_manager)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to generate admin token");
            AppError::internal(format!("Failed to generate admin token: {e}"))
        })?;

    info!("Admin token created: {}", generated_token.token_id);

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Admin token created successfully".to_owned(),
            data: to_value(json!({
                "token_id": generated_token.token_id,
                "service_name": generated_token.service_name,
                "jwt_token": generated_token.jwt_token,
                "token_prefix": generated_token.token_prefix,
                "is_super_admin": generated_token.is_super_admin,
                "expires_at": generated_token.expires_at.map(|t| t.to_rfc3339()),
            }))
            .ok(),
        },
        StatusCode::CREATED,
    )
    .into_response())
}

/// Handle listing admin tokens
///
/// A caller that is not super-admin is not shown super-admin tokens.
pub(crate) async fn handle_list_admin_tokens(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<ListAdminTokensQuery>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_admin_tokens(&admin_token) {
        return Ok(denied);
    }

    info!(
        include_inactive = query.include_inactive,
        "Listing admin tokens by token: {}", admin_token.token_id
    );

    let tokens = context
        .repos
        .admin
        .list_tokens(query.include_inactive)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to list admin tokens");
            AppError::internal(format!("Failed to list admin tokens: {e}"))
        })?;

    let visible: Vec<AdminTokenSummary> = tokens
        .into_iter()
        .filter(|token| admin_token.is_super_admin || !token.is_super_admin)
        .map(AdminTokenSummary::from)
        .collect();

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Retrieved {} admin tokens", visible.len()),
            data: to_value(json!({
                "count": visible.len(),
                "tokens": visible
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// Handle getting admin token details
pub(crate) async fn handle_get_admin_token(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(token_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_admin_tokens(&admin_token) {
        return Ok(denied);
    }

    let token = load_token(&context, &token_id).await?;
    if token.is_super_admin && !admin_token.is_super_admin {
        return Ok(forbidden(
            "Only super-admin tokens can read super-admin tokens",
        ));
    }

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Admin token retrieved successfully".to_owned(),
            data: to_value(AdminTokenSummary::from(token)).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// Handle revoking admin token
///
/// An unknown id answers 404, and a super-admin token can be revoked only by a
/// super-admin caller: revoking one is a denial of service against the
/// platform's own operators.
pub(crate) async fn handle_revoke_admin_token(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(token_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_admin_tokens(&admin_token) {
        return Ok(denied);
    }

    info!(
        "Revoking admin token {} by token: {}",
        token_id, admin_token.token_id
    );

    let existing = load_token(&context, &token_id).await?;
    if existing.is_super_admin && !admin_token.is_super_admin {
        return Ok(forbidden(
            "Only super-admin tokens can revoke super-admin tokens",
        ));
    }

    context
        .repos
        .admin
        .deactivate_token(&token_id)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to revoke admin token");
            AppError::internal(format!("Failed to revoke admin token: {e}"))
        })?;

    info!("Admin token {} revoked successfully", token_id);

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Admin token revoked successfully".to_owned(),
            data: to_value(json!({ "token_id": token_id })).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// Handle rotating admin token
///
/// Deactivates the token and mints a replacement with the same name, scope and
/// super-admin flag, returned once in this response.
pub(crate) async fn handle_rotate_admin_token(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(token_id): Path<String>,
    body: Option<Json<RotateAdminTokenBody>>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_admin_tokens(&admin_token) {
        return Ok(denied);
    }

    info!(
        "Rotating admin token {} by token: {}",
        token_id, admin_token.token_id
    );

    let ctx = context.as_ref();
    let existing_token = load_token(ctx, &token_id).await?;

    // Rotation mints the replacement at the old token's privilege level, so a
    // super-admin token rotated by a caller who is not super-admin would hand
    // that caller a fresh super-admin JWT. Checked before `deactivate_token` —
    // a refused rotation must leave the existing credential intact.
    if existing_token.is_super_admin && !admin_token.is_super_admin {
        return Ok(forbidden(
            "Only super-admin tokens can rotate super-admin tokens",
        ));
    }

    ctx.repos
        .admin
        .deactivate_token(&token_id)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to deactivate old token");
            AppError::internal(format!("Failed to deactivate old token: {e}"))
        })?;

    let expires_in_days = body
        .map(|Json(body)| body)
        .unwrap_or_default()
        .expires_in_days
        .unwrap_or(DEFAULT_ROTATION_DAYS);
    let token_request = CreateAdminTokenRequest::rotation_of(
        existing_token,
        expires_in_days,
        admin_token.operator_user_id,
    );

    let new_token = ctx
        .repos
        .admin
        .create_token(&token_request, &ctx.admin_jwt_secret, &ctx.jwks_manager)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to generate new admin token");
            AppError::internal(format!("Failed to generate new admin token: {e}"))
        })?;

    info!(
        "Admin token {} rotated successfully, new token: {}",
        token_id, new_token.token_id
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Admin token rotated successfully".to_owned(),
            data: to_value(json!({
                "old_token_id": token_id,
                "token_id": new_token.token_id,
                "service_name": new_token.service_name,
                "jwt_token": new_token.jwt_token,
                "token_prefix": new_token.token_prefix,
                "is_super_admin": new_token.is_super_admin,
                "expires_at": new_token.expires_at.map(|t| t.to_rfc3339()),
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}
