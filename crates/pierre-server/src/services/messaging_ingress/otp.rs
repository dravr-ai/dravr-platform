// ABOUTME: In-chat OTP linking flow + logout + supporting helpers (email validation, code gen, replies)
// ABOUTME: Drives the awaiting-email -> awaiting-otp -> verified state machine over MessagingRepository

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{Duration, Utc};
use pierre_core::models::messaging::{
    channel_label, ChannelType, IncomingMessage, MessageContent, OutgoingMessage, MAX_OTP_ATTEMPTS,
    OTP_TTL_MINUTES,
};
use pierre_core::models::{normalize_email, TenantId, User};
use pierre_database::backends::{
    CreateChannelLinkParams, CreateLinkStateParams, MessagingRepository, TenantRepository,
    UserRepository,
};
use rand::Rng;
use secrecy::SecretString;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::addressing::reply_recipient;
use super::linking::{self, AWAITING_LINK_CONFIRM};
use super::prelink::prelink_locale;
use crate::mcp::resources::ServerContext;
use crate::routes::messaging::linking::generate_link_code;
use pierre_auth::dto::auth::{RegisterRequest, RegisterResponse};
use pierre_contremaitre::messaging_strings::{
    format_template, KEY_LINK_CANCELLED, KEY_LINK_EMAIL_NOT_CONFIGURED, KEY_LINK_EMAIL_PROMPT,
    KEY_LINK_EMAIL_SEND_FAILED, KEY_LINK_GENERIC_ERROR, KEY_LINK_IDENTITY_COLLISION,
    KEY_LINK_INCORRECT_CODE, KEY_LINK_INVALID_EMAIL, KEY_LINK_LOGOUT_COMPLETE, KEY_LINK_NO_TENANT,
    KEY_LINK_OTP_PROMPT, KEY_LINK_OTP_SENT, KEY_LINK_SESSION_EXPIRED, KEY_LINK_SIGNUP_CREATED,
    KEY_LINK_SIGNUP_FAILED, KEY_LINK_SIGNUP_OFFER, KEY_LINK_TOO_MANY_ATTEMPTS,
    KEY_LINK_VERIFICATION_ERROR,
};
use pierre_core::errors::AppError;
use pierre_middleware::redaction::{mask_email_for_display, mask_recipient};
use pierre_services::locale::resolve_channel_locale;
use pierre_services::messaging_broadcast::proactive_text;

/// One step of a sender's live in-chat linking flow, as its link state holds it
pub(super) struct OtpFlowStep<'a> {
    pub(super) resources: &'a ServerContext,
    pub(super) tenant_id: TenantId,
    pub(super) channel_type: ChannelType,
    pub(super) channel: &'a str,
    pub(super) sender_id: &'a str,
    state_id: &'a str,
    /// The address the flow links; empty while it waits for one.
    email: &'a str,
    /// The language every reply of the flow is written in.
    pub(super) locale: &'a str,
}

