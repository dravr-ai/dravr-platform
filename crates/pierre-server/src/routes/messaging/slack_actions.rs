// ABOUTME: Slack interactive actions handler for ops notifications and messaging command postbacks
// ABOUTME: Verifies HMAC-SHA256 signature via dravr-tronc, routes ops actions and command callbacks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::str;
use std::sync::Arc;

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use dravr_tronc::notifications::{SlackClient, SlackConfig};
use pierre_core::models::TenantId;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress::resolve_messaging_locale;
use crate::services::user_approval_notifier::ApprovalNotifier;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserStatus;
use pierre_services::analytics::cache_user_email;
use pierre_services::tenant_admin as tenant_admin_service;

/// Channel type key Slack channel links are stored under in
/// `messaging_channel_links`, matching the webhook path segment the ingress
/// path uses.
const SLACK_CHANNEL: &str = "slack";

/// Handle Slack interactive action payloads (button clicks)
///
/// Routes two kinds of actions:
/// - **Command postbacks**: `action_id` starts with `/` (e.g. `/coach add @handle`).
///   These come from messaging card buttons and are routed through the command system.
/// - **Ops actions**: `approve_user:` / `reject_user:` prefixed actions from admin
///   notifications. Requires Pierre admin role.
///
/// Security chain:
/// 1. HMAC-SHA256 signature verification (proves request comes from Slack)
/// 2. Timestamp replay protection (rejects requests older than 5 minutes)
/// 3. Per-route authorization — the ops channel for approve/reject, the
///    channel link and user-status gate for command postbacks. The signature
///    proves the click came from Slack, never that the clicking Slack identity
///    may act as a given Dravr user.
pub(crate) async fn handle_slack_action(
    resources: &Arc<ServerContext>,
    body: &Bytes,
) -> AppResult<(StatusCode, Json<Value>)> {
    // Parse the Slack interactive payload (form-encoded with `payload` key)
    let payload = parse_interactive_payload(body)?;

    // Extract action details
    let action = extract_action(&payload)?;

    info!(
        action_id = %action.action_id,
        slack_user = %action.slack_user_id,
        "Processing Slack interactive action"
    );

    // Normalize action_id: Slack may URL-encode spaces as `+`
    let normalized_action_id = action.action_id.replace('+', " ");

    // Route command postbacks (action_id starts with `/`) through the command system
    if normalized_action_id.starts_with('/') {
        return handle_command_postback(resources, &action, &normalized_action_id).await;
    }

    // --- Ops actions below (approve/reject users) ---

    // Authorization: trust the ops channel. The approve/reject buttons are
    // only ever posted to the configured ops users channel — a private
    // channel whose membership IS the set of authorized operators. The HMAC
    // signature (verified by the HTTP handler) proves the click is genuinely
    // from Slack; we additionally require the action to originate from that
    // channel so the buttons are honored nowhere else. No Pierre admin role
    // is required: operators approve users without holding admin privileges.
    verify_ops_channel(&action)?;

    // Attribute the action to the Slack handle that clicked. The username
    // rides in the interactive payload, so no `users.info` call or
    // `users:read.email` scope is needed.
    let approver = format!("@{}", action.slack_username);

    // The bot token is still required to update the message in place.
    let bot_token = env::var("SLACK_BOT_TOKEN")
        .map_err(|_| AppError::internal("SLACK_BOT_TOKEN not configured"))?;

    // Parse the action to determine approve or reject
    let (action_type, user_id_str) = parse_action_id(&action.action_id)?;
    let user_uuid = Uuid::parse_str(user_id_str)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID in action: {e}")))?;

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

    // Fire-and-forget message update via tronc SlackClient
    let status_emoji = if result.is_ok() {
        ":white_check_mark:"
    } else {
        ":x:"
    };
    let blocks = json!([{
        "type": "section",
        "text": { "type": "mrkdwn", "text": format!("{status_emoji} {update_text}") }
    }]);

    let slack_client = build_slack_client(&bot_token);
    slack_client.update_message(&action.channel_id, &action.message_ts, &blocks);

    // Return 200 immediately (Slack expects a response within 3 seconds)
    Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
}

/// Verify the Slack request signature using dravr-tronc's `SlackClient`
///
/// Extracts timestamp and signature headers, delegates HMAC verification
/// to the shared implementation.
pub(crate) fn verify_slack_signature(
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

    let config = SlackConfig {
        bot_token: String::new(),
        error_channel: String::new(),
        signing_secret: Some(signing_secret.to_owned()),
    };
    let client = SlackClient::new(&config);

    client
        .verify_signature(timestamp, signature, body)
        .map_err(|e| AppError::auth_invalid(format!("Slack signature verification failed: {e}")))
}

