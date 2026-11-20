// ABOUTME: Channel-linking commands ("/start <code>", "LINK <code>", m.me referrals), confirmed by a yes
// ABOUTME: Plus analytics-consent cache hydration triggered when a freshly linked user appears

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

use pierre_core::errors::ErrorCode;
use pierre_core::models::messaging::{
    channel_label, ChannelType, IncomingMessage, MessageContent, OutgoingMessage,
};
use pierre_core::models::TenantId;
use pierre_database::backends::{CreateChannelLinkParams, MessagingRepository};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::otp::{self, OtpFlowStep};
use super::prelink::prelink_locale;
use crate::mcp::resources::ServerContext;
use pierre_contremaitre::messaging_strings::{
    KEY_LINK_CONFIRM_PROMPT, KEY_LINK_IDENTITY_COLLISION, KEY_LINK_SESSION_EXPIRED,
    KEY_LINK_SUCCESS, KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
};
use pierre_middleware::redaction::mask_email_for_display;
use pierre_services::analytics::{analytics, cache_user_email, hash_id};
use pierre_services::messaging_broadcast::proactive_text;
use pierre_services::user_status_gate::messaging_key_for_status;

use pierre_services::locale::resolve_channel_locale;

/// `otp_step` of a pairing code held for its sender until they confirm the
/// account it links to. The repository's hold and release statements write
/// the same literal.
pub(super) const AWAITING_LINK_CONFIRM: &str = "awaiting_link_confirm";

/// Result of checking an inbound message for a channel linking command
pub(super) enum LinkingAction {
    /// Message contains a linking command — handle it and do not dispatch to LLM
    LinkCode(String),
    /// Normal message — proceed with standard routing
    Normal,
}