/// The language a sender's live linking flow speaks: the one its link state
/// was opened with, so the flow never switches language mid-way, else
/// `prelink`, what the channel suggests now, for a state that holds none.
fn flow_locale<'a>(state: &'a Value, prelink: &'a str) -> &'a str {
    state["locale"].as_str().unwrap_or(prelink)
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
///
/// A sender who was never linked is answered in the language the channel
/// suggests for them ([`prelink_locale`]).
pub(super) async fn handle_logout(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &str,
    message: &IncomingMessage,
) -> OutgoingMessage {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let sender_id = message.sender_id.as_str();

    // Read before the link goes: afterwards the sender is a stranger again.
    let prelink = prelink_locale(message);
    let locale = logout_locale(resources, tenant_id, channel, sender_id, prelink).await;

    if let Err(e) = db
        .logout_channel_sender(tenant_id, channel, sender_id)
        .await
    {
        error!(error = %e, "Failed to logout channel sender");
    }

    info!(
        channel = %channel,
        sender_id = %mask_recipient(sender_id),
        "User logged out from messaging channel"
    );

    // Logout fires before the channel link is re-resolved, so the only
    // identity in scope is the channel `sender_id`. The event carries the
    // pre-link identity hash, never the sender id, and lands in Cloud Logging
    // only.
    info!(
        target: "notify",
        event = "messaging.session_dropped",
        user_id = %super::pre_link_identity(channel, sender_id),
        tenant_id = %tenant_id,
        channel = %channel,
        reason = "logout",
        "messaging session dropped"
    );

    otp_reply(
        channel_type,
        sender_id,
        resources
            .mcp
            .messaging_strings_registry
            .get(KEY_LINK_LOGOUT_COMPLETE, &locale),
    )
}

/// The language a logout is acknowledged in, read before the link it deletes:
/// the athlete's own when the sender is linked, else `prelink`.
async fn logout_locale(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    prelink: &str,
) -> String {
    let messaging: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let linked_user: Option<Uuid> = messaging
        .get_channel_link(tenant_id, channel, sender_id)
        .await
        .ok()
        .flatten()
        .and_then(|link| link["user_id"].as_str().map(Uuid::parse_str))
        .and_then(Result::ok);
    match linked_user {
        Some(user_id) => {
            resolve_channel_locale(
                messaging,
                resources.common.repos.users.as_ref(),
                tenant_id,
                channel,
                sender_id,
                Some(user_id),
            )
            .await
        }
        None => prelink.to_owned(),
    }
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
    proactive_text(channel_type, sender_id.to_owned(), body)
}

/// Override `recipient_id` with conversation ID for channel-based platforms like Discord
///
/// Discord REST API sends to channels, not users. If the message came from a guild
/// channel, we must reply to that channel — not to the user ID. Delegates the
/// present-and-non-empty rule to [`reply_recipient`] (the single source of
/// truth for the conversation-id-or-user-id choice), so a `Some("")`
/// conversation id from an inbound webhook can never overwrite a valid sender
/// recipient with an empty one — the silent-drop class fixed in 5df2c1706.
pub(super) fn apply_conversation_recipient(
    msg: &mut OutgoingMessage,
    conversation_id: Option<&str>,
) {
    if msg.channel_type == ChannelType::Discord || msg.channel_type == ChannelType::Slack {
        msg.recipient_id = reply_recipient(conversation_id, &msg.recipient_id).to_owned();
    }
}

/// Handle an in-chat OTP linking flow step
///
/// Returns `Some(OutgoingMessage)` if the OTP flow handled the message (reply to send),
/// or `None` if no active OTP flow exists (proceed to normal routing). A flow that
/// recorded no language answers in the one the channel suggests ([`prelink_locale`]).
pub(super) async fn handle_otp_flow(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel_type: ChannelType,
    channel: &str,
    message: &IncomingMessage,
) -> Option<OutgoingMessage> {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let sender_id = message.sender_id.as_str();
    let content = &message.content;
    let prelink = prelink_locale(message);

    // Handle cancel command: invalidate any active flow
    if is_cancel_command(content) {
        if let Ok(Some(state)) = db
            .get_active_otp_link_state(tenant_id, channel, sender_id)
            .await
        {
            let locale = flow_locale(&state, prelink);
            if state["otp_step"].as_str() == Some(AWAITING_LINK_CONFIRM) {
                // A pairing code belongs to the account that started it: the
                // sender who declines hands it back unspent (carnet#1596).
                if let Err(e) = db
                    .release_link_confirmations(tenant_id, channel, sender_id)
                    .await
                {
                    error!(error = %e, "Failed to release a held pairing code on cancel");
                }
            } else {
                invalidate_otp_flows(db, tenant_id, channel, sender_id, "on cancel").await;
            }
            return Some(otp_reply(
                channel_type,
                sender_id,
                resources
                    .mcp
                    .messaging_strings_registry
                    .get(KEY_LINK_CANCELLED, locale),
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
        // A held pairing code waits for a yes, so a photo or a sticker asks
        // again. Falling through would open an OTP flow that spends the code.
        _ if otp_step == AWAITING_LINK_CONFIRM => String::new(),
        _ => return None,
    };

    // Distinguish sub-states: awaiting_otp with empty email = awaiting email input
    let email = state["email"].as_str().unwrap_or_default().to_owned();
    let params = OtpFlowStep {
        resources,
        tenant_id,
        channel_type,
        channel,
        sender_id,
        state_id: &state_id,
        email: &email,
        locale: flow_locale(&state, prelink),
    };

    match otp_step {
        AWAITING_LINK_CONFIRM => Some(linking::confirm_held_link(&params, &state, &text).await),
        "awaiting_otp" if email.is_empty() => Some(handle_email_step(&params, &text).await),
        // The address is known and has no account: the reply is a yes/no on
        // creating one, not an OTP code.
        "awaiting_signup" => Some(handle_signup_confirm_step(&params, &text).await),
        "awaiting_otp" => Some(handle_otp_verification_step(&params, &text).await),
        _ => None,
    }
}

/// Validate user exists and belongs to at least one tenant
///
/// Returns the user on success, or an error reply for the caller to return.
/// Why an address could not be linked to an existing account.
///
/// `NoAccount` is deliberately not an `OutgoingMessage`: it is not an error to
/// show the user, it is the branch that offers to create the account.
pub(super) enum EmailLookupFailure {
    /// The address is unknown — offer to sign them up.
    NoAccount,
    /// A real failure the user should see as-is.
    Reply(Box<OutgoingMessage>),
}

use EmailLookupFailure::{NoAccount, Reply};

impl From<OutgoingMessage> for EmailLookupFailure {
    fn from(message: OutgoingMessage) -> Self {
        Reply(Box::new(message))
    }
}

async fn validate_email_user(
    resources: &ServerContext,
    channel_type: ChannelType,
    sender_id: &str,
    email: &str,
    locale: &str,
) -> Result<User, EmailLookupFailure> {
    let db_user: &dyn UserRepository = resources.common.repos.users.as_ref();

    // Cross-tenant user lookup by email
    let user = match db_user.get_by_email(email).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            // Signals "no account yet" to the caller, which parks the flow on
            // `awaiting_signup` and offers to create one. This used to hand out
            // a register-at-URL message and end the exchange — a dead end at the
            // highest-intent moment the funnel gets, since the person had
            // already opened a conversation and asked a real question.
            return Err(NoAccount);
        }
        Err(e) => {
            error!(error = %e, "Failed to look up user by email during OTP flow");
            return Err(otp_reply(
                channel_type,
                sender_id,
                resources
                    .mcp
                    .messaging_strings_registry
                    .get(KEY_LINK_GENERIC_ERROR, locale),
            )
            .into());
        }
    };

    // Verify user belongs to a tenant (shared bot model: accept any tenant the user belongs to)
    let db_tenant: &dyn TenantRepository = resources.common.repos.tenants.as_ref();
    let tenants = match db_tenant.list_for_user(user.id).await {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, user_id = %user.id, "Failed to list tenants for user during OTP flow");
            return Err(otp_reply(
                channel_type,
                sender_id,
                resources
                    .mcp
                    .messaging_strings_registry
                    .get(KEY_LINK_GENERIC_ERROR, locale),
            )
            .into());
        }
    };

    if tenants.is_empty() {
        return Err(otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_NO_TENANT, locale),
        )
        .into());
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
    locale: &str,
) -> Result<String, Box<OutgoingMessage>> {
    let db_msg: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    let otp_code = generate_otp();
    let otp_hashed = hash_otp(&otp_code);

    if let Err(e) = db_msg
        .set_otp_on_link_state(state_id, email, &otp_hashed)
        .await
    {
        error!(error = %e, "Failed to set OTP on link state");
        return Err(Box::new(otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_GENERIC_ERROR, locale),
        )));
    }

    // Send the OTP code via email, naming the channel as the athlete knows it
    let Some(email_svc) = &resources.common.email_service else {
        warn!("Email service not configured, cannot send OTP for channel linking");
        return Err(Box::new(otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_EMAIL_NOT_CONFIGURED, locale),
        )));
    };

    if let Err(e) = email_svc
        .send_channel_linking_code(email, &otp_code, channel_label(&channel_type.to_string()))
        .await
    {
        error!(error = %e, "Failed to send OTP email for channel linking");
        return Err(Box::new(otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_EMAIL_SEND_FAILED, locale),
        )));
    }

    Ok(mask_email_for_display(email))
}

