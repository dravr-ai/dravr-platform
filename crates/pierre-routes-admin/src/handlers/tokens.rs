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
//! them — and a caller that is not super-admin grants only permissions it
//! holds, by minting a token or by rotating one: the ones a request names,
//! and the defaults one that names none is given.
//!
//! A caller bound to one tenant stays inside it on every write: it revokes
//! and rotates only tokens bound to that tenant, and what it mints is bound
//! to that tenant too ([`ScopeCaller::BoundTo`]). A token bound to none, or
//! to another tenant, is out of its reach, so a `ManageAdminTokens` holder
//! scoped to one tenant cannot rotate or mint its way to the platform.
//!
//! A caller bound to no tenant that does not carry the super-admin flag
//! ([`ScopeCaller::Unbound`]) reads no tenant's data, so it revokes and
//! rotates only tokens bound to no tenant, and mints only those: a rotation
//! would otherwise hand it a credential that reads a tenant it cannot. An
//! admin's console session and a super-admin token
//! ([`ScopeCaller::Platform`]) write every token.

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
    AdminPermission, AdminPermission as AdminPerm, AdminToken, AdminTokenSummary,
    CreateAdminTokenRequest, GeneratedAdminToken, ValidatedAdminToken,
};
use pierre_core::admin::scope::ScopeCaller;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::TokenRotation;

use super::api_keys::json_response;
use super::types::AdminResponse;
use crate::audit::AuditDetail;
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

/// Deny a token `permission` when the caller neither carries the
/// super-admin flag nor holds it: a caller cannot hand out authority it does
/// not hold, or a `ManageAdminTokens` holder could mint or rotate its way to
/// user or configuration access. One rule and one sentence for the two ways
/// a caller is handed a token, minting one and rotating one.
fn deny_granting_beyond_the_caller(
    caller: &ValidatedAdminToken,
    permission: &AdminPermission,
) -> Option<Response> {
    (!caller.is_super_admin && !caller.permissions.has_permission(permission)).then(|| {
        forbidden(&format!(
            "Cannot grant {permission}: the caller does not hold it"
        ))
    })
}

/// The tenant what a caller mints is bound to: `Some` for a token scoped to
/// one tenant that does not carry the super-admin flag, `None` for every
/// other caller. `None` is not one standing: an admin's console session and
/// a super-admin token reach every tenant, and a token bound to no tenant
/// without the flag reaches none, so what each may write is not read from
/// here ([`deny_outside_the_callers_reach`]). The one definition is the
/// scope rule's ([`ValidatedAdminToken::scope_caller`]).
///
/// # Errors
/// `InvalidInput` when the tenant the caller carries is not a tenant id.
fn bound_tenant(caller: &ValidatedAdminToken) -> AppResult<Option<TenantId>> {
    Ok(match caller.scope_caller()? {
        ScopeCaller::BoundTo(tenant) => Some(tenant),
        ScopeCaller::Platform | ScopeCaller::Unbound => None,
    })
}

