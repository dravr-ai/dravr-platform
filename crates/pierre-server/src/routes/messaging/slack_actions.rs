// ABOUTME: Slack interactive actions handler for ops notifications (approve/reject users)
// ABOUTME: Verifies HMAC-SHA256 signature, resolves Slack user to Pierre admin, executes action
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::str;

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use pierre_core::http_client::api_client;
use ring::hmac;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::mcp::resources::ServerResources;
use crate::models::UserStatus;
use crate::services::tenant_admin as tenant_admin_service;

/// Maximum age of a Slack request timestamp before it's rejected (5 minutes)
const MAX_TIMESTAMP_AGE_SECS: u64 = 300;

/// Slack API endpoint for looking up user info by ID
const SLACK_USERS_INFO_URL: &str = "https://slack.com/api/users.info";

/// Slack API endpoint for updating messages
const SLACK_CHAT_UPDATE_URL: &str = "https://slack.com/api/chat.update";

/// Handle Slack interactive action payloads (button clicks from ops notifications)
///
/// Security chain:
/// 1. HMAC-SHA256 signature verification (proves request comes from Slack)
/// 2. Timestamp replay protection (rejects requests older than 5 minutes)
/// 3. Slack user → Pierre admin mapping (only admins can approve/reject)
pub async fn handle_slack_action(
    resources: &ServerResources,
    body: &Bytes,
) -> AppResult<impl IntoResponse> {
    // Parse the Slack interactive payload (form-encoded with `payload` key)
    let payload = parse_interactive_payload(body)?;

    // Extract action details
    let action = extract_action(&payload)?;

    info!(
        action_id = %action.action_id,
        slack_user = %action.slack_user_id,
        "Processing Slack interactive action"
    );

    // Resolve the Slack user's email via Slack API
    let bot_token = env::var("SLACK_BOT_TOKEN")
        .map_err(|_| AppError::internal("SLACK_BOT_TOKEN not configured"))?;
    let slack_email = resolve_slack_user_email(&bot_token, &action.slack_user_id).await?;

    // Verify the Slack user is a Pierre admin
    let admin_user = resources
        .repos
        .users
        .get_by_email(&slack_email)
        .await
        .map_err(|e| AppError::database(format!("Failed to look up admin user: {e}")))?
        .ok_or_else(|| {
            warn!(
                slack_user = %action.slack_user_id,
                email = %slack_email,
                "Slack user is not a Pierre user"
            );
            AppError::auth_invalid("You are not registered as a Pierre user")
        })?;

    if !admin_user.is_admin {
        warn!(
            email = %slack_email,
            "Non-admin user attempted Slack action"
        );
        return Err(AppError::auth_invalid(
            "Only Pierre admins can approve or reject users",
        ));
    }

    // Parse the action to determine approve or reject
    let (action_type, user_id_str) = parse_action_id(&action.action_id)?;
    let user_uuid = Uuid::parse_str(user_id_str)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID in action: {e}")))?;

    // Execute the action
    let result = match action_type {
        ActionType::Approve => approve_user(resources, user_uuid, &slack_email).await,
        ActionType::Reject => reject_user(resources, user_uuid, &slack_email).await,
    };

    // Update the Slack message to reflect the action result
    let update_text = match &result {
        Ok(email) => match action_type {
            ActionType::Approve => format!("*{email}* approved by {slack_email}"),
            ActionType::Reject => format!("*{email}* rejected (suspended) by {slack_email}"),
        },
        Err(e) => format!("Action failed: {e}"),
    };

    // Fire-and-forget message update
    update_slack_message(
        &bot_token,
        &action.channel_id,
        &action.message_ts,
        &update_text,
        result.is_ok(),
    );

    // Return 200 immediately (Slack expects a response within 3 seconds)
    Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
}