/// Handle the email collection step of the OTP flow
///
/// Validates the email, looks up the Pierre user, generates and sends the OTP code.
async fn handle_email_step(params: &OtpFlowStep<'_>, text: &str) -> OutgoingMessage {
    let OtpFlowStep {
        resources,
        tenant_id,
        channel_type,
        channel,
        sender_id,
        state_id,
        locale,
        ..
    } = *params;
    if !looks_like_email(text) {
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_INVALID_EMAIL, locale),
        );
    }

    let email = normalize_email(text);

    match validate_email_user(resources, channel_type, sender_id, &email, locale).await {
        Ok(_) => {}
        Err(Reply(reply)) => return *reply,
        Err(NoAccount) => {
            return offer_signup(resources, channel_type, sender_id, state_id, &email, locale)
                .await;
        }
    }

    let masked =
        match generate_and_send_otp(resources, channel_type, sender_id, state_id, &email, locale)
            .await
        {
            Ok(m) => m,
            Err(reply) => return *reply,
        };

    info!(
        channel = %channel,
        tenant_id = %tenant_id,
        sender_id = %mask_recipient(sender_id),
        email_masked = %masked,
        "OTP code sent for channel linking"
    );

    let template = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_LINK_OTP_SENT, locale);
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
    params: &OtpFlowStep<'_>,
    db: &dyn MessagingRepository,
) -> OutgoingMessage {
    let OtpFlowStep {
        resources,
        tenant_id,
        channel_type,
        channel,
        sender_id,
        state_id,
        locale,
        ..
    } = *params;
    let attempts = db
        .increment_otp_attempts(state_id)
        .await
        .unwrap_or(MAX_OTP_ATTEMPTS);

    if attempts >= MAX_OTP_ATTEMPTS {
        invalidate_otp_flows(db, tenant_id, channel, sender_id, "after max attempts").await;
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_TOO_MANY_ATTEMPTS, locale),
        );
    }

    let remaining = (MAX_OTP_ATTEMPTS - attempts).to_string();
    let template = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_LINK_INCORRECT_CODE, locale);
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
    params: &OtpFlowStep<'_>,
) -> Result<User, Box<OutgoingMessage>> {
    let db_user: &dyn UserRepository = params.resources.common.repos.users.as_ref();
    let db_msg: &dyn MessagingRepository = params.resources.common.repos.messaging.as_ref();

    let Ok(Some(user)) = db_user.get_by_email(params.email).await else {
        return Err(Box::new(otp_reply(
            params.channel_type,
            params.sender_id,
            params
                .resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_VERIFICATION_ERROR, params.locale),
        )));
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
        // An identity already linked to another account is the sender's
        // situation, and its error names the sender's phone number, so only
        // the insert failing is logged with its detail.
        if e.is_server_fault() {
            error!(error = %e, "Failed to create channel link during OTP verification");
        } else {
            warn!(channel = %params.channel, "OTP verification refused: the channel identity is already linked");
        }
        return Err(Box::new(otp_reply(
            params.channel_type,
            params.sender_id,
            params
                .resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_IDENTITY_COLLISION, params.locale),
        )));
    }

    // Mark the OTP link state as used
    invalidate_otp_flows(
        db_msg,
        params.tenant_id,
        params.channel,
        params.sender_id,
        "after verification",
    )
    .await;

    stamp_verified_after_otp(params.resources, user.id).await;

    Ok(user)
}

