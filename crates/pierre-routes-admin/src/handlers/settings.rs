// ABOUTME: Admin system settings route handlers
// ABOUTME: Handles the auto-approval configuration endpoints for the CLI and the console
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde_json::to_value;
use tracing::info;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::AppResult;
use pierre_services::admin_settings::{get_auto_approval_settings, set_auto_approval};

use super::api_keys::json_response;
use super::types::{AdminResponse, AutoApprovalResponse, UpdateAutoApprovalRequest};
use super::users::deny_without_manage_users;
use crate::context::AdminApiContext;

/// What the setting means, returned with every read and write.
const AUTO_APPROVAL_DESCRIPTION: &str = "When enabled, all new registrations are auto-approved. \
     When disabled, only emails from auto_approve_domains are auto-approved.";

/// Handle getting the effective auto-approval setting.
///
/// `AUTO_APPROVE_USERS` in the environment outranks the stored row, so the
/// value reported is the one registration actually applies, with
/// `overridden_by_env` telling a client the toggle cannot change it.
pub(crate) async fn handle_get_auto_approval(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }

    info!(
        "Getting auto-approval setting by token: {}",
        admin_token.token_id
    );

    let settings = get_auto_approval_settings(&context.database, &context.app_behavior).await?;

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Auto-approval setting retrieved".to_owned(),
            data: to_value(AutoApprovalResponse {
                enabled: settings.enabled,
                auto_approve_domains: settings.auto_approve_domains,
                overridden_by_env: settings.overridden_by_env,
                description: AUTO_APPROVAL_DESCRIPTION.to_owned(),
            })
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// Handle setting auto-approval.
///
/// The response reports the effective value read back after the write, not the
/// requested one: with `AUTO_APPROVE_USERS` set the write persists and changes
/// nothing, and echoing the request would report a change that did not happen.
pub(crate) async fn handle_set_auto_approval(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<UpdateAutoApprovalRequest>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }

    set_auto_approval(&context.database, request.enabled).await?;
    let effective = get_auto_approval_settings(&context.database, &context.app_behavior).await?;

    info!(
        requested = request.enabled,
        effective = effective.enabled,
        overridden_by_env = effective.overridden_by_env,
        token_id = %admin_token.token_id,
        "Auto-approval setting updated"
    );

    let message = if effective.overridden_by_env {
        "Auto-approval is controlled by the AUTO_APPROVE_USERS environment variable; \
         the stored setting was saved but has no effect while that override is set."
            .to_owned()
    } else {
        format!(
            "Auto-approval has been {}",
            if effective.enabled {
                "enabled"
            } else {
                "disabled"
            }
        )
    };

    Ok(json_response(
        AdminResponse {
            success: true,
            message,
            data: to_value(AutoApprovalResponse {
                enabled: effective.enabled,
                auto_approve_domains: effective.auto_approve_domains,
                overridden_by_env: effective.overridden_by_env,
                description: AUTO_APPROVAL_DESCRIPTION.to_owned(),
            })
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}
