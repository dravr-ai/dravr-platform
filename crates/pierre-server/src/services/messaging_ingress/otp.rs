// ABOUTME: In-chat OTP linking flow + logout + supporting helpers (email validation, code gen, replies)
// ABOUTME: Drives the awaiting-email -> awaiting-otp -> verified state machine over MessagingRepository

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{Duration, Utc};
use pierre_core::models::messaging::{
    ChannelType, MessageContent, OutgoingMessage, MAX_OTP_ATTEMPTS, OTP_TTL_MINUTES,
};
use pierre_core::models::{TenantId, User};
use pierre_database::backends::{
    CreateChannelLinkParams, CreateLinkStateParams, MessagingRepository, TenantRepository,
    UserRepository,
};
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use rand::Rng;
use sha2::{Digest, Sha256};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::contremaitre::messaging_strings::{
    format_template, DEFAULT_LOCALE, KEY_LINK_CANCELLED, KEY_LINK_EMAIL_NOT_CONFIGURED,
    KEY_LINK_EMAIL_SEND_FAILED, KEY_LINK_GENERIC_ERROR, KEY_LINK_IDENTITY_COLLISION,
    KEY_LINK_INCORRECT_CODE, KEY_LINK_INVALID_EMAIL, KEY_LINK_LOGOUT_COMPLETE, KEY_LINK_NO_ACCOUNT,
    KEY_LINK_NO_TENANT, KEY_LINK_OTP_PROMPT, KEY_LINK_OTP_SENT, KEY_LINK_SESSION_EXPIRED,
    KEY_LINK_TOO_MANY_ATTEMPTS, KEY_LINK_VERIFICATION_ERROR,
};
use crate::mcp::resources::ServerContext;
use crate::routes::messaging::linking::generate_link_code;
use crate::services::analytics::{analytics, hash_id};

/// Parameters for the OTP code verification step of the channel linking flow
struct OtpVerificationParams<'a> {
    resources: &'a ServerContext,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &'a str,
    sender_id: &'a str,
    state_id: &'a str,
    email: &'a str,
}

/// Check if a message is a cancel command for the OTP linking flow
fn is_cancel_command(content: &MessageContent) -> bool {
    matches!(content, MessageContent::Text { body } if body.trim().eq_ignore_ascii_case("cancel"))
}

/// Check if a message is a logout/disconnect command
pub(super) fn is_logout_command(content: &MessageContent) -> bool {
    matches!(content, MessageContent::Text { body } if body.trim().eq_ignore_ascii_case("logout"))
}

/// Handle logout: delete channel link, sessions, and OTP states atomically
pub(super) async fn handle_logout(
    resources: &ServerContext,
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
        error!(error = %e, "Failed to logout channel sender");
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
pub(super) fn otp_reply(
    channel_type: ChannelType,
    sender_id: &str,
    body: String,
) -> OutgoingMessage {
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
pub(super) fn apply_conversation_recipient(
    msg: &mut OutgoingMessage,
    conversation_id: Option<&str>,
) {
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
pub(super) async fn handle_otp_flow(
    resources: &ServerContext,
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
    resources: &ServerContext,
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
            error!(error = %e, "Failed to look up user by email during OTP flow");
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
            error!(error = %e, user_id = %user.id, "Failed to list tenants for user during OTP flow");
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
    resources: &ServerContext,
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
        error!(error = %e, "Failed to set OTP on link state");
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
        error!(error = %e, "Failed to send OTP email for channel linking");
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
    resources: &ServerContext,
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
    resources: &ServerContext,
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
        error!(error = %e, "Failed to create channel link during OTP verification");
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

    // Run the unified channel-auth path so the post-OTP reply matches what
    // the next inbound message will produce. Pending / Suspended users see
    // the translated denial copy at link time; everyone else sees the
    // localized link-success template.
    let body = super::linking::link_time_reply(
        params.resources,
        params.tenant_id,
        params.channel,
        params.sender_id,
    )
    .await;
    otp_reply(params.channel_type, params.sender_id, body)
}

/// Start a new OTP linking flow for an unlinked user
///
/// Creates a link state with `otp_step` set (via `set_otp_on_link_state`) and sends a prompt
/// asking for the user's email. The flow uses `awaiting_otp` with an empty email field to
/// represent the "awaiting email" state, and `awaiting_otp` with a non-empty email for
/// the "awaiting OTP code" state.
pub(super) async fn start_otp_flow(
    resources: &ServerContext,
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
        error!(error = %e, "Failed to create OTP link state for unlinked user");
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
        error!(error = %e, "Failed to initialize OTP step on link state");
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