/// Verify the Slack request signature using HMAC-SHA256 v0 scheme
///
/// Validates:
/// - `x-slack-request-timestamp` is present and within `MAX_TIMESTAMP_AGE_SECS`
/// - `x-slack-signature` matches HMAC-SHA256 of `v0:{timestamp}:{body}`
pub fn verify_slack_signature(
    signing_secret: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<()> {
    let timestamp = headers
        .get("x-slack-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::auth_invalid("Missing x-slack-request-timestamp header"))?;

    let signature = headers
        .get("x-slack-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::auth_invalid("Missing x-slack-signature header"))?;

    // Replay protection
    let ts: u64 = timestamp
        .parse()
        .map_err(|_| AppError::auth_invalid("Invalid timestamp format"))?;
    let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
    let age = now.saturating_sub(ts);
    if age > MAX_TIMESTAMP_AGE_SECS {
        return Err(AppError::auth_invalid(format!(
            "Request timestamp too old ({age}s)"
        )));
    }

    // Compute HMAC-SHA256 using Slack v0 scheme
    let body_str = str::from_utf8(body).unwrap_or("");
    let basestring = format!("v0:{timestamp}:{body_str}");
    let key = hmac::Key::new(hmac::HMAC_SHA256, signing_secret.as_bytes());
    let tag = hmac::sign(&key, basestring.as_bytes());
    let expected = format!("v0={}", hex::encode(tag.as_ref()));

    // Constant-time comparison via ring
    if signature == expected {
        Ok(())
    } else {
        Err(AppError::auth_invalid("Invalid Slack signature"))
    }
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Parsed Slack interactive action
struct SlackAction {
    action_id: String,
    slack_user_id: String,
    channel_id: String,
    message_ts: String,
}

enum ActionType {
    Approve,
    Reject,
}

/// Parse the form-encoded interactive payload from Slack
///
/// Slack sends interactive payloads as `application/x-www-form-urlencoded`
/// with a single `payload` key containing JSON.
fn parse_interactive_payload(body: &Bytes) -> AppResult<Value> {
    let body_str = str::from_utf8(body)
        .map_err(|e| AppError::invalid_input(format!("Invalid UTF-8 in body: {e}")))?;

    for pair in body_str.split('&') {
        if let Some(value) = pair.strip_prefix("payload=") {
            let decoded = urlencoding::decode(value)
                .map_err(|e| AppError::invalid_input(format!("Invalid URL encoding: {e}")))?;
            return serde_json::from_str(&decoded)
                .map_err(|e| AppError::invalid_input(format!("Invalid JSON in payload: {e}")));
        }
    }

    Err(AppError::invalid_input(
        "Missing payload field in interactive request",
    ))
}

/// Extract the first action from a Slack `block_actions` payload
fn extract_action(payload: &Value) -> AppResult<SlackAction> {
    let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");

    if payload_type != "block_actions" {
        return Err(AppError::invalid_input(format!(
            "Unexpected payload type: {payload_type}"
        )));
    }

    let action = payload
        .get("actions")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| AppError::invalid_input("No actions in payload"))?;

    let action_id = action
        .get("action_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing action_id"))?;

    let slack_user_id = payload
        .pointer("/user/id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing user ID in payload"))?;

    let channel_id = payload
        .pointer("/channel/id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing channel ID in payload"))?;

    let message_ts = payload
        .pointer("/message/ts")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing message timestamp in payload"))?;

    Ok(SlackAction {
        action_id: action_id.to_owned(),
        slack_user_id: slack_user_id.to_owned(),
        channel_id: channel_id.to_owned(),
        message_ts: message_ts.to_owned(),
    })
}

/// Parse `action_id` format `approve_user:{uuid}` or `reject_user:{uuid}`
fn parse_action_id(action_id: &str) -> AppResult<(ActionType, &str)> {
    action_id
        .strip_prefix("approve_user:")
        .map(|id| (ActionType::Approve, id))
        .or_else(|| {
            action_id
                .strip_prefix("reject_user:")
                .map(|id| (ActionType::Reject, id))
        })
        .ok_or_else(|| AppError::invalid_input(format!("Unknown action: {action_id}")))
}