/// Record the inbox proof the OTP already gave us, then re-run the approval
/// decision.
///
/// The code was mailed to this address and typed back, which is the same proof
/// the confirmation link provides — so record it rather than asking the user to
/// prove the same inbox twice. Idempotent: a user who already confirmed keeps
/// their original timestamp, and whether it promotes them is the same decision
/// every other surface uses, since in-chat signup gets no special path.
///
/// Best-effort on both writes. The link exists and the user is already talking
/// to us; failing the turn over bookkeeping would be the wrong trade.
async fn stamp_verified_after_otp(resources: &ServerContext, user_id: Uuid) {
    let first_proof = match resources
        .common
        .repos
        .email_verification
        .mark_verified(user_id)
        .await
    {
        Ok(first_proof) => first_proof,
        Err(e) => {
            warn!(error = %e, user_id = %user_id, "failed to stamp email verified after OTP");
            false
        }
    };

    if let Err(e) = resources
        .account_service()
        .apply_approval_after_verification(user_id, first_proof)
        .await
    {
        warn!(error = %e, user_id = %user_id, "approval decision failed after OTP verification");
    }
}

/// Handle the OTP verification step of the linking flow
///
/// Validates the code, checks attempts, and creates the permanent channel link on success.
async fn handle_otp_verification_step(params: &OtpFlowStep<'_>, text: &str) -> OutgoingMessage {
    // Check if input looks like a 6-digit code
    let trimmed = text.trim();
    if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return otp_reply(
            params.channel_type,
            params.sender_id,
            params
                .resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_OTP_PROMPT, params.locale),
        );
    }

    let db_msg: &dyn MessagingRepository = params.resources.common.repos.messaging.as_ref();

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
                    .mcp
                    .messaging_strings_registry
                    .get(KEY_LINK_SESSION_EXPIRED, params.locale),
            );
        }
    };

    if input_hash != stored_hash {
        return handle_otp_mismatch(params, db_msg).await;
    }

    // OTP matches — look up user and create permanent link
    let user = match create_verified_channel_link(params).await {
        Ok(u) => u,
        Err(reply) => return *reply,
    };

    info!(
        channel = %params.channel,
        user_id = %user.id,
        sender_id = %mask_recipient(params.sender_id),
        "Account linked via in-chat OTP verification"
    );

    linking::emit_linking_success(
        &user.id.to_string(),
        params.tenant_id,
        params.channel,
        "otp",
    );

    // Run the unified channel-auth path so the post-OTP reply matches what
    // the next inbound message will produce. Pending / Suspended users see
    // the translated denial copy at link time; everyone else sees the
    // localized link-success template.
    let body = linking::link_time_reply(
        params.resources,
        params.tenant_id,
        params.channel,
        params.sender_id,
        Some(user.id),
    )
    .await;
    otp_reply(params.channel_type, params.sender_id, body)
}

