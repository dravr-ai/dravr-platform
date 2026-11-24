// ABOUTME: Admin token management route handlers
// ABOUTME: Create, list, revoke and rotate admin service tokens from the console
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One handler per operation, mounted for the console under
//! `/api/admin/tokens`, behind an admin's session.
//!
//! An admin's console session reaches the platform: it carries every
//! permission and the super-admin flag
//! (`pierre_middleware::cookie_admin_token`), so it lists, mints, revokes and
//! rotates every token, whatever tenant the token is bound to and whether or
//! not it is a super-admin one. The handlers still check what opens the
//! surface, `ManageAdminTokens`, and that a super-admin token is minted by a
//! super-admin caller alone.
//!
//! A revoke or a rotation of an id no token has answers 404. What it answers
//! for a token that was rotated or revoked earlier is on each handler
//! (carnet#1589).

use secrecy::ExposeSecret;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, to_value, Value};
use tracing::{error, info};

use pierre_core::admin::models::{
    rerotated_token_refusal, rotated_token_refusal, AdminPermission, AdminPermission as AdminPerm,
    AdminToken, AdminTokenSummary, CreateAdminTokenRequest, GeneratedAdminToken,
    ValidatedAdminToken,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{TokenRevocation, TokenRotation};

use super::api_keys::json_response;
use super::types::AdminResponse;
use crate::audit::AuditDetail;
use crate::context::AdminApiContext;
use dravr_tronc::dates::Timestamp;

/// Lifetime of a rotated token when the request names none.
const DEFAULT_ROTATION_DAYS: u64 = 365;

/// Body of `POST /api/admin/tokens`.
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

/// Query of `GET /api/admin/tokens`.
#[derive(Debug, Deserialize)]
pub(crate) struct ListAdminTokensQuery {
    /// Return revoked tokens alongside live ones. The console asks for them
    /// to render its Inactive badge; a bare listing is live tokens only.
    #[serde(default)]
    include_inactive: bool,
}

/// Body of `POST /api/admin/tokens/{id}/rotate`; every field is optional, so a
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

/// Load the token `token_id` names: the row a revoke or a rotation decides
/// on before it writes.
///
/// # Errors
/// A not-found error for an id no token has, and an internal error when the
/// store cannot be read.
async fn load_token(ctx: &AdminApiContext, token_id: &str) -> AppResult<AdminToken> {
    ctx.repos
        .admin
        .get_token_by_id(token_id)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get admin token");
            AppError::internal(format!("Failed to get admin token: {e}"))
        })?
        .ok_or_else(|| missing_token(token_id))
}

/// What an id no token has is answered.
fn missing_token(token_id: &str) -> AppError {
    AppError::not_found(format!("Admin token {token_id}"))
}

/// Handle admin token creation
///
/// The token carries the permissions the request names, or, for a request
/// that names none, the set the store gives it
/// ([`CreateAdminTokenRequest::granted_permissions`]). A name that is no
/// permission answers 400 and nothing is minted. The request names no
/// tenant, so the token is bound to none.
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

    // A minted token names no admin and outlives the standing of the one who
    // made it, so the trail is where it is tied to its maker: the row's actor
    // is the admin behind the request, and its target the token.
    let audit = AuditDetail::new(
        ADMIN_TOKEN_CREATED,
        format!("Minted admin token {}", generated_token.service_name),
    )
    .target(minted_token_target(&generated_token));
    Ok(audit.attach(
        json_response(
            AdminResponse {
                success: true,
                message: "Admin token created successfully".to_owned(),
                data: to_value(json!({
                    "token_id": generated_token.token_id,
                    "service_name": generated_token.service_name,
                    "jwt_token": generated_token.jwt_token.expose_secret(),
                    "token_prefix": generated_token.token_prefix,
                    "is_super_admin": generated_token.is_super_admin,
                    "expires_at": generated_token.expires_at.map(Timestamp::to_rfc3339),
                }))
                .ok(),
            },
            StatusCode::CREATED,
        )
        .into_response(),
    ))
}

/// The action key of the trail row a minted admin token writes.
const ADMIN_TOKEN_CREATED: &str = "admin_token.create";

/// The action key of the trail row a rotated admin token writes.
const ADMIN_TOKEN_ROTATED: &str = "admin_token.rotate";

/// What the trail keeps of a token just minted: which one, for what, and
/// how much it can do. Never the token itself.
fn minted_token_target(token: &GeneratedAdminToken) -> Value {
    json!({
        "token_id": token.token_id,
        "service_name": token.service_name,
        "is_super_admin": token.is_super_admin,
        "expires_at": token.expires_at.map(Timestamp::to_rfc3339),
    })
}

/// Handle listing admin tokens
///
/// An admin's console session is shown every token, super-admin ones
/// included; revoked ones too when the query asks for them.
///
/// # Errors
/// An internal error when the tokens cannot be read.
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

    let visible: Vec<AdminTokenSummary> = tokens.into_iter().map(AdminTokenSummary::from).collect();

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