// =============================================================================
// Command postback routing
// =============================================================================

/// Handle a messaging command postback from a Slack interactive action.
///
/// Executes the command on behalf of the authorized clicker and updates the
/// originating card with the reply via the Bot Token.
async fn handle_command_postback(
    resources: &Arc<ServerContext>,
    action: &SlackAction,
    command_text: &str,
) -> AppResult<(StatusCode, Json<Value>)> {
    // The bot token updates the card in place once the command has run.
    let bot_token = env::var("SLACK_BOT_TOKEN")
        .map_err(|_| AppError::internal("SLACK_BOT_TOKEN not configured"))?;

    let response_text = execute_postback_command(
        resources,
        &action.slack_user_id,
        &action.channel_id,
        command_text,
    )
    .await?;

    // Send response to Slack channel
    let slack_client = build_slack_client(&bot_token);
    let blocks = json!([{
        "type": "section",
        "text": { "type": "mrkdwn", "text": response_text }
    }]);
    slack_client.update_message(&action.channel_id, &action.message_ts, &blocks);

    Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
}

/// Authorize the Slack identity that clicked a card button and run the
/// command it posted back, returning the reply text.
///
/// Split from [`handle_command_postback`] so the authorization gate and the
/// command dispatch are exercised without the outbound Slack call. Exposed
/// for the postback authorization integration tests.
///
/// # Errors
///
/// Propagates the channel-authorization outcome (unlinked Slack identity,
/// pending / suspended account, rate limit) and any command-dispatch failure.
pub async fn execute_postback_command(
    resources: &Arc<ServerContext>,
    slack_user_id: &str,
    channel_id: &str,
    command_text: &str,
) -> AppResult<String> {
    use pierre_messaging::commands::CommandMatcher;

    use pierre_commands::{ConversationRotation, PlatformCommandContext};

    let (user_id, user_tenant) = authorize_postback(resources, slack_user_id).await?;

    // Match command against registry
    let cmd_registry = resources
        .common
        .command_registry
        .as_ref()
        .ok_or_else(|| AppError::internal("Command registry not initialized"))?;
    let handler_registry = resources
        .common
        .command_handler_registry
        .as_ref()
        .ok_or_else(|| AppError::internal("Command handler registry not initialized"))?;

    let matcher = CommandMatcher::from_registry(cmd_registry);
    let parsed = matcher
        .try_match(command_text, cmd_registry)
        .ok_or_else(|| AppError::invalid_input(format!("Unknown command: {command_text}")))?;

    let handler = handler_registry.get(&parsed.name).ok_or_else(|| {
        AppError::invalid_input(format!("No handler for command: {}", parsed.name))
    })?;

    let locale = resolve_messaging_locale(
        resources,
        user_tenant,
        user_id,
        SLACK_CHANNEL,
        slack_user_id,
    )
    .await;

    let ctx_dyn: Arc<dyn pierre_runtime_context::CommandCtx> =
        Arc::<ServerContext>::clone(resources);
    let tool_runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
    let ctx = PlatformCommandContext {
        user_id,
        tenant_id: user_tenant,
        channel_type: SLACK_CHANNEL.to_owned(),
        args: parsed.args,
        raw_text: parsed.raw_text,
        ctx: ctx_dyn,
        locale,
        // Slack block_actions payloads don't expose event.channel_type;
        // use the channel ID prefix convention ("D" = IM, "C" = channel,
        // "G" = private group).
        is_direct_message: channel_id.starts_with('D'),
        // A messaging surface: the group a button means is the group the
        // caller is in.
        ambient_group_fallback: true,
        // Block-actions buttons fire outside a Pierre conversation
        // turn — there's no chat_conversations row associated with the
        // click. Group-scoped commands fall back to the user's most
        // recent group via list_groups_for_user when this is None.
        conversation_id: None,
        // No conversation to scope, so this tracks the caller's tenant.
        conversation_tenant_id: user_tenant,
        sender_id: Some(slack_user_id.to_owned()),
        rotation: ConversationRotation::default(),
        tool_runtime,
    };

    let response = handler.execute(&ctx).await?;

    info!(
        command = %parsed.name,
        user_id = %user_id,
        slack_user = %slack_user_id,
        "Slack command postback executed"
    );

    Ok(response.text)
}

