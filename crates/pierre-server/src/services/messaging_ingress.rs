// ABOUTME: Business logic for messaging ingress: OTP flow, channel linking, session resolution,
// ABOUTME: slash command dispatch, message persistence, LLM dispatch, and outbound response handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{Duration, Utc};
use dashmap::DashMap;
use pierre_core::models::messaging::{
    CardAction, ChannelConfig, ChannelType, IncomingMessage, MessageContent, OutgoingMessage,
    LINK_CODE_TTL_MINUTES, MAX_OTP_ATTEMPTS, OTP_TTL_MINUTES,
};
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{ConversationTurnId, TenantId, User, TURN_SUMMARY_CALL_TYPE};
use pierre_core::safety::{scan as scan_for_injection, SanitizationOutcome};
use pierre_core::tokens::estimate_chat_tokens;
use pierre_database::plugins::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, InsertMessageParams,
    MessagingRepository, TenantRepository, UserRepository,
};
use pierre_llm::TokenUsage;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::env;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use serde_json::Value;

use pierre_messaging::agui_status::StatusAdapter;

use crate::agui::{AgUiEventFilter, BroadcastSink, RunOwner, RunScope};
use crate::contremaitre::messaging_strings::{
    format_template, DEFAULT_LOCALE, KEY_EMPTY_REPLY, KEY_ERROR_GENERIC, KEY_LINK_CANCELLED,
    KEY_LINK_EMAIL_NOT_CONFIGURED, KEY_LINK_EMAIL_SEND_FAILED, KEY_LINK_FALLBACK_PROMPT,
    KEY_LINK_GENERIC_ERROR, KEY_LINK_IDENTITY_COLLISION, KEY_LINK_INCORRECT_CODE,
    KEY_LINK_INITIAL_PROMPT, KEY_LINK_INVALID_EMAIL, KEY_LINK_LOGOUT_COMPLETE, KEY_LINK_NO_ACCOUNT,
    KEY_LINK_NO_TENANT, KEY_LINK_OTP_PROMPT, KEY_LINK_OTP_SENT, KEY_LINK_SESSION_EXPIRED,
    KEY_LINK_SUCCESS, KEY_LINK_TOO_MANY_ATTEMPTS, KEY_LINK_VERIFICATION_ERROR,
};
use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::routes::messaging::linking::generate_link_code;
use crate::services::analytics::{analytics, hash_id};
use crate::services::chat_orchestration;
use crate::services::chat_pipeline::{
    self, ChannelProfile, DispatchResult, PipelineHooks, TurnInput,
};
use crate::services::messaging_status_bridge::{
    open_status_adapter, spawn_status_consumer, OpenStatusParams,
};
use crate::services::usage_counter::UsageCounterService;

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

