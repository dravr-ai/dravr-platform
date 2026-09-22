// ABOUTME: Admin routes that remove a user completely, or disconnect one provider for them
// ABOUTME: Both go through the provider-disconnect chokepoint, so every grant is revoked at the provider

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Operator removal over the admin API.
//!
//! `DELETE /admin/users/{user_id}` used to delete the row and lean on the
//! cascade: the token rows went, the grant at Strava did not, so the athlete
//! kept a seat on Strava's side after losing it on ours. Both routes here hand
//! the work to [`pierre_services::user_removal`], which reaches the same
//! disconnect chokepoint the athlete's own disconnect uses.
//!
//! Emails appear in response bodies (the operator asked about that account)
//! and never in logs, which carry the user id.
//!
//! Both routes reach every tenant that holds the user, so a token scoped to
//! one tenant acts only on a user who is held entirely within it; an unscoped
//! `ManageUsers` token acts globally, as it does on the rest of the users
//! surface.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::{json, to_value, Value};
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::User;
use pierre_services::user_removal::{
    self, HeldProvider, Interruption, ProviderDisconnection, UserRemoval,
};

use super::api_keys::json_response;
use super::types::{AdminResponse, DeleteUserRequest};
use super::users::deny_without_manage_users;
use crate::context::AdminApiContext;

/// What a delete answers when the database refused it over a reference the
/// blocker read did not see (one written between that read and the delete).
const RESIDUAL_REFERENCE: &str =
    "User is still referenced by a row that does not cascade; reassign it and retry";

/// A refusal in the neighbouring handlers' response shape.
fn refusal(status: StatusCode, message: String, data: Option<Value>) -> Response {
    json_response(
        AdminResponse {
            success: false,
            message,
            data,
        },
        status,
    )
    .into_response()
}

/// The answer to a removal that failed part-way: the failing step's status,
/// and a message and body naming what was done, so an operator never reads a
/// partial removal as "nothing happened".
fn interrupted(user_id: Uuid, interruption: &Interruption) -> Response {
    let error = &interruption.error;
    warn!(
        user_id = %user_id,
        disconnected = interruption.disconnected.len(),
        failed_provider = interruption.failed.as_ref().map(|f| f.provider.as_str()),
        error = %error.internal_details(),
        "Operator removal interrupted"
    );
    let cause = if error.code == ErrorCode::ResourceLocked {
        RESIDUAL_REFERENCE.to_owned()
    } else {
        error.sanitized_message()
    };
    let status =
        StatusCode::from_u16(error.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    refusal(
        status,
        interruption.describe(&cause),
        to_value(json!({
            "disconnected": interruption.disconnected,
            "failed": interruption.failed,
            "account_removed": false,
        }))
        .ok(),
    )
}

/// Refuse a tenant-scoped token acting on a user held outside its tenant.
///
/// A delete or a provider disconnect reaches every tenant the user belongs
/// to or holds a provider in, so a token scoped to one tenant may act only
/// when every such tenant is its own. Every membership row counts, an
/// inactive or ownerless tenant's too: the delete clears the user's rows
/// there as well. Super-admin and unscoped tokens pass.
async fn require_within_token_tenant(
    ctx: &AdminApiContext,
    admin_token: &ValidatedAdminToken,
    user_id: Uuid,
    held: &[HeldProvider],
) -> AppResult<()> {
    let Some(bound) = admin_token
        .tenant_id
        .as_deref()
        .filter(|_| !admin_token.is_super_admin)
    else {
        return Ok(());
    };
    let mut tenants: BTreeSet<String> = ctx
        .repos
        .tenants
        .list_membership_tenant_ids(user_id)
        .await?
        .into_iter()
        .map(|tenant_id| tenant_id.to_string())
        .collect();
    tenants.extend(held.iter().map(|h| h.tenant_id.to_string()));
    if !tenants.is_empty() && tenants.iter().all(|tenant| tenant == bound) {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::PermissionDenied,
        format!(
            "Token is scoped to tenant {bound}; the user belongs to or holds a provider in a tenant outside it"
        ),
    ))
}

/// The refusal a disconnect or delete takes when the composition root wired
/// no chokepoint: acting without it would leave the grant authorized upstream.
fn disconnect_unavailable() -> AppError {
    AppError::resource_unavailable("Provider disconnect is not wired on this server")
}

/// Parse the path id and read the user, 404 when there is none.
async fn load_user(ctx: &AdminApiContext, user_id: &str) -> AppResult<(Uuid, User)> {
    let user_uuid = Uuid::parse_str(user_id).map_err(|e| {
        error!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;
    let user = ctx
        .repos
        .users
        .get_global(user_uuid)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to fetch user from database");
            AppError::internal(format!("Failed to fetch user: {e}"))
        })?
        .ok_or_else(|| {
            warn!("User not found: {user_id}");
            AppError::not_found("User not found")
        })?;
    Ok((user_uuid, user))
}

