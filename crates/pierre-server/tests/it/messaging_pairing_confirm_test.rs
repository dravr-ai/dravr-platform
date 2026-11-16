// ABOUTME: A chat pairing link names the account it links to and links only on the redeemer's yes
// ABOUTME: Drives real Telegram and Messenger webhooks and reads the bot's captured replies and the link rows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1596.
//!
//! A pairing link (`t.me/<bot>?start=<code>`, `wa.me/…?text=LINK <code>`,
//! `m.me/<page>?ref=<code>`) names the Dravr account that started it and
//! nothing about who opens it. Forwarded, it bound whoever tapped Start to the
//! sender's account, which then answered that person's messages to the bot.
//!
//! Redeeming a code now holds it for the sender and asks: the reply names the
//! account by its masked email, and only a yes from that same sender links.
//! Anything else asks again; `cancel` hands the code back unspent, so the
//! account that started it can still finish it.

#[cfg(feature = "client-messaging")]
mod pairing_confirm {
    use std::sync::Arc;
    use std::time::Duration as StdDuration;

    use axum::http::StatusCode;
    use axum::Router;
    use chrono::{Duration, Utc};
    use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_LINK_CANCELLED};
    use pierre_core::models::messaging::MessageContent;
    use pierre_core::models::TenantId;
    use pierre_database::backends::{
        CreateLinkStateParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_email::ResendEmailService;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_services::channel_adapters::ChannelAdapterFactory;
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::{create_test_server_resources, create_test_user_with_plan};
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::messaging_webhooks::{
        messenger_referral_webhook, messenger_webhook, telegram_photo_webhook, telegram_webhook,
        ChannelSecrets, WebhookRequest,
    };
    use crate::helpers::offline_channel::OfflineSendAdapters;

    const STARTER_EMAIL: &str = "pairing-starter-1596@example.com";
    /// How the starter's email reads in the question.
    const STARTER_MASKED: &str = "p***@example.com";
    const PAGE_ID: &str = "pairing_page_1596";
    /// The person the starter forwarded the link to.
    const FORWARDED_TO: &str = "5551596";
    /// The starter's own chat identity.
    const STARTER_CHAT: &str = "4441596";

    struct Bot {
        resources: Arc<ServerContext>,
        router: Router,
        adapters: Arc<OfflineSendAdapters>,
        tenant_id: TenantId,
        starter_id: Uuid,
        secrets: ChannelSecrets,
    }

    impl Bot {
        fn db(&self) -> &dyn MessagingRepository {
            &*self.resources.common.repos.messaging
        }

        /// A pairing code the starter minted for `channel`, as the link init
        /// route stores it.
        async fn started(&self, channel: &str) -> String {
            let code = Uuid::new_v4().simple().to_string();
            let starter = self.starter_id.to_string();
            let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();
            self.db()
                .create_link_state(&CreateLinkStateParams {
                    id: &Uuid::new_v4().to_string(),
                    tenant_id: self.tenant_id,
                    user_id: Some(&starter),
                    channel_type: channel,
                    code: &code,
                    method: "deep_link",
                    channel_user_id: None,
                    sender_name: None,
                    app_return_uri: None,
                    expires_at: &expires_at,
                })
                .await
                .unwrap();
            code
        }

        /// Deliver `request` and return the text of the first reply the bot
        /// sent to it. The webhook answers before the reply goes out, so the
        /// reply is waited for.
        async fn deliver(&self, request: WebhookRequest) -> String {
            let before = self.adapters.sends().len();
            let mut req = AxumTestRequest::post(&request.path);
            for (key, value) in &request.headers {
                req = req.header(key, value);
            }
            let resp = req
                .json(&request.body_value)
                .send(self.router.clone())
                .await;
            assert_eq!(resp.status_code(), StatusCode::OK, "{}", request.path);
            for _ in 0..200 {
                if let Some(reply) = self.adapters.sends().get(before) {
                    return match &reply.content {
                        MessageContent::Text { body } => body.clone(),
                        other => format!("{other:?}"),
                    };
                }
                sleep(StdDuration::from_millis(50)).await;
            }
            panic!("the bot answered nothing to {}", request.path);
        }

        async fn telegram(&self, sender: &str, text: &str, msg_id: i64) -> String {
            self.deliver(telegram_webhook(text, msg_id, sender, &self.secrets))
                .await
        }

        async fn linked_to(&self, channel: &str, sender: &str) -> Option<String> {
            self.db()
                .get_channel_link(self.tenant_id, channel, sender)
                .await
                .unwrap()
                .map(|link| link["user_id"].as_str().unwrap().to_owned())
        }

        async fn unspent(&self, code: &str) -> bool {
            self.db().get_link_state(code).await.unwrap().is_some()
        }
    }

    /// The bot as production runs it, with an email service, so a sender
    /// with no flow is offered the in-chat OTP exchange.
    async fn bot() -> Bot {
        let mut resources = create_test_server_resources().await.unwrap();
        Arc::get_mut(&mut resources)
            .expect("no clone of the fresh server context exists yet")
            .common
            .email_service = Some(Arc::new(
            ResendEmailService::new(
                "re_fake_test_api_key".to_owned(),
                "Pierre <noreply@test.pierre.dev>".to_owned(),
            )
            .expect("a non-empty key builds the email service"),
        ));
        let (starter_id, _user, tenant_id) =
            create_test_user_with_plan(&resources.agent.database, STARTER_EMAIL, "professional")
                .await
                .unwrap();
        let secrets = ChannelSecrets::generate();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(&secrets.telegram_webhook_secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:PAIRING_BOT"),
            is_active: true,
        })
        .await
        .unwrap();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "messenger",
            api_key: Some("messenger-pairing-token"),
            api_secret: Some(&secrets.messenger_app_secret),
            webhook_secret: Some(&secrets.messenger_app_secret),
            verify_token: None,
            account_id: Some(PAGE_ID),
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
        let adapters = Arc::new(OfflineSendAdapters::default());
        let factory: Arc<dyn ChannelAdapterFactory> = adapters.clone();
        let router = MessagingRoutes::routes_with_adapters(Arc::clone(&resources), factory);
        Bot {
            resources,
            router,
            adapters,
            tenant_id,
            starter_id,
            secrets,
        }
    }

    /// The forwarded-link case end to end: the person who taps Start is told
    /// whose account the link reaches, and nothing links until they say yes.
    #[tokio::test]
    async fn a_redeemed_link_names_the_account_and_links_only_on_yes() {
        let bot = bot().await;
        let code = bot.started("telegram").await;

        let question = bot
            .telegram(FORWARDED_TO, &format!("/start {code}"), 1)
            .await;
        assert!(
            question.contains(STARTER_MASKED) && question.contains("Telegram"),
            "the question names the channel and the account: {question}"
        );
        assert!(!question.contains(STARTER_EMAIL), "the email is masked");
        assert_eq!(
            bot.linked_to("telegram", FORWARDED_TO).await,
            None,
            "bug: the code alone linked"
        );
        assert!(
            bot.unspent(&code).await,
            "bug: asking the question spent the code"
        );

        let asked_again = bot.telegram(FORWARDED_TO, "what is this?", 2).await;
        assert_eq!(asked_again, question, "anything but yes asks again");
        assert_eq!(
            bot.linked_to("telegram", FORWARDED_TO).await,
            None,
            "bug: a non-yes linked"
        );

        bot.telegram(FORWARDED_TO, "Yes", 3).await;
        assert_eq!(
            bot.linked_to("telegram", FORWARDED_TO).await,
            Some(bot.starter_id.to_string()),
            "bug: the holder's yes did not link them to the starter's account"
        );
        assert!(!bot.unspent(&code).await, "a confirmed code is spent");
    }

    /// The person a link was forwarded to refuses it: the code goes back
    /// unspent and the starter finishes their own link with it.
    #[tokio::test]
    async fn cancel_hands_the_code_back_to_the_account_that_started_it() {
        let bot = bot().await;
        let code = bot.started("telegram").await;
        bot.telegram(FORWARDED_TO, &format!("/start {code}"), 1)
            .await;

        let cancelled = bot.telegram(FORWARDED_TO, "cancel", 2).await;

        let expected = bot
            .resources
            .mcp
            .messaging_strings_registry
            .get(KEY_LINK_CANCELLED, DEFAULT_LOCALE);
        assert_eq!(
            cancelled, expected,
            "bug: cancel did not confirm the cancellation"
        );
        assert!(bot.unspent(&code).await, "a refused code stays unspent");
        assert_eq!(
            bot.linked_to("telegram", FORWARDED_TO).await,
            None,
            "bug: cancel linked"
        );

        bot.telegram(STARTER_CHAT, &format!("/start {code}"), 3)
            .await;
        bot.telegram(STARTER_CHAT, "yes", 4).await;
        assert_eq!(
            bot.linked_to("telegram", STARTER_CHAT).await,
            Some(bot.starter_id.to_string()),
            "bug: the starter could not finish a code handed back by cancel"
        );
    }

    /// A photo or a sticker is not a yes: it asks again and leaves the code
    /// held, rather than opening an in-chat OTP flow that would spend the
    /// starter's code.
    #[tokio::test]
    async fn a_non_text_reply_asks_again_and_keeps_the_code_held() {
        let bot = bot().await;
        let code = bot.started("telegram").await;
        let question = bot
            .telegram(FORWARDED_TO, &format!("/start {code}"), 1)
            .await;

        let asked_again = bot
            .deliver(telegram_photo_webhook(2, FORWARDED_TO, &bot.secrets))
            .await;

        assert_eq!(
            asked_again, question,
            "bug: a photo from the holder did not ask the question again"
        );
        assert!(
            bot.unspent(&code).await,
            "bug: a photo from the holder spent the starter's code"
        );
        let held = bot
            .db()
            .get_active_otp_link_state(bot.tenant_id, "telegram", FORWARDED_TO)
            .await
            .unwrap()
            .expect("bug: a photo from the holder dropped the hold");
        assert_eq!(
            held["code"],
            code.as_str(),
            "bug: the holder's live flow is no longer the pairing code"
        );
    }

    /// A yes from someone who does not hold the code is an ordinary message:
    /// it links nobody, and the holder's question still stands.
    #[tokio::test]
    async fn a_yes_from_another_sender_links_nobody() {
        let bot = bot().await;
        let code = bot.started("telegram").await;
        bot.telegram(FORWARDED_TO, &format!("/start {code}"), 1)
            .await;

        bot.telegram(STARTER_CHAT, "yes", 2).await;

        assert_eq!(
            bot.linked_to("telegram", STARTER_CHAT).await,
            None,
            "bug: a non-holder's yes linked them"
        );
        assert_eq!(
            bot.linked_to("telegram", FORWARDED_TO).await,
            None,
            "bug: a non-holder's yes linked the holder"
        );
        assert!(
            bot.unspent(&code).await,
            "bug: a non-holder's yes spent the code"
        );
    }

    /// An `m.me` referral reaches the same question. Before carnet#1596 the
    /// bare code it arrives as was read as ordinary text, so a Messenger link
    /// never completed.
    #[tokio::test]
    async fn a_messenger_referral_asks_then_links_on_yes() {
        let bot = bot().await;
        let code = bot.started("messenger").await;

        let question = bot
            .deliver(messenger_referral_webhook(
                &code,
                FORWARDED_TO,
                PAGE_ID,
                &bot.secrets,
            ))
            .await;
        assert!(
            question.contains(STARTER_MASKED) && question.contains("Messenger"),
            "the question names the channel and the account: {question}"
        );
        assert_eq!(
            bot.linked_to("messenger", FORWARDED_TO).await,
            None,
            "bug: the referral alone linked"
        );

        bot.deliver(messenger_webhook(
            "oui",
            2,
            FORWARDED_TO,
            PAGE_ID,
            &bot.secrets,
        ))
        .await;
        assert_eq!(
            bot.linked_to("messenger", FORWARDED_TO).await,
            Some(bot.starter_id.to_string()),
            "bug: a yes after a Messenger referral did not link"
        );
    }
}
