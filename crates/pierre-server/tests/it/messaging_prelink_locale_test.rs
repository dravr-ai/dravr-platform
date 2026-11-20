// ABOUTME: A sender with no account is asked in the language their channel suggests, from first prompt to last
// ABOUTME: Drives real WhatsApp and Telegram webhooks, reads the bot's captured replies and each flow's stored language

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1669.
//!
//! Every prompt a sender read before linking an account was French, the
//! default locale, whatever they spoke: an English athlete writing to the
//! `WhatsApp` number was asked « écris ton adresse e-mail ». The flow now opens
//! in the language the channel suggests for the sender — a `WhatsApp` number's
//! country calling code (a +1 number splits by area code: Quebec's read
//! French, the rest English), a Telegram update's `language_code` — records it
//! on the link state, and every later prompt of the flow reads it from there.

#[cfg(feature = "client-messaging")]
mod prelink_locale {
    use std::sync::Arc;
    use std::time::Duration as StdDuration;

    use axum::http::StatusCode;
    use axum::Router;
    use chrono::{Duration, Utc};
    use pierre_contremaitre::messaging_strings::{
        DEFAULT_LOCALE, KEY_ACCOUNT_PENDING, KEY_LINK_CANCELLED, KEY_LINK_CONFIRM_PROMPT,
        KEY_LINK_EMAIL_PROMPT, KEY_LINK_EMAIL_SEND_FAILED, KEY_LINK_INITIAL_PROMPT,
        KEY_LINK_INVALID_EMAIL, KEY_LINK_LOGOUT_COMPLETE, KEY_LINK_SIGNUP_OFFER, KEY_LINK_SUCCESS,
    };
    use pierre_core::models::messaging::{channel_label, ChannelType, MessageContent};
    use pierre_core::models::{TenantId, User, UserStatus};
    use pierre_database::backends::{
        CreateChannelLinkParams, CreateLinkStateParams, MessagingRepository,
        UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_mcp_server::services::messaging_ingress::create_link_and_prompt;
    use pierre_services::channel_adapters::ChannelAdapterFactory;
    use serde_json::json;
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::{
        create_test_server_resources, create_test_user_with_plan, offline_email_service,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::messaging_webhooks::{
        whatsapp_webhook, ChannelSecrets, TestChannel, WebhookRequest,
    };
    use crate::helpers::offline_channel::OfflineSendAdapters;

    /// The business number's id, as the webhook names it and the config stores it.
    const PHONE_NUMBER_ID: &str = "15550001669";
    const STARTER_EMAIL: &str = "prelink-starter-1669@example.com";
    /// How the starter's email reads in a pairing question.
    const STARTER_MASKED: &str = "p***@example.com";

    const MONTREAL: &str = "15145550101";
    const NEW_YORK: &str = "12125550102";
    const MADRID: &str = "34612345103";
    const BERLIN: &str = "4915112345104";
    const LISBON: &str = "351912345105";
    /// A country whose language the platform does not speak, which reads
    /// English (owner decision, carnet#1669).
    const AMSTERDAM: &str = "31612345106";
    /// A sender whose webhook showed no phone number, whom dravr-canot names by
    /// their business-scoped user id alone: its country places them. (The
    /// `"unknown"` canot v0.11.0 wrote for such a sender is refused before any
    /// reply since carnet#1668.)
    const NUMBERLESS_IN_SPAIN: &str = "ES.13491208655302741918";

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

        /// What the catalogue says for `key` in `locale`.
        fn says(&self, key: &str, locale: &str) -> String {
            self.resources
                .mcp
                .messaging_strings_registry
                .get(key, locale)
        }

        /// The language the `WhatsApp` sender's live flow stored.
        async fn flow_locale(&self, sender: &str) -> String {
            self.flow_locale_on("whatsapp", sender).await
        }

        /// The language the sender's live flow on `channel` stored.
        async fn flow_locale_on(&self, channel: &str, sender: &str) -> String {
            let flow = self
                .db()
                .get_active_otp_link_state(self.tenant_id, channel, sender)
                .await
                .unwrap()
                .expect("the sender has a live flow");
            flow["locale"]
                .as_str()
                .expect("the flow holds a language")
                .to_owned()
        }

        /// Deliver a `WhatsApp` text from `sender` and return the text of the
        /// first reply the bot sent to it. The webhook answers before the
        /// reply goes out, so the reply is waited for.
        async fn whatsapp(&self, sender: &str, text: &str, msg_id: i64) -> String {
            self.deliver(whatsapp_webhook(
                text,
                msg_id,
                sender,
                PHONE_NUMBER_ID,
                &self.secrets,
            ))
            .await
        }

        /// Deliver a Telegram text from `sender` whose Telegram app speaks
        /// `language_code`, and return the text of the first reply.
        async fn telegram(
            &self,
            sender: i64,
            language_code: &str,
            text: &str,
            msg_id: i64,
        ) -> String {
            self.deliver(WebhookRequest {
                path: TestChannel::Telegram.webhook_path(),
                headers: vec![(
                    "x-telegram-bot-api-secret-token".to_owned(),
                    self.secrets.telegram_webhook_secret.clone(),
                )],
                body_value: json!({
                    "update_id": 1669 + msg_id,
                    "message": {
                        "message_id": msg_id,
                        "from": { "id": sender, "first_name": "Ana", "language_code": language_code },
                        "chat": { "id": sender, "type": "private" },
                        "text": text,
                    }
                }),
            })
            .await
        }

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
            assert_eq!(
                resp.status_code(),
                StatusCode::OK,
                "bug: the webhook at {} refused a signed delivery",
                request.path
            );
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

        /// A pairing code `owner` minted for `WhatsApp`, as the link init route
        /// stores it.
        async fn started_by(&self, owner: Uuid) -> String {
            let code = Uuid::new_v4().simple().to_string();
            let owner = owner.to_string();
            let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();
            self.db()
                .create_link_state(&CreateLinkStateParams {
                    id: &Uuid::new_v4().to_string(),
                    tenant_id: self.tenant_id,
                    user_id: Some(&owner),
                    channel_type: "whatsapp",
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

        /// Set the language an account reads.
        async fn reads(&self, user_id: Uuid, locale: &str) {
            self.resources
                .common
                .repos
                .users
                .update_locale(user_id, locale)
                .await
                .unwrap();
        }
    }

    /// The bot as production runs it, with an email service, so a sender
    /// with no flow is offered the in-chat OTP exchange.
    async fn bot() -> Bot {
        let mut resources = create_test_server_resources().await.unwrap();
        Arc::get_mut(&mut resources)
            .expect("no clone of the fresh server context exists yet")
            .common
            .email_service = Some(offline_email_service());
        let (starter_id, _user, tenant_id) =
            create_test_user_with_plan(&resources.agent.database, STARTER_EMAIL, "professional")
                .await
                .unwrap();
        let secrets = ChannelSecrets::generate();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("wa-prelink-access-token"),
            api_secret: None,
            webhook_secret: Some(&secrets.whatsapp_app_secret),
            verify_token: None,
            account_id: Some("test-wa-account"),
            phone_number: Some(PHONE_NUMBER_ID),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
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
            bot_token: Some("16690:PRELINK_BOT"),
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

    #[tokio::test]
    async fn a_stranger_is_asked_in_the_language_of_their_number() {
        let bot = bot().await;
        assert_ne!(
            bot.says(KEY_LINK_EMAIL_PROMPT, "en"),
            bot.says(KEY_LINK_EMAIL_PROMPT, DEFAULT_LOCALE),
            "bug: the catalogue words the prompt the same in English and French, so this test proves nothing"
        );

        let numbered_senders = (1..).zip([
            (MONTREAL, "fr"),
            (NEW_YORK, "en"),
            (MADRID, "es"),
            (BERLIN, "de"),
            (LISBON, "pt"),
            (AMSTERDAM, "en"),
            (NUMBERLESS_IN_SPAIN, "es"),
        ]);
        for (msg_id, (sender, locale)) in numbered_senders {
            let asked = bot.whatsapp(sender, "Hello", msg_id).await;

            assert_eq!(
                asked,
                bot.says(KEY_LINK_EMAIL_PROMPT, locale),
                "bug: {sender} was not asked for their address in {locale}"
            );
            assert_eq!(
                bot.flow_locale(sender).await,
                locale,
                "bug: {sender}'s flow did not record {locale}"
            );
        }
    }

    /// A Telegram stranger is asked in their Telegram app's language, and the
    /// flow keeps it when a later message arrives from a device set to
    /// another one.
    #[tokio::test]
    async fn a_telegram_stranger_is_asked_in_their_apps_language_and_kept_in_it() {
        let bot = bot().await;
        let sender = 4_441_669;

        let asked = bot.telegram(sender, "en", "Hello", 1).await;
        assert_eq!(
            asked,
            bot.says(KEY_LINK_EMAIL_PROMPT, "en"),
            "bug: a Telegram sender in English was not asked in English"
        );
        assert_eq!(
            bot.flow_locale_on("telegram", &sender.to_string()).await,
            "en",
            "bug: the Telegram flow did not record its language"
        );

        let from_another_device = bot.telegram(sender, "es", "not an address", 2).await;
        assert_eq!(
            from_another_device,
            bot.says(KEY_LINK_INVALID_EMAIL, "en"),
            "bug: the flow switched language with the device the sender wrote from"
        );

        let unspoken = bot.telegram(4_441_670, "nl", "Hallo", 3).await;
        assert_eq!(
            unspoken,
            bot.says(KEY_LINK_EMAIL_PROMPT, "en"),
            "bug: a Telegram app language the platform does not speak was not answered in English"
        );
    }

    /// Without an email service the stranger gets a link to the web page
    /// instead, worded in the language they were given.
    #[tokio::test]
    async fn the_link_page_prompt_is_worded_in_the_language_given() {
        let bot = bot().await;
        let prompt = create_link_and_prompt(
            &bot.resources,
            bot.db(),
            bot.tenant_id,
            ChannelType::WhatsApp,
            MADRID,
            None,
            "es",
        )
        .await;

        let MessageContent::Text { body } = &prompt.content else {
            panic!("the link prompt is text: {:?}", prompt.content);
        };
        let template = bot.says(KEY_LINK_INITIAL_PROMPT, "es");
        let (before_link, _) = template
            .split_once("{0}")
            .expect("the prompt template carries the link");
        assert!(
            body.starts_with(before_link) && body.contains("/messaging/link/"),
            "bug: the link prompt is not the Spanish one: {body}"
        );
    }

    /// Every prompt of one exchange is in the language it opened in: a wrong
    /// address, an unknown one, an unclear answer to the offer, and cancel.
    #[tokio::test]
    async fn the_flow_keeps_its_language_to_the_end() {
        let bot = bot().await;
        bot.whatsapp(NEW_YORK, "Hi there", 1).await;

        let not_an_address = bot.whatsapp(NEW_YORK, "my email is secret", 2).await;
        assert_eq!(
            not_an_address,
            bot.says(KEY_LINK_INVALID_EMAIL, "en"),
            "bug: a wrong address was answered outside the flow's language"
        );

        let unknown = "no-account-1669@example.com";
        let offer = bot.whatsapp(NEW_YORK, unknown, 3).await;
        let offer_in_english = bot.resources.mcp.messaging_strings_registry.render(
            KEY_LINK_SIGNUP_OFFER,
            "en",
            &[unknown],
        );
        assert_eq!(
            offer, offer_in_english,
            "bug: the sign-up offer was made outside the flow's language"
        );
        let unclear = bot.whatsapp(NEW_YORK, "hmm, not sure", 4).await;
        assert_eq!(
            unclear, offer_in_english,
            "bug: the offer was asked again outside the flow's language"
        );

        let cancelled = bot.whatsapp(NEW_YORK, "cancel", 5).await;
        assert_eq!(
            cancelled,
            bot.says(KEY_LINK_CANCELLED, "en"),
            "bug: cancel was confirmed outside the flow's language"
        );
    }

    /// The language is read from the flow, not chosen again per message: a
    /// flow recorded in German stays German for a New York number.
    #[tokio::test]
    async fn a_flow_answers_in_the_language_it_recorded() {
        let bot = bot().await;
        let state_id = Uuid::new_v4().to_string();
        let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();
        bot.db()
            .create_link_state(&CreateLinkStateParams {
                id: &state_id,
                tenant_id: bot.tenant_id,
                user_id: None,
                channel_type: "whatsapp",
                code: &Uuid::new_v4().to_string(),
                method: "otp",
                channel_user_id: Some(NEW_YORK),
                sender_name: None,
                app_return_uri: None,
                expires_at: &expires_at,
            })
            .await
            .unwrap();
        bot.db()
            .open_otp_flow_on_link_state(&state_id, bot.tenant_id, "de")
            .await
            .unwrap();

        let answered = bot.whatsapp(NEW_YORK, "not an address", 1).await;

        assert_eq!(
            answered,
            bot.says(KEY_LINK_INVALID_EMAIL, "de"),
            "bug: the flow chose its language again instead of reading the one it recorded"
        );
    }

    /// An account created in the chat reads the language the chat was held
    /// in, so the replies that follow the link do not switch to the default.
    #[tokio::test]
    async fn an_account_made_in_the_chat_reads_the_language_of_the_chat() {
        let bot = bot().await;
        let address = "new-athlete-1669@example.com";
        bot.whatsapp(NEW_YORK, "Hello", 1).await;
        bot.whatsapp(NEW_YORK, address, 2).await;

        // The offline email service refuses the code it is handed, so the
        // account is made and the send fails, in the flow's language.
        let after_yes = bot.whatsapp(NEW_YORK, "yes", 3).await;

        assert_eq!(
            after_yes,
            bot.says(KEY_LINK_EMAIL_SEND_FAILED, "en"),
            "bug: the step after the sign-up was answered outside the flow's language"
        );
        let account = bot
            .resources
            .common
            .repos
            .users
            .get_by_email(address)
            .await
            .unwrap()
            .expect("yes created the account");
        assert_eq!(
            account.locale, "en",
            "bug: the account made in an English chat does not read English"
        );
    }

    /// A pairing code redeemed on `WhatsApp` asks in the sender's language and
    /// keeps it; the link itself is confirmed in the account's own language.
    #[tokio::test]
    async fn a_pairing_question_is_asked_in_the_language_of_the_number() {
        let bot = bot().await;
        bot.reads(bot.starter_id, "pt").await;
        let code = bot.started_by(bot.starter_id).await;

        let question = bot.whatsapp(NEW_YORK, &format!("LINK {code}"), 1).await;
        let in_english = bot.resources.mcp.messaging_strings_registry.render(
            KEY_LINK_CONFIRM_PROMPT,
            "en",
            &[channel_label("whatsapp"), STARTER_MASKED],
        );
        assert_eq!(
            question, in_english,
            "bug: the pairing question was not asked in the number's language"
        );
        assert_eq!(
            bot.flow_locale(NEW_YORK).await,
            "en",
            "bug: the held code did not record the language its sender is asked in"
        );
        let asked_again = bot.whatsapp(NEW_YORK, "what is this?", 2).await;
        assert_eq!(
            asked_again, in_english,
            "bug: the question was asked again outside its language"
        );

        let linked = bot.whatsapp(NEW_YORK, "yes", 3).await;
        assert_eq!(
            linked,
            bot.says(KEY_LINK_SUCCESS, "pt"),
            "bug: the link was not confirmed in the account's own language"
        );
    }

    /// An account still waiting for approval is told so in its own language
    /// the moment the link is made, not in the default.
    #[tokio::test]
    async fn a_pending_account_hears_it_is_pending_in_its_own_language() {
        let bot = bot().await;
        let mut pending = User::new(
            "pending-1669@example.com".to_owned(),
            String::new(),
            Some("Pending".to_owned()),
        );
        pending.user_status = UserStatus::Pending;
        pending.locale = "es".to_owned();
        bot.resources
            .common
            .repos
            .users
            .create(&pending)
            .await
            .unwrap();
        let code = bot.started_by(pending.id).await;
        bot.whatsapp(MADRID, &format!("LINK {code}"), 1).await;

        let linked = bot.whatsapp(MADRID, "si", 2).await;

        assert_eq!(
            linked,
            bot.says(KEY_ACCOUNT_PENDING, "es"),
            "bug: a pending account was told so outside its own language"
        );
    }

    /// A logout is acknowledged in the athlete's own language while the link
    /// still names them, and in the number's language for a stranger.
    #[tokio::test]
    async fn a_logout_is_acknowledged_in_the_senders_language() {
        let bot = bot().await;
        bot.reads(bot.starter_id, "de").await;
        bot.db()
            .create_channel_link(&CreateChannelLinkParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: bot.tenant_id,
                user_id: &bot.starter_id.to_string(),
                channel_type: "whatsapp",
                channel_user_id: MONTREAL,
                display_name: None,
            })
            .await
            .unwrap();

        let linked = bot.whatsapp(MONTREAL, "logout", 1).await;
        assert_eq!(
            linked,
            bot.says(KEY_LINK_LOGOUT_COMPLETE, "de"),
            "bug: a linked athlete's logout was not acknowledged in their own language"
        );

        let stranger = bot.whatsapp(NEW_YORK, "logout", 2).await;
        assert_eq!(
            stranger,
            bot.says(KEY_LINK_LOGOUT_COMPLETE, "en"),
            "bug: a stranger's logout was not acknowledged in their number's language"
        );
    }
}