/// Detect if an inbound message contains a channel linking command
///
/// `Telegram`: `/start {code}` — bot deep link with verification code
/// `WhatsApp`: `LINK {code}` — text message with verification code
/// `Messenger`: an `m.me/{page}?ref={code}` referral, which dravr-canot hands
/// over as the bare code with `referral_ref` in the metadata. Without that
/// mark a bare code is ordinary text, so only a real referral redeems one.
pub(super) fn detect_linking_code(
    channel_type: ChannelType,
    content: &MessageContent,
    metadata: &serde_json::Value,
) -> LinkingAction {
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
        ChannelType::Messenger => {
            let referral = metadata
                .get("referral_ref")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|code| !code.is_empty());
            if let Some(code) = referral.filter(|code| *code == text.trim()) {
                return LinkingAction::LinkCode(code.to_owned());
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

/// The account a pairing code was started on, when the code may be redeemed
/// in this channel's chat.
///
/// Only a pairing code minted for this channel by the deep-link method, on an
/// account, is redeemed in a chat. An OAuth link's code rides in the provider's
/// authorize URL and is finished only by the account that started it
/// (carnet#1580); typed into another channel's chat it would bind that sender
/// to the starter's account. Reading never spends the code.
async fn pairing_code_owner(
    db: &dyn MessagingRepository,
    channel: &str,
    code: &str,
) -> Result<String, DeepLinkError> {
    let preview = db
        .get_link_state(code)
        .await
        .map_err(|e| DeepLinkError::SessionExpired(e.to_string()))?
        .ok_or_else(|| {
            DeepLinkError::SessionExpired("link code is unknown or expired".to_owned())
        })?;
    match (
        preview["channel_type"].as_str(),
        preview["method"].as_str(),
        preview["user_id"].as_str(),
    ) {
        (Some(started_on), Some("deep_link"), Some(user_id))
            if started_on == channel && !user_id.is_empty() =>
        {
            Ok(user_id.to_owned())
        }
        _ => Err(DeepLinkError::SessionExpired(
            "link code was not started as a pairing code for this channel".to_owned(),
        )),
    }
}

/// Hold a redeemed pairing code for its sender until they confirm, returning
/// the account it would link them to (carnet#1596).
///
/// A pairing link names the account that started it and nothing about who
/// taps it, so a forwarded link would bind whoever opened it. Nothing links
/// here and the code stays unspent: the sender is asked first, and
/// [`confirm_held_link`] links on their yes. A sender whose chat identity is
/// already linked is refused now rather than after they confirm. The hold
/// records `locale`, the language the sender is asked in.
async fn hold_pairing_code(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
    locale: &str,
) -> Result<String, DeepLinkError> {
    let owner = pairing_code_owner(db, channel, code).await?;
    let already_linked = db
        .get_channel_link(tenant_id, channel, sender_id)
        .await
        .map_err(|e| DeepLinkError::SessionExpired(e.to_string()))?
        .is_some();
    if already_linked {
        return Err(DeepLinkError::IdentityCollision(
            "this chat identity is already linked to a Dravr account".to_owned(),
        ));
    }
    let held = db
        .hold_link_for_confirmation(code, tenant_id, channel, sender_id, locale)
        .await
        .map_err(|e| DeepLinkError::SessionExpired(e.to_string()))?;
    if held {
        Ok(owner)
    } else {
        Err(DeepLinkError::SessionExpired(
            "link code expired or was spent before it could be held".to_owned(),
        ))
    }
}

/// Spend a pairing code this sender holds and create the permanent channel
/// link, returning the linked user ID.
///
/// The code must still be held by `sender_id`: a code another sender took over
/// since is refused and left unspent.
async fn execute_link_code(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> Result<String, DeepLinkError> {
    let user_id = pairing_code_owner(db, channel, code).await?;
    let spent = db
        .spend_held_link_code(code, tenant_id, channel, sender_id)
        .await
        .map_err(|e| DeepLinkError::SessionExpired(e.to_string()))?;
    if !spent {
        return Err(DeepLinkError::SessionExpired(
            "link code is not held by this sender, or has expired".to_owned(),
        ));
    }
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
/// Build the reply body for the moment a fresh channel link gets created.
///
/// Calls the unified channel-authentication path (same call every inbound
/// message after this will hit) and translates the outcome into a link-time
/// reply, in the language of `user_id`, the account just linked:
/// - `Ok(_)` → [`KEY_LINK_SUCCESS`].
/// - `AccountPending` / `AccountSuspended` → translated denial copy.
/// - Any other auth error → falls back to [`KEY_LINK_SUCCESS`] (the link did
///   persist, the transient auth-side error doesn't justify scaring the user
///   off; the next inbound message will retry the gate).
pub(super) async fn link_time_reply(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    user_id: Option<Uuid>,
) -> String {
    let reg = &resources.mcp.messaging_strings_registry;
    // Resolved by the account rather than by the auth result, so a denied
    // account reads its denial in its own language too: the link is fresh and
    // carries no override, and without the account the chain is the default.
    let locale = resolve_channel_locale(
        resources.common.repos.messaging.as_ref(),
        resources.common.repos.users.as_ref(),
        tenant_id,
        channel,
        sender_id,
        user_id,
    )
    .await;
    match resources
        .auth
        .auth_middleware
        .authenticate_channel(tenant_id, channel, sender_id)
        .await
    {
        Ok(_) => reg.get(KEY_LINK_SUCCESS, &locale),
        Err(e) => {
            if let Some(key) = messaging_key_for_status(e.code) {
                // Mirror `build_auth_denial_reply`: NoProviderConnected
                // carries a `{0}` URL placeholder and a `{1}` email when
                // resolvable from the channel link. Keeps the two render
                // paths in sync so a future gate addition here doesn't
                // leak literal `{0}`/`{1}` to users.
                if e.code == ErrorCode::NoProviderConnected {
                    let connect_url = format!(
                        "{}/providers",
                        resources
                            .common
                            .config
                            .frontend_url
                            .as_deref()
                            .unwrap_or(&resources.common.config.base_url)
                    );
                    let email =
                        super::resolve_channel_user_email(resources, tenant_id, channel, sender_id)
                            .await;
                    email.map_or_else(
                        || reg.render(key, &locale, &[&connect_url]),
                        |email| {
                            reg.render(
                                KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
                                &locale,
                                &[&connect_url, &email],
                            )
                        },
                    )
                } else {
                    reg.get(key, &locale)
                }
            } else {
                // Operator-category failure (DB blip, etc.). Log and fall
                // back to the link-success template — the link is real, the
                // next inbound message hits the gate again.
                warn!(error = %e, channel = %channel, "Link-time auth check failed; using success template");
                reg.get(KEY_LINK_SUCCESS, &locale)
            }
        }
    }
}

/// Emit the `messaging.linking_completed` notify event. `link_method` is
/// `deep_link` or `otp`.
///
/// `user_id` is the raw (un-hashed) Pierre user id. Shared by the deep-link
/// path here and the in-chat OTP path in `otp.rs`.
pub(super) fn emit_linking_success(
    user_id: &str,
    tenant_id: TenantId,
    channel: &str,
    link_method: &str,
) {
    info!(
        target: "notify",
        event = "messaging.linking_completed",
        user_id = %user_id,
        tenant_id = %tenant_id,
        channel = %channel,
        link_method = link_method,
        "channel linking completed"
    );
}

/// Emit the `messaging.linking_failed` notify event. `failure_reason` is the
/// operator-facing detail string from the `DeepLinkError`.
fn emit_linking_failed(tenant_id: TenantId, channel: &str, failure_reason: &str) {
    info!(
        target: "notify",
        event = "messaging.linking_failed",
        tenant_id = %tenant_id,
        channel = %channel,
        failure_reason = %failure_reason,
        "channel linking failed"
    );
}

/// Reply text for a failed link step, in `locale`, after logging it and
/// emitting `messaging.linking_failed`.
fn link_failure_reply(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    err: &DeepLinkError,
    locale: &str,
) -> String {
    let (key, detail) = match err {
        DeepLinkError::SessionExpired(d) => (KEY_LINK_SESSION_EXPIRED, d.as_str()),
        DeepLinkError::IdentityCollision(d) => (KEY_LINK_IDENTITY_COLLISION, d.as_str()),
    };
    warn!(error = %detail, "Channel linking failed");
    emit_linking_failed(tenant_id, channel, detail);
    resources.mcp.messaging_strings_registry.get(key, locale)
}

/// The question a held pairing code asks, in `locale`: which account this chat
/// would be linked to, by its masked email, and that a yes links it.
async fn link_confirm_prompt(
    resources: &ServerContext,
    channel: &str,
    owner_user_id: &str,
    locale: &str,
) -> Result<String, DeepLinkError> {
    let owner = Uuid::parse_str(owner_user_id)
        .map_err(|e| DeepLinkError::SessionExpired(format!("link code names no account: {e}")))?;
    // SECURITY: global lookup by the id the link state names, as the channel
    // auth gate reads a link's user.
    let email = resources
        .common
        .repos
        .users
        .get_global(owner)
        .await
        .map_err(|e| DeepLinkError::SessionExpired(e.to_string()))?
        .ok_or_else(|| {
            DeepLinkError::SessionExpired("the account that started the link is gone".to_owned())
        })?
        .email;
    Ok(resources.mcp.messaging_strings_registry.render(
        KEY_LINK_CONFIRM_PROMPT,
        locale,
        &[channel_label(channel), &mask_email_for_display(&email)],
    ))
}

/// Handle a redeemed pairing code: hold it for the sender and ask them to
/// confirm the account it links to. Nothing is linked until they answer yes
/// (carnet#1596).
///
/// The question is asked in the language the channel suggests for the sender
/// ([`prelink_locale`]), which the hold records so every later prompt of
/// the exchange reads it back (carnet#1669).
///
/// Returns the question, or why the code cannot be redeemed here.
pub(super) async fn handle_linking_command(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    message: &IncomingMessage,
    code: &str,
) -> OutgoingMessage {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let channel_type = ChannelType::from_str(channel).unwrap_or(ChannelType::Telegram);
    let sender_id = message.sender_id.as_str();
    let locale = prelink_locale(message);
    let asked = match hold_pairing_code(db, tenant_id, channel, sender_id, code, locale).await {
        Ok(owner) => link_confirm_prompt(resources, channel, &owner, locale).await,
        Err(err) => Err(err),
    };
    let response_text =
        asked.unwrap_or_else(|err| link_failure_reply(resources, tenant_id, channel, &err, locale));

    proactive_text(channel_type, sender_id.to_owned(), response_text)
}

/// Answer a sender who holds a pairing code: a yes spends the code and links
/// them to the account it names; anything else asks again. `cancel` is handled
/// before this, by handing the code back unspent. The question and any
/// refusal are in the language the hold recorded, `step.locale`; the reply
/// to a link that lands is in the linked account's own ([`link_time_reply`]).
pub(super) async fn confirm_held_link(
    step: &OtpFlowStep<'_>,
    held: &serde_json::Value,
    answer: &str,
) -> OutgoingMessage {
    let OtpFlowStep {
        resources,
        tenant_id,
        channel_type,
        channel,
        sender_id,
        locale,
        ..
    } = *step;
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let code = held["code"].as_str().unwrap_or_default();
    let response_text = if otp::is_yes_reply(answer) {
        match execute_link_code(db, tenant_id, channel, sender_id, code).await {
            Ok(user_id) => {
                info!(channel = %channel, user_id = %user_id, "Channel linked via deep link");
                emit_linking_success(&user_id, tenant_id, channel, "deep_link");
                // Run the unified channel-auth path on the freshly-minted link so
                // the immediate reply reflects approval state exactly as every
                // subsequent inbound message will.
                let linked = Uuid::parse_str(&user_id).ok();
                link_time_reply(resources, tenant_id, channel, sender_id, linked).await
            }
            Err(err) => link_failure_reply(resources, tenant_id, channel, &err, locale),
        }
    } else {
        let owner = held["user_id"].as_str().unwrap_or_default();
        link_confirm_prompt(resources, channel, owner, locale)
            .await
            .unwrap_or_else(|err| link_failure_reply(resources, tenant_id, channel, &err, locale))
    };

    proactive_text(channel_type, sender_id.to_owned(), response_text)
}

/// Hydrate the analytics consent cache for a messaging user on cache miss
///
/// The cache is in-memory and empties on every Cloud Run cold start, so each
/// fresh pod needs to learn each user's durable `analytics_consent` value from
/// the database before their events will be captured. Once hydrated the entry
/// persists for the life of the pod and `/privacy on|off` commands keep it
/// current via `set_consent`.
pub(super) async fn hydrate_analytics_consent(resources: &ServerContext, user_id: &str) {
    let hashed_user = hash_id(user_id);
    if analytics().has_consent_cached(&hashed_user) {
        return;
    }
    let Ok(parsed) = Uuid::parse_str(user_id) else {
        return;
    };
    match resources.common.repos.users.get_global(parsed).await {
        Ok(Some(user)) => {
            analytics().hydrate_consent(&hashed_user, user.analytics_consent);
            // Warm the identity cache from the same record so this user's
            // messaging-channel notify events carry their email.
            cache_user_email(user_id, &user.email);
        }
        Ok(None) => {}
        Err(e) => {
            error!(error = %e, user_id = %user_id, "Failed to load user for analytics consent hydration");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration as StdDuration;

    use chrono::{Duration, Utc};
    use pierre_core::models::{Tenant, User};
    use pierre_database::backends::{CreateLinkStateParams, TenantRepository};
    use pierre_database::RepositoryRegistry;
    use pierre_test_support::db::create_test_db;
    use tokio::time;

    use super::*;

    const SENDER: &str = "4242";
    /// The language a test sender is asked in.
    const LOCALE: &str = "en";

    /// An account with a tenant; returns `(user_id, tenant_id)`.
    async fn account(repos: &RepositoryRegistry) -> (Uuid, TenantId) {
        let user = User::new(
            format!("{}@pairing.test", Uuid::new_v4()),
            String::new(),
            None,
        );
        let user_id = repos.users.create(&user).await.expect("user row");
        let tenant_id = TenantId::generate();
        let tenants: &dyn TenantRepository = repos.tenants.as_ref();
        tenants
            .create(&Tenant {
                id: tenant_id,
                name: "Pairing tenant".to_owned(),
                slug: format!("pairing-{tenant_id}"),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: user_id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .expect("tenant row");
        (user_id, tenant_id)
    }

    /// A pending link the account started for `channel` by `method`.
    async fn started(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        user_id: Uuid,
        channel: &str,
        method: &str,
    ) -> String {
        let code = Uuid::new_v4().simple().to_string();
        let user = user_id.to_string();
        let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();
        db.create_link_state(&CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: Some(&user),
            channel_type: channel,
            code: &code,
            method,
            channel_user_id: None,
            sender_name: None,
            app_return_uri: None,
            expires_at: &expires_at,
        })
        .await
        .expect("link state");
        code
    }

    /// A pairing code that expires `ttl` from now.
    async fn started_for(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        user_id: Uuid,
        ttl: Duration,
    ) -> String {
        let code = Uuid::new_v4().simple().to_string();
        let user = user_id.to_string();
        let expires_at = (Utc::now() + ttl).to_rfc3339();
        db.create_link_state(&CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: Some(&user),
            channel_type: "telegram",
            code: &code,
            method: "deep_link",
            channel_user_id: None,
            sender_name: None,
            app_return_uri: None,
            expires_at: &expires_at,
        })
        .await
        .expect("link state");
        code
    }

    async fn linked(db: &dyn MessagingRepository, tenant_id: TenantId, sender: &str) -> bool {
        db.get_channel_link(tenant_id, "telegram", sender)
            .await
            .expect("link read")
            .is_some()
    }

    async fn unspent(db: &dyn MessagingRepository, code: &str) -> bool {
        db.get_link_state(code).await.expect("state read").is_some()
    }

    /// carnet#1596: redeeming a pairing code names the account and links
    /// nothing; the code stays unspent until the sender confirms.
    #[tokio::test]
    async fn a_redeemed_pairing_code_is_held_not_linked() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let code = started(db, tenant_id, user_id, "telegram", "deep_link").await;

        let owner = hold_pairing_code(db, tenant_id, "telegram", SENDER, &code, LOCALE).await;

        assert_eq!(
            owner.ok(),
            Some(user_id.to_string()),
            "bug: a held code does not name the account that started it"
        );
        assert!(
            !linked(db, tenant_id, SENDER).await,
            "bug: redeeming a code linked before any yes"
        );
        assert!(unspent(db, &code).await, "bug: holding a code spent it");
        let held = db
            .get_active_otp_link_state(tenant_id, "telegram", SENDER)
            .await
            .expect("held read")
            .expect("the code is held for its sender");
        assert_eq!(
            held["code"],
            code.as_str(),
            "bug: the sender's live flow is not the code they redeemed"
        );
        assert_eq!(
            held["otp_step"], AWAITING_LINK_CONFIRM,
            "bug: a held code is not parked on the confirmation step"
        );
        assert_eq!(
            held["locale"], LOCALE,
            "bug: a held code does not record the language its sender is asked in"
        );
    }

    /// carnet#1669: the language of a held code is its holder's, so a sender
    /// who takes a forwarded code over is asked in their own language.
    #[tokio::test]
    async fn a_code_taken_over_speaks_the_language_of_its_new_holder() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let code = started(db, tenant_id, user_id, "telegram", "deep_link").await;
        let (forwarded_to, starter) = ("5151", "4242");

        hold_pairing_code(db, tenant_id, "telegram", forwarded_to, &code, "es")
            .await
            .ok()
            .expect("held by the first redeemer");
        hold_pairing_code(db, tenant_id, "telegram", starter, &code, "de")
            .await
            .ok()
            .expect("the starter takes the hold over");

        let held = db
            .get_active_otp_link_state(tenant_id, "telegram", starter)
            .await
            .expect("held read")
            .expect("the code is held for the starter");
        assert_eq!(
            held["locale"], "de",
            "bug: a code taken over kept the language of the sender it was taken from"
        );
    }

    #[tokio::test]
    async fn a_held_code_links_its_sender_once_confirmed() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let code = started(db, tenant_id, user_id, "telegram", "deep_link").await;
        hold_pairing_code(db, tenant_id, "telegram", SENDER, &code, LOCALE)
            .await
            .ok()
            .expect("held");

        let linked_to = execute_link_code(db, tenant_id, "telegram", SENDER, &code).await;

        assert_eq!(
            linked_to.ok(),
            Some(user_id.to_string()),
            "bug: a confirmed code did not link to the account that started it"
        );
        assert!(
            linked(db, tenant_id, SENDER).await,
            "bug: a confirmed code left no channel link"
        );
        assert!(
            !unspent(db, &code).await,
            "bug: a confirmed code is not spent"
        );
    }

    /// The forwarded-link case: one sender's yes never finishes a code another
    /// sender holds, and the latest redeemer takes the hold over, so a
    /// forwarded copy cannot lock the starter out.
    #[tokio::test]
    async fn only_the_sender_holding_a_code_can_confirm_it() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let code = started(db, tenant_id, user_id, "telegram", "deep_link").await;
        let (forwarded_to, starter) = ("5151", "4242");

        hold_pairing_code(db, tenant_id, "telegram", forwarded_to, &code, LOCALE)
            .await
            .ok()
            .expect("held by the first redeemer");
        let stranger = execute_link_code(db, tenant_id, "telegram", starter, &code).await;
        assert!(
            matches!(stranger, Err(DeepLinkError::SessionExpired(_))),
            "bug: a sender who does not hold the code confirmed it"
        );

        hold_pairing_code(db, tenant_id, "telegram", starter, &code, LOCALE)
            .await
            .ok()
            .expect("the starter takes the hold over");
        let displaced = execute_link_code(db, tenant_id, "telegram", forwarded_to, &code).await;
        assert!(
            matches!(displaced, Err(DeepLinkError::SessionExpired(_))),
            "bug: a displaced holder confirmed the code"
        );
        assert!(
            !linked(db, tenant_id, forwarded_to).await,
            "bug: a displaced holder was linked"
        );
        assert!(
            unspent(db, &code).await,
            "bug: a refused confirmation spent the code"
        );

        let confirmed = execute_link_code(db, tenant_id, "telegram", starter, &code).await;
        assert_eq!(
            confirmed.ok(),
            Some(user_id.to_string()),
            "bug: the current holder could not confirm the code"
        );
    }

    /// A sender who declines hands the code back unspent and unheld, so the
    /// account that started it can still finish it.
    #[tokio::test]
    async fn a_released_code_is_unspent_and_held_by_nobody() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let code = started(db, tenant_id, user_id, "telegram", "deep_link").await;
        hold_pairing_code(db, tenant_id, "telegram", SENDER, &code, LOCALE)
            .await
            .ok()
            .expect("held");

        db.release_link_confirmations(tenant_id, "telegram", SENDER)
            .await
            .expect("release");

        assert!(unspent(db, &code).await, "bug: a release spent the code");
        assert!(
            db.get_active_otp_link_state(tenant_id, "telegram", SENDER)
                .await
                .expect("held read")
                .is_none(),
            "bug: a released code is still held by its sender"
        );
        let after = execute_link_code(db, tenant_id, "telegram", SENDER, &code).await;
        assert!(
            matches!(after, Err(DeepLinkError::SessionExpired(_))),
            "bug: a released code was confirmed by its former holder"
        );
        assert!(
            !linked(db, tenant_id, SENDER).await,
            "bug: a released code linked its former holder"
        );
    }

    /// A sender who declines after another sender took the code over releases
    /// nothing of theirs: the new holder's question still stands.
    #[tokio::test]
    async fn a_displaced_sender_cannot_release_the_new_holders_code() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let code = started(db, tenant_id, user_id, "telegram", "deep_link").await;
        let (forwarded_to, starter) = ("5151", "4242");
        hold_pairing_code(db, tenant_id, "telegram", forwarded_to, &code, LOCALE)
            .await
            .ok()
            .expect("held by the first redeemer");
        hold_pairing_code(db, tenant_id, "telegram", starter, &code, LOCALE)
            .await
            .ok()
            .expect("the starter takes the hold over");

        db.release_link_confirmations(tenant_id, "telegram", forwarded_to)
            .await
            .expect("release");

        let held = db
            .get_active_otp_link_state(tenant_id, "telegram", starter)
            .await
            .expect("held read");
        assert_eq!(
            held.as_ref().and_then(|state| state["code"].as_str()),
            Some(code.as_str()),
            "bug: a displaced sender's cancel released the new holder's code"
        );
    }

    /// A chat identity that is already linked is refused when it redeems a
    /// code, not asked to confirm and then refused; the code stays free.
    #[tokio::test]
    async fn an_already_linked_sender_is_refused_before_any_question() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let first = started(db, tenant_id, user_id, "telegram", "deep_link").await;
        hold_pairing_code(db, tenant_id, "telegram", SENDER, &first, LOCALE)
            .await
            .ok()
            .expect("held");
        execute_link_code(db, tenant_id, "telegram", SENDER, &first)
            .await
            .ok()
            .expect("linked");
        let second = started(db, tenant_id, user_id, "telegram", "deep_link").await;

        let refused = hold_pairing_code(db, tenant_id, "telegram", SENDER, &second, LOCALE).await;

        assert!(
            matches!(refused, Err(DeepLinkError::IdentityCollision(_))),
            "bug: an already linked sender was not refused at redemption"
        );
        assert!(
            unspent(db, &second).await,
            "bug: a refused redemption spent the code"
        );
        assert!(
            db.get_active_otp_link_state(tenant_id, "telegram", SENDER)
                .await
                .expect("held read")
                .is_none(),
            "bug: a refused redemption left the code held"
        );
    }

    /// A yes that arrives after the code expired links nobody.
    #[tokio::test]
    async fn a_yes_after_the_code_expired_links_nobody() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let code = started_for(db, tenant_id, user_id, Duration::seconds(1)).await;
        hold_pairing_code(db, tenant_id, "telegram", SENDER, &code, LOCALE)
            .await
            .ok()
            .expect("held");

        time::sleep(StdDuration::from_millis(1_500)).await;
        let late = execute_link_code(db, tenant_id, "telegram", SENDER, &code).await;

        assert!(
            matches!(late, Err(DeepLinkError::SessionExpired(_))),
            "bug: a yes after expiry was accepted"
        );
        assert!(
            !linked(db, tenant_id, SENDER).await,
            "bug: a yes after expiry linked the sender"
        );
    }

    /// An expired code cannot be held in the first place.
    #[tokio::test]
    async fn an_expired_code_is_not_held() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;
        let code = started_for(db, tenant_id, user_id, Duration::seconds(-1)).await;

        let refused = hold_pairing_code(db, tenant_id, "telegram", SENDER, &code, LOCALE).await;

        assert!(
            matches!(refused, Err(DeepLinkError::SessionExpired(_))),
            "bug: an expired code was accepted for holding"
        );
        assert!(
            db.get_active_otp_link_state(tenant_id, "telegram", SENDER)
                .await
                .expect("held read")
                .is_none(),
            "bug: an expired code was held"
        );
    }

    /// carnet#1580: an OAuth link's code rides in the provider's authorize URL
    /// and is finished only by its starter. Typed into a chat as `/start <code>`
    /// it would bind that sender to the starter's account.
    #[tokio::test]
    async fn an_oauth_link_code_is_never_redeemed_in_a_chat() {
        let database = create_test_db().await.expect("test database");
        let repos = database.repositories();
        let db: &dyn MessagingRepository = repos.messaging.as_ref();
        let (user_id, tenant_id) = account(repos).await;

        for (started_on, method, typed_on) in [
            ("slack", "oauth", "telegram"),
            ("telegram", "oauth", "telegram"),
            ("whatsapp", "deep_link", "telegram"),
        ] {
            let code = started(db, tenant_id, user_id, started_on, method).await;

            let refused = hold_pairing_code(db, tenant_id, typed_on, SENDER, &code, LOCALE).await;

            assert!(
                matches!(refused, Err(DeepLinkError::SessionExpired(_))),
                "a {method} code started on {started_on} must not be held on {typed_on}"
            );
            assert!(unspent(db, &code).await, "a refused code is left unspent");
            assert!(
                db.get_active_otp_link_state(tenant_id, typed_on, SENDER)
                    .await
                    .expect("held read")
                    .is_none(),
                "bug: a {method} code started on {started_on} was held on {typed_on}"
            );
        }
    }

    #[test]
    fn a_messenger_referral_is_a_pairing_code_and_bare_text_is_not() {
        let body = MessageContent::Text {
            body: "a1b2c3".to_owned(),
        };

        let referral = detect_linking_code(
            ChannelType::Messenger,
            &body,
            &serde_json::json!({ "referral_ref": "a1b2c3" }),
        );
        let typed = detect_linking_code(ChannelType::Messenger, &body, &serde_json::json!({}));

        assert!(
            matches!(referral, LinkingAction::LinkCode(code) if code == "a1b2c3"),
            "bug: a Messenger referral was not read as its pairing code"
        );
        assert!(
            matches!(typed, LinkingAction::Normal),
            "bug: bare Messenger text without a referral was read as a pairing code"
        );
    }
}