/// Start a new OTP linking flow for an unlinked user
///
/// Creates a link state with `otp_step` set (via `open_otp_flow_on_link_state`) and sends a
/// prompt asking for the user's email. The flow uses `awaiting_otp` with an empty email field
/// to represent the "awaiting email" state, and `awaiting_otp` with a non-empty email for
/// the "awaiting OTP code" state. The flow is opened in `locale`, the language the channel
/// suggests for the sender, which every later prompt of it reads back from the link state.
/// `pub` for the same reason as [`super::session::create_link_and_prompt`]:
/// the built message is the last point an integration test can read the reply an
/// unlinked sender receives, before it reaches an adapter that posts to a
/// hardcoded host.
pub async fn start_otp_flow(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel_type: ChannelType,
    sender_id: &str,
    sender_name: Option<&str>,
    locale: &str,
) -> OutgoingMessage {
    // Invalidate any existing OTP flows for this sender before starting a new one
    let channel_str = channel_type.to_string();
    invalidate_otp_flows(db, tenant_id, &channel_str, sender_id, "before a new flow").await;

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
        app_return_uri: None,
        expires_at: &expires_at.to_rfc3339(),
    };

    if let Err(e) = db.create_link_state(&params).await {
        error!(error = %e, "Failed to create OTP link state for unlinked user");
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_GENERIC_ERROR, locale),
        );
    }

    // Open the flow on 'awaiting_otp' with an empty email, which handle_otp_flow
    // interprets as "awaiting email input", in the language it will speak throughout
    if let Err(e) = db.open_otp_flow_on_link_state(&id, tenant_id, locale).await {
        error!(error = %e, "Failed to initialize OTP step on link state");
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_GENERIC_ERROR, locale),
        );
    }

    otp_reply(
        channel_type,
        sender_id,
        resources
            .mcp
            .messaging_strings_registry
            .get(KEY_LINK_EMAIL_PROMPT, locale),
    )
}

