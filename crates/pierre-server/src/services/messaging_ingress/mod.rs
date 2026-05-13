// ABOUTME: Business logic for messaging ingress: OTP flow, channel linking, session resolution,
// ABOUTME: slash command dispatch, message persistence, LLM dispatch, and outbound response handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// AG-UI run wiring + per-channel status-bridge setup for messaging dispatch.
mod agui;
/// LLM dispatch + outbound delivery + retry queue for messaging turns.
mod dispatch;
/// Channel-linking commands (`/start <code>`, `LINK <code>`) + analytics-consent hydration.
mod linking;
/// Messaging-turn locale resolution + content-language detection helpers.
pub mod locale;
/// In-chat OTP linking flow + logout + supporting helpers.
mod otp;
/// Session resolution for linked channel users + unlinked-user link-and-prompt.
mod session;
/// Slash-command dispatch wrapper (channel-link auth/locale + `commands::dispatch::try_dispatch`).
#[cfg(feature = "client-messaging")]
mod slash;

pub use locale::{detect_turn_locale, resolve_messaging_locale};

pub(crate) use dispatch::dispatch_and_respond;
use dispatch::load_channel_config;
use linking::{detect_linking_code, handle_linking_command, LinkingAction};
use otp::{
    apply_conversation_recipient, handle_logout, handle_otp_flow, is_logout_command, start_otp_flow,
};
use session::{create_link_and_prompt, resolve_linked_session, ChannelChatRef};
#[cfg(feature = "client-messaging")]
use slash::{try_handle_slash_command, SlashCommandContext};

use dashmap::DashMap;
use pierre_auth::auth::AuthResult;
use pierre_core::errors::ErrorCode;
use pierre_core::models::messaging::{
    ChannelType, IncomingMessage, MessageContent, OutgoingMessage,
};
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::safety::{scan as scan_for_injection, SanitizationOutcome};
use pierre_database::backends::{InsertMessageParams, MessagingRepository};
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use serde_json::Value;

use crate::contremaitre::messaging_strings::DEFAULT_LOCALE;
use crate::errors::AppError;
use crate::mcp::resources::ServerContext;
use crate::middleware::auth::record_jwt_usage_for_request;
use crate::services::analytics::{analytics, hash_id};
use crate::services::channel_error_reply::ChannelErrorReply;
use crate::services::chat_pipeline::ChannelProfile;
use crate::services::user_status_gate::messaging_key_for_status;

/// Outcome of persisting a single inbound message
pub(crate) enum PersistOutcome {
    /// Message was stored in DB and an LLM dispatch is pending
    StoredWithDispatch(Box<PendingDispatch>),
    /// Message was stored in DB but no LLM dispatch (non-text content)
    StoredNoDispatch,
    /// Message was handled but not stored (linking command or unlinked user prompt)
    HandledNotStored,
}

/// Resolved messaging session linking a channel user to a Pierre conversation
pub(crate) struct ResolvedSession {
    /// Messaging session identifier
    pub(crate) session_id: String,
    /// Pierre conversation identifier
    pub(crate) conversation: String,
    /// Pierre user identifier resolved from the channel link
    pub(crate) user_id: String,
}

