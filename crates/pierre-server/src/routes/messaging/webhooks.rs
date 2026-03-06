// ABOUTME: Webhook ingress handler for multi-channel messaging gateway
// ABOUTME: Resolves tenant by matching webhook signature against DB channel configs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{Duration, Utc};
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, IncomingMessage, MessageContent, OutgoingMessage,
    LINK_CODE_TTL_MINUTES,
};
use pierre_core::models::TenantId;
use pierre_database::plugins::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, InsertMessageParams,
    MessagingRepository,
};
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::linking::generate_link_code;
use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::services::chat_orchestration;

/// Outcome of persisting a single inbound message
enum PersistOutcome {
    /// Message was stored in DB and an LLM dispatch is pending
    StoredWithDispatch(PendingDispatch),
    /// Message was stored in DB but no LLM dispatch (non-text content)
    StoredNoDispatch,
    /// Message was handled but not stored (linking command or unlinked user prompt)
    HandledNotStored,
}

/// Result of tenant-aware webhook verification
struct WebhookVerification {
    /// Resolved tenant from the matching channel config
    tenant_id: TenantId,
    /// Channel type resolved from URL path
    channel_type: ChannelType,
    /// Adapter constructed from the matching config
    adapter: Arc<dyn MessagingChannel>,
    /// Parsed inbound messages
    messages: Vec<IncomingMessage>,
}

/// Resolved messaging session linking a channel user to a Pierre conversation
struct ResolvedSession {
    /// Messaging session identifier
    session_id: String,
    /// Pierre conversation identifier
    conversation: String,
    /// Pierre user identifier resolved from the channel link
    user_id: String,
}

/// Result of checking an inbound message for a channel linking command
enum LinkingAction {
    /// Message contains a linking command — handle it and do not dispatch to LLM
    LinkCode(String),
    /// Normal message — proceed with standard routing
    Normal,
}

/// Data needed to dispatch a message through the LLM pipeline after HTTP 200
struct PendingDispatch {
    /// Server resources for LLM access
    resources: Arc<ServerResources>,
    /// Channel adapter for outbound send
    adapter: Arc<dyn MessagingChannel>,
    /// Resolved session info
    session: ResolvedSession,
    /// Tenant that owns this channel config
    tenant_id: TenantId,
    /// Channel type enum
    channel_type: ChannelType,
    /// Channel name string (e.g., "slack")
    channel: String,
    /// Original sender to reply to
    sender_id: String,
    /// Text content to dispatch
    text_content: String,
}

/// Query parameters for Meta webhook verification (GET request)
///
/// Meta platforms (Messenger, `WhatsApp`) send a GET request with these params
/// when you configure the webhook URL in the App Dashboard. The server must
/// validate `hub.verify_token` and echo back `hub.challenge` as plain text.
#[derive(Debug, Deserialize)]
pub struct MetaVerifyQuery {
    /// Must be `"subscribe"`
    #[serde(rename = "hub.mode")]
    pub mode: String,
    /// Random string that must be echoed back as the response body
    #[serde(rename = "hub.challenge")]
    pub challenge: String,
    /// Token configured in the Meta App Dashboard — must match our stored verify token
    #[serde(rename = "hub.verify_token")]
    pub verify_token: String,
}

