// ABOUTME: Auto-discovering coverage invariant: every slash command dispatches on every channel
// ABOUTME: Loads command markdown at runtime; new commands require no test maintenance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(
    missing_docs,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::similar_names,
    clippy::module_name_repetitions
)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod coverage {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::messaging_webhooks::{
        discord_webhook, messenger_webhook, slack_webhook, telegram_webhook, whatsapp_webhook,
        ChannelSecrets, TestChannel, WebhookRequest,
    };
    use axum::http::StatusCode;
    use chrono::Utc;
    use pierre_database::plugins::{
        CreateChannelLinkParams, CreateSessionParams, MessagingRepository,
        UpsertChannelConfigParams,
    };
    use pierre_mcp_server::commands::load_command_definitions;
    use pierre_mcp_server::mcp::resources::ServerResources;
    use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    /// Matches `phone_number_id` sent in the `WhatsApp` webhook fixture.
    const TEST_PHONE_NUMBER_ID: &str = "test_wa_phone_123";
    /// Matches both `entry[].id` and `recipient.id` in the Messenger fixture.
    const TEST_PAGE_ID: &str = "test_messenger_page_123";

    struct HarnessSenders {
        telegram: String,
        slack: String,
        discord: String,
        whatsapp: String,
        messenger: String,
    }

    impl HarnessSenders {
        fn for_channel(&self, channel: TestChannel) -> &str {
            match channel {
                TestChannel::Telegram => &self.telegram,
                TestChannel::Slack => &self.slack,
                TestChannel::Discord => &self.discord,
                TestChannel::WhatsApp => &self.whatsapp,
                TestChannel::Messenger => &self.messenger,
            }
        }
    }

    /// Resolve the repo-root `commands/` directory from `CARGO_MANIFEST_DIR`.
    fn commands_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates directory")
            .parent()
            .expect("repo root")
            .join("commands")
    }

    async fn create_test_user_with_tenant(
        resources: &ServerResources,
        email: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Coverage User".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: "Coverage Tenant".to_owned(),
            slug: format!("coverage-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources.repos.tenants.create(&tenant).await.unwrap();

        (user_id, tenant_id)
    }

    async fn upsert_config_for_channel(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        channel: TestChannel,
        secrets: &ChannelSecrets,
    ) {
        let config_id = Uuid::new_v4().to_string();
        let params = match channel {
            TestChannel::Telegram => UpsertChannelConfigParams {
                id: &config_id,
                tenant_id,
                channel_type: "telegram",
                api_key: None,
                api_secret: None,
                webhook_secret: Some(&secrets.telegram_webhook_secret),
                verify_token: None,
                account_id: None,
                phone_number: None,
                bot_token: Some("12345:COVERAGE_BOT"),
                is_active: true,
            },
            TestChannel::Slack => UpsertChannelConfigParams {
                id: &config_id,
                tenant_id,
                channel_type: "slack",
                api_key: Some("xoxb-coverage-slack"),
                api_secret: None,
                webhook_secret: Some(&secrets.slack_signing_secret),
                verify_token: None,
                account_id: None,
                phone_number: None,
                bot_token: None,
                is_active: true,
            },
            TestChannel::Discord => UpsertChannelConfigParams {
                id: &config_id,
                tenant_id,
                channel_type: "discord",
                api_key: Some("discord-coverage-token"),
                api_secret: None,
                webhook_secret: Some(&secrets.discord_public_key_hex),
                verify_token: None,
                account_id: Some("coverage-app-id"),
                phone_number: None,
                bot_token: Some("discord-bot-token"),
                is_active: true,
            },
            TestChannel::WhatsApp => UpsertChannelConfigParams {
                id: &config_id,
                tenant_id,
                channel_type: "whatsapp",
                api_key: Some("wa-coverage-token"),
                api_secret: None,
                webhook_secret: Some(&secrets.whatsapp_app_secret),
                verify_token: None,
                account_id: Some("coverage-wa-account"),
                phone_number: Some(TEST_PHONE_NUMBER_ID),
                bot_token: None,
                is_active: true,
            },
            TestChannel::Messenger => UpsertChannelConfigParams {
                id: &config_id,
                tenant_id,
                channel_type: "messenger",
                api_key: Some("messenger-coverage-token"),
                api_secret: Some(&secrets.messenger_app_secret),
                webhook_secret: Some(&secrets.messenger_app_secret),
                verify_token: None,
                account_id: Some(TEST_PAGE_ID),
                phone_number: None,
                bot_token: None,
                is_active: true,
            },
        };
        db.upsert_channel_config(&params).await.unwrap();
    }

    async fn create_link_and_session(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        user_id: Uuid,
        channel: TestChannel,
        sender_id: &str,
    ) {
        let link_id = Uuid::new_v4().to_string();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &link_id,
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: channel.as_str(),
            channel_user_id: sender_id,
            display_name: Some("Coverage User"),
        })
        .await
        .unwrap();

        let session_id = Uuid::new_v4().to_string();
        let conversation_id = Uuid::new_v4().to_string();
        db.create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &user_id.to_string(),
            tenant_id,
            channel_type: channel.as_str(),
            channel_user_id: sender_id,
            channel_conversation_id: None,
            pierre_conversation_id: Some(&conversation_id),
        })
        .await
        .unwrap();
    }

    fn build_webhook(
        channel: TestChannel,
        text: &str,
        msg_id: i64,
        senders: &HarnessSenders,
        secrets: &ChannelSecrets,
    ) -> WebhookRequest {
        let sender = senders.for_channel(channel);
        match channel {
            TestChannel::Telegram => telegram_webhook(text, msg_id, sender, secrets),
            TestChannel::Slack => slack_webhook(text, msg_id, sender, secrets),
            TestChannel::Discord => discord_webhook(text, msg_id, sender, secrets),
            TestChannel::WhatsApp => {
                whatsapp_webhook(text, msg_id, sender, TEST_PHONE_NUMBER_ID, secrets)
            }
            TestChannel::Messenger => {
                messenger_webhook(text, msg_id, sender, TEST_PAGE_ID, secrets)
            }
        }
    }

    async fn send_webhook(
        router: &axum::Router,
        request: WebhookRequest,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = AxumTestRequest::post(&request.path);
        for (key, value) in &request.headers {
            req = req.header(key, value);
        }
        let resp = req.json(&request.body_value).send(router.clone()).await;
        let status = resp.status_code();
        let json = resp.json::<serde_json::Value>();
        (status, json)
    }

    async fn assert_command_dispatched(
        router: &axum::Router,
        channel: TestChannel,
        command: &str,
        msg_id: i64,
        senders: &HarnessSenders,
        secrets: &ChannelSecrets,
    ) {
        let request = build_webhook(channel, command, msg_id, senders, secrets);
        let (status, body) = send_webhook(router, request).await;

        assert_eq!(
            status,
            StatusCode::OK,
            "webhook POST for {channel:?} / `{command}` returned non-200: {body}"
        );

        // `messages_stored == 0` proves the command was matched by the dispatcher
        // and handled synchronously via `try_handle_slash_command`. If it falls
        // through to the LLM pipeline, `persist_single_message` stores the
        // message and increments the counter.
        let stored = body
            .get("messages_stored")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(-1);

        assert_eq!(
            stored, 0,
            "command `{command}` on {channel:?} was NOT recognized: messages_stored={stored}, body={body}"
        );
    }

    /// Exhaustive invariant: every command in `commands/*.md` dispatches on every channel.
    ///
    /// This test reads command definitions from the `commands/` directory using
    /// the production parser. Adding a new command markdown file automatically
    /// adds it to the matrix — there is no exception list to maintain. New
    /// channels require exactly one entry in `TestChannel::ALL`.
    #[tokio::test]
    async fn every_command_dispatches_on_every_channel() {
        let commands_dir = commands_dir();
        let definitions = load_command_definitions(&commands_dir);
        assert!(
            !definitions.is_empty(),
            "no command definitions loaded from {commands_dir:?}"
        );

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) =
            create_test_user_with_tenant(&resources, "coverage@test.com").await;

        let senders = HarnessSenders {
            telegram: "99".to_owned(),
            slack: "U_COVERAGE_SLACK".to_owned(),
            discord: "discord_coverage_user".to_owned(),
            whatsapp: "15559876543".to_owned(),
            messenger: "fb_coverage_user".to_owned(),
        };

        let secrets = ChannelSecrets::generate();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        for channel in TestChannel::ALL {
            upsert_config_for_channel(db, tenant_id, channel, &secrets).await;
            create_link_and_session(
                db,
                tenant_id,
                user_id,
                channel,
                senders.for_channel(channel),
            )
            .await;
        }

        let router = MessagingRoutes::routes(Arc::clone(&resources));

        let mut msg_id: i64 = 1;
        for def in &definitions {
            for channel in TestChannel::ALL {
                assert_command_dispatched(
                    &router,
                    channel,
                    &def.command,
                    msg_id,
                    &senders,
                    &secrets,
                )
                .await;
                msg_id += 1;
            }
        }

        let total = definitions.len() * TestChannel::ALL.len();
        println!(
            "dispatched {total} (command, channel) combinations across {} commands",
            definitions.len()
        );
    }

    /// Secondary invariant: every command alias is also matched and dispatched.
    ///
    /// Loops over all defined aliases (not just primary command strings) and
    /// asserts the same `messages_stored == 0` property via Telegram (one
    /// channel is sufficient — alias resolution is channel-independent).
    #[tokio::test]
    async fn every_command_alias_dispatches() {
        let definitions = load_command_definitions(&commands_dir());
        assert!(!definitions.is_empty(), "no command definitions loaded");

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) =
            create_test_user_with_tenant(&resources, "aliases@test.com").await;

        let senders = HarnessSenders {
            telegram: "99".to_owned(),
            slack: String::new(),
            discord: String::new(),
            whatsapp: String::new(),
            messenger: String::new(),
        };
        let secrets = ChannelSecrets::generate();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        upsert_config_for_channel(db, tenant_id, TestChannel::Telegram, &secrets).await;
        create_link_and_session(
            db,
            tenant_id,
            user_id,
            TestChannel::Telegram,
            &senders.telegram,
        )
        .await;

        let router = MessagingRoutes::routes(Arc::clone(&resources));

        let mut msg_id: i64 = 10_000;
        let mut alias_count = 0usize;
        for def in &definitions {
            for alias in &def.aliases {
                assert_command_dispatched(
                    &router,
                    TestChannel::Telegram,
                    alias,
                    msg_id,
                    &senders,
                    &secrets,
                )
                .await;
                msg_id += 1;
                alias_count += 1;
            }
        }

        println!(
            "dispatched {alias_count} aliases across {} commands",
            definitions.len()
        );
    }
}
