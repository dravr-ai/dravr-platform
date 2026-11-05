// ABOUTME: Slack interactive actions handler for ops notifications, command postbacks and athlete taps
// ABOUTME: Verifies and parses the payload through canot's Slack transport, routes each tap to its handler
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use dravr_canot::channels::slack::transport::{
    parse_slack_body, verify_slack_signature as verify_slack_v0,
};
use dravr_tronc::notifications::{SlackClient, SlackConfig};
use pierre_chat_pipeline::suggestions::is_postback;
use pierre_contremaitre::notify_routing::NOTIFY_ROUTING_PROVIDER;
use pierre_core::models::TenantId;
use pierre_services::channel_adapters::ChannelAdapterFactory;
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

mod coach_access_tap;
mod feedback_reply_tap;
mod ops_card;
mod ops_decisions;
mod quota_unlock_tap;
mod sign_up_tap;

use super::webhooks;
use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress::parse_choice;
use coach_access_tap::answer_coach_access_tap;
use feedback_reply_tap::{answer_feedback_tap, answer_reply_submission};
pub use ops_decisions::{channel_matches, OpsDecisionActions};
use ops_decisions::{
    parse_action_id, verify_ops_channel, ActionType, CoachAccessDecision, OpsAction,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use pierre_services::locale::resolve_channel_locale;
use pierre_services::provider_notice::under_ai_consent;
use quota_unlock_tap::answer_quota_unlock_tap;
use sign_up_tap::answer_sign_up_tap;

/// Channel type key Slack channel links are stored under in
/// `messaging_channel_links`, matching the webhook path segment the ingress
/// path uses.
const SLACK_CHANNEL: &str = "slack";

/// Where a tapped button's `action_id` is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionRoute {
    /// A `/command` postback from a messaging card, run through the command
    /// system for the clicker.
    Command,
    /// A button standing for the athlete's own words — a welcome starter's
    /// suggestion postback, an agent's number on the onboarding proposal —
    /// ingested as the message it spells, into the athlete's turn.
    AthleteTurn,
    /// Anything else: an ops room's decision (approve/reject a sign-up,
    /// grant/decline coach access, raise a capped athlete to Enterprise),
    /// which refuses what it cannot parse.
    Ops,
}

impl ActionRoute {
    /// The route `action_id` takes.
    ///
    /// Only a value the athlete's turn knows how to read is routed there: a
    /// suggestion postback the turn resolves back into its words, or the bare
    /// number a pick from the proposal is. Everything else stays on the ops
    /// path, which refuses it outside the ops channel, so an unknown button
    /// never becomes something the athlete said.
    fn of(action_id: &str) -> Self {
        if action_id.starts_with('/') {
            Self::Command
        } else if is_postback(action_id) || parse_choice(action_id).is_some() {
            Self::AthleteTurn
        } else {
            Self::Ops
        }
    }
}