/// Deny a write (`act`: "revoke", "rotate") on a token outside what the
/// caller reaches. Revoking one is a denial of service there, and rotating
/// one hands the caller its replacement, which carries the old token's
/// tenant, or none.
///
/// A caller that reaches the platform, an admin's console session or a
/// super-admin token, writes every token. A caller bound to one tenant
/// writes only tokens bound to that tenant: one bound to none reaches
/// further than it does, and one bound elsewhere reaches a tenant it does
/// not. A caller bound to no tenant, without the super-admin flag, writes
/// only tokens bound to no tenant: it reads no tenant's data itself
/// ([`ValidatedAdminToken::require_tenant_access`], [`ScopeCaller::resolve`]),
/// and the replacement of a tenant-bound token would.
///
/// # Errors
/// `InvalidInput` when the tenant the caller carries is not a tenant id.
fn deny_outside_the_callers_reach(
    caller: &ValidatedAdminToken,
    token: &AdminToken,
    act: &str,
) -> AppResult<Option<Response>> {
    Ok(match caller.scope_caller()? {
        ScopeCaller::Platform => None,
        ScopeCaller::BoundTo(own) => {
            let bound_to_own = token
                .tenant_id
                .as_deref()
                .and_then(|raw| TenantId::parse_str(raw).ok())
                == Some(own);
            (!bound_to_own).then(|| {
                forbidden(&format!(
                    "Token is scoped to tenant {own}; it can {act} only admin tokens bound to that tenant"
                ))
            })
        }
        ScopeCaller::Unbound => token.tenant_id.is_some().then(|| {
            forbidden(&format!(
                "Token is bound to no tenant; it can {act} only admin tokens bound to no tenant"
            ))
        }),
    })
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
///
/// A caller without the super-admin flag is handed only permissions it
/// holds. That covers the list a request names and, for a request that
/// names none, the set the store would give it
/// ([`CreateAdminTokenRequest::granted_permissions`]): either way the first
/// permission the caller lacks refuses the request, and nothing is minted.
///
/// A caller bound to one tenant mints a token bound to that tenant, never
/// one that reaches the platform: the request names no tenant, so the
/// caller's own is the only one it can be.
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
                if let Some(denied) = deny_granting_beyond_the_caller(&admin_token, &permission) {
                    return Ok(denied);
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
        // The caller's own tenant, when it is bound to one: what it mints
        // reaches no further than it does.
        tenant_id: bound_tenant(&admin_token)?.map(|tenant| tenant.to_string()),
        operator_user_id: None,
    }
    .for_creator()?;

    // A request that names no permission is granted a set all the same,
    // the one the store resolves for it, and each of those is the caller's
    // to hand out or it is not, as a named one is. In name order, so the
    // refusal names the same permission every time.
    if token_request.permissions.is_none() {
        let mut granted = token_request.granted_permissions().to_vec();
        granted.sort_by_key(ToString::to_string);
        for permission in &granted {
            if let Some(denied) = deny_granting_beyond_the_caller(&admin_token, permission) {
                return Ok(denied);
            }
        }
    }

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
                    "expires_at": generated_token.expires_at.map(|t| t.to_rfc3339()),
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
        "expires_at": token.expires_at.map(|t| t.to_rfc3339()),
    })
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
/// platform's own admins. A caller bound to one tenant revokes only a
/// token bound to that tenant, and one bound to no tenant, without the
/// super-admin flag, only a token bound to none (403 otherwise, the token
/// left live).
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
    if let Some(denied) = deny_outside_the_callers_reach(&admin_token, &existing, "revoke")? {
        return Ok(denied);
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
/// Deactivates the token and mints a replacement with the same name,
/// permissions, scope and super-admin flag
/// ([`CreateAdminTokenRequest::rotation_of`]), returned once in this
/// response. A revoked token answers 400 and mints nothing: it has no
/// replacement. A caller without the super-admin flag rotates only a token
/// without it whose every permission the caller holds (403 otherwise, the
/// token left live): the replacement is the caller's to keep. A caller bound
/// to one tenant rotates only a token bound to that tenant, and one bound to
/// no tenant, without the super-admin flag, only a token bound to none, for
/// the same reason: the replacement carries the old token's tenant, or none.
///
/// The deactivation and the mint are one transaction in the store
/// ([`AdminRepository::rotate_token`]), which builds the replacement from
/// the row it claims, not from the one read here: a token revoked, or
/// retired with its admin, since that read is refused as a revoked one is.
/// One whose flag or permissions are no longer the ones checked here stays
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

    // Rotation mints the replacement at the old token's privilege level, so a
    // super-admin token rotated by a caller who is not super-admin would hand
    // that caller a fresh super-admin JWT. Checked before the token is
    // touched — a refused rotation must leave the existing credential intact.
    if existing_token.is_super_admin && !admin_token.is_super_admin {
        return Ok(forbidden(
            "Only super-admin tokens can rotate super-admin tokens",
        ));
    }

    // The replacement carries the old token's tenant scope, so a caller
    // without the super-admin flag receives it only when that scope is its
    // own: its tenant, or none.
    if let Some(denied) = deny_outside_the_callers_reach(&admin_token, &existing_token, "rotate")? {
        return Ok(denied);
    }

    // The replacement carries the old token's permissions and is returned to
    // the caller, so it is the caller's to receive only where it could have
    // minted it. Checked before the token is touched, as the flag is.
    for permission in existing_token.permissions.to_vec() {
        if let Some(denied) = deny_granting_beyond_the_caller(&admin_token, &permission) {
            return Ok(denied);
        }
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
                    "expires_at": new_token.expires_at.map(|t| t.to_rfc3339()),
                }))
                .ok(),
            },
            StatusCode::OK,
        )
        .into_response(),
    ))
}
