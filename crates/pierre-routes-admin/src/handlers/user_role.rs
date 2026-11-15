// ABOUTME: Make an account an admin, or a user again, over the admin API (pierre-cli user set --role)
// ABOUTME: Takes a super-admin token that acts as an admin; the shared admin_ops promote and demote do the write

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::permissions::UserRole;
use pierre_middleware::redaction::mask_email;
use pierre_services::admin_ops::{self, AdminStandingLost};

use super::api_keys::json_response;
use super::types::AdminResponse;
use crate::audit::{record_withdrawn_credentials, AuditDetail};
use crate::context::AdminApiContext;

/// Body for [`handle_set_user_role`].
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SetUserRoleRequest {
    /// `admin` or `user`.
    pub(crate) role: String,
}

/// Admin: make an account an admin, or a user again.
///
/// `POST /admin/users/{user_id}/role`, which `pierre-cli user set --role`
/// drives, so a deployed environment's admins are managed without a SQL write
/// (carnet#1575). Making an admin hands over every power and unmaking one
/// takes them away, so it takes the rank acting on an admin account does: a
/// super-admin token. And the token must act as an admin, since the write is
/// [`admin_ops::promote_user_to_admin`] or
/// [`admin_ops::demote_user_from_admin`], which check that admin is active,
/// refuse a self-demotion and keep the last active admin.
///
/// # Errors
///
/// Returns a permission error for a token without the super-admin flag or one
/// that acts as no admin, `InvalidInput` for a malformed id, a role that is
/// neither `admin` nor `user`, a role the account already holds, a
/// self-demotion or the last active admin, and `NotFound` for an unknown
/// account.
pub(crate) async fn handle_set_user_role(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Json(request): Json<SetUserRoleRequest>,
) -> AppResult<Response> {
    if !admin_token.is_super_admin {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Changing an account's role requires a super-admin token",
        ));
    }
    let Some(actor) = admin_token.operator_user_id else {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Changing an account's role requires a token that acts as an admin",
        ));
    };
    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;
    let role = match request.role.to_ascii_lowercase().as_str() {
        "admin" => UserRole::Admin,
        "user" => UserRole::User,
        other => {
            return Err(AppError::invalid_input(format!(
                "Unknown role '{other}': expected admin or user"
            )));
        }
    };

    let previous = match role {
        UserRole::Admin => UserRole::User,
        UserRole::User => UserRole::Admin,
    };
    let change = match role {
        UserRole::Admin => {
            admin_ops::promote_user_to_admin(&context.repos, actor, user_uuid).await?
        }
        UserRole::User => {
            let demotion =
                admin_ops::demote_user_from_admin(&context.repos, actor, user_uuid).await?;
            record_withdrawn_credentials(
                &context.repos,
                &admin_token,
                &demotion.withdrawn,
                AdminStandingLost::Demoted,
                Some(user_uuid),
            )
            .await;
            demotion.change
        }
    };

    info!(
        target_user_id = %user_uuid,
        target_email = %mask_email(&change.email),
        role = %change.role,
        token_id = %admin_token.token_id,
        "Account role changed via the admin token surface"
    );

    let audit = AuditDetail::new(
        "user.role",
        format!("Set the role of {} to {}", change.email, change.role),
    )
    .target(json!({ "user_id": change.user_id, "email": change.email }))
    // Each write refuses an account already holding the role, so the other
    // role is the one it held.
    .change(
        json!({ "role": previous.as_str() }),
        json!({ "role": change.role }),
    );
    Ok(audit.attach(
        json_response(
            AdminResponse {
                success: true,
                message: format!("{} is now {}", change.email, change.role),
                data: Some(json!({
                    "user_id": change.user_id,
                    "email": change.email,
                    "role": change.role,
                    "is_admin": change.is_admin,
                })),
            },
            StatusCode::OK,
        )
        .into_response(),
    ))
}
