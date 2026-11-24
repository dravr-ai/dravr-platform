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
//! them, is not found when they read, revoke or rotate it, and cannot be
//! minted by them — and a caller that is not super-admin grants only
//! permissions it holds, by minting a token or by rotating one: the ones a
//! request names, and the defaults one that names none is given.
//!
//! A caller bound to one tenant stays inside it on every read and write: it
//! lists, reads, revokes and rotates only tokens bound to that tenant, and
//! what it mints is bound to that tenant too ([`ScopeCaller::BoundTo`]). A
//! token bound to none, or to another tenant, is out of its reach, so a
//! `ManageAdminTokens` holder scoped to one tenant cannot rotate or mint its
//! way to the platform, nor read what the platform or another tenant holds.
//!
//! A caller bound to no tenant that does not carry the super-admin flag
//! ([`ScopeCaller::Unbound`]) reads no tenant's data, so it lists, reads,
//! revokes and rotates only tokens bound to no tenant, and mints only
//! those: a rotation would otherwise hand it a credential that reads a
//! tenant it cannot. An admin's console session and a super-admin token
//! ([`ScopeCaller::Platform`]) read and write every token.
//!
//! One rule decides the reach, for the reads and the writes alike
//! ([`within_the_callers_reach`]). A read, a revoke or a rotation of one
//! token outside it answers as a missing token does (404), and the list
//! leaves it out: no answer says that a token the caller does not reach
//! exists, where it is bound, or what kind it is. A super-admin token is
//! outside the reach of every caller without the flag, wherever it is
//! bound, so it answers 404 too, as the list that hides it says
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
use pierre_core::admin::scope::ScopeCaller;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{TokenRevocation, TokenRotation};

use super::api_keys::json_response;
use super::types::AdminResponse;
use crate::audit::AuditDetail;
use crate::context::AdminApiContext;
use dravr_tronc::dates::Timestamp;

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
/// here ([`within_the_callers_reach`]). The one definition is the
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

/// Whether `token` is one `caller` reaches: the one rule for listing,
/// reading, revoking and rotating a token.
///
/// A caller that reaches the platform, an admin's console session or a
/// super-admin token, reaches every token. Any other caller reaches no
/// super-admin token, which reaches further than it does. A caller bound to
/// one tenant reaches only tokens bound to that tenant: one bound to none
/// reaches further than it does, and one bound elsewhere reaches a tenant
/// it does not. A caller bound to no tenant, without the super-admin flag,
/// reaches only tokens bound to no tenant: it reads no tenant's data itself
/// ([`ValidatedAdminToken::reach`]), and the replacement of a tenant-bound
/// token would.
fn within_the_callers_reach(caller: ScopeCaller, token: &AdminToken) -> bool {
    match caller {
        ScopeCaller::Platform => true,
        ScopeCaller::BoundTo(_) | ScopeCaller::Unbound if token.is_super_admin => false,
        ScopeCaller::BoundTo(own) => {
            token
                .tenant_id
                .as_deref()
                .and_then(|raw| TenantId::parse_str(raw).ok())
                == Some(own)
        }
        ScopeCaller::Unbound => token.tenant_id.is_none(),
    }
}

/// Load the token `caller` names by id: the row every handler decides on
/// before it reads, revokes or rotates. An unknown id answers 404, and so
/// does a token outside what the caller reaches
/// ([`within_the_callers_reach`]), in the same words ([`missing_token`]).
/// Revoking a token outside it is a denial of service there, and rotating
/// one hands the caller its replacement, which carries the old token's
/// tenant, or none.
///
/// # Errors
/// A not-found error for both, and `InvalidInput` when the tenant the
/// caller carries is not a tenant id.
async fn load_reachable_token(
    ctx: &AdminApiContext,
    caller: &ValidatedAdminToken,
    token_id: &str,
) -> AppResult<AdminToken> {
    let token = ctx
        .repos
        .admin
        .get_token_by_id(token_id)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get admin token");
            AppError::internal(format!("Failed to get admin token: {e}"))
        })?
        .ok_or_else(|| missing_token(token_id))?;
    if !within_the_callers_reach(caller.scope_caller()?, &token) {
        return Err(missing_token(token_id));
    }
    Ok(token)
}