/// Handle Slack interactive action payloads (button clicks)
///
/// Routes three kinds of actions:
/// - **Command postbacks**: `action_id` starts with `/` (e.g. `/agent add @handle`).
///   These come from messaging card buttons and are routed through the command system.
/// - **The athlete's own words**: a welcome starter's suggestion postback or
///   a numbered pick from the agent proposal (carnet#829). Ingested through
///   the messaging webhook path as the message the button spells, so it
///   reaches the athlete's turn under the tenant whose Slack config verifies
///   it — the same session, gates and turn a typed message gets.
/// - **Ops actions**: the buttons [`OpsDecisionActions`] posts — approve or
///   reject under `user.pending_approval`, grant or decline under
///   `coach_access.requested`, raise to Enterprise under
///   `quota.limit_reached`, reply to or read the whole of an athlete's
///   feedback under `feedback.recorded` — each honoured only in the room its
///   event's routing rule names.
///
/// A `view_submission` is the feedback reply modal being sent; it re-checks
/// the room its card was tapped in before the reply lands.
///
/// Security chain:
/// 1. HMAC-SHA256 signature verification (proves request comes from Slack)
/// 2. Timestamp replay protection (rejects requests older than 5 minutes)
/// 3. Per-route authorization — the event's room for approve/reject, the
///    room plus an active super-admin behind the Slack identity's channel
///    link for grant/decline and for raising a capped athlete's tier, the
///    channel link and user-status gate for command postbacks and the
///    athlete's taps. The signature proves the
///    click came from Slack, never that the clicking Slack identity may act
///    as a given Dravr user.
pub(crate) async fn handle_slack_action(
    resources: &Arc<ServerContext>,
    adapters: &dyn ChannelAdapterFactory,
    headers: &HeaderMap,
    body: &Bytes,
) -> AppResult<(StatusCode, Json<Value>)> {
    let payload = parse_slack_payload(body)?;
    if payload.get("type").and_then(Value::as_str) == Some(VIEW_SUBMISSION) {
        return answer_reply_submission(resources, &payload).await;
    }
    let action = extract_action(&payload)?;

    info!(
        action_id = %action.action_id,
        slack_user = %action.slack_user_id,
        "Processing Slack interactive action"
    );

    match ActionRoute::of(&action.action_id) {
        ActionRoute::Command => {
            return handle_command_postback(resources, &action, &action.action_id).await;
        }
        ActionRoute::AthleteTurn => {
            return webhooks::ingest(resources, adapters, SLACK_CHANNEL, headers, body).await;
        }
        ActionRoute::Ops => {}
    }

    // --- Ops actions below (approve/reject users, grant/decline coach access,
    // raise a capped athlete to Enterprise, answer athlete feedback) ---

    let ops_action = parse_action_id(&action.action_id)?;

    // Authorization, first: trust the room. Each decision's buttons are only
    // ever posted to the room its event's routing rule names — a private
    // channel whose membership IS the set of operators. The HMAC signature
    // (verified by the HTTP handler) proves the click is genuinely from
    // Slack; we additionally require the action to originate from that room
    // so the buttons are honored nowhere else.
    verify_ops_channel(
        &action,
        ops_action.event(),
        NOTIFY_ROUTING_PROVIDER.as_ref(),
    )?;

    match ops_action {
        OpsAction::SignUp(action_type, user_uuid) => {
            answer_sign_up_tap(resources, &action, action_type, user_uuid).await
        }
        OpsAction::CoachAccess(decision, request_id) => {
            answer_coach_access_tap(resources, &action, decision, request_id).await
        }
        OpsAction::RaiseToEnterprise(user_uuid) => {
            answer_quota_unlock_tap(resources, &action, user_uuid).await
        }
        OpsAction::Feedback(tap, feedback) => {
            answer_feedback_tap(resources, &action, tap, feedback).await
        }
    }
}

/// Redraw the card a tap came from as `blocks`.
///
/// Fire-and-forget through tronc's `SlackClient`, after the decision: the
/// notify layer posts no buttons without `SLACK_BOT_TOKEN`, and a decision
/// already taken stands whether or not its card can be redrawn.
fn update_card(resources: &ServerContext, action: &SlackAction, blocks: &Value) {
    update_card_at(resources, &action.channel_id, &action.message_ts, blocks);
}

/// Redraw the card at `message_ts` in `channel_id` as `blocks`: what
/// [`update_card`] does for a tap, for a modal's submission, which names its
/// card rather than carrying it.
fn update_card_at(resources: &ServerContext, channel_id: &str, message_ts: &str, blocks: &Value) {
    let Some(bot_token) = configured_bot_token(resources) else {
        warn!(
            channel_id = %channel_id,
            "SLACK_BOT_TOKEN not configured; the Slack card was not updated"
        );
        return;
    };
    build_slack_client(bot_token.expose_secret(), resources)
        .update_message(channel_id, message_ts, blocks);
}