/// Handle revoking admin token
///
/// An unknown id answers 404 ([`load_token`]).
///
/// A rotated token is not revoked a second time (409): the rotation retired
/// it and minted a replacement that is still live, so answering success
/// would tell the caller a credential had ended that only changed id. The
/// refusal names the replacement by id and says to revoke that one
/// ([`rotated_token_refusal`]). A token revoked earlier names
/// no replacement and keeps its idempotent 200. So does a token rotated
/// before `admin_tokens.rotated_to_token_id` existed, which cannot be told
/// from a revoked one. The store decides on the row as its own guarded
/// write left it ([`TokenRevocation`]), so a rotation that commits after
/// the read here is still answered as a rotation.
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

    load_token(&context, &token_id).await?;

    let revocation = context
        .repos
        .admin
        .deactivate_token(&token_id)
        .await
        .inspect_err(|e| error!(error = %e, "Failed to revoke admin token"))?;
    if let TokenRevocation::Rotated { replacement } = revocation {
        info!(
            "Admin token {} was rotated into {}; nothing revoked",
            token_id, replacement
        );
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: rotated_token_refusal(&token_id, &replacement),
                data: to_value(json!({
                    "token_id": token_id,
                    "rotated_to_token_id": replacement,
                }))
                .ok(),
            },
            StatusCode::CONFLICT,
        )
        .into_response());
    }

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
/// Deactivates the token and mints a replacement with the same name,
/// permissions, scope and super-admin flag
/// ([`CreateAdminTokenRequest::rotation_of`]), returned once in this
/// response. A revoked token answers 400 and mints nothing: it has no
/// replacement. A rotated token answers 409 and names the replacement it
/// already has ([`rerotated_token_refusal`]). An unknown id answers 404
/// ([`load_token`]).
///
/// The deactivation and the mint are one transaction in the store
/// ([`AdminRepository::rotate_token`]), which builds the replacement from
/// the row it claims, not from the one read here: a token revoked, or
/// retired with its admin, since that read is refused as a revoked one is.
/// One whose flag or permissions are no longer the ones read here stays
/// live and answers 409, with the conflict code's generic message: the
/// store's own sentence is not one that reaches a caller. No write changes
/// a stored token's flag or permissions today, so that 409 is a guard
/// nothing reaches yet.
///
/// [`AdminRepository::rotate_token`]: pierre_database::repositories::AdminRepository::rotate_token
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

    // A rotated token already has its replacement: the answer names it,
    // as the revoke's does. One rotated after the read above is refused by
    // the store in the same sentence, as a 400.
    if let Some(replacement) = existing_token
        .rotated_to_token_id
        .as_deref()
        .filter(|_| !existing_token.is_active)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: rerotated_token_refusal(&token_id, replacement),
                data: to_value(json!({
                    "token_id": token_id,
                    "rotated_to_token_id": replacement,
                }))
                .ok(),
            },
            StatusCode::CONFLICT,
        )
        .into_response());
    }

    // The store claims the token and mints from the row it claimed: the
    // row checked above is what it holds the claimed one to, and a revoked
    // token is refused there with nothing minted. Its refusals keep their
    // status (404, 400, 409), so they are not rewrapped; only the 400's
    // message is the store's own, the other two read as their code's
    // generic one.
    let expires_in_days = body
        .map_or_default(|Json(body)| body)
        .expires_in_days
        .unwrap_or(DEFAULT_ROTATION_DAYS);
    let new_token = ctx
        .repos
        .admin
        .rotate_token(
            &TokenRotation {
                checked: &existing_token,
                expires_in_days,
                rotated_by: admin_token.operator_user_id,
            },
            &ctx.admin_jwt_secret,
            &ctx.jwks_manager,
        )
        .await?;

    info!(
        "Admin token {} rotated successfully, new token: {}",
        token_id, new_token.token_id
    );

    // The replacement is handed to whoever rotated it: the trail ties it to
    // that admin, as it does a minted one.
    let mut target = minted_token_target(&new_token);
    target["old_token_id"] = json!(token_id);
    let audit = AuditDetail::new(
        ADMIN_TOKEN_ROTATED,
        format!("Rotated admin token {}", new_token.service_name),
    )
    .target(target);
    Ok(audit.attach(
        json_response(
            AdminResponse {
                success: true,
                message: "Admin token rotated successfully".to_owned(),
                data: to_value(json!({
                    "old_token_id": token_id,
                    "token_id": new_token.token_id,
                    "service_name": new_token.service_name,
                    "jwt_token": new_token.jwt_token.expose_secret(),
                    "token_prefix": new_token.token_prefix,
                    "is_super_admin": new_token.is_super_admin,
                    "expires_at": new_token.expires_at.map(Timestamp::to_rfc3339),
                }))
                .ok(),
            },
            StatusCode::OK,
        )
        .into_response(),
    ))
}
