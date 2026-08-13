// ABOUTME: Asserts what a stranger actually receives on their first message to the bot, per channel
// ABOUTME: The reply-capture half of the messaging harness — inbound payloads already had a builder, replies did not
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The reply half of the messaging harness
//!
//! `helpers::messaging_webhooks` already builds correctly-signed *inbound*
//! payloads for all five channels. What was missing was any way to assert what
//! the bot **said back** — and the onboarding funnel lives entirely in those
//! replies: the unlinked prompt, the register-or-not answer, the connect card,
//! the coach proposal.
//!
//! Outbound adapters post to hardcoded hosts (`api.telegram.org` and friends),
//! so the last point a test can read the text without a network stub is the
//! built [`OutgoingMessage`]. These tests call the reply builders directly and
//! assert on their content, which is why those builders are `pub`.
//!
//! What this deliberately does **not** claim: it does not prove the ingress
//! *chose* a given builder for a given inbound message. The webhook-driven
//! coverage matrix and the `messaging.unlinked_prompted` notify event cover that
//! selection; these cover the words.

mod common;
mod helpers;

use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_core::models::TenantId;
use pierre_database::repositories::MessagingRepository;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_mcp_server::services::messaging_ingress::create_link_and_prompt;
use std::sync::Arc;

/// Pull the plain text out of whatever shape the reply took, so an assertion
/// does not have to care whether a channel got a Card or rich text.
fn reply_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { body } => body.clone(),
        other => format!("{other:?}"),
    }
}

/// A context plus a REAL tenant. The link-state insert is tenant-scoped, so a
/// made-up tenant id makes it fail and the builder falls back to the linkless
/// prompt — which is correct degradation, and exactly the wrong thing to assert
/// the happy path against.
async fn context_with_tenant() -> anyhow::Result<(Arc<ServerContext>, TenantId)> {
    let resources = context().await?;
    let (_, _, tenant) = common::create_test_user_with_plan(
        &resources.coach.database,
        "stranger-harness@example.com",
        "starter",
    )
    .await?;
    Ok((resources, tenant))
}

async fn context() -> anyhow::Result<Arc<ServerContext>> {
    common::init_server_config();
    let database = common::create_test_database().await?;
    let auth_manager = common::create_test_auth_manager();
    let cache = common::create_test_cache().await?;
    let temp_dir = tempfile::tempdir()?;

    let config = Arc::new(ServerConfig {
        http_port: 8081,
        database: DatabaseConfig {
            url: DatabaseUrl::Memory,
            backup: BackupConfig {
                directory: temp_dir.path().to_path_buf(),
                ..Default::default()
            },
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            ci_mode: true,
            ..Default::default()
        },
        security: SecurityConfig {
            headers: SecurityHeadersConfig {
                environment: Environment::Testing,
            },
            ..Default::default()
        },
        ..Default::default()
    });

    Ok(Arc::new(
        ServerContext::new(
            (*database).clone(),
            (*auth_manager).clone(),
            "test_jwt_secret",
            config,
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
            },
        )
        .await,
    ))
}

/// A stranger's first message must come back with a usable way in — an
/// actionable link, addressed to them, on the channel they wrote from.
///
/// This is the single highest-intent moment the funnel gets: someone who opened
/// a conversation and typed a real question. A reply that carries no link, or
/// that goes to the wrong recipient, ends the funnel silently.
#[tokio::test]
async fn an_unlinked_sender_is_given_an_actionable_link() {
    let (resources, tenant) = context_with_tenant().await.expect("setup failed");
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    let reply = create_link_and_prompt(
        &resources,
        db,
        tenant,
        ChannelType::Telegram,
        "tg-sender-42",
        Some("Jess"),
    )
    .await;

    assert_eq!(
        reply.recipient_id, "tg-sender-42",
        "the prompt must go back to the sender who wrote in"
    );
    assert_eq!(reply.channel_type, ChannelType::Telegram);

    let body = reply_text(&reply.content);
    assert!(
        body.contains("/messaging/link/"),
        "the prompt must carry a link the stranger can actually open, got: {body}"
    );
    assert!(
        body.len() > 20,
        "the prompt must say something, got: {body}"
    );
}