/// Authorize the Slack identity behind a command postback through the same
/// gate every inbound messaging turn walks.
///
/// Resolves the Slack identity to the tenant that owns its channel link, then
/// calls [`pierre_middleware::auth::McpAuthMiddleware::authenticate_channel`]
/// — the single source of truth that requires a `messaging_channel_links` row
/// (proof the clicker completed the link flow and owns the Pierre account),
/// applies the approval policy, and checks the per-user rate limit. Identity
/// therefore rests on the verified link, not on the profile email Slack
/// reports for the clicker: the HMAC signature proves the click came from
/// Slack, nothing more.
///
/// Returns the Pierre user id and the tenant the command runs under — the
/// user's own first-membership tenant, falling back to the link owner's
/// tenant, the same resolution `messaging_ingress` performs on every turn.
async fn authorize_postback(
    resources: &ServerContext,
    slack_user_id: &str,
) -> AppResult<(Uuid, TenantId)> {
    // The link row is the authoritative `(channel, channel_user_id) -> owner
    // tenant` map; the ops-action endpoint carries no tenant of its own.
    let link_tenant = resources
        .common
        .repos
        .messaging
        .get_channel_link_tenant(SLACK_CHANNEL, slack_user_id)
        .await?
        .ok_or_else(|| {
            warn!(
                slack_user = %slack_user_id,
                "Slack command postback from an unlinked Slack identity"
            );
            AppError::auth_invalid("This Slack account is not linked to a Dravr account")
        })?;

    let auth = resources
        .auth
        .auth_middleware
        .authenticate_channel(link_tenant, SLACK_CHANNEL, slack_user_id)
        .await?;

    let tenant_id = auth
        .active_tenant_id
        .map_or(link_tenant, TenantId::from_uuid);

    Ok((auth.user_id, tenant_id))
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Build a `SlackClient` from a bot token for message updates
fn build_slack_client(bot_token: &str) -> SlackClient {
    let signing_secret = env::var("SLACK_SIGNING_SECRET")
        .ok()
        .filter(|s| !s.is_empty());

    let config = SlackConfig {
        bot_token: bot_token.to_owned(),
        error_channel: String::new(),
        signing_secret,
    };

    SlackClient::new(&config)
}

/// Parsed Slack interactive action
struct SlackAction {
    action_id: String,
    slack_user_id: String,
    /// Slack display handle of the clicker, carried in the interactive
    /// payload — used for attribution without a `users.info` lookup.
    slack_username: String,
    channel_id: String,
    /// Channel name (without leading `#`) the action originated from, used to
    /// enforce the ops-channel trust boundary.
    channel_name: String,
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

    // Slack handle for attribution; `username` is the preferred field, `name`
    // an older fallback. Fall back to the user ID so attribution is never empty.
    let slack_username = payload
        .pointer("/user/username")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/user/name").and_then(Value::as_str))
        .unwrap_or(slack_user_id);

    let channel_id = payload
        .pointer("/channel/id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing channel ID in payload"))?;

    // Channel name (without `#`) for the ops-channel trust check. Absent in
    // some payload shapes; an empty name simply fails the channel match.
    let channel_name = payload
        .pointer("/channel/name")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let message_ts = payload
        .pointer("/message/ts")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing message timestamp in payload"))?;

    Ok(SlackAction {
        action_id: action_id.to_owned(),
        slack_user_id: slack_user_id.to_owned(),
        slack_username: slack_username.to_owned(),
        channel_id: channel_id.to_owned(),
        channel_name: channel_name.to_owned(),
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

/// Authorize an ops action by trusting the configured ops channel.
///
/// Reads `SLACK_OPS_USERS_CHANNEL` — the channel the approve/reject buttons are
/// posted to — and honors the action only when it originates there. Fails
/// closed (rejects) when the channel is unset or does not match.
fn verify_ops_channel(action: &SlackAction) -> AppResult<()> {
    let configured = env::var("SLACK_OPS_USERS_CHANNEL")
        .map_err(|_| AppError::internal("SLACK_OPS_USERS_CHANNEL not configured"))?;

    if channel_matches(&action.channel_name, &action.channel_id, &configured) {
        return Ok(());
    }

    warn!(
        channel_id = %action.channel_id,
        channel_name = %action.channel_name,
        "Slack ops action rejected: not from the ops channel"
    );
    Err(AppError::auth_invalid(
        "Approvals are only accepted from the ops channel",
    ))
}

/// Does an action's channel match the configured ops channel?
///
/// Compares the configured channel (a name like `#dravr-dev-users` or a raw
/// channel ID) against the action's channel name and ID. The leading `#` is
/// optional. An empty configured value never matches, keeping the check
/// fail-closed.
#[must_use]
pub fn channel_matches(channel_name: &str, channel_id: &str, configured: &str) -> bool {
    let expected = configured.trim().trim_start_matches('#');
    !expected.is_empty() && (channel_name == expected || channel_id == expected)
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
        email = %updated_user.email,
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
        email = %updated_user.email,
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