/// Extract forum topic thread ID from incoming message metadata
///
/// Telegram groups with Topics enabled include `message_thread_id` in each
/// message. This ID must be included in outbound replies so they route to
/// the correct topic thread instead of the main chat.
fn extract_thread_id(metadata: &Value) -> Option<String> {
    metadata
        .get("message_thread_id")
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

/// Build a [`ChannelProfile`] for the originating messaging channel.
///
/// All four supported messaging channels currently share the same knobs
/// (env-override model policy, five tool-loop iterations, messaging
/// context prompt appended). Channel-specific overrides can be added by
/// branching inside this helper without touching pipeline code.
pub(super) fn build_messaging_profile(dispatch: &PendingDispatch) -> ChannelProfile {
    let suffix = dispatch.resources.messaging_context_prompt();
    match dispatch.channel_type {
        ChannelType::Telegram => ChannelProfile::telegram(suffix),
        ChannelType::WhatsApp => ChannelProfile::whatsapp(suffix),
        ChannelType::Discord => ChannelProfile::discord(suffix),
        ChannelType::Slack => ChannelProfile::slack(suffix),
        ChannelType::Messenger => ChannelProfile::messenger(suffix),
    }
}

/// Data needed to dispatch a message through the LLM pipeline after HTTP 200
pub(crate) struct PendingDispatch {
    /// Server resources for LLM access
    pub(crate) resources: Arc<ServerContext>,
    /// Channel adapter for outbound send
    pub(crate) adapter: Arc<dyn MessagingChannel>,
    /// Authenticated principal for the inbound turn.
    ///
    /// Produced by
    /// [`crate::middleware::auth::McpAuthMiddleware::authenticate_channel`] at
    /// the top of `persist_single_message` — same shape and semantics as the
    /// `AuthResult` the JWT middleware produces for HTTP callers. Carries the
    /// user id, rate-limit state, and the user's own `active_tenant_id` so
    /// downstream stages don't re-derive any of this from the channel link.
    pub(super) auth_result: AuthResult,
    /// Resolved session info
    pub(super) session: ResolvedSession,
    /// Channel config tenant — used for conversation/message persistence (the
    /// conversation was created under this tenant).
    pub(super) channel_tenant_id: TenantId,
    /// User's own tenant — used for tool execution (OAuth, activities, etc.).
    /// May differ from `channel_tenant_id` when the user belongs to a different
    /// tenant than the bot that owns the webhook.
    pub(super) user_tenant_id: TenantId,
    /// Channel type enum
    pub(super) channel_type: ChannelType,
    /// Channel name string (e.g., "slack")
    pub(super) channel: String,
    /// Original sender to reply to
    pub(super) sender_id: String,
    /// Channel-specific conversation/thread identifier (channel ID, chat ID, etc.)
    pub(super) conversation_id: Option<String>,
    /// Text content to dispatch
    pub(super) text_content: String,
    /// Channel-native message ID for reply/thread context (Slack ts, Telegram `message_id`)
    pub(super) channel_message_id: String,
    /// Forum topic thread ID (Telegram Topics `message_thread_id`)
    pub(super) thread_id: Option<String>,
    /// Resolved BCP-47 locale for this turn.
    ///
    /// Threaded into `TurnInput` so the chat pipeline's guardrail /
    /// verification / empty-reply fallbacks speak the user's
    /// language. Populated via [`resolve_messaging_locale`] when the
    /// dispatch is enqueued.
    pub(super) locale: String,
    /// Conversation-turn correlation identifier set by canot's webhook
    /// adapter on [`IncomingMessage::turn_id`]. Threaded through the
    /// pipeline so every LLM call, tool invocation, and outbound reply
    /// shares one id end-to-end.
    pub(super) turn_id: ConversationTurnId,
}

/// Persist inbound messages, handling linking, OTP, logout, slash commands, and session resolution
///
/// Returns (`stored_count`, `pending_dispatches`) — the dispatches are processed
/// asynchronously after the webhook returns HTTP 200.
pub(crate) async fn persist_inbound(
    resources: &Arc<ServerContext>,
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
                pending_dispatches.push(*dispatch);
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
                error!(error = %e, "Failed to send channel response");
            }
        });
    }
}