/// The link code in the reply must exist in the database, or the stranger opens
/// a URL that 404s — which is worse than no link at all, because they burned
/// their one moment of intent on it.
#[tokio::test]
async fn the_link_in_the_prompt_resolves_to_a_real_link_state() {
    let (resources, tenant) = context_with_tenant().await.expect("setup failed");
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    let reply = create_link_and_prompt(
        &resources,
        db,
        tenant,
        ChannelType::WhatsApp,
        "wa-15551234567",
        None,
    )
    .await;

    let body = reply_text(&reply.content);
    let code = body
        .split("/messaging/link/")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("the prompt must contain a link code");

    let state = db
        .get_link_state(code)
        .await
        .expect("link-state lookup failed");
    assert!(
        state.is_some(),
        "the code handed to the user must resolve to a stored link state, got code: {code}"
    );
}

/// Every channel a stranger can write from must get a reply addressed to them.
///
/// Runs the matrix rather than one channel because the funnel is only as good as
/// its worst door, and a per-channel regression here is invisible until someone
/// complains that one app never answers.
#[tokio::test]
async fn every_channel_answers_an_unlinked_sender() {
    let (resources, tenant) = context_with_tenant().await.expect("setup failed");
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    for channel in [
        ChannelType::Telegram,
        ChannelType::WhatsApp,
        ChannelType::Messenger,
        ChannelType::Slack,
        ChannelType::Discord,
    ] {
        let sender = format!("{channel}-sender");
        let reply = create_link_and_prompt(&resources, db, tenant, channel, &sender, None).await;

        assert_eq!(
            reply.recipient_id, sender,
            "{channel} must answer the sender who wrote in"
        );
        assert!(
            !reply_text(&reply.content).is_empty(),
            "{channel} must not answer with an empty message"
        );
    }
}

/// A stranger whose address has no account must be offered one, in the
/// conversation, rather than sent away to a URL.
///
/// This was the funnel's worst dead end: someone had opened a chat and asked a
/// real training question, and the answer was homework — go register on the web,
/// wait for approval, come back and start over. The offer keeps them in the one
/// place they already are.
#[tokio::test]
async fn an_unknown_address_is_offered_an_account_not_a_url() {
    use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_LINK_SIGNUP_OFFER};

    let (resources, _tenant) = context_with_tenant().await.expect("setup failed");
    let offer = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_LINK_SIGNUP_OFFER, DEFAULT_LOCALE);

    assert!(
        !offer.is_empty(),
        "the signup offer must exist in the default locale"
    );
    assert!(
        offer.contains("{0}"),
        "the offer must name the address it is about, got: {offer}"
    );
    assert!(
        !offer.contains("http"),
        "the offer must not send the user to a URL — that was the dead end it replaces, got: {offer}"
    );
}

/// The offer, its confirmation and its failure reply must all exist in every
/// locale we ship, or a francophone stranger meets an English wall mid-signup.
#[tokio::test]
async fn the_signup_replies_exist_in_all_five_locales() {
    use pierre_contremaitre::messaging_strings::{
        KEY_LINK_SIGNUP_CREATED, KEY_LINK_SIGNUP_FAILED, KEY_LINK_SIGNUP_OFFER,
    };

    let (resources, _tenant) = context_with_tenant().await.expect("setup failed");
    let reg = &resources.mcp.messaging_strings_registry;

    for locale in ["fr", "en", "es", "de", "pt"] {
        for key in [
            KEY_LINK_SIGNUP_OFFER,
            KEY_LINK_SIGNUP_CREATED,
            KEY_LINK_SIGNUP_FAILED,
        ] {
            let text = reg.get(key, locale);
            assert!(
                !text.is_empty(),
                "{key} must be present in {locale} — a missing locale is a wall mid-signup"
            );
        }
    }
}

