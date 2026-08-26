// ABOUTME: Admin system settings route handlers
// ABOUTME: Handles the auto-approval configuration endpoints
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde_json::to_value;
use tracing::{error, info};

use pierre_core::admin::models::{AdminPermission as AdminPerm, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};

use super::api_keys::json_response;
use super::types::{AdminResponse, AutoApprovalResponse, UpdateAutoApprovalRequest};
use crate::context::AdminApiContext;

/// Handle getting auto-approval setting
pub(crate) async fn handle_get_auto_approval(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    info!(
        "Getting auto-approval setting by service: {}",
        admin_token.service_name
    );

    let ctx = context.as_ref();

    let enabled = ctx
        .database
        .is_auto_approval_enabled()
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get auto-approval setting");
            AppError::internal(format!("Failed to get auto-approval setting: {e}"))
        })?
        .unwrap_or(false);

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Auto-approval setting retrieved".to_owned(),
            data: to_value(AutoApprovalResponse {
                enabled,
                description: "When enabled, new user registrations are automatically approved without admin intervention".to_owned(),
            })
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Handle setting auto-approval
pub(crate) async fn handle_set_auto_approval(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<UpdateAutoApprovalRequest>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    info!(
        "Setting auto-approval to {} by service: {}",
        request.enabled, admin_token.service_name
    );

    let ctx = context.as_ref();

    ctx.database
        .set_auto_approval_enabled(request.enabled)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to set auto-approval setting");
            AppError::internal(format!("Failed to set auto-approval setting: {e}"))
        })?;

    info!(
        "Auto-approval setting updated to {} by {}",
        request.enabled, admin_token.service_name
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!(
                "Auto-approval has been {}",
                if request.enabled { "enabled" } else { "disabled" }
            ),
            data: to_value(AutoApprovalResponse {
                enabled: request.enabled,
                description: "When enabled, new user registrations are automatically approved without admin intervention".to_owned(),
            })
            .ok(),
        },
        StatusCode::OK,
    ))
}