/// Result of checking an inbound message for a channel linking command
enum LinkingAction {
    /// Message contains a linking command — handle it and do not dispatch to LLM
    LinkCode(String),
    /// Normal message — proceed with standard routing
    Normal,
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
fn build_messaging_profile(dispatch: &PendingDispatch) -> ChannelProfile {
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
    pub(crate) resources: Arc<ServerResources>,
    /// Channel adapter for outbound send
    pub(crate) adapter: Arc<dyn MessagingChannel>,
    /// Resolved session info
    session: ResolvedSession,
    /// Channel config tenant — used for conversation/message persistence (the
    /// conversation was created under this tenant).
    channel_tenant_id: TenantId,
    /// User's own tenant — used for tool execution (OAuth, activities, etc.).
    /// May differ from `channel_tenant_id` when the user belongs to a different
    /// tenant than the bot that owns the webhook.
    user_tenant_id: TenantId,
    /// Channel type enum
    channel_type: ChannelType,
    /// Channel name string (e.g., "slack")
    channel: String,
    /// Original sender to reply to
    sender_id: String,
    /// Channel-specific conversation/thread identifier (channel ID, chat ID, etc.)
    conversation_id: Option<String>,
    /// Text content to dispatch
    text_content: String,
    /// Channel-native message ID for reply/thread context (Slack ts, Telegram `message_id`)
    channel_message_id: String,
    /// Forum topic thread ID (Telegram Topics `message_thread_id`)
    thread_id: Option<String>,
    /// Resolved BCP-47 locale for this turn.
    ///
    /// Threaded into `TurnInput` so the chat pipeline's guardrail /
    /// verification / empty-reply fallbacks speak the user's
    /// language. Populated via [`resolve_messaging_locale`] when the
    /// dispatch is enqueued.
    locale: String,
}

/// Parameters for the OTP code verification step of the channel linking flow
struct OtpVerificationParams<'a> {
    resources: &'a ServerResources,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &'a str,
    sender_id: &'a str,
    state_id: &'a str,
    email: &'a str,
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

/// Enumerated failure reasons from [`execute_link_code`].
///
/// Held as a structured variant (rather than a pre-formatted `String`) so the
/// user-facing translation can happen at the outer boundary where the
/// `MessagingStringsRegistry` is available.
enum DeepLinkError {
    /// `consume_link_state` returned an error — expired or already used code.
    SessionExpired(String),
    /// Creating the channel link row failed (most commonly a uniqueness
    /// collision on the `(tenant, channel_type, channel_user_id)` triple).
    IdentityCollision(String),
}

/// Consume a link code and create the permanent channel link, returning the user ID
async fn execute_link_code(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> Result<String, DeepLinkError> {
    let link_state = db
        .consume_link_state(code, tenant_id)
        .await
        .map_err(|e| DeepLinkError::SessionExpired(e.to_string()))?;

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
        .map_err(|e| DeepLinkError::IdentityCollision(e.to_string()))?;

    Ok(user_id)
}

/// Consume a link code and create the permanent channel link
///
/// Returns a user-facing message describing the result, translated through
/// the messaging-strings registry.
async fn consume_and_link(
    resources: &ServerResources,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> String {
    let reg = &resources.messaging_strings_registry;
    match execute_link_code(db, tenant_id, channel, sender_id, code).await {
        Ok(user_id) => {
            info!(channel = %channel, user_id = %user_id, channel_user_id = %sender_id, "Channel linked via deep link");
            let hashed_tenant = hash_id(&tenant_id.to_string());
            let hashed_user = hash_id(&user_id);
            let hashed_channel_id = hash_id(&format!("{channel}:{sender_id}"));
            analytics().alias(&hashed_channel_id, &hashed_user);
            analytics().track_linking_completed(channel, &hashed_tenant, &hashed_user, "deep_link");
            reg.get(KEY_LINK_SUCCESS, DEFAULT_LOCALE)
        }
        Err(err) => {
            let (key, detail) = match &err {
                DeepLinkError::SessionExpired(d) => (KEY_LINK_SESSION_EXPIRED, d.as_str()),
                DeepLinkError::IdentityCollision(d) => (KEY_LINK_IDENTITY_COLLISION, d.as_str()),
            };
            warn!(error = %detail, "Channel linking failed");
            analytics().track_linking_failed(channel, &hash_id(&tenant_id.to_string()), detail);
            reg.get(key, DEFAULT_LOCALE)
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
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let channel_type = ChannelType::from_str(channel).unwrap_or(ChannelType::Telegram);
    let response_text = consume_and_link(resources, db, tenant_id, channel, sender_id, code).await;

    OutgoingMessage {
        channel_type,
        recipient_id: sender_id.to_owned(),
        content: MessageContent::Text {
            body: response_text,
        },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}

/// Hydrate the analytics consent cache for a messaging user on cache miss
///
/// The cache is in-memory and empties on every Cloud Run cold start, so each
/// fresh pod needs to learn each user's durable `analytics_consent` value from
/// the database before their events will be captured. Once hydrated the entry
/// persists for the life of the pod and `/privacy on|off` commands keep it
/// current via `set_consent`.
async fn hydrate_analytics_consent(resources: &ServerResources, user_id: &str) {
    let hashed_user = hash_id(user_id);
    if analytics().has_consent_cached(&hashed_user) {
        return;
    }
    let Ok(parsed) = Uuid::parse_str(user_id) else {
        return;
    };
    match resources.repos.users.get_global(parsed).await {
        Ok(Some(user)) => {
            analytics().hydrate_consent(&hashed_user, user.analytics_consent);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, user_id = %user_id, "Failed to load user for analytics consent hydration");
        }
    }
}

/// Create a fresh `chat_conversation` and repoint the session at it.
///
/// Called from `resolve_linked_session` when a session's `pierre_conversation_id`
/// is NULL (FK `ON DELETE SET NULL` fired after the referenced conversation was
/// deleted). Returns the new conversation id.
async fn forge_fresh_session_conversation(
    resources: &ServerResources,
    db: &dyn MessagingRepository,
    session_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    channel_type: &str,
) -> Result<String, AppError> {
    warn!(
        session_id = %session_id,
        user_id = %user_id,
        channel_type = %channel_type,
        "Session has no pierre_conversation_id; self-healing with a fresh conversation"
    );
    let title = format!("Messaging: {channel_type}");
    let conversation = chat_orchestration::create_conversation(
        resources.repos.chat.as_ref(),
        user_id,
        tenant_id,
        &title,
        None,
        None,
    )
    .await?;
    let new_id = conversation.conversation.id.clone();
    db.set_session_conversation(session_id, &new_id).await?;
    Ok(new_id)
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
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

    // Check for existing session first (fast path)
    if let Some(session) = db
        .get_session_by_channel_identity(tenant_id, channel_type, sender_id)
        .await?
    {
        let session_id = session["id"]
            .as_str()
            .ok_or_else(|| AppError::internal("Session missing id field"))?
            .to_owned();
        let user_id = session["user_id"]
            .as_str()
            .ok_or_else(|| AppError::internal("Session missing user_id field"))?
            .to_owned();

        // Self-heal: if the session has no conversation (the chat_conversations row
        // was deleted and the FK ON DELETE SET NULL fired, or the row is otherwise
        // unreachable), forge a fresh one and repoint the session before returning.
        // Without this, dispatch_and_respond would fail on every message for this
        // channel user until they manually re-link.
        let conversation = if let Some(id) = session["pierre_conversation_id"].as_str() {
            id.to_owned()
        } else {
            forge_fresh_session_conversation(
                resources,
                db,
                &session_id,
                &user_id,
                tenant_id,
                channel_type,
            )
            .await?
        };

        if let Err(e) = db.touch_session(&session_id).await {
            warn!(error = %e, session_id = %session_id, "Failed to touch session");
        }

        hydrate_analytics_consent(resources, &user_id).await;

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
        resources.repos.chat.as_ref(),
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

    hydrate_analytics_consent(resources, &user_id).await;

    analytics().track_session_started(
        channel_type,
        &hash_id(&tenant_id.to_string()),
        &hash_id(&user_id),
        true,
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
    resources: &ServerResources,
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
        let body = resources
            .messaging_strings_registry
            .get(KEY_LINK_FALLBACK_PROMPT, DEFAULT_LOCALE);
        return OutgoingMessage {
            channel_type,
            recipient_id: sender_id.to_owned(),
            content: MessageContent::Text { body },
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: None,
        };
    }

    let base_url = env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8081".to_owned());
    let link_url = format!("{base_url}/messaging/link/{code}");

    let template = resources
        .messaging_strings_registry
        .get(KEY_LINK_INITIAL_PROMPT, DEFAULT_LOCALE);
    let body = format_template(&template, &[&link_url]);

    OutgoingMessage {
        channel_type,
        recipient_id: sender_id.to_owned(),
        content: MessageContent::Text { body },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}

// ══════════════════════════════════════════════════════════════
// In-Chat OTP Linking Helpers
// ══════════════════════════════════════════════════════════════

/// Check if a message is a cancel command for the OTP linking flow
fn is_cancel_command(content: &MessageContent) -> bool {
    matches!(content, MessageContent::Text { body } if body.trim().eq_ignore_ascii_case("cancel"))
}

/// Check if a message is a logout/disconnect command
fn is_logout_command(content: &MessageContent) -> bool {
    matches!(content, MessageContent::Text { body } if body.trim().eq_ignore_ascii_case("logout"))
}

/// Handle logout: delete channel link, sessions, and OTP states atomically
async fn handle_logout(
    resources: &ServerResources,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &str,
    sender_id: &str,
) -> OutgoingMessage {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

    if let Err(e) = db
        .logout_channel_sender(tenant_id, channel, sender_id)
        .await
    {
        warn!(error = %e, "Failed to logout channel sender");
    }

    info!(
        channel = %channel,
        sender_id = %sender_id,
        "User logged out from messaging channel"
    );

    analytics().track_session_dropped(
        channel,
        &hash_id(&tenant_id.to_string()),
        &hash_id(sender_id),
        "logout",
    );

    otp_reply(
        channel_type,
        sender_id,
        resources
            .messaging_strings_registry
            .get(KEY_LINK_LOGOUT_COMPLETE, DEFAULT_LOCALE),
    )
}

/// Basic email format validation (not RFC 5322, just good enough for UX)
fn looks_like_email(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.contains('@') && trimmed.contains('.') && trimmed.len() > 5 && !trimmed.contains(' ')
}

/// Generate a cryptographically random 6-digit OTP code
fn generate_otp() -> String {
    let code: u32 = rand::rng().random_range(100_000..1_000_000);
    code.to_string()
}

/// SHA-256 hash of an OTP code for secure storage
fn hash_otp(code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    hex::encode(hasher.finalize())
}

/// Mask an email address for display (e.g., "j***@dravr.ai")
fn mask_email(email: &str) -> String {
    if let Some(at_pos) = email.find('@') {
        if at_pos > 1 {
            let first = &email[..1];
            let domain = &email[at_pos..];
            return format!("{first}***{domain}");
        }
    }
    "***".to_owned()
}

/// Create a text reply message for OTP flow responses
///
/// For channel-based platforms (Discord, Slack), `recipient_id` must be the channel ID
/// where the message was received. For DM-based platforms (`WhatsApp`, Telegram, Messenger),
/// `recipient_id` is the sender's user ID.
fn otp_reply(channel_type: ChannelType, sender_id: &str, body: String) -> OutgoingMessage {
    OutgoingMessage {
        channel_type,
        recipient_id: sender_id.to_owned(),
        content: MessageContent::Text { body },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}

/// Override `recipient_id` with conversation ID for channel-based platforms like Discord
///
/// Discord REST API sends to channels, not users. If the message came from a guild
/// channel, we must reply to that channel — not to the user ID.
fn apply_conversation_recipient(msg: &mut OutgoingMessage, conversation_id: Option<&str>) {
    if msg.channel_type == ChannelType::Discord || msg.channel_type == ChannelType::Slack {
        if let Some(conv_id) = conversation_id {
            conv_id.clone_into(&mut msg.recipient_id);
        }
    }
}

/// Handle an in-chat OTP linking flow step
///
/// Returns `Some(OutgoingMessage)` if the OTP flow handled the message (reply to send),
/// or `None` if no active OTP flow exists (proceed to normal routing).
async fn handle_otp_flow(
    resources: &ServerResources,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &str,
    sender_id: &str,
    content: &MessageContent,
) -> Option<OutgoingMessage> {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

    // Handle cancel command: invalidate any active flow
    if is_cancel_command(content) {
        if let Ok(Some(_)) = db
            .get_active_otp_link_state(tenant_id, channel, sender_id)
            .await
        {
            let _ = db
                .invalidate_otp_link_states(tenant_id, channel, sender_id)
                .await;
            return Some(otp_reply(
                channel_type,
                sender_id,
                resources
                    .messaging_strings_registry
                    .get(KEY_LINK_CANCELLED, DEFAULT_LOCALE),
            ));
        }
        // No active flow, cancel is just a normal message
        return None;
    }

    // Look up active OTP flow
    let state = db
        .get_active_otp_link_state(tenant_id, channel, sender_id)
        .await
        .ok()??;

    let state_id = state["id"].as_str()?.to_owned();
    let otp_step = state["otp_step"].as_str()?;

    let text = match content {
        MessageContent::Text { body } => body.trim().to_owned(),
        _ => return None,
    };

    // Distinguish sub-states: awaiting_otp with empty email = awaiting email input
    let email = state["email"].as_str().unwrap_or_default().to_owned();

    match otp_step {
        "awaiting_otp" if email.is_empty() => Some(
            handle_email_step(
                resources,
                tenant_id,
                channel_type,
                channel,
                sender_id,
                &state_id,
                &text,
            )
            .await,
        ),
        "awaiting_otp" => {
            let params = OtpVerificationParams {
                resources,
                tenant_id,
                channel_type,
                channel,
                sender_id,
                state_id: &state_id,
                email: &email,
            };
            Some(handle_otp_verification_step(params, &text).await)
        }
        _ => None,
    }
}

/// Validate user exists and belongs to at least one tenant
///
/// Returns the user on success, or an error reply for the caller to return.
async fn validate_email_user(
    resources: &ServerResources,
    channel_type: ChannelType,
    sender_id: &str,
    email: &str,
) -> Result<User, OutgoingMessage> {
    let db_user: &dyn UserRepository = resources.repos.users.as_ref();

    // Cross-tenant user lookup by email
    let user = match db_user.get_by_email(email).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            let register_url = resources
                .config
                .frontend_url
                .as_deref()
                .unwrap_or(&resources.config.base_url);
            let template = resources
                .messaging_strings_registry
                .get(KEY_LINK_NO_ACCOUNT, DEFAULT_LOCALE);
            return Err(otp_reply(
                channel_type,
                sender_id,
                format_template(&template, &[register_url]),
            ));
        }
        Err(e) => {
            warn!(error = %e, "Failed to look up user by email during OTP flow");
            return Err(otp_reply(
                channel_type,
                sender_id,
                resources
                    .messaging_strings_registry
                    .get(KEY_LINK_GENERIC_ERROR, DEFAULT_LOCALE),
            ));
        }
    };

    // Verify user belongs to a tenant (shared bot model: accept any tenant the user belongs to)
    let db_tenant: &dyn TenantRepository = resources.repos.tenants.as_ref();
    let tenants = match db_tenant.list_for_user(user.id).await {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, user_id = %user.id, "Failed to list tenants for user during OTP flow");
            return Err(otp_reply(
                channel_type,
                sender_id,
                resources
                    .messaging_strings_registry
                    .get(KEY_LINK_GENERIC_ERROR, DEFAULT_LOCALE),
            ));
        }
    };

    if tenants.is_empty() {
        return Err(otp_reply(
            channel_type,
            sender_id,
            resources
                .messaging_strings_registry
                .get(KEY_LINK_NO_TENANT, DEFAULT_LOCALE),
        ));
    }

    Ok(user)
}

/// Generate an OTP code, store it, and send the verification email
///
/// Returns the masked email on success, or an error reply for the caller to return.
async fn generate_and_send_otp(
    resources: &ServerResources,
    channel_type: ChannelType,
    sender_id: &str,
    state_id: &str,
    email: &str,
) -> Result<String, OutgoingMessage> {
    let db_msg: &dyn MessagingRepository = resources.repos.messaging.as_ref();

    let otp_code = generate_otp();
    let otp_hashed = hash_otp(&otp_code);

    if let Err(e) = db_msg
        .set_otp_on_link_state(state_id, email, &otp_hashed)
        .await
    {
        warn!(error = %e, "Failed to set OTP on link state");
        return Err(otp_reply(
            channel_type,
            sender_id,
            resources
                .messaging_strings_registry
                .get(KEY_LINK_GENERIC_ERROR, DEFAULT_LOCALE),
        ));
    }

    // Send the OTP code via email
    let channel_display_name = channel_type.to_string();
    let Some(email_svc) = &resources.email_service else {
        warn!("Email service not configured, cannot send OTP for channel linking");
        return Err(otp_reply(
            channel_type,
            sender_id,
            resources
                .messaging_strings_registry
                .get(KEY_LINK_EMAIL_NOT_CONFIGURED, DEFAULT_LOCALE),
        ));
    };

    if let Err(e) = email_svc
        .send_channel_linking_code(email, &otp_code, &channel_display_name)
        .await
    {
        warn!(error = %e, "Failed to send OTP email for channel linking");
        return Err(otp_reply(
            channel_type,
            sender_id,
            resources
                .messaging_strings_registry
                .get(KEY_LINK_EMAIL_SEND_FAILED, DEFAULT_LOCALE),
        ));
    }

    Ok(mask_email(email))
}

/// Handle the email collection step of the OTP flow
///
/// Validates the email, looks up the Pierre user, generates and sends the OTP code.
async fn handle_email_step(
    resources: &ServerResources,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &str,
    sender_id: &str,
    state_id: &str,
    text: &str,
) -> OutgoingMessage {
    if !looks_like_email(text) {
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .messaging_strings_registry
                .get(KEY_LINK_INVALID_EMAIL, DEFAULT_LOCALE),
        );
    }

    let email = text.trim().to_lowercase();

    if let Err(reply) = validate_email_user(resources, channel_type, sender_id, &email).await {
        return reply;
    }

    let masked =
        match generate_and_send_otp(resources, channel_type, sender_id, state_id, &email).await {
            Ok(m) => m,
            Err(reply) => return reply,
        };

    info!(
        channel = %channel,
        tenant_id = %tenant_id,
        sender_id = %sender_id,
        email_masked = %masked,
        "OTP code sent for channel linking"
    );

    let template = resources
        .messaging_strings_registry
        .get(KEY_LINK_OTP_SENT, DEFAULT_LOCALE);
    otp_reply(
        channel_type,
        sender_id,
        format_template(&template, &[&masked]),
    )
}

/// Handle an incorrect OTP code: increment attempts and return feedback
///
/// Invalidates the linking session if max attempts are reached, otherwise
/// returns the remaining attempt count.
async fn handle_otp_mismatch(
    resources: &ServerResources,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &str,
    sender_id: &str,
    state_id: &str,
) -> OutgoingMessage {
    let attempts = db
        .increment_otp_attempts(state_id)
        .await
        .unwrap_or(MAX_OTP_ATTEMPTS);

    if attempts >= MAX_OTP_ATTEMPTS {
        let _ = db
            .invalidate_otp_link_states(tenant_id, channel, sender_id)
            .await;
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .messaging_strings_registry
                .get(KEY_LINK_TOO_MANY_ATTEMPTS, DEFAULT_LOCALE),
        );
    }

    let remaining = (MAX_OTP_ATTEMPTS - attempts).to_string();
    let template = resources
        .messaging_strings_registry
        .get(KEY_LINK_INCORRECT_CODE, DEFAULT_LOCALE);
    otp_reply(
        channel_type,
        sender_id,
        format_template(&template, &[&remaining]),
    )
}

/// Create a permanent channel link for a verified user
///
/// Looks up the user, resolves their tenant, and creates the DB link record.
async fn create_verified_channel_link(
    params: &OtpVerificationParams<'_>,
) -> Result<User, OutgoingMessage> {
    let db_user: &dyn UserRepository = params.resources.repos.users.as_ref();
    let db_msg: &dyn MessagingRepository = params.resources.repos.messaging.as_ref();

    let Ok(Some(user)) = db_user.get_by_email(params.email).await else {
        return Err(otp_reply(
            params.channel_type,
            params.sender_id,
            params
                .resources
                .messaging_strings_registry
                .get(KEY_LINK_VERIFICATION_ERROR, DEFAULT_LOCALE),
        ));
    };

    // Use the bot's tenant for the channel link — the webhook handler resolves
    // tenant from the channel config signature, and get_channel_link queries by
    // that same tenant_id. Using the user's tenant would cause a lookup miss.
    let link_id = Uuid::new_v4().to_string();
    let user_id_str = user.id.to_string();
    let link_params = CreateChannelLinkParams {
        id: &link_id,
        tenant_id: params.tenant_id,
        user_id: &user_id_str,
        channel_type: params.channel,
        channel_user_id: params.sender_id,
        display_name: user.display_name.as_deref(),
    };

    if let Err(e) = db_msg.create_channel_link(&link_params).await {
        warn!(error = %e, "Failed to create channel link during OTP verification");
        return Err(otp_reply(
            params.channel_type,
            params.sender_id,
            params
                .resources
                .messaging_strings_registry
                .get(KEY_LINK_IDENTITY_COLLISION, DEFAULT_LOCALE),
        ));
    }

    // Mark the OTP link state as used
    let _ = db_msg
        .invalidate_otp_link_states(params.tenant_id, params.channel, params.sender_id)
        .await;

    Ok(user)
}

/// Handle the OTP verification step of the linking flow
///
/// Validates the code, checks attempts, and creates the permanent channel link on success.
async fn handle_otp_verification_step(
    params: OtpVerificationParams<'_>,
    text: &str,
) -> OutgoingMessage {
    // Check if input looks like a 6-digit code
    let trimmed = text.trim();
    if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return otp_reply(
            params.channel_type,
            params.sender_id,
            params
                .resources
                .messaging_strings_registry
                .get(KEY_LINK_OTP_PROMPT, DEFAULT_LOCALE),
        );
    }

    let db_msg: &dyn MessagingRepository = params.resources.repos.messaging.as_ref();

    // Hash input and compare against stored hash
    let input_hash = hash_otp(trimmed);

    let active_state = db_msg
        .get_active_otp_link_state(params.tenant_id, params.channel, params.sender_id)
        .await;
    let stored_hash = match &active_state {
        Ok(Some(s)) => s["otp_hash"].as_str().unwrap_or_default().to_owned(),
        _ => {
            return otp_reply(
                params.channel_type,
                params.sender_id,
                params
                    .resources
                    .messaging_strings_registry
                    .get(KEY_LINK_SESSION_EXPIRED, DEFAULT_LOCALE),
            );
        }
    };

    if input_hash != stored_hash {
        return handle_otp_mismatch(
            params.resources,
            db_msg,
            params.tenant_id,
            params.channel_type,
            params.channel,
            params.sender_id,
            params.state_id,
        )
        .await;
    }

    // OTP matches — look up user and create permanent link
    let user = match create_verified_channel_link(&params).await {
        Ok(u) => u,
        Err(reply) => return reply,
    };

    info!(
        channel = %params.channel,
        user_id = %user.id,
        sender_id = %params.sender_id,
        "Account linked via in-chat OTP verification"
    );

    let hashed_tenant = hash_id(&params.tenant_id.to_string());
    let hashed_user = hash_id(&user.id.to_string());
    let hashed_channel_id = hash_id(&format!("{}:{}", params.channel, params.sender_id));
    analytics().alias(&hashed_channel_id, &hashed_user);
    analytics().track_linking_completed(params.channel, &hashed_tenant, &hashed_user, "otp");

    otp_reply(
        params.channel_type,
        params.sender_id,
        params
            .resources
            .messaging_strings_registry
            .get(KEY_LINK_SUCCESS, DEFAULT_LOCALE),
    )
}

/// Start a new OTP linking flow for an unlinked user
///
/// Creates a link state with `otp_step` set (via `set_otp_on_link_state`) and sends a prompt
/// asking for the user's email. The flow uses `awaiting_otp` with an empty email field to
/// represent the "awaiting email" state, and `awaiting_otp` with a non-empty email for
/// the "awaiting OTP code" state.
async fn start_otp_flow(
    resources: &ServerResources,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel_type: ChannelType,
    sender_id: &str,
    sender_name: Option<&str>,
) -> OutgoingMessage {
    // Invalidate any existing OTP flows for this sender before starting a new one
    let channel_str = channel_type.to_string();
    let _ = db
        .invalidate_otp_link_states(tenant_id, &channel_str, sender_id)
        .await;

    let code = generate_link_code();
    let expires_at = Utc::now() + Duration::minutes(OTP_TTL_MINUTES);
    let id = Uuid::new_v4().to_string();

    let params = CreateLinkStateParams {
        id: &id,
        tenant_id,
        user_id: None,
        channel_type: &channel_str,
        code: &code,
        method: "otp",
        channel_user_id: Some(sender_id),
        sender_name,
        expires_at: &expires_at.to_rfc3339(),
    };

    if let Err(e) = db.create_link_state(&params).await {
        warn!(error = %e, "Failed to create OTP link state for unlinked user");
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .messaging_strings_registry
                .get(KEY_LINK_GENERIC_ERROR, DEFAULT_LOCALE),
        );
    }