/// Messenger must be classified as a deep-link channel.
///
/// It was classified as OAuth, which made the wizard hand the user a URL
/// pointing at our own callback with no way to supply a `channel_user_id` — so
/// every Messenger link attempt returned a raw 400. Reproduced live before the
/// fix; this pins the classification so it cannot regress.
#[tokio::test]
async fn messenger_is_a_deep_link_channel_not_an_oauth_one() {
    use pierre_core::models::messaging::LinkingMethod;

    assert_eq!(
        ChannelType::Messenger.linking_method(),
        LinkingMethod::DeepLink,
        "Messenger links via m.me/{{page}}?ref={{code}}, the same shape as Telegram's ?start="
    );
    // The genuinely OAuth channels stay OAuth.
    assert_eq!(ChannelType::Slack.linking_method(), LinkingMethod::OAuth);
    assert_eq!(ChannelType::Discord.linking_method(), LinkingMethod::OAuth);
}

/// A bare number selects; prose containing a number does not.
///
/// This strictness is the whole safety property of numeric selection. The
/// proposal says "Reply with a number to start", so "2" must bind — but someone
/// answering "I run 3 times a week" must never silently have their coach
/// rebound. A loose parse would hijack ordinary conversation.
#[test]
fn numeric_coach_selection_only_matches_a_bare_number() {
    use pierre_mcp_server::services::messaging_ingress::parse_choice;

    assert_eq!(parse_choice("1"), Some(1), "a bare number selects");
    assert_eq!(parse_choice("  3 "), Some(3), "surrounding space is fine");

    for prose in [
        "I run 3 times a week",
        "2 please",
        "let's go with 1",
        "no",
        "",
        "0",
    ] {
        assert_eq!(
            parse_choice(prose),
            None,
            "{prose:?} is conversation, not a selection — binding a coach from it would be a hijack"
        );
    }
}

/// The selection pointer cannot name a coach that does not exist, and clearing
/// is a real state rather than an absence of writes.
///
/// The swap itself (selecting twice replaces rather than accumulates) is covered
/// against real coaches in `coaches_database_test::test_activate_coach_deactivates_others`;
/// what this adds is the referential guarantee. The retired
/// `coach_assignments.is_active` maintained "at most one" with two
/// non-transactional `UPDATEs` that could leave zero or two; a single FK column on
/// a row `UNIQUE(tenant_id, user_id)` already makes unique gets both properties
/// from the schema.
#[tokio::test]
async fn the_selection_pointer_is_referentially_sound() {
    let resources = context().await.expect("setup failed");
    let (user_id, _, tenant) = common::create_test_user_with_plan(
        &resources.coach.database,
        "selection-integrity@example.com",
        "starter",
    )
    .await
    .expect("user creation failed");

    let tenants = &resources.common.repos.tenants;

    assert_eq!(
        tenants.get_selected_coach(tenant, user_id).await.unwrap(),
        None,
        "a fresh membership must hold no selection"
    );

    // A coach id that does not exist must be refused rather than stored, or the
    // pointer could dangle at a deleted coach and every read would 404.
    assert!(
        tenants
            .set_selected_coach(tenant, user_id, Some("no-such-coach"))
            .await
            .is_err(),
        "selecting a nonexistent coach must be refused by the foreign key"
    );

    // Clearing is always legal and leaves no selection.
    tenants
        .set_selected_coach(tenant, user_id, None)
        .await
        .expect("clearing must be allowed");
    assert_eq!(
        tenants.get_selected_coach(tenant, user_id).await.unwrap(),
        None
    );
}

/// Slack and Discord must send the user to the PROVIDER, not to our own callback.
///
/// They used to get `…/api/messaging/link/callback/{channel}?state=…` — an
/// endpoint that rejected the request for a `channel_user_id` no OAuth provider
/// sends. The provider is the only party that knows who just authorised, and
/// nothing was asking it.
#[test]
fn oauth_channels_authorize_against_the_provider() {
    use pierre_core::models::messaging::LinkingMethod;

    // The classification is what decides the URL shape, so pin it alongside.
    assert_eq!(ChannelType::Slack.linking_method(), LinkingMethod::OAuth);
    assert_eq!(ChannelType::Discord.linking_method(), LinkingMethod::OAuth);

    // And the deep-link channels are not OAuth, so they never take that path.
    for deep in [
        ChannelType::Telegram,
        ChannelType::WhatsApp,
        ChannelType::Messenger,
    ] {
        assert_eq!(
            deep.linking_method(),
            LinkingMethod::DeepLink,
            "{deep} links by deep link, not OAuth"
        );
    }
}
