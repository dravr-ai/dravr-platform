// ABOUTME: The one-time usage-analytics question a chat account is asked when its link completes
// ABOUTME: A card whose buttons run /privacy on|off, sent only to an active account that never answered (carnet#1366)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The usage-analytics question for an account made or linked in a chat.
//!
//! A chat sign-up has no form to carry the opt-in the web and phone sign-up
//! forms carry, and its athlete may never open either app, so the question
//! is asked in the conversation, once, right after the link completes, or,
//! for an account linked while it waited for review, right after the
//! approval message. The buttons post `/privacy on` and `/privacy off`: a tap
//! is exactly typing
//! the command, so the answer goes through the one consent writer the
//! command already uses, and a channel without buttons lists the commands to
//! type. Declining is one tap, like accepting (Law 25 s. 14).

use dravr_canot::rich_text::escape_markdown;
use dravr_canot::turn::ConversationTurnId as CanotTurnId;
use pierre_chat_pipeline::envelope::ActionKind;
use pierre_chat_pipeline::RenderCapabilities;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PRIVACY_CONSENT_ASK_ACCEPT, KEY_PRIVACY_CONSENT_ASK_BODY,
    KEY_PRIVACY_CONSENT_ASK_DECLINE, KEY_PRIVACY_CONSENT_ASK_TITLE,
};
use pierre_core::models::messaging::{CardAction, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::{TenantId, User, UserStatus};
use pierre_database::backends::{MessagingRepository, UserRepository};
use pierre_database::RepositoryRegistry;
use pierre_services::locale::resolve_channel_locale;
use pierre_services::messaging_broadcast::{resolve_linked_targets, resolve_target_sender};
use tracing::warn;
use uuid::Uuid;

use super::surface::messaging_render_profile;
use super::viz_delivery::card_or_rich_text;

/// The privacy policy's section on usage analytics: the address the web and
/// phone clients link beside the same question (`ANALYTICS_POLICY_URL` in
/// `@pierre/shared-constants`).
const ANALYTICS_POLICY_URL: &str = "https://dravr.ai/privacy#what-we-do-with-it";

/// What the accept button posts: the command that records a yes.
const ACCEPT_COMMAND: &str = "/privacy on";
/// What the decline button posts: the command that records a no.
const DECLINE_COMMAND: &str = "/privacy off";

/// Whether `user` is asked: an active account that has never answered, yes
/// or no. A pending or suspended account is not, because the gate in front of
/// `/privacy` would refuse its tap.
fn needs_consent_ask(user: &User) -> bool {
    user.user_status == UserStatus::Active && user.analytics_consent_at.is_none()
}

/// The question as the channel can show it: a card with the two buttons, or,
/// where buttons are not drawn, the same words with the commands to type.
fn consent_ask_content(
    render: &RenderCapabilities,
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> MessageContent {
    let postback = |key: &str, command: &str| CardAction {
        label: registry.get(key, locale),
        action_type: ActionKind::Postback.as_str().to_owned(),
        value: command.to_owned(),
    };
    let actions = vec![
        postback(KEY_PRIVACY_CONSENT_ASK_ACCEPT, ACCEPT_COMMAND),
        postback(KEY_PRIVACY_CONSENT_ASK_DECLINE, DECLINE_COMMAND),
    ];
    let body = registry.render(
        KEY_PRIVACY_CONSENT_ASK_BODY,
        locale,
        &[ANALYTICS_POLICY_URL],
    );
    // Escaped, the body's words stay literal through the card's markdown
    // reading. The title is never empty, which Slack and Messenger refuse.
    card_or_rich_text(
        render,
        registry.get(KEY_PRIVACY_CONSENT_ASK_TITLE, locale),
        escape_markdown(&body),
        actions,
    )
}

/// The question for the account `sender_id` is linked to on `channel`, when
/// that account should be asked; `None` when it is not linked, not active,
/// has already answered, or cannot be read.
///
/// Called once a pre-session exchange has been answered. A link code or an
/// OTP code that completed the link leaves the sender linked; every other
/// step of those exchanges leaves an unlinked sender unlinked, so for a new
/// sender the question follows exactly the reply that completed a link. A
/// sender already linked who sends a link code again, and has still not
/// answered, is asked again. Best-effort: a failed read is logged and nothing
/// is asked.
pub(super) async fn consent_ask_after_link(
    repos: &RepositoryRegistry,
    registry: &MessagingStringsRegistry,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    sender_id: &str,
) -> Option<OutgoingMessage> {
    let messaging: &dyn MessagingRepository = repos.messaging.as_ref();
    let users: &dyn UserRepository = repos.users.as_ref();
    let link = match messaging
        .get_channel_link(tenant_id, channel, sender_id)
        .await
    {
        Ok(link) => link?,
        Err(e) => {
            warn!(error = %e, channel = %channel, "Could not read the channel link for the analytics question");
            return None;
        }
    };
    let user_id = link["user_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())?;
    // SECURITY: global lookup by the id the tenant-scoped link names, as the
    // channel auth gate reads it.
    let user = match users.get_global(user_id).await {
        Ok(user) => user?,
        Err(e) => {
            warn!(error = %e, user_id = %user_id, "Could not read the user for the analytics question");
            return None;
        }
    };
    if !needs_consent_ask(&user) {
        return None;
    }
    let locale = resolve_channel_locale(
        messaging,
        users,
        tenant_id,
        channel,
        sender_id,
        Some(user_id),
    )
    .await;
    let render = messaging_render_profile(channel_type, &locale).render;
    Some(OutgoingMessage {
        channel_type,
        recipient_id: sender_id.to_owned(),
        content: consent_ask_content(&render, registry, &locale),
        // A proactive message, not a reply to the inbound turn.
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: None,
    })
}

/// Ask the account an operator has just approved, on every chat it is linked
/// to, when it has never answered, and return how many chats it reached.
///
/// A chat sign-up made while sign-ups wait for review is linked while still
/// pending, so the question at link time passes it by (a pending account's
/// tap would be refused). The approval message is where its chat use starts,
/// so the question follows it, through the same bot that posts that message
/// into each chat. Best-effort like the approval message: a chat whose bot or
/// send fails is logged and skipped.
pub(crate) async fn ask_linked_chats_after_approval(
    repos: &RepositoryRegistry,
    registry: &MessagingStringsRegistry,
    user_id: Uuid,
) -> usize {
    let user = match repos.users.get_global(user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => return 0,
        Err(e) => {
            warn!(error = %e, user_id = %user_id, "Could not read the approved user for the analytics question");
            return 0;
        }
    };
    if !needs_consent_ask(&user) {
        return 0;
    }
    let messaging: &dyn MessagingRepository = repos.messaging.as_ref();
    let mut reached = 0;
    for target in resolve_linked_targets(messaging, user_id).await {
        let Some((adapter, config)) = resolve_target_sender(messaging, &target).await else {
            continue;
        };
        let render = messaging_render_profile(target.channel_type, &target.locale).render;
        let outgoing = OutgoingMessage {
            channel_type: target.channel_type,
            recipient_id: target.recipient_id.clone(),
            content: consent_ask_content(&render, registry, &target.locale),
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: None,
        };
        match adapter.send(&outgoing, &config).await {
            Ok(_) => reached += 1,
            Err(e) => {
                warn!(error = %e, channel = %target.channel_type, "Could not send the analytics question after approval");
            }
        }
    }
    reached
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::models::Tenant;
    use pierre_database::backends::{CreateChannelLinkParams, TenantRepository};
    use pierre_test_support::db::create_test_db;

    use super::*;

    const CHANNEL: &str = "telegram";
    const SENDER: &str = "4242";

    /// Link `SENDER` on `CHANNEL` to a new account in `status`, answered or
    /// not, and return the tenant the link lives under.
    async fn linked(repos: &RepositoryRegistry, status: UserStatus, answered: bool) -> TenantId {
        let mut account = User::new(
            format!("{}@consent.test", Uuid::new_v4()),
            String::new(),
            None,
        );
        account.user_status = status;
        account.analytics_consent_at = answered.then(Utc::now);
        let user_id = repos.users.create(&account).await.expect("user row");
        let tenant_id = TenantId::generate();
        let tenants: &dyn TenantRepository = repos.tenants.as_ref();
        tenants
            .create(&Tenant {
                id: tenant_id,
                name: "Consent tenant".to_owned(),
                slug: format!("consent-{tenant_id}"),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: user_id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .expect("tenant row");
        let messaging: &dyn MessagingRepository = repos.messaging.as_ref();
        messaging
            .create_channel_link(&CreateChannelLinkParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id,
                user_id: &user_id.to_string(),
                channel_type: CHANNEL,
                channel_user_id: SENDER,
                display_name: None,
            })
            .await
            .expect("channel link");
        tenant_id
    }

    async fn ask_for(repos: &RepositoryRegistry, tenant_id: TenantId) -> Option<OutgoingMessage> {
        consent_ask_after_link(
            repos,
            &MessagingStringsRegistry::new(),
            tenant_id,
            CHANNEL,
            ChannelType::Telegram,
            SENDER,
        )
        .await
    }

    #[tokio::test]
    async fn a_link_to_an_account_that_never_answered_is_followed_by_the_question() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let tenant_id = linked(repos, UserStatus::Active, false).await;

        let ask = ask_for(repos, tenant_id)
            .await
            .expect("the question is asked");

        assert_eq!(ask.recipient_id, SENDER);
        let MessageContent::Card { actions, .. } = ask.content else {
            panic!(
                "Telegram draws the question as a card, got {:?}",
                ask.content
            );
        };
        assert_eq!(actions.len(), 2);
    }

    #[tokio::test]
    async fn an_account_that_answered_is_not_asked_again() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let tenant_id = linked(repos, UserStatus::Active, true).await;

        assert!(ask_for(repos, tenant_id).await.is_none());
    }

    #[tokio::test]
    async fn a_pending_account_is_not_asked() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let tenant_id = linked(repos, UserStatus::Pending, false).await;

        assert!(ask_for(repos, tenant_id).await.is_none());
    }

    #[tokio::test]
    async fn a_sender_the_exchange_left_unlinked_is_not_asked() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();

        assert!(ask_for(repos, TenantId::generate()).await.is_none());
    }

    fn user(status: UserStatus, answered: bool) -> User {
        let mut user = User::new("athlete@example.com".to_owned(), String::new(), None);
        user.user_status = status;
        user.analytics_consent_at = answered.then(Utc::now);
        user
    }

    #[test]
    fn only_an_active_account_that_never_answered_is_asked() {
        assert!(needs_consent_ask(&user(UserStatus::Active, false)));
        assert!(!needs_consent_ask(&user(UserStatus::Active, true)));
        assert!(!needs_consent_ask(&user(UserStatus::Pending, false)));
        assert!(!needs_consent_ask(&user(UserStatus::Suspended, false)));
    }

    #[test]
    fn a_button_channel_gets_yes_and_no_posting_the_privacy_commands() {
        let render = messaging_render_profile(ChannelType::Telegram, "en").render;
        assert!(render.blocks.action_buttons, "Telegram draws buttons");
        let registry = MessagingStringsRegistry::new();

        let content = consent_ask_content(&render, &registry, "en");

        let MessageContent::Card {
            title,
            body,
            actions,
        } = content
        else {
            panic!("a button channel gets a card, got {content:?}");
        };
        assert_eq!(title, registry.get(KEY_PRIVACY_CONSENT_ASK_TITLE, "en"));
        let pairs: Vec<(&str, &str, &str)> = actions
            .iter()
            .map(|a| (a.label.as_str(), a.action_type.as_str(), a.value.as_str()))
            .collect();
        let accept = registry.get(KEY_PRIVACY_CONSENT_ASK_ACCEPT, "en");
        let decline = registry.get(KEY_PRIVACY_CONSENT_ASK_DECLINE, "en");
        assert_eq!(
            pairs,
            [
                (accept.as_str(), "postback", "/privacy on"),
                (decline.as_str(), "postback", "/privacy off"),
            ]
        );
        assert!(body.contains("PostHog"), "{body}");
        assert!(
            body.contains("dravr.ai/privacy"),
            "the policy link is in the body: {body}"
        );
    }

    #[test]
    fn every_button_fits_a_whatsapp_reply_button_in_every_locale() {
        let registry = MessagingStringsRegistry::new();
        for locale in ["fr", "en", "es", "de", "pt"] {
            for key in [
                KEY_PRIVACY_CONSENT_ASK_ACCEPT,
                KEY_PRIVACY_CONSENT_ASK_DECLINE,
            ] {
                let label = registry.get(key, locale);
                assert!(
                    label.chars().count() <= 20,
                    "{locale} {key} is {label:?}, past WhatsApp's 20 characters"
                );
            }
        }
    }

    #[test]
    fn a_channel_without_buttons_names_the_commands_to_type() {
        let mut render = messaging_render_profile(ChannelType::Telegram, "en").render;
        render.blocks.action_buttons = false;

        let content = consent_ask_content(&render, &MessagingStringsRegistry::new(), "en");

        let MessageContent::RichText { body } = content else {
            panic!("a channel without buttons gets rich text, got {content:?}");
        };
        assert!(body.contains("/privacy on"), "{body}");
        assert!(body.contains("/privacy off"), "{body}");
    }
}