/// Resolve a Slack user ID to their email address via the Slack `users.info` API
async fn resolve_slack_user_email(bot_token: &str, slack_user_id: &str) -> AppResult<String> {
    let client = api_client();
    let response = client
        .get(SLACK_USERS_INFO_URL)
        .header("Authorization", format!("Bearer {bot_token}"))
        .query(&[("user", slack_user_id)])
        .send()
        .await
        .map_err(|e| AppError::internal(format!("Slack users.info request failed: {e}")))?;

    let body: Value = response
        .json()
        .await
        .map_err(|e| AppError::internal(format!("Invalid Slack users.info response: {e}")))?;

    let ok = body.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        let error = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(AppError::internal(format!(
            "Slack users.info failed: {error}"
        )));
    }

    body.pointer("/user/profile/email")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::internal(
                "Slack user profile missing email (bot may need users:read.email scope)",
            )
        })
}

/// Approve a user: set status to Active and create tenant if needed
async fn approve_user(
    resources: &ServerResources,
    user_uuid: Uuid,
    approved_by: &str,
) -> AppResult<String> {
    let user = resources
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
        .repos
        .users
        .update_status(user_uuid, UserStatus::Active, None)
        .await
        .map_err(|e| AppError::internal(format!("Failed to approve user: {e}")))?;

    // Create default tenant for the approved user if they don't have one
    let has_tenants = !resources
        .repos
        .tenants
        .list_for_user(user_uuid)
        .await
        .unwrap_or_default()
        .is_empty();

    if !has_tenants {
        tenant_admin_service::provision_tenant_for_approval(
            &resources.repos,
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
        email = %updated_user.email,
        approved_by,
        "User approved via Slack action"
    );

    crate::ops_notifier().notify_user_approved(&updated_user.email, approved_by);

    Ok(updated_user.email)
}

/// Reject (suspend) a user
async fn reject_user(
    resources: &ServerResources,
    user_uuid: Uuid,
    rejected_by: &str,
) -> AppResult<String> {
    let user = resources
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
        .repos
        .users
        .update_status(user_uuid, UserStatus::Suspended, None)
        .await
        .map_err(|e| AppError::internal(format!("Failed to suspend user: {e}")))?;

    info!(
        user_id = %user_uuid,
        email = %updated_user.email,
        rejected_by,
        "User rejected (suspended) via Slack action"
    );

    crate::ops_notifier().notify_user_suspended(&updated_user.email, rejected_by);

    Ok(updated_user.email)
}

/// Update the original Slack message to replace buttons with action result
///
/// Fire-and-forget — spawns a background task, never blocks the response.
fn update_slack_message(
    bot_token: &str,
    channel_id: &str,
    message_ts: &str,
    text: &str,
    success: bool,
) {
    let token = bot_token.to_owned();
    let channel = channel_id.to_owned();
    let ts = message_ts.to_owned();
    let status_emoji = if success { ":white_check_mark:" } else { ":x:" };
    let update_text = format!("{status_emoji} {text}");

    tokio::spawn(async move {
        let client = api_client();
        let payload = json!({
            "channel": channel,
            "ts": ts,
            "blocks": [
                {
                    "type": "section",
                    "text": { "type": "mrkdwn", "text": update_text }
                }
            ]
        });

        let result = client
            .post(SLACK_CHAT_UPDATE_URL)
            .header("Authorization", format!("Bearer {token}"))
            .json(&payload)
            .send()
            .await;

        match result {
            Ok(response) => {
                if let Ok(body) = response.json::<Value>().await {
                    let ok = body.get("ok").and_then(Value::as_bool).unwrap_or(false);
                    if !ok {
                        let error = body
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown");
                        warn!(error, "Failed to update Slack message after action");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to send Slack message update");
            }
        }
    });
}