/// Persist a single inbound message and optionally prepare an LLM dispatch
///
/// Handles three cases:
/// 1. Linking command -> consume code, create link, send confirmation (not stored)
/// 2. Linked user -> resolve session, store message, dispatch to LLM pipeline
/// 3. Unlinked user -> send prompt to authenticate (not stored)
///
/// Returns `Ok(StoredWithDispatch)` for linked-user text messages,
/// `Ok(StoredNoDispatch)` for stored non-text messages,
/// `Ok(HandledNotStored)` for linking commands or unlinked users,
/// or `Err(())` if persistence failed.
async fn persist_single_message(
    resources: &Arc<ServerContext>,
    channel: &str,
    tenant_id: TenantId,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
) -> Result<PersistOutcome, ()> {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let thread_id = extract_thread_id(&message.metadata);

    let hashed_tenant = hash_id(&tenant_id.to_string());
    let hashed_sender = hash_id(&format!("{channel}:{}", message.sender_id));

    // Check for linking commands (`Telegram` /start, `WhatsApp` LINK)
    if let LinkingAction::LinkCode(code) = detect_linking_code(channel_type, &message.content) {
        info!(channel = %channel, sender_id = %message.sender_id, "Processing channel linking command");
        analytics().track_intent(channel, &hashed_tenant, &hashed_sender, "link_code");
        let mut response =
            handle_linking_command(resources, tenant_id, channel, &message.sender_id, &code).await;
        response.thread_id = thread_id;
        apply_conversation_recipient(&mut response, message.conversation_id.as_deref());
        send_channel_response(db, tenant_id, channel, adapter, response).await;
        return Ok(PersistOutcome::HandledNotStored);
    }

    // Check for active in-chat OTP linking flow
    if let Some(otp_response) = handle_otp_flow(
        resources,
        tenant_id,
        channel_type,
        channel,
        &message.sender_id,
        &message.content,
    )
    .await
    {
        analytics().track_intent(channel, &hashed_tenant, &hashed_sender, "otp_flow");
        let mut otp_response = otp_response;
        otp_response.thread_id = thread_id;
        apply_conversation_recipient(&mut otp_response, message.conversation_id.as_deref());
        send_channel_response(db, tenant_id, channel, adapter, otp_response).await;
        return Ok(PersistOutcome::HandledNotStored);
    }

    // Check for logout command: unlink channel and destroy session
    if is_logout_command(&message.content) {
        analytics().track_intent(channel, &hashed_tenant, &hashed_sender, "logout");
        let logout_response = handle_logout(
            resources,
            tenant_id,
            channel_type,
            channel,
            &message.sender_id,
        )
        .await;
        let mut logout_response = logout_response;
        logout_response.thread_id = thread_id;
        apply_conversation_recipient(&mut logout_response, message.conversation_id.as_deref());
        send_channel_response(db, tenant_id, channel, adapter, logout_response).await;
        return Ok(PersistOutcome::HandledNotStored);
    }

    // Resolve session via channel link (returns None for unlinked / denied users)
    let resolved = resolve_or_prompt(
        resources,
        db,
        tenant_id,
        channel,
        channel_type,
        adapter,
        message,
    )
    .await?;

    let Some((auth_result, session)) = resolved else {
        return Ok(PersistOutcome::HandledNotStored);
    };

    // Check for slash commands before storing or dispatching to LLM.
    // Commands are handled immediately and not stored in conversation history.
    if let Some(text) = content_body_text(&message.content) {
        if let Some(response) = try_handle_slash_command(
            resources,
            SlashCommandContext {
                channel,
                channel_type,
                auth_result: &auth_result,
                session: &session,
                text: &text,
                sender_id: &message.sender_id,
                conversation_id: message.conversation_id.as_deref(),
                thread_id: thread_id.clone(),
                is_direct_message: message.is_direct_message,
            },
        )
        .await
        {
            send_channel_response(db, tenant_id, channel, adapter, response).await;
            return Ok(PersistOutcome::HandledNotStored);
        }
    }

    let stored = store_inbound_message(db, tenant_id, &session, channel, message).await?;
    if !stored {
        return Err(());
    }

    let hashed_user = hash_id(&session.user_id);
    analytics().track_message_received(
        channel,
        &hashed_tenant,
        &hashed_user,
        content_type_label(&message.content),
    );
    analytics().track_intent(channel, &hashed_tenant, &hashed_user, "normal_chat");

    // The user's tenant for tool execution comes straight from AuthResult —
    // authenticate_channel already resolved it (first tenant membership,
    // falling back to the webhook tenant), the same way the JWT path's
    // active_tenant_id flows. No second lookup.
    let user_tenant_id = auth_result
        .active_tenant_id
        .map_or(tenant_id, TenantId::from_uuid);

    // Resolve the user's preferred locale once per dispatch so every
    // downstream stage (guardrails, verification, empty-reply) speaks the
    // same language. Uses the channel-link override first, then the user
    // profile, then the registry default.
    let locale = match Uuid::parse_str(&session.user_id) {
        Ok(uuid) => {
            resolve_messaging_locale(resources, user_tenant_id, uuid, channel, &message.sender_id)
                .await
        }
        Err(_) => DEFAULT_LOCALE.to_owned(),
    };

    // Extract text content for LLM dispatch, then run the Phase C input
    // sanitization scanner. Verbatim user text is preserved in the stored
    // message above for audit/compliance; only the LLM-bound copy gets the
    // redaction so injection patterns never reach prompt assembly.
    content_body_text(&message.content).map_or_else(
        || {
            info!("Skipping non-text message for LLM dispatch");
            Ok(PersistOutcome::StoredNoDispatch)
        },
        |text_content| {
            let sanitized = sanitize_for_dispatch(channel, &session.user_id, text_content);
            // Prefer the language of the message itself over the stored
            // `users.locale` so status placeholders / refusal lines match
            // the language the LLM will reply in (LLMs mirror the user's
            // current message language, not the stored preference).
            let turn_locale = detect_turn_locale(&sanitized, &locale);
            Ok(PersistOutcome::StoredWithDispatch(Box::new(
                PendingDispatch {
                    resources: Arc::clone(resources),
                    adapter: Arc::clone(adapter),
                    auth_result,
                    session,
                    channel_tenant_id: tenant_id,
                    user_tenant_id,
                    channel_type,
                    channel: channel.to_owned(),
                    sender_id: message.sender_id.clone(),
                    conversation_id: message.conversation_id.clone(),
                    text_content: sanitized,
                    channel_message_id: message.channel_message_id.clone(),
                    thread_id,
                    locale: turn_locale,
                    // Canot generates the turn id at the webhook boundary
                    // (see canot's IncomingMessage::turn_id); adopt it so
                    // canot-emitted log spans and the platform's
                    // /internal/conversation-turn row share one key.
                    turn_id: message.turn_id.into(),
                },
            )))
        },
    )
}