/// Handle Meta webhook verification (GET request)
///
/// Meta sends `GET /webhook?hub.mode=subscribe&hub.verify_token=TOKEN&hub.challenge=CHALLENGE`
/// when configuring the webhook URL. We validate the verify token against the channel's
/// `webhook_secret` and echo `hub.challenge` back as plain text to confirm ownership.
///
/// Supports both Messenger and `WhatsApp` channels.
pub async fn verify_webhook(
    State(resources): State<Arc<ServerResources>>,
    Path(channel): Path<String>,
    Query(query): Query<MetaVerifyQuery>,
) -> Result<impl IntoResponse, AppError> {
    if query.mode != "subscribe" {
        return Err(AppError::invalid_input(format!(
            "Expected hub.mode=subscribe, got: {}",
            query.mode
        )));
    }

    let db: &dyn MessagingRepository = &*resources.database;
    let configs = db.get_configs_by_channel_type(&channel).await?;

    if configs.is_empty() {
        return Err(AppError::auth_invalid(format!(
            "No active channel configuration for {channel}"
        )));
    }

    // Check if any config's webhook_secret matches the verify token
    let token_matches = configs.iter().any(|config| {
        config
            .get("webhook_secret")
            .and_then(|v| v.as_str())
            .is_some_and(|secret| secret == query.verify_token)
    });

    if !token_matches {
        warn!(
            channel = %channel,
            "Meta webhook verification failed: verify_token mismatch"
        );
        return Err(AppError::auth_invalid(
            "Webhook verify token does not match",
        ));
    }

    info!(
        channel = %channel,
        "Meta webhook verification successful"
    );

    // Echo challenge as plain text (Meta requires exactly this)
    Ok((StatusCode::OK, query.challenge))
}

/// Handle an inbound webhook from a messaging channel
///
/// Flow:
/// 1. Verify signature and resolve tenant (synchronous, fast)
/// 2. Persist inbound messages (synchronous, fast)
/// 3. Return HTTP 200 immediately
/// 4. Spawn background tasks for LLM dispatch + outbound response
pub async fn handle_webhook(
    State(resources): State<Arc<ServerResources>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, AppError> {
    let verification = parse_and_verify(&resources, &channel, &headers, &body).await?;

    info!(
        channel = %channel,
        tenant_id = %verification.tenant_id,
        message_count = verification.messages.len(),
        "Processed inbound webhook"
    );

    // Persist messages synchronously and collect pending dispatches for background processing
    let (stored_count, pending_dispatches) = persist_inbound(
        &resources,
        &channel,
        verification.tenant_id,
        verification.channel_type,
        &verification.adapter,
        &verification.messages,
    )
    .await;

    let message_count = verification.messages.len();

    // Handle platform-specific handshake responses (Slack url_verification, Discord PING)
    if verification.messages.is_empty() {
        if let Some(handshake) = detect_handshake_response(&channel, &body) {
            return Ok((StatusCode::OK, Json(handshake)));
        }
    }

    // Spawn LLM dispatch as background tasks (non-blocking, webhook returns immediately)
    for dispatch in pending_dispatches {
        tokio::spawn(async move {
            dispatch_and_respond(dispatch).await;
        });
    }

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "messages_received": message_count,
            "messages_stored": stored_count
        })),
    ))
}

/// Detect platform-specific handshake payloads that require a non-generic response
///
/// Slack `url_verification`: must echo the `challenge` value back.
/// Discord PING (interaction type 1): must respond with `{"type": 1}` (PONG).
fn detect_handshake_response(channel: &str, body: &Bytes) -> Option<Value> {
    let payload: Value = serde_json::from_slice(body).ok()?;

    match channel {
        "slack" => {
            if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
                let challenge = payload.get("challenge")?.as_str()?;
                info!(
                    channel = "slack",
                    "Responding to Slack url_verification challenge"
                );
                return Some(json!({ "challenge": challenge }));
            }
            None
        }
        "discord" => {
            if payload.get("type").and_then(Value::as_u64) == Some(1) {
                info!(channel = "discord", "Responding to Discord PING with PONG");
                return Some(json!({ "type": 1 }));
            }
            None
        }
        _ => None,
    }
}