    // Set otp_step via set_otp_on_link_state (sets to 'awaiting_otp' with empty email,
    // which handle_otp_flow interprets as "awaiting email input")
    if let Err(e) = db.set_otp_on_link_state(&id, "", "").await {
        warn!(error = %e, "Failed to initialize OTP step on link state");
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .messaging_strings_registry
                .get(KEY_LINK_GENERIC_ERROR, DEFAULT_LOCALE),
        );
    }

    otp_reply(
        channel_type,
        sender_id,
        "Hi! To link your Pierre account, please type your email address.\n\
         Type \"cancel\" to stop."
            .to_owned(),
    )
}

/// Persist inbound messages, handling linking, OTP, logout, slash commands, and session resolution
///
/// Returns (`stored_count`, `pending_dispatches`) — the dispatches are processed
/// asynchronously after the webhook returns HTTP 200.
pub(crate) async fn persist_inbound(
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
                warn!(error = %e, "Failed to send channel response");
            }
        });
    }
}

/// Try to handle a slash command from the message text.
///
/// Returns `Some(OutgoingMessage)` if the message was a recognized command,
/// `None` if it should be passed through to the LLM pipeline.
///
/// Commands bypass the LLM entirely for deterministic, fast responses.
#[cfg(feature = "client-messaging")]
#[allow(clippy::too_many_arguments)]
async fn try_handle_slash_command(
    resources: &Arc<ServerResources>,
    channel: &str,
    channel_type: ChannelType,
    session: &ResolvedSession,
    text: &str,
    sender_id: &str,
    conversation_id: Option<&str>,
    thread_id: Option<String>,
) -> Option<OutgoingMessage> {
    use pierre_messaging::commands::CommandMatcher;

    // Fast path: not a command
    if !text.trim().starts_with('/') {
        return None;
    }

    // Access command registries from ServerResources
    {
        use crate::services::commands::PlatformCommandContext;
        use pierre_core::uuid_utils::parse_uuid;

        let cmd_registry = resources.command_registry.as_ref()?;
        let handler_registry = resources.command_handler_registry.as_ref()?;

        let matcher = CommandMatcher::from_registry(cmd_registry);
        let parsed = matcher.try_match(text, cmd_registry)?;

        // Look up handler
        let handler = handler_registry.get(&parsed.name)?;

        // Build platform context
        let user_uuid = parse_uuid(&session.user_id).ok()?;
        let fallback_tenant = TenantId::from_uuid(user_uuid);
        let user_tenant = resolve_user_tenant(resources, &session.user_id, fallback_tenant).await;
        let locale =
            resolve_messaging_locale(resources, user_tenant, user_uuid, channel, sender_id).await;

        let ctx = PlatformCommandContext {
            user_id: user_uuid,
            tenant_id: user_tenant,
            channel_type: channel.to_owned(),
            args: parsed.args,
            raw_text: parsed.raw_text,
            resources: Arc::clone(resources),
            locale,
        };

        // Use conversation_id (group chat) when available, fall back to sender_id
        // for DM platforms — same logic as the LLM dispatch path
        let reply_target = conversation_id.unwrap_or(sender_id).to_owned();

        let hashed_tenant = hash_id(&user_tenant.to_string());
        let hashed_user = hash_id(&session.user_id);

        // Execute command
        match handler.execute(&ctx).await {
            Ok(response) => {
                info!(
                    command = %parsed.name,
                    user_id = %session.user_id,
                    channel = %channel,
                    "Slash command executed"
                );
                analytics().track_command_executed(
                    channel,
                    &hashed_tenant,
                    &hashed_user,
                    &parsed.name,
                    true,
                );
                let content = if response.is_card() {
                    MessageContent::Card {
                        title: response.card_title.unwrap_or_default(),
                        body: response.text,
                        actions: response
                            .actions
                            .into_iter()
                            .map(|a| CardAction {
                                label: a.label,
                                action_type: a.action_type,
                                value: a.value,
                            })
                            .collect(),
                    }
                } else {
                    MessageContent::Text {
                        body: response.text,
                    }
                };
                Some(OutgoingMessage {
                    channel_type,
                    recipient_id: reply_target,
                    content,
                    turn_id: CanotTurnId::new(),
                    reply_to: None,
                    thread_id: thread_id.clone(),
                })
            }
            Err(e) => {
                analytics().track_command_executed(
                    channel,
                    &hashed_tenant,
                    &hashed_user,
                    &parsed.name,
                    false,
                );
                warn!(
                    command = %parsed.name,
                    error = %e,
                    "Slash command execution failed"
                );
                Some(OutgoingMessage {
                    channel_type,
                    recipient_id: reply_target,
                    content: MessageContent::Text {
                        body: format!("Command failed: {e}"),
                    },
                    turn_id: CanotTurnId::new(),
                    reply_to: None,
                    thread_id,
                })
            }
        }
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
    resources: &Arc<ServerResources>,
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

    // Check for slash commands before storing or dispatching to LLM.
    // Commands are handled immediately and not stored in conversation history.
    if let Some(text) = content_body_text(&message.content) {
        if let Some(response) = try_handle_slash_command(
            resources,
            channel,
            channel_type,
            &session,
            &text,
            &message.sender_id,
            message.conversation_id.as_deref(),
            thread_id.clone(),
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

    // Resolve the user's own tenant for tool execution (OAuth, activities, etc.).
    // The webhook tenant is the bot's tenant (channel config), but the user may
    // belong to a different tenant. Tool execution must use the user's tenant so
    // that OAuth connections and data queries match.
    let user_tenant_id = resolve_user_tenant(resources, &session.user_id, tenant_id).await;

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
            Ok(PersistOutcome::StoredWithDispatch(Box::new(
                PendingDispatch {
                    resources: Arc::clone(resources),
                    adapter: Arc::clone(adapter),
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
                    locale: locale.clone(),
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
    resources: &ServerResources,
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
    analytics().track_unlinked_prompted(channel, &hash_id(&tenant_id.to_string()), prompt_type);

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

/// Resolve the user's own tenant for tool execution.
///
/// The webhook tenant is the bot's channel config tenant. The user may belong to
/// a different tenant. This looks up the user's first tenant membership and uses
/// that for tool execution (OAuth, activities), falling back to the webhook tenant.
async fn resolve_user_tenant(
    resources: &ServerResources,
    user_id: &str,
    fallback_tenant_id: TenantId,
) -> TenantId {
    let Ok(user_uuid) = user_id.parse::<Uuid>() else {
        return fallback_tenant_id;
    };
    let db: &dyn TenantRepository = resources.repos.tenants.as_ref();
    match db.list_for_user(user_uuid).await {
        Ok(tenants) if !tenants.is_empty() => {
            let resolved = tenants[0].id;
            if resolved != fallback_tenant_id {
                info!(
                    user_id = %user_id,
                    user_tenant = %resolved,
                    channel_tenant = %fallback_tenant_id,
                    "Using user's own tenant for tool execution (differs from channel tenant)"
                );
            }
            resolved
        }
        _ => fallback_tenant_id,
    }
}

/// Resolve the user-facing locale for a messaging turn.
///
/// Walks the documented fallback chain:
///
/// 1. `messaging_channel_links.locale` for `(tenant, channel, channel_user_id)`
///    — explicit per-channel override (user set Telegram to EN while keeping
///    the web app in FR, for example)
/// 2. `users.locale` — the profile-wide preference edited from the Settings UI
/// 3. [`crate::contremaitre::messaging_strings::DEFAULT_LOCALE`] — hard-coded
///    French fallback
///
/// Never fails: any DB error silently degrades to the next rung. Called once
/// per command/dispatch so handlers and chat-pipeline stages work with a
/// single resolved `String` instead of re-querying.
pub async fn resolve_messaging_locale(
    resources: &ServerResources,
    tenant_id: TenantId,
    user_id: Uuid,
    channel_type: &str,
    channel_user_id: &str,
) -> String {
    if let Ok(Some(override_locale)) = resources
        .repos
        .messaging
        .get_channel_link_locale(tenant_id, channel_type, channel_user_id)
        .await
    {
        if !override_locale.trim().is_empty() {
            return override_locale;
        }
    }

    if let Ok(Some(user)) = resources.repos.users.get_global(user_id).await {
        if !user.locale.trim().is_empty() {
            return user.locale;
        }
    }

    DEFAULT_LOCALE.to_owned()
}

/// Per-conversation dispatch locks ensuring sequential LLM processing.
///
/// Without this, concurrent webhook calls for the same conversation race:
/// message 2's dispatch can finish before message 1's, producing out-of-order
/// replies. The lock serializes dispatches per conversation while allowing
/// different conversations to proceed in parallel.
static CONVERSATION_DISPATCH_LOCKS: LazyLock<DashMap<String, Arc<TokioMutex<()>>>> =
    LazyLock::new(DashMap::new);

/// AG-UI wiring for a single messaging turn.
///
/// Holds the [`RunScope`] (auto-unregisters on drop) plus the
/// [`BroadcastSink`] the pipeline emits events through.
///
/// The `run_id` is generated per-turn — messaging callers don't
/// pass one in the way HTTP web-chat does, so the server owns the
/// identifier and can log it for observability.
///
/// Optionally carries a per-channel `StatusAdapter` + a background
/// consumer task handle.
///
/// When the inbound channel supports in-place progress updates
/// (Telegram/Slack/Discord), the status bridge opens a placeholder
/// message and spawns a task that mirrors AG-UI events as
/// `editMessageText` / `chat.update` / `PATCH messages`.
///
/// On turn completion the caller calls [`Self::finalize_reply`] to
/// collapse the placeholder into the final assistant reply.
struct MessagingAgUiWiring {
    scope: RunScope,
    sink: BroadcastSink,
    thread_id: String,
    status_adapter: Option<Arc<dyn StatusAdapter + Send + Sync>>,
    /// Abort handle for the AG-UI → status adapter consumer task.
    ///
    /// The task normally terminates on its own when the broadcast
    /// closes (i.e. when `scope` drops); the handle is retained so
    /// early errors can stop it promptly rather than wait for the
    /// close event to propagate.
    status_consumer: Option<JoinHandle<()>>,
}

impl MessagingAgUiWiring {
    fn run(&self) -> chat_pipeline::AgUiRun<'_> {
        chat_pipeline::AgUiRun {
            run_id: self.scope.run_id().to_owned(),
            thread_id: Some(self.thread_id.clone()),
            sink: &self.sink,
        }
    }

    /// Render the final assistant reply through the status adapter
    /// (collapsing the placeholder into the final message).
    ///
    /// Returns `true` when the adapter actually sent the reply — the
    /// caller skips the standard `send_outbound_response` path to
    /// avoid posting the reply twice.
    async fn finalize_reply(&self, reply: &str) -> bool {
        let Some(adapter) = self.status_adapter.as_ref() else {
            return false;
        };
        match adapter.finalize(reply).await {
            Ok(()) => true,
            Err(e) => {
                warn!(
                    run_id = %self.scope.run_id(),
                    error = %e,
                    "status adapter finalize failed; falling back to normal send"
                );
                false
            }
        }
    }
}

impl Drop for MessagingAgUiWiring {
    fn drop(&mut self) {
        // When the dispatch errors out before `finalize_reply` runs,
        // abort the consumer task explicitly so it doesn't linger on
        // a broadcast that may still be open (the scope drop unblocks
        // it via `Closed`, but aborting is faster and covers the case
        // where the registry retains a clone of the sender elsewhere).
        if let Some(handle) = self.status_consumer.take() {
            handle.abort();
        }
    }
}

/// Build AG-UI wiring for a messaging dispatch.
///
/// Returns `None` when the user id on the session is not a valid UUID.
///
/// In that case the messaging turn still runs but without progress
/// feedback — the channel link layer already rejects malformed user
/// ids, so this arm is effectively dead in practice.
///
/// When the channel supports in-place progress updates (Telegram,
/// Slack, Discord), this also opens a [`StatusAdapter`] and spawns
/// a background task that mirrors AG-UI events as `editMessageText`
/// / `chat.update` / `PATCH messages` calls.
///
/// `WhatsApp` and Messenger fall through to progress-free dispatch
/// because their APIs cannot edit previously-sent messages.
async fn setup_messaging_agui(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
) -> Option<MessagingAgUiWiring> {
    let user_id = Uuid::parse_str(&dispatch.session.user_id).ok()?;
    let run_id = Uuid::new_v4().to_string();
    let owner = RunOwner::new(user_id, dispatch.user_tenant_id);
    let scope = dispatch
        .resources
        .agui_registry
        .register_scoped(&run_id, owner);
    let sink = BroadcastSink::new(
        (*dispatch.resources.agui_registry).clone(),
        AgUiEventFilter::default(),
    );
    info!(
        run_id = %run_id,
        channel = %dispatch.channel,
        user_id = %user_id,
        "AG-UI run registered for messaging turn"
    );

    // Open the per-channel status adapter and spin up the consumer
    // task *after* the run is registered — otherwise the consumer's
    // `subscribe_self` would race the register call and miss early
    // events even with the replay backlog.
    let (status_adapter, status_consumer) =
        maybe_open_status_bridge(dispatch, channel_config, &run_id).await;

    Some(MessagingAgUiWiring {
        scope,
        sink,
        thread_id: dispatch.session.conversation.clone(),
        status_adapter,
        status_consumer,
    })
}

/// Open a status adapter + consumer task against the pre-loaded
/// channel config.
///
/// Returns `(None, None)` when either:
///
/// - (a) the channel does not support progress rendering (`WhatsApp`/Messenger);
/// - (b) the dispatch lacks a `conversation_id`;
/// - (c) the placeholder send fails.
///
/// Any of these are treated as "skip progress, deliver a single
/// reply at the end" rather than a hard failure. The caller passes
/// the `channel_config` snapshot already loaded by
/// [`dispatch_and_respond`] so the bridge stays out of the DB path.
async fn maybe_open_status_bridge(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    run_id: &str,
) -> (
    Option<Arc<dyn StatusAdapter + Send + Sync>>,
    Option<JoinHandle<()>>,
) {
    let conversation_id = match dispatch.conversation_id.as_deref() {
        Some(c) if !c.is_empty() => c,
        _ => return (None, None),
    };

    let params = OpenStatusParams {
        channel_type: dispatch.channel_type,
        channel_config,
        conversation_id,
        thread_id: dispatch.thread_id.as_deref(),
        // Production always hits the real platform base URLs; the
        // override exists so integration tests can point at a local
        // mock server to verify dispatch routing per `ChannelType`.
        api_base_override: None,
    };
    let Some(adapter) = open_status_adapter(&params).await else {
        return (None, None);
    };

    let consumer = spawn_status_consumer(
        &dispatch.resources.agui_registry,
        run_id.to_owned(),
        Arc::clone(&adapter),
    );
    (Some(adapter), consumer)
}

/// Route the assistant reply back to the user.
///
/// Prefers finalizing the in-channel status placeholder when the
/// bridge is active (so the user sees status and reply collapse into
/// a single chat message). Falls back to the standard outbound send
/// path when the bridge is disabled (`WhatsApp`/Messenger), inactive
/// (credentials missing), or errored mid-turn (edit rejected).
async fn deliver_reply(
    dispatch: &PendingDispatch,
    messaging_agui: Option<&MessagingAgUiWiring>,
    channel_config: &ChannelConfig,
    content: String,
    turn_id: ConversationTurnId,
) {
    if let Some(wiring) = messaging_agui {
        if wiring.finalize_reply(&content).await {
            return;
        }
    }

    // Use conversation_id (channel/chat/thread) as the reply target
    // when available; fall back to sender_id for DM-only platforms
    // (e.g., WhatsApp).
    let reply_target = dispatch
        .conversation_id
        .as_deref()
        .unwrap_or(&dispatch.sender_id)
        .to_owned();

    // The outbound reply carries the turn id from the inbound utterance,
    // so a consumer inspecting the `DeliveryReceipt` can look up the full
    // turn trace via `/internal/conversation-turn`. `.into()` bridges
    // pierre-core's newtype to canot's.
    let outgoing = OutgoingMessage {
        channel_type: dispatch.channel_type,
        recipient_id: reply_target,
        content: MessageContent::Text { body: content },
        turn_id: turn_id.into(),
        reply_to: Some(dispatch.channel_message_id.clone()),
        thread_id: dispatch.thread_id.clone(),
    };
    send_outbound_response(dispatch, channel_config, &outgoing).await;
}

/// Log the pipeline failure, track analytics, and send a localized
/// generic-error reply with a short correlation id.
///
/// Extracted from `dispatch_and_respond` to keep the orchestrator's
/// cognitive complexity inside the workspace lint budget; the body
/// is otherwise a straight line of effects (log, track, template,
/// send) without branching.
async fn report_dispatch_failure(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    hashed_tenant: &str,
    err: &AppError,
) {
    // Correlation ID is surfaced in the user-facing reply and the
    // log record so an operator receiving a Slack alert can grep
    // Cloud Logging for the full error chain without access to
    // conversation IDs (which are PII-adjacent).
    let correlation_id = Uuid::new_v4();
    error!(
        correlation_id = %correlation_id,
        error = %err,
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
        "LLM dispatch failed for messaging"
    );
    analytics().track_error(&dispatch.channel, hashed_tenant, "llm_dispatch_failed");
    let short_id = correlation_id.to_string()[..8].to_owned();
    let template = dispatch
        .resources
        .messaging_strings_registry
        .get(KEY_ERROR_GENERIC, &dispatch.locale);
    let user_message = format_template(&template, &[&short_id]);
    send_error_reply(dispatch, channel_config, &user_message).await;
}

/// Dispatch a message through the LLM pipeline and send the response back via the channel
///
/// Runs as a background task after the webhook has returned HTTP 200.
/// Acquires a per-conversation lock to ensure messages are processed in order.
pub(crate) async fn dispatch_and_respond(dispatch: PendingDispatch) {
    let lock = CONVERSATION_DISPATCH_LOCKS
        .entry(dispatch.session.conversation.clone())
        .or_insert_with(|| Arc::new(TokioMutex::new(())))
        .clone();
    let dispatch_guard = lock.lock().await;

    let start = Instant::now();
    let hashed_tenant = hash_id(&dispatch.channel_tenant_id.to_string());
    let hashed_user = hash_id(&dispatch.session.user_id);

    let profile = build_messaging_profile(&dispatch);
    // Generate the turn id here — the messaging inbound webhook is the
    // boundary for platform-side observability. A single inbound message
    // plus its full LLM/tool chain is one turn.
    let turn_id = ConversationTurnId::new();
    let turn_input = TurnInput {
        conversation_id: dispatch.session.conversation.clone(),
        user_id: dispatch.session.user_id.clone(),
        conversation_tenant_id: dispatch.channel_tenant_id,
        tool_tenant_id: dispatch.user_tenant_id,
        content: dispatch.text_content.clone(),
        locale: Some(dispatch.locale.clone()),
        turn_id,
    };

    // Load the per-tenant channel config exactly once per turn.
    //
    // Both the AG-UI status bridge (placeholder open + finalize) and
    // the fallback outbound send need it; threading the same snapshot
    // through avoids the 2-3 DB round-trips a naive wiring would
    // incur, and keeps every reply-path consistent with the
    // credentials live at the moment the dispatch started.
    //
    // `None` means the tenant has no configured channel — we cannot
    // reply at all, so log and bail without spending compute on the
    // LLM pipeline.
    let db: &dyn MessagingRepository = dispatch.resources.repos.messaging.as_ref();
    let Some(channel_config) =
        load_channel_config(db, dispatch.channel_tenant_id, &dispatch.channel).await
    else {
        warn!(
            channel = %dispatch.channel,
            tenant_id = %dispatch.channel_tenant_id,
            "channel config unavailable at dispatch time; dropping turn with no reply"
        );
        drop(dispatch_guard);
        evict_idle_dispatch_lock(&dispatch.session.conversation, &lock);
        return;
    };

    // Register an AG-UI run for this messaging turn so in-process
    // consumers (channel-side status adapters, ops dashboards) can
    // subscribe via `resources.agui_registry.subscribe_self(run_id)`
    // and render pipeline progress to Telegram/Slack/Discord.
    // Without this wiring the pipeline's `hooks.agui` stayed `None` on
    // every messaging turn and AG-UI events were produced only for
    // HTTP web-chat callers that passed `agui_run_id`.
    //
    // The run is owned by `(session.user_id, user_tenant_id)` so any
    // cross-user HTTP subscriber (e.g. canot's `AgUiConsumer` against
    // the platform SSE route) still goes through the owner check in
    // `RunRegistry::authorize_and_subscribe`. Scope drops at function
    // exit, auto-unregistering on success, error, or panic.
    //
    // `setup_messaging_agui` also opens an in-channel status adapter
    // (Telegram editMessageText / Slack chat.update / Discord PATCH
    // messages) and spawns a background consumer that mirrors each
    // AG-UI event as a `set_status` call so the user sees the pipeline
    // stage in real time.
    let messaging_agui = setup_messaging_agui(&dispatch, &channel_config).await;
    let hooks = PipelineHooks {
        agui: messaging_agui.as_ref().map(|w| w.run()),
        ..PipelineHooks::none()
    };

    let dispatch_result =
        match chat_pipeline::run(&dispatch.resources, turn_input, &profile, &hooks).await {
            Ok(result) => result,
            Err(e) => {
                report_dispatch_failure(&dispatch, &channel_config, &hashed_tenant, &e).await;
                return;
            }
        };

    // Safe cast: execution time will never exceed u64::MAX milliseconds (~584 million years)
    #[allow(clippy::cast_possible_truncation)]
    let execution_time_ms = start.elapsed().as_millis() as u64;

    analytics().track_bot_response(
        &dispatch.channel,
        &hashed_tenant,
        &hashed_user,
        "llm",
        execution_time_ms,
        &dispatch_result.model,
    );

    // Record LLM usage for cost tracking and quota enforcement
    record_messaging_llm_usage(&dispatch, &dispatch_result, execution_time_ms).await;

    // Increment usage counters (message count, token count, tool call count)
    increment_messaging_usage_counters(&dispatch, &dispatch_result).await;

    // Guard: skip sending empty responses. The LLM occasionally returns empty
    // content (e.g., when the input is too technical or the context is exhausted).
    // Telegram rejects empty message text with HTTP 400.
    if dispatch_result.content.trim().is_empty() {
        warn!(
            conversation_id = %dispatch.session.conversation,
            "LLM returned empty response, sending fallback"
        );
        let empty_reply = dispatch
            .resources
            .messaging_strings_registry
            .get(KEY_EMPTY_REPLY, &dispatch.locale);
        send_error_reply(&dispatch, &channel_config, &empty_reply).await;
        return;
    }

    deliver_reply(
        &dispatch,
        messaging_agui.as_ref(),
        &channel_config,
        dispatch_result.content,
        dispatch_result.turn_id,
    )
    .await;

    // Dropping the wiring here aborts the consumer task (if still
    // live) and releases the RunScope so the registry entry is
    // cleaned up. Held until after `deliver_reply` so any
    // last events the pipeline emitted on the way out still render.
    drop(messaging_agui);

    // Held until here to serialize dispatches for the same conversation
    drop(dispatch_guard);
    evict_idle_dispatch_lock(&dispatch.session.conversation, &lock);
}

/// Remove the per-conversation lock from the shared map if no other task still holds it.
///
/// Prevents unbounded growth of `CONVERSATION_DISPATCH_LOCKS` under
/// high conversation cardinality while staying safe: if a concurrent
/// dispatch cloned the `Arc` before we got here, the strong count
/// exceeds 2 and we leave the entry in place. The next waiter will
/// simply reinsert on a later call if it was already evicted.
fn evict_idle_dispatch_lock(conversation_id: &str, local: &Arc<TokioMutex<()>>) {
    // Strong references: the one in the DashMap entry + `local` held here.
    // Any higher count means another dispatch task is waiting on this lock.
    CONVERSATION_DISPATCH_LOCKS.remove_if(conversation_id, |_, stored| {
        Arc::ptr_eq(stored, local) && Arc::strong_count(stored) <= 2
    });
}

/// Send a user-facing error message when LLM dispatch fails or returns empty content.
///
/// Ensures the user always gets feedback instead of silence when something goes wrong.
async fn send_error_reply(dispatch: &PendingDispatch, channel_config: &ChannelConfig, body: &str) {
    let reply_target = dispatch
        .conversation_id
        .as_deref()
        .unwrap_or(&dispatch.sender_id)
        .to_owned();

    let outgoing = OutgoingMessage {
        channel_type: dispatch.channel_type,
        recipient_id: reply_target,
        content: MessageContent::Text {
            body: body.to_owned(),
        },
        // Error replies emit a fresh turn id — the chat pipeline never
        // reached the point where a turn would have been recorded, so
        // there's no upstream id to thread. The platform records a
        // terminal failed turn under this id so operators can correlate.
        turn_id: CanotTurnId::new(),
        reply_to: Some(dispatch.channel_message_id.clone()),
        thread_id: dispatch.thread_id.clone(),
    };

    send_outbound_response(dispatch, channel_config, &outgoing).await;
}

/// Extract real token counts from provider usage, or estimate from content length.
///
/// Returns `(prompt_tokens, completion_tokens)` as i64 for direct use in usage recording.
/// When real usage is unavailable (CLI-based providers), estimates from the user message
/// and completion text using a character-based heuristic.
fn estimate_or_extract_messaging_tokens(
    usage: Option<&TokenUsage>,
    user_text: &str,
    completion_text: &str,
) -> (i64, i64) {
    usage.map_or_else(
        || {
            let (est_prompt, est_completion) = estimate_chat_tokens(user_text, completion_text);
            debug!(
                est_prompt,
                est_completion,
                "Using estimated token counts for messaging (provider returned no usage)"
            );
            (i64::from(est_prompt), i64::from(est_completion))
        },
        |u| (i64::from(u.prompt_tokens), i64::from(u.completion_tokens)),
    )
}

/// Record the terminal turn-summary row after a messaging dispatch.
///
/// Per-LLM-call token counts are already written by the chat pipeline's
/// [`crate::services::chat_pipeline::UsageRepoCallRecorder`] — this
/// row carries only the turn-level aggregates (`tool_calls_count`,
/// `tools_called`, end-to-end `execution_time_ms`) and is tagged with
/// [`pierre_core::models::TURN_SUMMARY_CALL_TYPE`] so aggregate
/// analytics can exclude it.
async fn record_messaging_llm_usage(
    dispatch: &PendingDispatch,
    result: &DispatchResult,
    execution_time_ms: u64,
) {
    let tenant_id_str = dispatch.channel_tenant_id.to_string();

    #[allow(clippy::cast_possible_wrap)]
    let exec_time = execution_time_ms as i64;

    let tools_called_json = serde_json::to_string(&result.tools_called).unwrap_or_else(|e| {
        warn!("Failed to serialize tools_called for messaging, storing empty array: {e}");
        "[]".to_owned()
    });

    if let Err(e) = dispatch
        .resources
        .repos
        .llm_usage
        .insert_llm_usage(&InsertLlmUsage {
            tenant_id: &tenant_id_str,
            user_id: &dispatch.session.user_id,
            conversation_id: Some(&dispatch.session.conversation),
            turn_id: result.turn_id,
            provider: &result.provider_name,
            model: &result.model,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            call_type: TURN_SUMMARY_CALL_TYPE,
            tool_calls_count: i64::from(result.tool_calls_count),
            tools_called: &tools_called_json,
            execution_time_ms: Some(exec_time),
        })
        .await
    {
        warn!("Failed to record LLM usage summary for messaging: {e}");
    }
}

/// Increment usage counters (messages, tokens, tool calls) after a messaging dispatch
async fn increment_messaging_usage_counters(dispatch: &PendingDispatch, result: &DispatchResult) {
    let Some(ref admin_config) = dispatch.resources.admin_config else {
        return;
    };

    let tenant_id_str = dispatch.channel_tenant_id.to_string();
    let usage_svc = UsageCounterService::new(
        dispatch.resources.repos.usage_counters.as_ref(),
        admin_config,
    );

    // Use real token counts when available, fall back to estimation for CLI providers
    let (est_prompt, est_completion) = estimate_or_extract_messaging_tokens(
        result.usage.as_ref(),
        &dispatch.text_content,
        &result.content,
    );
    let total_tokens = est_prompt + est_completion;

    // Build list of (counter_type, amount) pairs to increment
    let mut counters: Vec<(&str, i64)> = vec![("daily_messages", 1), ("weekly_messages", 1)];
    if total_tokens > 0 {
        counters.push(("daily_tokens", total_tokens));
        counters.push(("weekly_tokens", total_tokens));
    }
    if result.tool_calls_count > 0 {
        let tool_calls = i64::from(result.tool_calls_count);
        counters.push(("daily_tool_calls", tool_calls));
        counters.push(("weekly_tool_calls", tool_calls));
    }

    for (counter_type, amount) in counters {
        if let Err(e) = usage_svc
            .increment(
                &tenant_id_str,
                &dispatch.session.user_id,
                counter_type,
                amount,
            )
            .await
        {
            warn!("Failed to increment {counter_type} counter for messaging: {e}");
        }
    }
}

/// Load channel config, send outbound message, and persist the result
async fn send_outbound_response(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    outgoing: &OutgoingMessage,
) {
    let db: &dyn MessagingRepository = dispatch.resources.repos.messaging.as_ref();

    match dispatch.adapter.send(outgoing, channel_config).await {
        Ok(receipt) => {
            let channel_msg_id = receipt.channel_message_id.as_deref().unwrap_or("");
            info!(
                channel_message_id = %channel_msg_id,
                channel = %dispatch.channel,
                "Outbound message sent successfully"
            );
            persist_outbound_message(db, dispatch, channel_msg_id, outgoing).await;
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
    let correlation_str = outgoing.turn_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        tenant_id: dispatch.channel_tenant_id,
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

    // Persist the outbound message record first (FK requirement for queue entry).
    // Use a unique retry-prefixed ID to avoid colliding with the (tenant_id, channel_message_id)
    // uniqueness constraint — retry messages have no real channel ID yet.
    let out_msg_id = Uuid::new_v4().to_string();
    let retry_channel_msg_id = format!("retry-{out_msg_id}");
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.turn_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        tenant_id: dispatch.channel_tenant_id,
        session_id: &dispatch.session.session_id,
        direction: "outbound",
        channel_type: &dispatch.channel,
        channel_message_id: &retry_channel_msg_id,
        sender_id: "pierre",
        content_type: "text",
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: Some(&payload_str),
    };
    let inserted = db.insert_message(&out_params).await?;
    if !inserted {
        return Err(AppError::internal(
            "Failed to persist retry message: duplicate channel_message_id",
        ));
    }

    let queue_id = Uuid::new_v4().to_string();
    db.enqueue_outbound(
        &queue_id,
        &out_msg_id,
        dispatch.channel_tenant_id,
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