/// Verify the Slack request signature with canot's v0 verifier
///
/// canot owns the Slack v0 scheme (HMAC-SHA256 over `v0:{timestamp}:{body}`,
/// a 300 s replay window, a constant-time compare) for its own webhook
/// transport; the interactive route verifies with the same function.
///
/// canot also owns the blank-secret rule: an empty or whitespace-only
/// `signing_secret` verifies nothing, since anyone can compute a signature
/// under the empty key or a lone newline.
pub(crate) fn verify_slack_signature(
    signing_secret: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<()> {
    verify_slack_v0(signing_secret, headers, body)
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
    let bot_token = configured_bot_token(resources)
        .ok_or_else(|| AppError::internal("SLACK_BOT_TOKEN not configured"))?;

    let response_text = execute_postback_command(
        resources,
        &action.slack_user_id,
        &action.channel_id,
        command_text,
    )
    .await?;

    // Send response to Slack channel
    let slack_client = build_slack_client(bot_token.expose_secret(), resources);
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
    use dravr_canot::commands::CommandMatcher;

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

    let locale = resolve_channel_locale(
        resources.common.repos.messaging.as_ref(),
        resources.common.repos.users.as_ref(),
        user_tenant,
        SLACK_CHANNEL,
        slack_user_id,
        Some(user_id),
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

    // A button is a command typed on the athlete's own channel: its reads are
    // model input served there, as a typed command's are (`dispatch_slash`).
    // The athlete's AI consents govern those reads too (carnet#726).
    let response = ai_scope::serve_over(
        Transport::Messaging,
        ai_scope::for_model(under_ai_consent(
            &resources.common.repos,
            user_tenant.as_uuid(),
            user_id,
            handler.execute(&ctx),
        )),
    )
    .await?;

    info!(
        command = %parsed.name,
        user_id = %user_id,
        slack_user = %slack_user_id,
        "Slack command postback executed"
    );

    Ok(response.text)
}

/// Authorize the Slack identity behind a command postback, or a coach-access
/// decision, through the same gate every inbound messaging turn walks.
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
                "Slack action from an unlinked Slack identity"
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

/// The Slack signing secret this deployment is configured with
///
/// `None` when `SLACK_SIGNING_SECRET` was unset or blank at startup: a
/// variable that is present but empty or whitespace only configures no
/// secret, and the two read the same to every caller.
pub(crate) fn configured_signing_secret(resources: &ServerContext) -> Option<&SecretString> {
    resources
        .common
        .config
        .webhooks
        .slack_signing_secret
        .as_ref()
}

/// The Slack bot token, `None` when `SLACK_BOT_TOKEN` was unset or blank at
/// startup.
fn configured_bot_token(resources: &ServerContext) -> Option<&SecretString> {
    resources.common.config.webhooks.slack_bot_token.as_ref()
}

/// Build a `SlackClient` from a bot token for message updates
fn build_slack_client(bot_token: &str, resources: &ServerContext) -> SlackClient {
    let config = SlackConfig {
        bot_token: bot_token.to_owned(),
        error_channel: String::new(),
        signing_secret: configured_signing_secret(resources)
            .map(|secret| secret.expose_secret().to_owned()),
    };

    SlackClient::new(&config)
}

/// Parsed Slack interactive action
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackAction {
    /// The tapped button's `action_id`: a `/command` postback, an athlete's
    /// tap, or one of the ops actions [`OpsDecisionActions`] posts
    pub action_id: String,
    /// Slack user ID of the clicker
    pub slack_user_id: String,
    /// Slack display handle of the clicker, carried in the interactive
    /// payload — used for attribution without a `users.info` lookup.
    pub slack_username: String,
    /// Channel ID the card lives in
    pub channel_id: String,
    /// Channel name (without leading `#`) the action originated from, used to
    /// enforce the ops-channel trust boundary.
    pub channel_name: String,
    /// Timestamp of the card message, which addresses it for the update
    pub message_ts: String,
    /// The card's blocks as Slack sent them back with the tap, so a refused
    /// tap can redraw the card with its buttons kept; empty when the payload
    /// carries none.
    pub message_blocks: Vec<Value>,
    /// What a modal opens on for three seconds after the tap; empty when the
    /// payload carries none.
    pub trigger_id: String,
}

/// Parse a Slack interactive request body into the action it carries
///
/// Slack sends interactive payloads as `application/x-www-form-urlencoded`
/// with a single `payload` key containing JSON. canot's `parse_slack_body`
/// decodes it with the form rules — `+` is a space, `%2B` a literal plus — so
/// every field arrives as the clicker's client sent it.
///
/// # Errors
///
/// Returns an invalid-input error when the body is not a Slack payload or is
/// not a `block_actions` payload carrying an action, a user, a channel and a
/// message timestamp.
pub fn parse_slack_action(body: &[u8]) -> AppResult<SlackAction> {
    extract_action(&parse_slack_payload(body)?)
}

/// The `type` of the payload a feedback reply modal's submission arrives as.
const VIEW_SUBMISSION: &str = "view_submission";

/// Decode a Slack interactive request body into its JSON payload.
fn parse_slack_payload(body: &[u8]) -> AppResult<Value> {
    parse_slack_body(body)
        .map_err(|e| AppError::invalid_input(format!("Invalid Slack interactive payload: {e}")))
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

    let message_blocks = payload
        .pointer("/message/blocks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let trigger_id = payload
        .get("trigger_id")
        .and_then(Value::as_str)
        .unwrap_or_default();

    Ok(SlackAction {
        action_id: action_id.to_owned(),
        slack_user_id: slack_user_id.to_owned(),
        slack_username: slack_username.to_owned(),
        channel_id: channel_id.to_owned(),
        channel_name: channel_name.to_owned(),
        message_ts: message_ts.to_owned(),
        message_blocks,
        trigger_id: trigger_id.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};
    use chrono::Utc;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    use serde_json::{json, Value};

    use super::{parse_slack_action, verify_slack_signature, ActionRoute};

    #[test]
    fn a_suggestion_or_a_proposal_pick_is_the_athletes_turn() {
        for value in [
            "ex:0:0",
            "ex:2:5",
            "uc:0:connect",
            "uc:2:last_workout",
            "1",
            "3",
        ] {
            assert_eq!(ActionRoute::of(value), ActionRoute::AthleteTurn, "{value}");
        }
    }

    #[test]
    fn a_command_postback_is_a_command_and_the_rest_stay_on_the_ops_path() {
        assert_eq!(ActionRoute::of("/agent add @coach"), ActionRoute::Command);
        for value in [
            "approve_user:6f1c2b9e-0000-4000-8000-000000000001",
            "reject_user:6f1c2b9e-0000-4000-8000-000000000001",
            "grant_coach_access:6f1c2b9e-0000-4000-8000-000000000001",
            "decline_coach_access:6f1c2b9e-0000-4000-8000-000000000001",
            "raise_tier_enterprise:6f1c2b9e-0000-4000-8000-000000000001",
            "0",
            "ex:one:1",
            "uc:one:connect",
            "an unknown button",
        ] {
            assert_eq!(ActionRoute::of(value), ActionRoute::Ops, "{value}");
        }
    }

    const BODY: &[u8] = b"payload=%7B%22type%22%3A%22block_actions%22%7D";

    /// The reason canot's `refuse_blank_secret` gives: seeing it proves canot,
    /// not this wrapper, refused the blank secret.
    const CANOT_BLANK_SECRET_REASON: &str = "no verification secret is configured";

    /// Headers carrying Slack's v0 signature of [`BODY`], made now under `key`.
    fn headers_signed_with(key: &str) -> HeaderMap {
        let timestamp = Utc::now().timestamp().to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
            .unwrap_or_else(|e| panic!("an HMAC takes a key of any length: {e}"));
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(BODY);
        let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("x-slack-request-timestamp", timestamp),
            ("x-slack-signature", signature),
        ] {
            let value = HeaderValue::from_str(&value)
                .unwrap_or_else(|e| panic!("a signature header is ASCII: {e}"));
            headers.insert(name, value);
        }
        headers
    }

    #[test]
    fn a_signature_under_the_configured_secret_verifies() {
        let headers = headers_signed_with("ops_signing_secret");
        assert!(verify_slack_signature("ops_signing_secret", &headers, BODY).is_ok());
    }

    #[test]
    fn a_signature_under_another_key_does_not_verify() {
        let headers = headers_signed_with("some_other_secret");
        assert!(verify_slack_signature("ops_signing_secret", &headers, BODY).is_err());
    }

    #[test]
    fn an_empty_signing_secret_verifies_nothing() {
        // The signature is a correct one under the empty key: anyone can make it.
        let headers = headers_signed_with("");
        let err = verify_slack_signature("", &headers, BODY).unwrap_err();
        assert!(
            err.to_string().contains(CANOT_BLANK_SECRET_REASON),
            "canot must refuse the empty key, got: {err}"
        );
    }

    #[test]
    fn a_whitespace_only_signing_secret_verifies_nothing() {
        // A blank secret version is a lone newline; a signature under it is
        // as forgeable as one under the empty key.
        for blank in ["\n", " ", "\r\n", " \t "] {
            let headers = headers_signed_with(blank);
            let err = verify_slack_signature(blank, &headers, BODY).unwrap_err();
            assert!(
                err.to_string().contains(CANOT_BLANK_SECRET_REASON),
                "canot must refuse a signature under {blank:?}, got: {err}"
            );
        }
    }

    #[test]
    fn a_tap_carries_back_the_card_it_was_made_on() {
        // A card as the notify layer posts it: the notice, then its buttons.
        let card: Vec<Value> = vec![
            json!({ "type": "section", "block_id": "notice",
                    "text": { "type": "mrkdwn", "text": "coach access requested" } }),
            json!({ "type": "actions", "block_id": "notify-actions-coach_access.requested",
                    "elements": [{ "type": "button", "action_id": "grant_coach_access:1" }] }),
        ];
        let payload = json!({
            "type": "block_actions",
            "user": { "id": "U1", "username": "op" },
            "channel": { "id": "C0COACHES", "name": "dravr-dev-users" },
            "message": { "ts": "1.0", "blocks": card },
            "actions": [{ "action_id": "grant_coach_access:1" }],
        });
        let body = format!("payload={}", urlencoding::encode(&payload.to_string()));
        let action = parse_slack_action(body.as_bytes()).unwrap();
        assert_eq!(action.message_blocks, card);

        let without = json!({
            "type": "block_actions",
            "user": { "id": "U1" },
            "channel": { "id": "C0COACHES" },
            "message": { "ts": "1.0" },
            "actions": [{ "action_id": "grant_coach_access:1" }],
        });
        let body = format!("payload={}", urlencoding::encode(&without.to_string()));
        assert_eq!(
            parse_slack_action(body.as_bytes()).unwrap().message_blocks,
            Vec::<Value>::new()
        );
    }
}