/// `DELETE /admin/users/{user_id}/providers/{provider}` — disconnect one
/// provider for a user.
///
/// Runs the athlete's own disconnect on their behalf in every tenant that
/// holds the provider: the grant is revoked at the provider and the token and
/// connection rows go, freeing the seat on both sides. Each tenant's entry
/// says whether the provider confirmed the revocation. 404 for an unknown user
/// and for a provider the user holds no connection to; 400, changing nothing,
/// for a provider this build cannot disconnect. A disconnect that fails is
/// answered with the failing step's status and names what was already
/// disconnected, the failed tenant's possibly revoked grant included.
///
/// # Errors
///
/// Returns an invalid-input error for a malformed id, a not-found error for an
/// unknown user, a permission error for a tenant-scoped token and a user held
/// outside its tenant, a resource-unavailable error when no chokepoint is
/// wired, and a database error when the held providers cannot be read; nothing
/// was changed then.
pub async fn handle_disconnect_user_provider(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path((user_id, provider)): Path<(String, String)>,
) -> AppResult<Response> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }

    let ctx = context.as_ref();
    let (user_uuid, user) = load_user(ctx, &user_id).await?;
    let disconnector = ctx
        .provider_disconnector
        .as_deref()
        .ok_or_else(disconnect_unavailable)?;

    let held = user_removal::held_providers(&ctx.repos, user_uuid).await?;
    require_within_token_tenant(ctx, &admin_token, user_uuid, &held).await?;
    let targets = user_removal::providers_named(held, &provider);

    let disconnected = match user_removal::disconnect_user_provider(
        disconnector,
        user_uuid,
        targets,
    )
    .await
    {
        ProviderDisconnection::Disconnected(disconnected) => disconnected,
        ProviderDisconnection::NotRevocable(held) => {
            return Ok(refusal(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "This server cannot disconnect {provider}; nothing was revoked or removed. Deleting the user clears its rows."
                    ),
                    to_value(json!({ "not_revocable": held })).ok(),
                ));
        }
        ProviderDisconnection::Interrupted(interruption) => {
            return Ok(interrupted(user_uuid, &interruption));
        }
    };

    if disconnected.is_empty() {
        return Ok(refusal(
            StatusCode::NOT_FOUND,
            format!("User has no {provider} connection to disconnect"),
            None,
        ));
    }

    info!(
        user_id = %user_uuid,
        provider = %provider,
        tenants = disconnected.len(),
        service = %admin_token.service_name,
        "Operator disconnected a provider for a user"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Disconnected {provider} for {}", user.email),
            data: to_value(json!({
                "user_id": user_uuid.to_string(),
                "email": user.email,
                "disconnected": disconnected,
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `DELETE /admin/users/{user_id}` — remove a user completely.
///
/// Refuses with 409, naming each row, while the user owns or coaches a group,
/// created an invite, holds operator or audit attribution, is the only owner
/// of a tenant others belong to, authored an agent others rely on, or holds a
/// subscription the billing provider may still charge; nothing is touched
/// then. Otherwise every provider is disconnected through the
/// chokepoint (revoked at the provider) before the account is deleted in one
/// transaction with its memberships and every row no foreign key cascades
/// to. A reference that appears between that check and the delete is a 409
/// too, not a 500, and like any failure after a provider was disconnected it
/// names what was already done.
///
/// # Errors
///
/// Returns an invalid-input error for a malformed id, a not-found error for an
/// unknown user, a permission error for a tenant-scoped token and a user held
/// outside its tenant, a resource-unavailable error when the user holds
/// providers and no chokepoint is wired, and a database error from the reads
/// that precede any change. A disconnect or account delete that fails is
/// answered with the failing step's status, naming what was already done,
/// never returned as an error.
pub async fn handle_delete_user(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Json(request): Json<DeleteUserRequest>,
) -> AppResult<Response> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }

    let ctx = context.as_ref();
    let (user_uuid, user) = load_user(ctx, &user_id).await?;
    let held = user_removal::held_providers(&ctx.repos, user_uuid).await?;
    require_within_token_tenant(ctx, &admin_token, user_uuid, &held).await?;
    let reason = request.reason.as_deref().unwrap_or("No reason provided");

    let outcome =
        user_removal::remove_user(&ctx.repos, ctx.provider_disconnector.as_deref(), user_uuid)
            .await?;

    let report = match outcome {
        UserRemoval::Blocked(blockers) => {
            info!(
                user_id = %user_uuid,
                blockers = blockers.len(),
                service = %admin_token.service_name,
                "User delete refused: references must be reassigned first"
            );
            return Ok(refusal(
                StatusCode::CONFLICT,
                user_removal::blockers_message(&blockers),
                to_value(json!({ "blockers": blockers })).ok(),
            ));
        }
        UserRemoval::Interrupted(interruption) => {
            return Ok(interrupted(user_uuid, &interruption));
        }
        UserRemoval::Removed(report) => report,
    };

    info!(
        user_id = %user_uuid,
        service = %admin_token.service_name,
        reason = %reason,
        "User deleted by operator"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "User deleted successfully".to_owned(),
            data: to_value(json!({
                "deleted_user": {
                    "id": user_uuid.to_string(),
                    "email": user.email,
                },
                "disconnected": report.disconnected,
                "not_revocable": report.not_revocable,
                "memberships_removed": report.memberships_removed,
                "rows_removed": report.rows_removed,
                "reason": reason,
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}