/// Resolve tenant by matching webhook signature against DB channel configs
///
/// Queries all active configs for the channel type, constructs a temporary adapter
/// for each, and tries signature verification. The first config whose signature
/// passes identifies the tenant.
async fn parse_and_verify(
    resources: &ServerResources,
    channel: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<WebhookVerification, AppError> {
    let channel_type = ChannelType::from_str(channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = &*resources.database;
    let configs = db.get_configs_by_channel_type(channel).await?;

    if configs.is_empty() {
        return Err(AppError::auth_invalid(format!(
            "No active channel configuration for {channel}"
        )));
    }

    // Try each config's signing secret until one verifies
    for config in &configs {
        let adapter = match create_adapter_from_config(channel_type, config) {
            Ok(a) => a,
            Err(e) => {
                debug!(
                    channel = %channel_type,
                    error = %e,
                    "Skipping config with missing credentials"
                );
                continue;
            }
        };

        if adapter.verify_signature(headers, body).is_ok() {
            let tenant_id_str = config
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AppError::internal("Channel config missing tenant_id"))?;

            let tenant_id = TenantId::from_str(tenant_id_str).map_err(|_| {
                AppError::internal(format!(
                    "Channel config has invalid tenant_id: {tenant_id_str}"
                ))
            })?;

            let messages = adapter.receive(headers, body).await.map_err(|e| {
                warn!(channel = %channel_type, error = %e, "Failed to parse inbound webhook");
                AppError::invalid_input(format!("Invalid webhook payload: {e}"))
            })?;

            return Ok(WebhookVerification {
                tenant_id,
                channel_type,
                adapter,
                messages,
            });
        }
    }

    warn!(
        channel = %channel_type,
        config_count = configs.len(),
        "No matching channel configuration for webhook signature"
    );
    Err(AppError::auth_invalid("No matching channel configuration"))
}

/// Detect if an inbound message contains a channel linking command
///
/// `Telegram`: `/start {code}` — bot deep link with verification code
/// `WhatsApp`: `LINK {code}` — text message with verification code
fn detect_linking_code(channel_type: ChannelType, content: &MessageContent) -> LinkingAction {
    let text = match content {
        MessageContent::Text { body } => body.as_str(),
        _ => return LinkingAction::Normal,
    };

    match channel_type {
        ChannelType::Telegram => {
            if let Some(code) = text.strip_prefix("/start ") {
                let code = code.trim();
                if !code.is_empty() {
                    return LinkingAction::LinkCode(code.to_owned());
                }
            }
        }
        ChannelType::WhatsApp => {
            if let Some(code) = text.strip_prefix("LINK ") {
                let code = code.trim();
                if !code.is_empty() {
                    return LinkingAction::LinkCode(code.to_owned());
                }
            }
        }
        _ => {}
    }

    LinkingAction::Normal
}

/// Consume a link code and create the permanent channel link, returning the user ID
async fn execute_link_code(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> Result<String, String> {
    let link_state = db
        .consume_link_state(code, tenant_id)
        .await
        .map_err(|e| format!("Link code is invalid or expired: {e}"))?;

    let user_id = link_state["user_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let link_id = Uuid::new_v4().to_string();

    let link_params = CreateChannelLinkParams {
        id: &link_id,
        tenant_id,
        user_id: &user_id,
        channel_type: channel,
        channel_user_id: sender_id,
        display_name: None,
    };

    db.create_channel_link(&link_params)
        .await
        .map_err(|e| format!("Failed to link your account: {e}"))?;

    Ok(user_id)
}

/// Consume a link code and create the permanent channel link
///
/// Returns a user-facing message describing the result.
async fn consume_and_link(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> String {
    match execute_link_code(db, tenant_id, channel, sender_id, code).await {
        Ok(user_id) => {
            info!(channel = %channel, user_id = %user_id, channel_user_id = %sender_id, "Channel linked via deep link");
            "Your account has been linked successfully! You can now chat with Pierre \
             through this channel."
                .to_owned()
        }
        Err(msg) => {
            warn!(error = %msg, "Channel linking failed");
            msg
        }
    }
}

/// Handle a channel linking command: consume the code and create the link
///
/// Returns an outgoing confirmation or error message to send back to the user.
async fn handle_linking_command(
    resources: &ServerResources,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> OutgoingMessage {
    let db: &dyn MessagingRepository = &*resources.database;
    let channel_type = ChannelType::from_str(channel).unwrap_or(ChannelType::Telegram);
    let response_text = consume_and_link(db, tenant_id, channel, sender_id, code).await;

    OutgoingMessage {
        channel_type,
        recipient_id: sender_id.to_owned(),
        content: MessageContent::Text {
            body: response_text,
        },
        correlation_id: Uuid::new_v4(),
    }
}

/// Resolve a messaging session for a linked channel user
///
/// Looks up the channel link to find the Pierre user, then looks up or creates
/// a session. Returns `None` if the sender has no channel link (unlinked user).
async fn resolve_linked_session(
    resources: &ServerResources,
    tenant_id: TenantId,
    channel_type: &str,
    sender_id: &str,
    channel_conversation_id: Option<&str>,
) -> Result<Option<ResolvedSession>, AppError> {
    let db: &dyn MessagingRepository = &*resources.database;

    // Check for existing session first (fast path)
    if let Some(session) = db
        .get_session_by_channel_identity(tenant_id, channel_type, sender_id)
        .await?
    {
        let session_id = session["id"]
            .as_str()
            .ok_or_else(|| AppError::internal("Session missing id field"))?
            .to_owned();
        let conversation = session["pierre_conversation_id"]
            .as_str()
            .ok_or_else(|| AppError::internal("Session missing pierre_conversation_id field"))?
            .to_owned();
        let user_id = session["user_id"]
            .as_str()
            .ok_or_else(|| AppError::internal("Session missing user_id field"))?
            .to_owned();

        if let Err(e) = db.touch_session(&session_id).await {
            warn!(error = %e, session_id = %session_id, "Failed to touch session");
        }

        return Ok(Some(ResolvedSession {
            session_id,
            conversation,
            user_id,
        }));
    }

    // No existing session — check if user has linked this channel
    let channel_link = db
        .get_channel_link(tenant_id, channel_type, sender_id)
        .await?;

    let Some(link) = channel_link else {
        return Ok(None); // Unlinked user
    };

    let user_id = link["user_id"]
        .as_str()
        .ok_or_else(|| AppError::internal("Channel link missing user_id"))?
        .to_owned();

    // Create a new conversation and session for this linked user
    let title = format!("Messaging: {channel_type}");
    let conversation = chat_orchestration::create_conversation(
        resources.database.as_ref(),
        &user_id,
        tenant_id,
        &title,
        None,
        None,
    )
    .await?;

    let conversation_id = conversation.conversation.id.clone();
    let session_id = Uuid::new_v4().to_string();

    let session_params = CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id,
        channel_type,
        channel_user_id: sender_id,
        channel_conversation_id,
        pierre_conversation_id: Some(&conversation_id),
    };
    db.create_session(&session_params).await?;

    info!(
        session_id = %session_id,
        conversation_id = %conversation_id,
        channel_type = %channel_type,
        sender_id = %sender_id,
        user_id = %user_id,
        "Created messaging session for linked user"
    );

    Ok(Some(ResolvedSession {
        session_id,
        conversation: conversation_id,
        user_id,
    }))
}

/// Create a link state and return a prompt message with a clickable login URL
///
/// Generates a 32-character cryptographic code with a 10-minute TTL, stores it
/// in the database, and constructs a message with a clickable URL for the user.
async fn create_link_and_prompt(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel_type: ChannelType,
    sender_id: &str,
    sender_name: Option<&str>,
) -> OutgoingMessage {
    let code = generate_link_code();
    let expires_at = Utc::now() + Duration::minutes(LINK_CODE_TTL_MINUTES);
    let id = Uuid::new_v4().to_string();
    let channel_str = channel_type.to_string();

    let params = CreateLinkStateParams {
        id: &id,
        tenant_id,
        user_id: None,
        channel_type: &channel_str,
        code: &code,
        method: "channel_initiated",
        channel_user_id: Some(sender_id),
        sender_name,
        expires_at: &expires_at.to_rfc3339(),
    };

    if let Err(e) = db.create_link_state(&params).await {
        warn!(error = %e, "Failed to create link state for unlinked user");
        // Fall back to a generic prompt if DB fails
        return OutgoingMessage {
            channel_type,
            recipient_id: sender_id.to_owned(),
            content: MessageContent::Text {
                body: "To chat with Pierre, please link your account first. \
                       Visit the Pierre web app to connect this channel."
                    .to_owned(),
            },
            correlation_id: Uuid::new_v4(),
        };
    }

    let base_url = env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8081".to_owned());
    let link_url = format!("{base_url}/messaging/link/{code}");

    let body = format!(
        "Hi! To chat with Pierre, link your account first:\n\
         {link_url}\n\n\
         This link expires in 10 minutes."
    );

    OutgoingMessage {
        channel_type,
        recipient_id: sender_id.to_owned(),
        content: MessageContent::Text { body },
        correlation_id: Uuid::new_v4(),
    }
}

/// Persist inbound messages and collect pending dispatches for background processing
///
/// Returns (`stored_count`, `pending_dispatches`) — the dispatches are processed
/// asynchronously after the webhook returns HTTP 200.
async fn persist_inbound(
    resources: &Arc<ServerResources>,
    channel: &str,
    tenant_id: TenantId,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    messages: &[IncomingMessage],
) -> (usize, Vec<PendingDispatch>) {
    let mut stored_count: usize = 0;
    let mut pending_dispatches = Vec::new();

    for message in messages {
        match persist_single_message(
            resources,
            channel,
            tenant_id,
            channel_type,
            adapter,
            message,
        )
        .await
        {
            Ok(PersistOutcome::StoredWithDispatch(dispatch)) => {
                stored_count += 1;
                pending_dispatches.push(dispatch);
            }
            Ok(PersistOutcome::StoredNoDispatch) => {
                stored_count += 1;
            }
            Ok(PersistOutcome::HandledNotStored) | Err(()) => {}
        }
    }

    (stored_count, pending_dispatches)
}

/// Send an outgoing message to a channel user, loading config and spawning delivery
async fn send_channel_response(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    message: OutgoingMessage,
) {
    let config = load_channel_config(db, tenant_id, channel).await;
    if let Some(cfg) = config {
        let adapter_clone = Arc::clone(adapter);
        tokio::spawn(async move {
            if let Err(e) = adapter_clone.send(&message, &cfg).await {
                warn!(error = %e, "Failed to send channel response");
            }
        });
    }
}

/// Persist a single inbound message and optionally prepare an LLM dispatch
///
/// Handles three cases:
/// 1. Linking command → consume code, create link, send confirmation (not stored)
/// 2. Linked user → resolve session, store message, dispatch to LLM pipeline
/// 3. Unlinked user → send prompt to authenticate (not stored)
///
/// Returns `Ok(StoredWithDispatch)` for linked-user text messages,
/// `Ok(StoredNoDispatch)` for stored non-text messages,
/// `Ok(HandledNotStored)` for linking commands or unlinked users,
/// or `Err(())` if persistence failed.
async fn persist_single_message(
    resources: &Arc<ServerResources>,
    channel: &str,
    tenant_id: TenantId,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
) -> Result<PersistOutcome, ()> {
    let db: &dyn MessagingRepository = &*resources.database;

    // Check for linking commands (`Telegram` /start, `WhatsApp` LINK)
    if let LinkingAction::LinkCode(code) = detect_linking_code(channel_type, &message.content) {
        info!(channel = %channel, sender_id = %message.sender_id, "Processing channel linking command");
        let response =
            handle_linking_command(resources, tenant_id, channel, &message.sender_id, &code).await;
        send_channel_response(db, tenant_id, channel, adapter, response).await;
        return Ok(PersistOutcome::HandledNotStored);
    }

    // Resolve session via channel link (returns None for unlinked users)
    let session = resolve_or_prompt(
        resources,
        db,
        tenant_id,
        channel,
        channel_type,
        adapter,
        message,
    )
    .await?;

    let Some(session) = session else {
        return Ok(PersistOutcome::HandledNotStored);
    };

    let stored = store_inbound_message(db, tenant_id, &session, channel, message).await?;
    if !stored {
        return Err(());
    }

    // Extract text content for LLM dispatch
    content_body_text(&message.content).map_or_else(
        || {
            info!("Skipping non-text message for LLM dispatch");
            Ok(PersistOutcome::StoredNoDispatch)
        },
        |text_content| {
            Ok(PersistOutcome::StoredWithDispatch(PendingDispatch {
                resources: Arc::clone(resources),
                adapter: Arc::clone(adapter),
                session,
                tenant_id,
                channel_type,
                channel: channel.to_owned(),
                sender_id: message.sender_id.clone(),
                text_content,
            }))
        },
    )
}

/// Resolve a linked session or send an authentication prompt for unlinked users
///
/// Returns `Ok(Some(session))` for linked users, `Ok(None)` for unlinked users
/// (after sending them a prompt), or `Err(())` on session resolution failure.
async fn resolve_or_prompt(
    resources: &ServerResources,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
) -> Result<Option<ResolvedSession>, ()> {
    match resolve_linked_session(
        resources,
        tenant_id,
        channel,
        &message.sender_id,
        message.conversation_id.as_deref(),
    )
    .await
    {
        Ok(Some(session)) => Ok(Some(session)),
        Ok(None) => {
            info!(channel = %channel, sender_id = %message.sender_id, "Unlinked user, sending link URL");
            let prompt = create_link_and_prompt(
                db,
                tenant_id,
                channel_type,
                &message.sender_id,
                message.sender_name.as_deref(),
            )
            .await;
            send_channel_response(db, tenant_id, channel, adapter, prompt).await;
            Ok(None)
        }
        Err(e) => {
            warn!(error = %e, sender_id = %message.sender_id, "Failed to resolve messaging session, skipping message");
            Err(())
        }
    }
}

/// Store a single inbound message in the database
///
/// Returns `Ok(true)` if stored, `Err(())` on duplicate or DB error (already logged).
async fn store_inbound_message(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    session: &ResolvedSession,
    channel: &str,
    message: &IncomingMessage,
) -> Result<bool, ()> {
    let msg_id = Uuid::new_v4().to_string();
    let content_type = content_type_label(&message.content);
    let content_body = content_body_text(&message.content);
    let raw_payload = serde_json::to_string(&message.raw_payload).ok();
    let correlation_str = message.correlation_id.to_string();

    let params = InsertMessageParams {
        id: &msg_id,
        tenant_id,
        session_id: &session.session_id,
        direction: "inbound",
        channel_type: channel,
        channel_message_id: &message.channel_message_id,
        sender_id: &message.sender_id,
        content_type,
        content_body: content_body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: raw_payload.as_deref(),
    };

    match db.insert_message(&params).await {
        Ok(true) => Ok(true),
        Ok(false) => {
            info!(
                channel_message_id = %message.channel_message_id,
                "Duplicate message skipped (idempotent)"
            );
            Err(())
        }
        Err(e) => {
            warn!(
                error = %e,
                channel_message_id = %message.channel_message_id,
                "Failed to persist inbound message"
            );
            Err(())
        }
    }
}

/// Dispatch a message through the LLM pipeline and send the response back via the channel
///
/// Runs as a background task after the webhook has returned HTTP 200.
async fn dispatch_and_respond(dispatch: PendingDispatch) {
    let response_text = match chat_orchestration::dispatch_and_get_response(
        &dispatch.resources,
        &dispatch.session.conversation,
        &dispatch.session.user_id,
        dispatch.tenant_id,
        &dispatch.text_content,
    )
    .await
    {
        Ok(text) => text,
        Err(e) => {
            warn!(
                error = %e,
                conversation_id = %dispatch.session.conversation,
                "LLM dispatch failed for messaging"
            );
            return;
        }
    };

    let outgoing = OutgoingMessage {
        channel_type: dispatch.channel_type,
        recipient_id: dispatch.sender_id.clone(),
        content: MessageContent::Text {
            body: response_text,
        },
        correlation_id: Uuid::new_v4(),
    };

    send_outbound_response(&dispatch, &outgoing).await;
}

/// Load channel config, send outbound message, and persist the result
async fn send_outbound_response(dispatch: &PendingDispatch, outgoing: &OutgoingMessage) {
    let db: &dyn MessagingRepository = &*dispatch.resources.database;

    let Some(channel_config) = load_channel_config(db, dispatch.tenant_id, &dispatch.channel).await
    else {
        return;
    };

    match dispatch.adapter.send(outgoing, &channel_config).await {
        Ok(receipt) => {
            info!(
                message_id = %receipt.message_id,
                channel = %dispatch.channel,
                "Outbound message sent successfully"
            );
            persist_outbound_message(db, dispatch, &receipt.message_id, outgoing).await;
        }
        Err(e) => {
            warn!(
                error = %e,
                channel = %dispatch.channel,
                "Failed to send outbound message, enqueuing for retry"
            );
            enqueue_failed_outbound(db, dispatch, outgoing).await;
        }
    }
}

/// Load and deserialize a channel config for outbound sending
async fn load_channel_config(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
) -> Option<ChannelConfig> {
    let config = match db.get_channel_config(tenant_id, channel).await {
        Ok(Some(cfg)) => cfg,
        Ok(None) => {
            warn!(channel = %channel, "No channel config found for outbound send");
            return None;
        }
        Err(e) => {
            warn!(error = %e, "Failed to load channel config for outbound");
            return None;
        }
    };

    match serde_json::from_value::<ChannelConfig>(config) {
        Ok(c) => Some(c),
        Err(e) => {
            warn!(error = %e, "Failed to deserialize channel config");
            None
        }
    }
}

/// Persist an outbound message after successful delivery
async fn persist_outbound_message(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    channel_message_id: &str,
    outgoing: &OutgoingMessage,
) {
    let out_msg_id = Uuid::new_v4().to_string();
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.correlation_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        tenant_id: dispatch.tenant_id,
        session_id: &dispatch.session.session_id,
        direction: "outbound",
        channel_type: &dispatch.channel,
        channel_message_id,
        sender_id: "pierre",
        content_type: "text",
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: None,
    };
    if let Err(e) = db.insert_message(&out_params).await {
        warn!(error = %e, "Failed to persist outbound message");
    }
}

/// Enqueue a failed outbound message for retry delivery
///
/// Renders the outgoing message to the channel's native payload format, persists
/// the outbound message record, then enqueues it in the retry queue.
async fn enqueue_failed_outbound(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    outgoing: &OutgoingMessage,
) {
    if let Err(e) = try_enqueue_for_retry(db, dispatch, outgoing).await {
        warn!(error = %e, channel = %dispatch.channel, "Failed to enqueue outbound for retry");
    }
}

/// Render, persist, and enqueue an outbound message for retry
///
/// Returns an error if any step fails (rendering, persistence, or enqueue).
async fn try_enqueue_for_retry(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    outgoing: &OutgoingMessage,
) -> Result<(), AppError> {
    let payload = dispatch
        .adapter
        .render(outgoing)
        .map_err(|e| AppError::internal(format!("Failed to render for retry: {e}")))?;

    let payload_str = payload.to_string();

    // Persist the outbound message record first (FK requirement for queue entry)
    let out_msg_id = Uuid::new_v4().to_string();
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.correlation_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        tenant_id: dispatch.tenant_id,
        session_id: &dispatch.session.session_id,
        direction: "outbound",
        channel_type: &dispatch.channel,
        channel_message_id: "",
        sender_id: "pierre",
        content_type: "text",
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: Some(&payload_str),
    };
    db.insert_message(&out_params).await?;

    let queue_id = Uuid::new_v4().to_string();
    db.enqueue_outbound(
        &queue_id,
        &out_msg_id,
        dispatch.tenant_id,
        &dispatch.channel,
        &payload_str,
    )
    .await?;

    info!(
        queue_id = %queue_id,
        channel = %dispatch.channel,
        "Outbound message enqueued for retry"
    );
    Ok(())
}

/// Extract a content type label from the message content variant
fn content_type_label(content: &MessageContent) -> &'static str {
    match content {
        MessageContent::Text { .. } => "text",
        MessageContent::Media { .. } => "media",
        MessageContent::Location { .. } => "location",
        MessageContent::Card { .. } => "card",
    }
}

/// Extract the text body from the message content (if applicable)
fn content_body_text(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::Text { body } | MessageContent::Card { body, .. } => Some(body.clone()),
        MessageContent::Media { caption, .. } => caption.clone(),
        MessageContent::Location { .. } => None,
    }
}