/// Park the flow on `awaiting_signup` and offer to create the account.
///
/// Replaces the old register-at-a-URL dead end. The address is held on the link
/// state so the confirmation reply knows which one to sign up, and no OTP hash is
/// written yet — no code has been sent, and a hash here would let a reply be
/// verified against a code the user never received.
async fn offer_signup(
    resources: &ServerContext,
    channel_type: ChannelType,
    sender_id: &str,
    state_id: &str,
    email: &str,
    locale: &str,
) -> OutgoingMessage {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    if let Err(e) = db.set_signup_pending_on_link_state(state_id, email).await {
        error!(error = %e, "failed to park link state for in-chat signup");
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_GENERIC_ERROR, locale),
        );
    }

    let template = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_LINK_SIGNUP_OFFER, locale);
    otp_reply(
        channel_type,
        sender_id,
        format_template(&template, &[email]),
    )
}

/// Affirmative replies accepted on the signup offer and on a held pairing
/// code, in all five locales.
///
/// Kept deliberately narrow: anything else re-asks rather than guessing,
/// because the alternative is creating an account, or linking a chat, for
/// someone who typed something unrelated.
const YES_REPLIES: [&str; 8] = ["yes", "y", "oui", "si", "sí", "ja", "sim", "ok"];

/// Whether `text` is one of the [`YES_REPLIES`], ignoring case and padding.
pub(super) fn is_yes_reply(text: &str) -> bool {
    YES_REPLIES.contains(&text.trim().to_lowercase().as_str())
}

/// Handle the reply to the signup offer.
///
/// A "yes" creates the account and immediately sends the OTP — which is what
/// proves the address belongs to them, so no separate confirmation email is
/// needed. Anything unrecognised re-asks; `cancel` is handled upstream.
async fn handle_signup_confirm_step(params: &OtpFlowStep<'_>, text: &str) -> OutgoingMessage {
    let OtpFlowStep {
        resources,
        channel_type,
        sender_id,
        state_id,
        email,
        locale,
        ..
    } = *params;
    if !is_yes_reply(text) {
        let template = resources
            .mcp
            .messaging_strings_registry
            .get(KEY_LINK_SIGNUP_OFFER, locale);
        return otp_reply(
            channel_type,
            sender_id,
            format_template(&template, &[email]),
        );
    }

    let Ok(account) = create_chat_originated_user(resources, email, locale).await else {
        return otp_reply(
            channel_type,
            sender_id,
            resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_SIGNUP_FAILED, locale),
        );
    };

    match generate_and_send_otp(resources, channel_type, sender_id, state_id, email, locale).await {
        Ok(masked) => {
            // The operational tier keys on the tenant: without it the event
            // lands on the shared `service` identity.
            info!(
                target: "notify",
                event = "user.signed_up",
                user_id = %account.user_id,
                tenant_id = %account.tenant_id,
                source = "messaging",
                "user account created from a chat conversation"
            );
            let template = resources
                .mcp
                .messaging_strings_registry
                .get(KEY_LINK_SIGNUP_CREATED, locale);
            let mut reply = otp_reply(channel_type, sender_id, template);
            if let MessageContent::Text { body } = &mut reply.content {
                body.push(' ');
                body.push_str(&masked);
            }
            reply
        }
        Err(reply) => *reply,
    }
}

