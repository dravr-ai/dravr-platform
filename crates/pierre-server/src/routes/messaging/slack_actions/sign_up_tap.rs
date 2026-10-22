// ABOUTME: Approve or reject the account a sign-up notice names, from its Slack Approve/Reject buttons
// ABOUTME: The room is the whole authorization; the card turns into the outcome and the account is told
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::StatusCode;
use axum::Json;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserStatus;
use pierre_middleware::mask_email;
use pierre_services::analytics::cache_user_email;
use pierre_services::tenant_admin as tenant_admin_service;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use super::{update_card, ActionType, SlackAction};
use crate::mcp::resources::ServerContext;
use crate::services::user_approval_notifier::ApprovalNotifier;

/// Approve or reject the account a sign-up tap names, then turn its card
/// into the outcome.
///
/// The room is the whole authorization here: no Pierre admin role is
/// required, since operators approve users without holding admin privileges.
pub(super) async fn answer_sign_up_tap(
    resources: &ServerContext,
    action: &SlackAction,
    action_type: ActionType,
    user_uuid: Uuid,
) -> AppResult<(StatusCode, Json<Value>)> {
    // Attribute the action to the Slack handle that clicked. The username
    // rides in the interactive payload, so no `users.info` call or
    // `users:read.email` scope is needed.
    let approver = format!("@{}", action.slack_username);

    // Execute the action
    let result = match action_type {
        ActionType::Approve => approve_user(resources, user_uuid, &approver).await,
        ActionType::Reject => reject_user(resources, user_uuid, &approver).await,
    };

    // Update the Slack message to reflect the action result
    let update_text = match &result {
        Ok(email) => match action_type {
            ActionType::Approve => format!("*{email}* approved by {approver}"),
            ActionType::Reject => format!("*{email}* rejected (suspended) by {approver}"),
        },
        Err(e) => format!("Action failed: {e}"),
    };

    let status_emoji = if result.is_ok() {
        ":white_check_mark:"
    } else {
        ":x:"
    };
    let blocks = json!([{
        "type": "section",
        "text": { "type": "mrkdwn", "text": format!("{status_emoji} {update_text}") }
    }]);
    // Fire-and-forget message update via tronc SlackClient
    update_card(resources, action, &blocks);

    // Return 200 immediately (Slack expects a response within 3 seconds)
    Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
}

/// Approve a user: set status to Active and create tenant if needed
async fn approve_user(
    resources: &ServerContext,
    user_uuid: Uuid,
    approved_by: &str,
) -> AppResult<String> {
    let user = resources
        .common
        .repos
        .users
        .get_global(user_uuid)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if user.user_status == UserStatus::Active {
        return Ok(user.email);
    }

    let updated_user = resources
        .common
        .repos
        .users
        .update_status(user_uuid, UserStatus::Active, None)
        .await
        .map_err(|e| AppError::internal(format!("Failed to approve user: {e}")))?;

    // Create default tenant for the approved user if they don't have one
    let has_tenants = !resources
        .common
        .repos
        .tenants
        .list_for_user(user_uuid)
        .await
        .unwrap_or_default()
        .is_empty();

    if !has_tenants {
        tenant_admin_service::provision_tenant_for_approval(
            &resources.common.repos,
            user_uuid,
            &updated_user.email,
            updated_user.display_name.as_deref(),
            None,
            None,
        )
        .await?;
    }

    info!(
        user_id = %user_uuid,
        email = %mask_email(&updated_user.email),
        approved_by,
        "User approved via Slack action"
    );

    // Warm the identity cache first: the notify enricher resolves user_email
    // from user_id, and without it the Slack message loses the subject.
    cache_user_email(&user_uuid.to_string(), &updated_user.email);
    info!(
        target: "notify",
        event = "user.approved",
        user_id = %user_uuid,
        approved_by = approved_by,
        "user account approved"
    );

    // Notify the approved user: account email + a message on each linked channel.
    ApprovalNotifier::from_context(resources)
        .notify_user_approved(
            user_uuid,
            &updated_user.email,
            updated_user.display_name.as_deref(),
        )
        .await;

    Ok(updated_user.email)
}

/// Reject (suspend) a user
async fn reject_user(
    resources: &ServerContext,
    user_uuid: Uuid,
    rejected_by: &str,
) -> AppResult<String> {
    let user = resources
        .common
        .repos
        .users
        .get_global(user_uuid)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if user.user_status == UserStatus::Suspended {
        return Ok(user.email);
    }

    let updated_user = resources
        .common
        .repos
        .users
        .update_status(user_uuid, UserStatus::Suspended, None)
        .await
        .map_err(|e| AppError::internal(format!("Failed to suspend user: {e}")))?;

    info!(
        user_id = %user_uuid,
        email = %mask_email(&updated_user.email),
        rejected_by,
        "User rejected (suspended) via Slack action"
    );

    cache_user_email(&user_uuid.to_string(), &updated_user.email);
    info!(
        target: "notify",
        event = "user.suspended",
        user_id = %user_uuid,
        suspended_by = rejected_by,
        "user account suspended"
    );

    Ok(updated_user.email)
}