/// What an id no token has is answered, and a token the caller may not
/// know of: one error, so the two cannot be told apart.
fn missing_token(token_id: &str) -> AppError {
    AppError::not_found(format!("Admin token {token_id}"))
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
/// A caller is shown the tokens it reaches ([`within_the_callers_reach`]):
/// every token for an admin's console session or a super-admin token, its
/// tenant's for a caller bound to one, and the ones bound to no tenant for
/// a caller bound to none, never a super-admin token for a caller without
/// the flag.
///
/// # Errors
/// `InvalidInput` when the tenant the caller carries is not a tenant id,
/// and an internal error when the tokens cannot be read.
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

    let scope = admin_token.scope_caller()?;
    let visible: Vec<AdminTokenSummary> = tokens
        .into_iter()
        .filter(|token| within_the_callers_reach(scope, token))
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
///
/// A token outside what the caller reaches ([`within_the_callers_reach`]),
/// a super-admin token for a caller without the flag included, answers 404
/// in the words a missing id is answered with: the read does not say that a
/// token it will not show exists.
///
/// # Errors
/// A not-found error for an unknown id and for a token outside the caller's
/// reach, and `InvalidInput` when the tenant the caller carries is not a
/// tenant id.
pub(crate) async fn handle_get_admin_token(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(token_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_admin_tokens(&admin_token) {
        return Ok(denied);
    }

    let token = load_reachable_token(&context, &admin_token, &token_id).await?;

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
/// An unknown id answers 404. A caller bound to one tenant revokes only a
/// token bound to that tenant, and one bound to no tenant, without the
/// super-admin flag, only a token bound to none: a token outside that
/// reach answers 404 as an unknown id does, and is left live
/// ([`load_reachable_token`]). So does a super-admin token for a caller
/// without the flag: revoking one is a denial of service against the
/// platform's own admins.
///
/// A rotated token is not revoked a second time (409): the rotation retired
/// it and minted a replacement that is still live, so answering success
/// would tell the caller a credential had ended that only changed id. The
/// refusal names the replacement by id and says to revoke that one
/// ([`rotated_token_refusal`]); it is answered after the checks above, so
/// only to a caller that reaches the token. A token revoked earlier names
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

    load_reachable_token(&context, &admin_token, &token_id).await?;

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
/// already has ([`rerotated_token_refusal`]), after the checks below, so
/// only to a caller that could have rotated it. A caller without the
/// super-admin flag does not find a super-admin token (404), and rotates
/// only a token whose every permission the caller holds (403 otherwise, the
/// token left live): the replacement is the caller's to keep. A caller bound
/// to one tenant rotates only a token bound to that tenant, and one bound to
/// no tenant, without the super-admin flag, only a token bound to none, for
/// the same reason: the replacement carries the old token's tenant, or none.
/// A token outside that reach answers 404, as an unknown id does, and is
/// left live ([`load_reachable_token`]).
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
    // The replacement carries the old token's tenant scope and privilege
    // level, so a caller without the super-admin flag receives it only when
    // that scope is its own (its tenant, or none) and the token is not a
    // super-admin one, which would hand it a fresh super-admin JWT. Any
    // other token is not found, and is left intact.
    let existing_token = load_reachable_token(ctx, &admin_token, &token_id).await?;

    // The replacement carries the old token's permissions and is returned to
    // the caller, so it is the caller's to receive only where it could have
    // minted it. Checked before the token is touched, as the flag is.
    for permission in existing_token.permissions.to_vec() {
        if let Some(denied) = deny_granting_beyond_the_caller(&admin_token, &permission) {
            return Ok(denied);
        }
    }

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
