// ABOUTME: /discover, /discover install, /group create and /group join over the messaging path — a Telegram DM
// ABOUTME: The webhook proves dispatch, side effects and the persisted turn; the dispatcher proves what the athlete reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod discover_over_messaging {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::coach_fixtures::publish_catalogue_coach;
    use crate::helpers::messaging_webhooks::{telegram_webhook, ChannelSecrets};
    use crate::helpers::notify_capture::{capture_notify, named, only};
    use axum::http::StatusCode;
    use chrono::Utc;
    use pierre_commands::dispatch::{try_dispatch, DispatchOutcome, DispatchRequest};
    use pierre_commands::load_command_catalog;
    use pierre_contremaitre::messaging_strings::{
        KEY_DISCOVER_CARD_TITLE, KEY_DISCOVER_INSTALLED, KEY_DISCOVER_INSTALL_ALREADY,
        KEY_GROUP_CREATED, KEY_GROUP_JOINED, KEY_HELP_DOMAIN_DISCOVER,
    };
    use pierre_core::models::coaches::{CoachHandle, CreateCoachRequest};
    use pierre_core::models::groups::GroupInviteKind;
    use pierre_core::models::{
        ConnectionType, MessageRecord, PersistedReplyBlock, Tenant, TenantId, User, UserStatus,
    };
    use pierre_database::backends::{
        CreateChannelLinkParams, CreateSessionParams, MessagingRepository,
        UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_messaging::commands::CommandResponse;
    use pierre_runtime_context::CommandCtx;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    /// The locale the linked athlete reads replies in.
    const LOCALE: &str = "en";
    /// Telegram refuses a callback payload above this many bytes.
    const TELEGRAM_CALLBACK_LIMIT: usize = 64;

    /// One athlete linked to the Telegram bot: their tenant, their DM session
    /// conversation, the sender id the webhook fixture carries and the
    /// secrets the channel config was written with.
    struct LinkedAthlete {
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: String,
        sender_id: String,
        secrets: ChannelSecrets,
    }

    async fn active_user(email: &str) -> User {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Telegram Athlete".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        LOCALE.clone_into(&mut user.locale);
        user
    }

    /// An active user owning a professional tenant, with a real provider
    /// connection so the onboarding gate lets the dispatcher run.
    async fn seed_user_tenant(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId) {
        let user = active_user(email).await;
        let user_id = user.id;
        let repos = &resources.common.repos;
        repos.users.create(&user).await.unwrap();
        let tenant_id = TenantId::generate();
        repos
            .tenants
            .create(&Tenant {
                id: tenant_id,
                name: format!("Tenant of {email}"),
                slug: format!("tg-discover-{tenant_id}"),
                domain: None,
                plan: "professional".to_owned(),
                owner_user_id: user_id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        repos
            .users
            .update_tenant_id(user_id, tenant_id)
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        (user_id, tenant_id)
    }

    /// Link `email` to the Telegram bot as `sender_id`: channel config, the
    /// channel link and a DM session bound to a real conversation — exactly
    /// what the ingress has on hand when a linked athlete types a command.
    async fn link_telegram(
        resources: &Arc<ServerContext>,
        email: &str,
        sender_id: &str,
    ) -> LinkedAthlete {
        let (user_id, tenant_id) = seed_user_tenant(resources, email).await;
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
            bot_token: Some("12345:DISCOVER_BOT"),
            is_active: true,
        })
        .await
        .unwrap();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: sender_id,
            display_name: Some("Telegram Athlete"),
        })
        .await
        .unwrap();
        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                tenant_id,
                "Telegram DM",
                "test-model",
                None,
                None,
            )
            .await
            .unwrap();
        // A Telegram DM's chat id is the sender's own id; the ingress looks a
        // session up by that chat, so the fixture keys it the same way.
        db.create_session(&CreateSessionParams {
            id: &Uuid::new_v4().to_string(),
            user_id: &user_id.to_string(),
            tenant_id,
            channel_type: "telegram",
            channel_user_id: sender_id,
            channel_conversation_id: Some(sender_id),
            pierre_conversation_id: Some(&conversation.id),
        })
        .await
        .unwrap();
        LinkedAthlete {
            user_id,
            tenant_id,
            conversation_id: conversation.id,
            sender_id: sender_id.to_owned(),
            secrets,
        }
    }

    /// Deliver `text` as a Telegram update and return the webhook's answer.
    /// `messages_stored == 0` is the dispatcher's own proof that the text was
    /// answered as a command instead of being stored for an LLM turn.
    async fn webhook(
        router: &axum::Router,
        athlete: &LinkedAthlete,
        text: &str,
        msg_id: i64,
    ) -> serde_json::Value {
        let request = telegram_webhook(text, msg_id, &athlete.sender_id, &athlete.secrets);
        let mut req = AxumTestRequest::post(&request.path);
        for (key, value) in &request.headers {
            req = req.header(key, value);
        }
        let resp = req.json(&request.body_value).send(router.clone()).await;
        assert_eq!(resp.status_code(), StatusCode::OK, "`{text}` webhook");
        let body: serde_json::Value = resp.json();
        assert_eq!(
            body["messages_stored"], 0,
            "`{text}` must dispatch as a command, got {body}"
        );
        body
    }

    /// Run `text` through the one dispatcher the ingress uses, shaped as a
    /// Telegram DM turn, and return the reply the athlete would read. The
    /// transport is the only thing left out: Telegram's Bot API is not
    /// reachable from a test, so the card is read here rather than off the
    /// wire.
    async fn dispatch(
        resources: &Arc<ServerContext>,
        athlete: &LinkedAthlete,
        text: &str,
    ) -> CommandResponse {
        let ctx: Arc<dyn CommandCtx> = Arc::<ServerContext>::clone(resources);
        let tool_runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
        let outcome = try_dispatch(DispatchRequest {
            ctx: &ctx,
            command_registry: resources.common.command_registry.as_ref().unwrap(),
            command_handler_registry: resources.common.command_handler_registry.as_ref().unwrap(),
            user_id: athlete.user_id,
            tenant_id: athlete.tenant_id,
            channel_type: "telegram",
            locale: LOCALE,
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: Some(&athlete.conversation_id),
            conversation_tenant_id: athlete.tenant_id,
            sender_id: Some(&athlete.sender_id),
            text,
            tool_runtime: &tool_runtime,
        })
        .await
        .unwrap();
        match outcome {
            DispatchOutcome::Executed { response, .. } => response,
            DispatchOutcome::UnknownCommand { body } => panic!("`{text}` is unknown: {body}"),
            DispatchOutcome::NotACommand => panic!("`{text}` is not a command"),
        }
    }

    /// The DM conversation's transcript, oldest first — where the ingress
    /// files every command turn a linked athlete runs from Telegram.
    async fn transcript(
        resources: &Arc<ServerContext>,
        athlete: &LinkedAthlete,
    ) -> Vec<MessageRecord> {
        resources
            .common
            .repos
            .chat
            .get_messages(
                &athlete.conversation_id,
                &athlete.user_id.to_string(),
                athlete.tenant_id,
            )
            .await
            .unwrap()
    }

    /// The one `actions` entry a persisted card carries: its title and the
    /// postback values of its buttons, in order.
    fn persisted_actions(row: &MessageRecord) -> (Option<String>, Vec<String>) {
        let stored = row
            .content_blocks
            .as_deref()
            .expect("a card persists its buttons");
        let blocks: Vec<PersistedReplyBlock> = serde_json::from_str(stored).unwrap();
        let [PersistedReplyBlock::Actions { title, actions }] = blocks.as_slice() else {
            panic!("one actions entry, got {blocks:?}");
        };
        (
            title.clone(),
            actions.iter().map(|a| a.value.clone()).collect(),
        )
    }

    fn rendered(resources: &Arc<ServerContext>, key: &str, args: &[&str]) -> String {
        resources
            .mcp
            .messaging_strings_registry
            .render(key, LOCALE, args)
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

    #[tokio::test]
    async fn telegram_dm_answers_discover_with_the_catalogue_card() {
        let resources = create_test_server_resources().await.unwrap();
        let athlete = link_telegram(&resources, "tg-discover@test.com", "9901").await;
        let (author_id, author_tenant) =
            seed_user_tenant(&resources, "tg-discover-author@test.com").await;
        for title in ["Recovery Coach", "Tempo Coach"] {
            publish_catalogue_coach(
                &resources.common.repos,
                author_id,
                author_tenant,
                title,
                "You coach over Telegram.",
            )
            .await;
        }
        let router = MessagingRoutes::routes(Arc::clone(&resources));

        webhook(&router, &athlete, "/discover", 1).await;

        // The DM keeps the turn like Telegram keeps a bot's answer: the
        // command line and the card, stamped as a command turn, the install
        // buttons persisted so a reload shows them.
        let history = transcript(&resources, &athlete).await;
        assert_eq!(history.len(), 2, "the command line and its card");
        assert!(history.iter().all(MessageRecord::is_command_turn));
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "/discover");
        let card_row = &history[1];
        assert_eq!(card_row.role, "assistant");
        for handle in ["recovery-coach", "tempo-coach"] {
            assert!(
                card_row.content.contains(&format!("@{handle}")),
                "the persisted card names @{handle}: {}",
                card_row.content
            );
        }
        let (title, mut values) = persisted_actions(card_row);
        assert_eq!(
            title.as_deref(),
            Some(rendered(&resources, KEY_DISCOVER_CARD_TITLE, &[]).as_str())
        );
        values.sort();
        assert_eq!(
            values,
            vec![
                "/discover install @recovery-coach".to_owned(),
                "/discover install @tempo-coach".to_owned(),
            ]
        );

        let card = dispatch(&resources, &athlete, "/discover").await;
        assert_eq!(
            card.card_title.as_deref(),
            Some(rendered(&resources, KEY_DISCOVER_CARD_TITLE, &[]).as_str())
        );
        for handle in ["recovery-coach", "tempo-coach"] {
            assert!(
                card.text.contains(&format!("@{handle}")),
                "the card names @{handle}: {}",
                card.text
            );
            assert!(
                card.actions
                    .iter()
                    .any(|a| a.value == format!("/discover install @{handle}")),
                "one install button per coach"
            );
        }
        assert_eq!(card.actions.len(), 2, "two coaches, no More button");
        for action in &card.actions {
            assert_eq!(action.action_type, "postback");
            assert!(action.value.len() <= TELEGRAM_CALLBACK_LIMIT);
        }
    }

    #[tokio::test]
    async fn telegram_dm_installs_by_handle_and_teaches_coach_add() {
        let resources = create_test_server_resources().await.unwrap();
        let athlete = link_telegram(&resources, "tg-install@test.com", "9902").await;
        let (author_id, author_tenant) =
            seed_user_tenant(&resources, "tg-install-author@test.com").await;
        let repos = &resources.common.repos;
        let recovery = publish_catalogue_coach(
            repos,
            author_id,
            author_tenant,
            "Recovery Coach",
            "You recover.",
        )
        .await;
        publish_catalogue_coach(repos, author_id, author_tenant, "Tempo Coach", "You pace.").await;
        let router = MessagingRoutes::routes(Arc::clone(&resources));

        // Over the wire: the webhook dispatches the install and the copy lands.
        webhook(&router, &athlete, "/discover install @recovery-coach", 2).await;
        let copy = repos
            .coaches
            .find_installed_by_handle(
                &CoachHandle::parse("recovery-coach").unwrap(),
                athlete.user_id,
                athlete.tenant_id,
            )
            .await
            .unwrap()
            .expect("installed from the Telegram DM");
        assert_eq!(copy.forked_from, Some(recovery));

        // The DM keeps the hint the athlete read, with its one button.
        let history = transcript(&resources, &athlete).await;
        let hint_row = history
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .expect("the hint is persisted");
        assert!(hint_row.is_command_turn());
        assert_eq!(
            hint_row.content,
            rendered(
                &resources,
                KEY_DISCOVER_INSTALLED,
                &["Recovery Coach", "recovery-coach"]
            )
        );
        let (title, values) = persisted_actions(hint_row);
        assert_eq!(title.as_deref(), Some("Recovery Coach"));
        assert_eq!(values, vec!["/coach add @recovery-coach".to_owned()]);

        // What the athlete reads back, for the second coach.
        let (events, _guard) = capture_notify();
        let hint = dispatch(&resources, &athlete, "/discover install @tempo-coach").await;
        assert_eq!(hint.card_title.as_deref(), Some("Tempo Coach"));
        assert_eq!(
            hint.text,
            rendered(
                &resources,
                KEY_DISCOVER_INSTALLED,
                &["Tempo Coach", "tempo-coach"]
            )
        );
        assert_eq!(hint.actions.len(), 1);
        assert_eq!(hint.actions[0].value, "/coach add @tempo-coach");
        let installed = only(&events, "coach.installed");
        assert_eq!(installed.field("user_id"), athlete.user_id.to_string());

        let again = dispatch(&resources, &athlete, "/discover install @tempo-coach").await;
        assert_eq!(
            again.text,
            rendered(
                &resources,
                KEY_DISCOVER_INSTALL_ALREADY,
                &["Tempo Coach", "tempo-coach"]
            )
        );
        assert_eq!(named(&events, "coach.installed").len(), 1, "counted once");
        let library = repos
            .store_listings
            .get_installed_coaches(athlete.user_id, athlete.tenant_id)
            .await
            .unwrap();
        assert_eq!(library.len(), 2, "one copy of each, none twice");
    }

    #[tokio::test]
    async fn telegram_help_lists_the_new_commands_under_their_domains() {
        let resources = create_test_server_resources().await.unwrap();
        let athlete = link_telegram(&resources, "tg-help@test.com", "9903").await;

        let help = dispatch(&resources, &athlete, "/help").await;
        for command in [
            "/discover",
            "/discover install",
            "/group create",
            "/group join",
        ] {
            assert!(
                help.text.contains(command),
                "/help lists {command}: {}",
                help.text
            );
        }
        assert!(
            help.text
                .contains(&rendered(&resources, KEY_HELP_DOMAIN_DISCOVER, &[])),
            "the catalogue commands sit under their own heading: {}",
            help.text
        );

        // The same catalogue every surface serves, with the argument hints
        // the palettes show.
        let catalog = load_command_catalog(&commands_dir());
        for (name, command, domain, args) in [
            ("discover", "/discover", "discover", "[query|category]"),
            (
                "discover-install",
                "/discover install",
                "discover",
                "@handle",
            ),
            ("group-create", "/group create", "group", "name"),
            ("group-join", "/group join", "group", "invite-code"),
        ] {
            let def = catalog
                .definitions
                .iter()
                .find(|d| d.name == name)
                .unwrap_or_else(|| panic!("{name} is in the catalogue"));
            assert_eq!(def.command, command);
            assert_eq!(def.domain, domain);
            assert_eq!(catalog.arg_specs.get(name).map(String::as_str), Some(args));
        }
    }

    #[tokio::test]
    async fn telegram_dm_creates_and_joins_a_group_by_command() {
        let resources = create_test_server_resources().await.unwrap();
        let owner = link_telegram(&resources, "tg-group-owner@test.com", "9904").await;
        let repos = &resources.common.repos;
        let request: CreateCoachRequest = serde_json::from_value(json!({
            "title": "Ride Coach",
            "description": null,
            "system_prompt": "You coach the ride.",
        }))
        .unwrap();
        let coach = repos
            .coaches
            .create(owner.user_id, owner.tenant_id, &request)
            .await
            .unwrap();
        repos
            .tenants
            .set_selected_coach(owner.tenant_id, owner.user_id, Some(&coach.id.to_string()))
            .await
            .unwrap();

        let created = dispatch(&resources, &owner, "/group create Ride Club").await;
        assert_eq!(
            created.text,
            rendered(&resources, KEY_GROUP_CREATED, &["Ride Club", "Ride Coach"])
        );
        // A DM session conversation is the athlete's one thread on that
        // channel: it is never rebound, the group gets its own conversation.
        let session = repos
            .chat
            .get_conversation(
                &owner.conversation_id,
                &owner.user_id.to_string(),
                owner.tenant_id,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.group_id, None);
        assert_eq!(session.title, "Telegram DM");
        let groups = repos
            .groups
            .list_groups_for_user(owner.user_id)
            .await
            .unwrap();
        assert_eq!(groups.len(), 1);
        let group_id = groups[0].id;
        let listed = repos
            .chat
            .list_conversations(&owner.user_id.to_string(), owner.tenant_id, 50, 0)
            .await
            .unwrap()
            .items;
        assert!(
            listed.iter().any(|c| c.title == "Ride Club"),
            "the creator's group conversation is filed"
        );

        let invite = resources
            .group_service()
            .create_invite(
                group_id,
                owner.user_id,
                owner.tenant_id,
                None,
                None,
                GroupInviteKind::Member,
            )
            .await
            .unwrap();
        let member = link_telegram(&resources, "tg-group-member@test.com", "9905").await;
        let joined = dispatch(&resources, &member, &format!("/group join {}", invite.code)).await;
        assert_eq!(
            joined.text,
            rendered(&resources, KEY_GROUP_JOINED, &["Ride Club"])
        );
        assert!(repos
            .groups
            .get_member(&group_id.to_string(), member.user_id)
            .await
            .unwrap()
            .is_some());
        let member_rows = repos
            .chat
            .list_conversations(&member.user_id.to_string(), member.tenant_id, 50, 0)
            .await
            .unwrap()
            .items;
        let row = member_rows
            .iter()
            .find(|c| c.title == "Ride Club")
            .expect("the group appears in the member's list");
        let record = repos
            .chat
            .get_conversation(&row.id, &member.user_id.to_string(), member.tenant_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.group_id.as_deref(),
            Some(group_id.to_string().as_str())
        );
    }
}