/// Phase C input sanitization wrapper.
///
/// Runs [`pierre_core::safety::scan`] on the inbound text and returns the
/// version that should reach the LLM. When sanitization fires the function
/// emits a structured warn-level log entry tagged with the matched
/// signature names so SOC tooling can react. The verbatim text remains in
/// `chat_messages` for audit purposes.
fn sanitize_for_dispatch(channel: &str, user_id: &str, text_content: String) -> String {
    match scan_for_injection(&text_content) {
        SanitizationOutcome::Clean => text_content,
        SanitizationOutcome::Sanitized { redacted, matches } => {
            let signatures: Vec<&'static str> =
                matches.iter().map(|m| m.signature.as_str()).collect();
            let signatures_str = signatures.join(",");
            warn!(
                channel = %channel,
                user_id = %hash_id(user_id),
                signatures = %signatures_str,
                match_count = matches.len(),
                "input sanitization fired — redacting injection patterns from LLM-bound text"
            );
            redacted
        }
    }
}

/// Send an authentication prompt to an unlinked user
///
/// Chooses between in-chat OTP flow (when email service is available) and
/// link-URL flow (fallback), then sends the response via the channel adapter.
async fn send_unlinked_user_prompt(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
) {
    let prompt_type = if resources.email_service.is_some() {
        "otp"
    } else {
        "link_url"
    };
    // Pre-link event — there is no Pierre user yet, so we use a
    // `{channel}:{sender_id}` identity hash as the PostHog distinct_id.
    // Once linking completes, `analytics().alias(channel_identity,
    // pierre_user_id)` glues the pre-link history to the user id.
    let pre_link_identity = format!("{channel}:{}", message.sender_id);
    analytics().track_unlinked_prompted(channel, &hash_id(&pre_link_identity), prompt_type);

    let mut prompt = if resources.email_service.is_some() {
        info!(channel = %channel, sender_id = %message.sender_id, "Unlinked user, starting OTP flow");
        start_otp_flow(
            resources,
            db,
            tenant_id,
            channel_type,
            &message.sender_id,
            message.sender_name.as_deref(),
        )
        .await
    } else {
        info!(channel = %channel, sender_id = %message.sender_id, "Unlinked user, sending link URL (no email service)");
        create_link_and_prompt(
            resources,
            db,
            tenant_id,
            channel_type,
            &message.sender_id,
            message.sender_name.as_deref(),
        )
        .await
    };
    prompt.thread_id = extract_thread_id(&message.metadata);
    apply_conversation_recipient(&mut prompt, message.conversation_id.as_deref());
    send_channel_response(db, tenant_id, channel, adapter, prompt).await;
}