/// Create an account for someone who signed up inside a conversation.
///
/// No password is set. They proved the inbox with the OTP that follows, and a
/// password typed into a chat transcript would be a password in a chat
/// transcript — they set one later through the normal reset flow, which mails a
/// link to the address they just proved.
///
/// The account is created Active or Pending by the same
/// [`pierre_services::auth::AuthService::self_register`] rules every other
/// self-serve signup uses, so the approval posture and the operator's sign-up
/// bounds stay in one place rather than being special-cased per surface.
///
/// The account reads `locale`, the language the sign-up conversation was held
/// in: its owner has chosen no other, and the link reply that follows is
/// written in the account's language, so the flow does not switch to the
/// default at its last step.
async fn create_chat_originated_user(
    resources: &ServerContext,
    email: &str,
    locale: &str,
) -> Result<RegisterResponse, AppError> {
    let auth_service = resources.account_service();

    // A random unusable secret: `register` requires one, and nothing may ever
    // authenticate with it. The reset flow is the only way in until they set a
    // real password.
    let unusable = format!("chat-signup-{}", Uuid::new_v4());
    let account = auth_service
        .self_register(RegisterRequest {
            email: email.to_owned(),
            password: SecretString::from(unusable),
            display_name: None,
            // Asked in the conversation once the account is linked.
            analytics_consent: None,
        })
        .await
        .map_err(|e| {
            if e.is_server_fault() {
                error!(error = %e, "in-chat signup failed");
            } else {
                warn!(error = %e, "in-chat signup refused");
            }
            e
        })?;

    // Best-effort: the account exists either way, and one left on the default
    // locale still works; it only reads the default until its owner says
    // otherwise.
    let recorded = match Uuid::parse_str(&account.user_id) {
        Ok(user_id) => {
            resources
                .common
                .repos
                .users
                .update_locale(user_id, locale)
                .await
        }
        Err(e) => Err(AppError::internal(format!("new account id: {e}"))),
    };
    if let Err(e) = recorded {
        warn!(error = %e, user_id = %account.user_id, "could not record the in-chat sign-up's language on the new account");
    }
    Ok(account)
}

/// Invalidate the sender's OTP link states, logging a failure.
///
/// A failed invalidation leaves a flow live until it expires, so it is a
/// warning. The sender id is a phone number or handle and stays out of the log.
async fn invalidate_otp_flows(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    when: &str,
) {
    if let Err(e) = db
        .invalidate_otp_link_states(tenant_id, channel, sender_id)
        .await
    {
        warn!(tenant_id = %tenant_id, channel, when, error = %e, "could not invalidate OTP link states");
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_flow_keeps_the_language_it_was_opened_in() {
        let opened_in_german = json!({ "otp_step": "awaiting_otp", "locale": "de" });
        assert_eq!(
            flow_locale(&opened_in_german, "en"),
            "de",
            "bug: a flow switched from the language it was opened in"
        );

        let opened_with_none = json!({ "otp_step": "awaiting_otp", "locale": null });
        assert_eq!(
            flow_locale(&opened_with_none, "en"),
            "en",
            "bug: a flow holding no language did not take what the channel suggests"
        );
    }
}