/// Resolve a linked session and authenticated principal, or send a prompt /
/// denial reply when the channel sender cannot proceed.
///
/// Calls [`crate::middleware::auth::McpAuthMiddleware::authenticate_channel`]
/// for the unified auth/status/rate-limit gate, then resolves the session
/// row on success. Returns `Ok(Some((auth_result, session)))` for authorized
/// linked users, `Ok(None)` after surfacing the right reply (no link → prompt;
/// pending / suspended / rate-limited → translated denial), or `Err(())` on
/// operator-category failure.
async fn resolve_or_prompt(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
) -> Result<Option<(AuthResult, ResolvedSession)>, ()> {
    let auth_outcome = resources
        .auth_middleware
        .authenticate_channel(tenant_id, channel, &message.sender_id)
        .await;
    let Some(auth_result) = handle_channel_auth_outcome(ChannelAuthOutcomeInputs {
        resources,
        db,
        tenant_id,
        channel,
        channel_type,
        adapter,
        message,
        outcome: auth_outcome,
    })
    .await?
    else {
        return Ok(None);
    };

    let chat_ref = ChannelChatRef {
        chat_id: message.conversation_id.as_deref(),
        chat_title: message.chat_title.as_deref(),
    };
    match resolve_linked_session(
        resources,
        tenant_id,
        channel,
        &message.sender_id,
        chat_ref,
        message.is_direct_message,
    )
    .await
    {
        Ok(Some(session)) => Ok(Some((auth_result, session))),
        Ok(None) => {
            // authenticate_channel succeeded but channel link disappeared
            // between calls — race or operator action. Treat as unlinked.
            send_unlinked_user_prompt(
                resources,
                db,
                tenant_id,
                channel,
                channel_type,
                adapter,
                message,
            )
            .await;
            Ok(None)
        }
        Err(e) => {
            // Error-level: the user sent a message and got no reply — this is a
            // production incident, not a warning. Triggers the dravr-tronc
            // Slack notifier so operators see the outage within seconds.
            error!(
                error = %e,
                sender_id = %message.sender_id,
                channel = %channel_type,
                "Failed to resolve messaging session, dropping message"
            );
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
    // The `messaging_messages` table column is still called
    // `correlation_id` — it existed before the turn-id threading — so
    // the string from `turn_id` is what gets persisted there.
    let correlation_str = message.turn_id.to_string();

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

/// Per-conversation dispatch locks ensuring sequential LLM processing.
///
/// Without this, concurrent webhook calls for the same conversation race:
/// message 2's dispatch can finish before message 1's, producing out-of-order
/// replies. The lock serializes dispatches per conversation while allowing
/// different conversations to proceed in parallel.
pub(super) static CONVERSATION_DISPATCH_LOCKS: LazyLock<DashMap<String, Arc<TokioMutex<()>>>> =
    LazyLock::new(DashMap::new);

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
pub(super) fn content_body_text(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::Text { body } | MessageContent::Card { body, .. } => Some(body.clone()),
        MessageContent::Media { caption, .. } => caption.clone(),
        MessageContent::Location { .. } => None,
    }
}

/// Inputs for [`handle_channel_auth_outcome`].
struct ChannelAuthOutcomeInputs<'a> {
    resources: &'a ServerContext,
    db: &'a dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &'a str,
    channel_type: ChannelType,
    adapter: &'a Arc<dyn MessagingChannel>,
    message: &'a IncomingMessage,
    outcome: Result<AuthResult, AppError>,
}

/// Refresh `users.last_active` after a successful channel authentication.
///
/// Mirrors the JWT/MCP tool-handler behavior so admin "last seen" views and
/// activity reports treat messaging-only users as live. Best-effort: a
/// failure here is logged but does not block dispatch — activity tracking is
/// observability, not correctness.
async fn refresh_channel_last_active(
    resources: &ServerContext,
    user_id: Uuid,
    channel_type: ChannelType,
) {
    if let Err(e) = resources.repos.users.update_last_active(user_id).await {
        warn!(
            user_id = %user_id,
            channel = %channel_type,
            error = %e,
            "Failed to update last_active on channel auth (activity tracking impacted)"
        );
    }
}

/// Record a `JwtUsage` row for a successful channel-authenticated turn.
///
/// Delegates to [`crate::middleware::auth::record_jwt_usage_for_request`] so
/// the JWT (`HTTP` cookie / Bearer / `MCP`) path and the channel-link path
/// share **one** write site for the rate-limit counter — no symmetry gap
/// between transports. The endpoint label is rendered as
/// `messaging:<channel>` (e.g. `messaging:telegram`) so admin usage reports
/// can disaggregate by transport.
async fn record_channel_usage(resources: &ServerContext, user_id: Uuid, channel: &str) {
    let endpoint = format!("messaging:{channel}");
    record_jwt_usage_for_request(&resources.repos, user_id, &endpoint, "WEBHOOK").await;
}

/// Branch on the channel-authentication outcome, surfacing the right reply
/// for each terminal state and returning the [`AuthResult`] only on success.
async fn handle_channel_auth_outcome(
    inputs: ChannelAuthOutcomeInputs<'_>,
) -> Result<Option<AuthResult>, ()> {
    match inputs.outcome {
        Ok(r) => {
            refresh_channel_last_active(inputs.resources, r.user_id, inputs.channel_type).await;
            record_channel_usage(inputs.resources, r.user_id, inputs.channel).await;
            Ok(Some(r))
        }
        Err(e) if e.code == ErrorCode::AuthInvalid => {
            debug!(
                sender_id = %inputs.message.sender_id,
                channel = %inputs.channel_type,
                "Sender not linked, prompting"
            );
            send_unlinked_user_prompt(
                inputs.resources,
                inputs.db,
                inputs.tenant_id,
                inputs.channel,
                inputs.channel_type,
                inputs.adapter,
                inputs.message,
            )
            .await;
            Ok(None)
        }
        Err(e)
            if matches!(
                e.code,
                ErrorCode::AccountPending
                    | ErrorCode::AccountSuspended
                    | ErrorCode::RateLimitExceeded
            ) =>
        {
            let reply = build_auth_denial_reply(AuthDenialReplyInputs {
                resources: inputs.resources,
                tenant_id: inputs.tenant_id,
                channel: inputs.channel,
                channel_type: inputs.channel_type,
                sender_id: &inputs.message.sender_id,
                conversation_id: inputs.message.conversation_id.as_deref(),
                thread_id: extract_thread_id(&inputs.message.metadata),
                err: &e,
            })
            .await;
            send_channel_response(
                inputs.db,
                inputs.tenant_id,
                inputs.channel,
                inputs.adapter,
                reply,
            )
            .await;
            Ok(None)
        }
        Err(e) => {
            // Operator-category failure — drop the message, let dravr-tronc
            // page on-call via the ERROR subscriber.
            error!(
                error = %e,
                sender_id = %inputs.message.sender_id,
                channel = %inputs.channel_type,
                "Channel authentication failed, dropping message"
            );
            Err(())
        }
    }
}

/// Inputs for [`build_auth_denial_reply`].
struct AuthDenialReplyInputs<'a> {
    resources: &'a ServerContext,
    tenant_id: TenantId,
    channel: &'a str,
    channel_type: ChannelType,
    sender_id: &'a str,
    conversation_id: Option<&'a str>,
    thread_id: Option<String>,
    err: &'a AppError,
}

/// Build a localized "denied" reply for the authentication outcomes that need
/// to surface user-facing text (`Pending`, `Suspended`, `RateLimitExceeded`).
async fn build_auth_denial_reply(inputs: AuthDenialReplyInputs<'_>) -> OutgoingMessage {
    let body = if let Some(key) = messaging_key_for_status(inputs.err.code) {
        let locale = inputs
            .resources
            .repos
            .messaging
            .get_channel_link_locale(inputs.tenant_id, inputs.channel, inputs.sender_id)
            .await
            .ok()
            .flatten()
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_LOCALE.to_owned());
        inputs
            .resources
            .messaging_strings_registry
            .get(key, &locale)
    } else {
        inputs
            .err
            .to_channel_reply(inputs.resources, DEFAULT_LOCALE, "channel_auth")
            .0
    };

    let mut message = OutgoingMessage {
        channel_type: inputs.channel_type,
        recipient_id: inputs.sender_id.to_owned(),
        content: MessageContent::Text { body },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: inputs.thread_id,
    };
    apply_conversation_recipient(&mut message, inputs.conversation_id);
    message
}
