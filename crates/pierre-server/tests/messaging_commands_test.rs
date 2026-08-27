// ABOUTME: E2E tests for messaging slash commands
// ABOUTME: Tests /help, /status, /logout, /group commands via Telegram webhook flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(
    missing_docs,
    clippy::wildcard_in_or_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args,
    clippy::redundant_closure_for_method_calls
)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod command_tests {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use chrono::Utc;
    use pierre_core::models::coaches::Coach;
    use pierre_core::models::ConnectionType;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::{
        CreateChannelLinkParams, CreateSessionParams, InsertMessageParams, MessagingRepository,
        UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    // ════════════════════════════════════════════════════════════════
    // Helpers
    // ════════════════════════════════════════════════════════════════

    const TG_SECRET: &str = "cmd_test_webhook_secret";
    const BOT_TOKEN: &str = "12345:CMD_TEST_BOT";
    const SENDER_ID: &str = "99";

    async fn create_test_user(resources: &ServerContext, email: &str) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Cmd Test User".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());

        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: "Cmd Test Tenant".to_owned(),
            slug: format!("cmd-test-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources
            .common
            .repos
            .tenants
            .create(&tenant)
            .await
            .unwrap();

        // Onboarding gate: register a synthetic provider so messaging-ingress
        // (the provider gate, removed in Phase 5) lets the
        // turn through. Test exercises command dispatch, not provider data.
        resources
            .common
            .repos
            .provider_connections
            .register_connection(
                user_id,
                tenant_id,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();

        (user_id, tenant_id)
    }

    async fn setup_linked_user(resources: &ServerContext) -> (axum::Router, Uuid, TenantId) {
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let (user_id, tenant_id) = create_test_user(resources, "cmduser@test.com").await;

        // Configure Telegram channel
        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(TG_SECRET),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some(BOT_TOKEN),
            is_active: true,
        })
        .await
        .unwrap();

        // Create channel link (user linked to Telegram)
        let link_id = Uuid::new_v4().to_string();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &link_id,
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: SENDER_ID,
            display_name: Some("Cmd Test User"),
        })
        .await
        .unwrap();

        // Create a real chat_conversation so pierre_conversation_id satisfies its
        // FK. Before the messaging_* FK migration, tests passed a random UUID
        // that never existed; now the column references chat_conversations(id).
        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                tenant_id,
                "Cmd Test Conversation",
                "test-model",
                None,
                None,
            )
            .await
            .unwrap();
        let conversation_id = conversation.id;

        // Create messaging session linked to the conversation
        let session_id = Uuid::new_v4().to_string();
        db.create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &user_id.to_string(),
            tenant_id,
            channel_type: "telegram",
            channel_user_id: SENDER_ID,
            channel_conversation_id: None,
            pierre_conversation_id: Some(&conversation_id),
        })
        .await
        .unwrap();

        let router = MessagingRoutes::routes(Arc::new(resources.clone()));
        (router, user_id, tenant_id)
    }

    fn tg_webhook(text: &str, msg_id: i64) -> serde_json::Value {
        json!({
            "update_id": 2000 + msg_id,
            "message": {
                "message_id": msg_id,
                "from": { "id": 99, "first_name": "CmdTest" },
                "chat": { "id": 99 },
                "text": text
            }
        })
    }

    async fn send_command(
        router: &axum::Router,
        text: &str,
        msg_id: i64,
    ) -> (StatusCode, serde_json::Value) {
        let body = tg_webhook(text, msg_id);
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", TG_SECRET)
            .json(&body)
            .send(router.clone())
            .await;
        let status = resp.status_code();
        let json = resp.json::<serde_json::Value>();
        (status, json)
    }

    // ════════════════════════════════════════════════════════════════
    // Tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_help_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, body) = send_command(&router, "/help", 1).await;
        assert_eq!(status, StatusCode::OK);
        // The webhook returns 200 OK — the command response is sent asynchronously
        assert!(body["status"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_help_alias() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/?", 2).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_status_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/status", 3).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_logout_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, _user_id, tenant_id) = setup_linked_user(&resources).await;
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        // Linked before /logout.
        assert!(
            db.get_channel_link(tenant_id, "telegram", SENDER_ID)
                .await
                .unwrap()
                .is_some(),
            "sender must be linked after setup_linked_user"
        );

        let (status, _) = send_command(&router, "/logout", 4).await;
        assert_eq!(status, StatusCode::OK);

        // /logout must actually unlink the channel — not just show a prompt.
        assert!(
            db.get_channel_link(tenant_id, "telegram", SENDER_ID)
                .await
                .unwrap()
                .is_none(),
            "/logout must delete the channel link"
        );
    }

    /// Logout retains `messaging_sessions` and `messaging_messages` for
    /// support and audit, deleting only the channel link. Regression test
    /// for the production FK violation when a sender with stored messages
    /// typed `/logout` (`messaging_messages_session_id_fkey` blocked the
    /// session DELETE on Postgres).
    #[tokio::test]
    async fn test_logout_retains_history() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, _user_id, tenant_id) = setup_linked_user(&resources).await;
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let session = db
            .get_session_by_channel_identity(tenant_id, "telegram", SENDER_ID, None)
            .await
            .unwrap()
            .expect("session must exist after setup_linked_user");
        let session_id = session["id"].as_str().unwrap().to_owned();

        // Persist an inbound message so logout has child rows the FK protects.
        db.insert_message(&InsertMessageParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            session_id: &session_id,
            direction: "inbound",
            channel_type: "telegram",
            channel_message_id: "tg-logout-history-1",
            sender_id: SENDER_ID,
            content_type: "text",
            content_body: Some("hello"),
            correlation_id: "test-logout-history",
            raw_payload: None,
            chat_message_id: None,
        })
        .await
        .unwrap();

        // `/logout` unlinks the channel directly (the same DELETE the bare-word
        // "logout" path performs); this exercises the FK-protected unlink with
        // stored history present.
        let (status, _) = send_command(&router, "/logout", 100).await;
        assert_eq!(status, StatusCode::OK);

        assert!(
            db.get_channel_link(tenant_id, "telegram", SENDER_ID)
                .await
                .unwrap()
                .is_none(),
            "channel link must be deleted on logout"
        );

        assert!(
            db.get_session_by_channel_identity(tenant_id, "telegram", SENDER_ID, None)
                .await
                .unwrap()
                .is_some(),
            "session must be retained for support/audit"
        );

        let messages = db
            .get_session_messages(&session_id, tenant_id, 100, 0)
            .await
            .unwrap();
        assert!(
            !messages.is_empty(),
            "messages must be retained for support/audit"
        );
    }

    #[tokio::test]
    async fn test_group_list_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/group", 5).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_group_alias() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/groups", 6).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_group_status_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        // This will return OK even though user has no groups
        // (the handler returns a "not a member" error which is caught and sent as text)
        let (status, _) = send_command(&router, "/group status", 7).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_group_members_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/group members", 8).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_group_invite_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/group invite", 9).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_group_leave_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/group leave", 10).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_group_status_alias() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/gs", 11).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_unknown_command_short_circuits() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        // Per commit 4602505, a `/slash` prefix with no matching handler
        // is replied to inline with KEY_UNKNOWN_COMMAND so typos like
        // `/.coach` don't eat LLM quota or spin up a "thinking…"
        // placeholder. The webhook still returns OK; the message is
        // handled synchronously (not stored for dispatch).
        let (status, body) = send_command(&router, "/unknown_command", 12).await;
        assert_eq!(status, StatusCode::OK);
        let stored = body["messages_stored"].as_i64().unwrap_or(0);
        assert_eq!(
            stored, 0,
            "Unknown /command must short-circuit, not go through LLM dispatch"
        );
    }

    #[tokio::test]
    async fn test_non_command_passes_through() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        // Regular message (no /) should pass through to LLM
        let (status, body) = send_command(&router, "How is my training going?", 13).await;
        assert_eq!(status, StatusCode::OK);
        let stored = body["messages_stored"].as_i64().unwrap_or(0);
        assert!(stored > 0, "Regular message should be stored for LLM");
    }

    #[tokio::test]
    async fn test_command_case_insensitive() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/HELP", 14).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_command_with_extra_spaces() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/group   status", 15).await;
        // May or may not match depending on matcher — at minimum should not crash
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_privacy_status_command() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/privacy", 20).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_privacy_status_alias() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        // Verify the 2-word alias `/privacy status` routes to PrivacyStatusHandler
        let (status, _) = send_command(&router, "/privacy status", 21).await;
        assert_eq!(status, StatusCode::OK);
    }

    // ────────────────────────────────────────────────────────────────
    // Direct handler unit tests
    //
    // The webhook-based tests above only verify HTTP 200 OK. For
    // `/privacy on` and `/privacy off`, we also need to verify the
    // database is actually updated. We call the handler's execute()
    // directly with a constructed PlatformCommandContext, bypassing
    // the command matcher and webhook layer.
    // ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_privacy_on_handler_enables_consent() {
        use pierre_commands::privacy::PrivacyOnHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;

        // Confirm initial state: analytics_consent is false
        let before = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert!(!before.analytics_consent);

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![],
            raw_text: "/privacy on".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        let response = PrivacyOnHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("enabled"));

        // Verify the database reflects the enabled state
        let after = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert!(after.analytics_consent);
        assert!(after.analytics_consent_at.is_some());
    }

    #[tokio::test]
    async fn test_privacy_off_handler_disables_consent() {
        use pierre_commands::privacy::PrivacyOffHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;

        // Seed with consent enabled so we can verify the toggle
        resources
            .common
            .repos
            .users
            .update_analytics_consent(user_id, true)
            .await
            .unwrap();
        let before = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert!(before.analytics_consent);

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![],
            raw_text: "/privacy off".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        let response = PrivacyOffHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("disabled"));

        // Verify the database reflects the disabled state
        let after = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert!(!after.analytics_consent);
    }

    // ════════════════════════════════════════════════════════════════
    // /coach DM coverage
    //
    // These tests pin the personal-thread branch of `/coach add`:
    //   - DM: /coach add <id> writes tenant_users.selected_coach_id, never
    //     touches a group, and the confirmation copy omits "group"/"groupe".
    //   - /coach rendering: card body is plain text with no literal
    //     asterisks, even when the coach description contains CommonMark
    //     emphasis (`*not*`).
    // ════════════════════════════════════════════════════════════════

    async fn seed_coach(
        resources: &ServerContext,
        user_id: Uuid,
        tenant_id: TenantId,
        title: &str,
        description: &str,
    ) -> String {
        use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};

        let request = CreateCoachRequest {
            title: title.to_owned(),
            description: Some(description.to_owned()),
            system_prompt: "You are a test coach.".to_owned(),
            category: CoachCategory::Training,
            tags: vec![],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        };
        let coach = resources
            .common
            .repos
            .coaches
            .create(user_id, tenant_id, &request)
            .await
            .unwrap();
        coach.id.to_string()
    }

    #[tokio::test]
    async fn coach_add_in_dm_sets_users_default_coach() {
        use pierre_commands::coach::CoachAddHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;
        let coach_id = seed_coach(
            &resources,
            user_id,
            tenant_id,
            "Activity Analysis Coach",
            "Analyzes training data.",
        )
        .await;

        // Pre-condition: user has no default coach
        let before = resources
            .common
            .repos
            .tenants
            .get_selected_coach(tenant_id, user_id)
            .await
            .unwrap();
        assert!(before.is_none(), "nothing selected before the command");

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![coach_id.clone()],
            raw_text: format!("/coach add {coach_id}"),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "fr".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        let response = CoachAddHandler.execute(&ctx).await.unwrap();

        // Confirmation mentions coach title but NEVER "groupe"/"group".
        assert!(
            response.text.contains("Activity Analysis Coach"),
            "expected coach title in response, got: {}",
            response.text
        );
        assert!(
            !response.text.to_lowercase().contains("groupe"),
            "DM response must not mention 'groupe': {}",
            response.text
        );
        assert!(
            !response.text.to_lowercase().contains("group"),
            "DM response must not mention 'group': {}",
            response.text
        );

        // Post-condition: the selection lands on the membership row — the one
        // pointer every surface now reads, replacing users.default_coach_id.
        let selected = resources
            .common
            .repos
            .tenants
            .get_selected_coach(tenant_id, user_id)
            .await
            .unwrap();
        assert_eq!(selected.as_deref(), Some(coach_id.as_str()));
    }

    #[tokio::test]
    async fn coach_add_in_dm_twice_swaps_the_selected_coach() {
        use pierre_commands::coach::CoachAddHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;
        let coach_a = seed_coach(&resources, user_id, tenant_id, "Coach A", "First.").await;
        let coach_b = seed_coach(&resources, user_id, tenant_id, "Coach B", "Second.").await;

        let mk_ctx = |coach: &str| PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![coach.to_owned()],
            raw_text: format!("/coach add {coach}"),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        CoachAddHandler.execute(&mk_ctx(&coach_a)).await.unwrap();
        // Reselecting must SWAP, not accumulate — the property the old
        // clear-all-then-set pair maintained non-atomically and a single pointer
        // gets structurally.
        let after_a = resources
            .common
            .repos
            .tenants
            .get_selected_coach(tenant_id, user_id)
            .await
            .unwrap();
        assert_eq!(after_a.as_deref(), Some(coach_a.as_str()));

        CoachAddHandler.execute(&mk_ctx(&coach_b)).await.unwrap();
        let after_b = resources
            .common
            .repos
            .tenants
            .get_selected_coach(tenant_id, user_id)
            .await
            .unwrap();
        assert_eq!(after_b.as_deref(), Some(coach_b.as_str()));
    }

    #[tokio::test]
    async fn coach_list_renders_without_markdown_asterisks() {
        use pierre_commands::coach::CoachListHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;
        // Seed with a description that mimics the real coach markdown that
        // leaked literal asterisks to Telegram.
        let _coach = seed_coach(
            &resources,
            user_id,
            tenant_id,
            "Pre-Workout Mobility Coach",
            "Expert in warm-ups — clear guidance on what *not* to do before a workout.",
        )
        .await;

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![],
            raw_text: "/coach".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        let response = CoachListHandler.execute(&ctx).await.unwrap();

        // Card body must not carry literal markdown emphasis — assert on the
        // merged text (title + body) because both flow to every channel.
        let rendered = response.text;
        assert!(
            !rendered.contains('*'),
            "coach list card rendered literal asterisk — markdown strip regressed: {rendered}"
        );
        assert!(
            rendered.contains("what not to do"),
            "expected emphasis-stripped description to survive, got: {rendered}"
        );
    }

    /// Regression: `/group consent yes` must flip the consent flag on
    /// the chat-bound group when the conversation has a `group_id`,
    /// not on `list_groups_for_user.first()` (which is non-deterministic
    /// for a user in multiple groups). This covers the production bug
    /// where Phil's Telegram-chat consent could land on the wrong
    /// `coaching_groups` row, leaving his peer data hidden in the
    /// chat-bound group.
    #[tokio::test]
    async fn test_group_consent_uses_conversation_group_id() {
        use chrono::Utc;
        use pierre_commands::group::GroupConsentHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};
        use pierre_core::models::coaches::{
            CoachCategory, CoachVisibility, CreateSystemCoachRequest,
        };
        use pierre_core::models::groups::{
            CoachingGroup, GroupMember, GroupRespondMode, GroupRole,
        };
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;
        let user_id_str = user_id.to_string();

        // Coach (required as FK for both groups). Seed a minimal system
        // coach directly via the repo — `create_test_server_resources`
        // does not auto-seed coaches.
        let coach = resources
            .common
            .repos
            .coaches
            .create_system_coach(
                user_id,
                tenant_id,
                &CreateSystemCoachRequest {
                    title: "Test Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap();
        let coach_id = coach.id;

        // Two groups, both with this user as Owner+Member. Pre-fix,
        // `groups.first()` (ORDER BY updated_at DESC) flipped consent
        // on whichever was most recently touched; here we make the
        // OTHER group the most-recent so the chat-bound group would
        // miss the update under the buggy code path.
        let chat_group_id = Uuid::new_v4();
        let other_group_id = Uuid::new_v4();
        let now = Utc::now();
        let mk_group = |id: Uuid, name: &str, updated_at| CoachingGroup {
            id,
            tenant_id: tenant_id.to_string(),
            name: name.to_owned(),
            description: None,
            coach_id: coach_id.to_string(),
            owner_id: user_id,
            coach_user_id: None,
            peer_data_sharing: true,
            respond_mode: GroupRespondMode::default(),
            max_members: 10,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at,
        };
        let chat_group = mk_group(chat_group_id, "Chat-bound group", now);
        // Other group has a *later* updated_at — it would win under
        // `list_groups_for_user.first()`.
        let other_group = mk_group(
            other_group_id,
            "Other group",
            now + chrono::Duration::seconds(60),
        );
        resources
            .common
            .repos
            .groups
            .create_group(tenant_id, &chat_group)
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .create_group(tenant_id, &other_group)
            .await
            .unwrap();

        let mk_member = |group_id: Uuid| GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        };
        resources
            .common
            .repos
            .groups
            .add_member(&mk_member(chat_group_id))
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .add_member(&mk_member(other_group_id))
            .await
            .unwrap();

        // Conversation explicitly bound to the chat group.
        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(
                &user_id_str,
                tenant_id,
                "Group chat",
                "gpt-4",
                None,
                Some(&chat_group_id.to_string()),
            )
            .await
            .unwrap();

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec!["yes".to_owned()],
            raw_text: "/group consent yes".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            ambient_group_fallback: true,
            conversation_id: Some(conversation.id.clone()),
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        let response = GroupConsentHandler.execute(&ctx).await.unwrap();
        assert!(
            response.text.contains("Chat-bound group"),
            "consent confirmation should name the chat-bound group, got: {}",
            response.text
        );

        // Verify the chat-bound group has consent=true while the other
        // group remains untouched. This is the behavioral assertion
        // that fails under the pre-fix `groups.first()` path because
        // `other_group` (more recently updated) would be chosen.
        let chat_members = resources
            .common
            .repos
            .groups
            .list_members(&chat_group_id.to_string())
            .await
            .unwrap();
        let chat_member_consent = chat_members
            .iter()
            .find(|m| m.user_id == user_id)
            .expect("chat-group member row")
            .peer_sharing_consent;
        assert!(
            chat_member_consent,
            "consent must flip on the conversation-bound group"
        );

        let other_members = resources
            .common
            .repos
            .groups
            .list_members(&other_group_id.to_string())
            .await
            .unwrap();
        let other_member_consent = other_members
            .iter()
            .find(|m| m.user_id == user_id)
            .expect("other-group member row")
            .peer_sharing_consent;
        assert!(
            !other_member_consent,
            "consent must NOT leak to a different group the user belongs to"
        );
    }

    /// Shared-bot group deployment: the member belongs to tenant A while the
    /// Telegram bot (and therefore the conversation + the coaching group) lives
    /// under the channel tenant `T_bot`. `/group consent yes` must still land on
    /// the chat-bound group even though the two tenants differ — the case a
    /// single-tenant fixture cannot reach.
    #[tokio::test]
    async fn group_consent_binds_to_chat_group_across_tenants() {
        use pierre_commands::group::GroupConsentHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let fixture = cross_tenant_group_fixture(&resources).await;

        let ctx = PlatformCommandContext {
            user_id: fixture.user,
            // The caller's OWN tenant — what AuthResult carries for a member of
            // a shared bot's group.
            tenant_id: fixture.member_tenant,
            // The conversation row was written under the bot tenant.
            conversation_tenant_id: fixture.channel_tenant,
            channel_type: "telegram".to_owned(),
            args: vec!["yes".to_owned()],
            raw_text: "/group consent yes".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            ambient_group_fallback: true,
            conversation_id: Some(fixture.conversation.clone()),
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        let response = GroupConsentHandler.execute(&ctx).await.unwrap();
        assert!(
            response.text.contains("Bot-tenant chat group"),
            "consent confirmation should name the chat-bound group, got: {}",
            response.text
        );

        assert!(
            member_consent(&resources, fixture.chat_group, fixture.user).await,
            "consent must flip on the group bound to the chat, even though the \
             member's tenant differs from the channel tenant"
        );
        assert!(
            !member_consent(&resources, fixture.other_group, fixture.user).await,
            "consent must NOT land on the member's own more-recently-updated group"
        );
    }

    /// Fail-closed: a conversation id that does not resolve under the
    /// conversation tenant must refuse rather than silently retarget the
    /// consent write at `list_groups_for_user().first()`.
    #[tokio::test]
    async fn group_consent_refuses_unresolvable_conversation() {
        use pierre_commands::group::GroupConsentHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let fixture = cross_tenant_group_fixture(&resources).await;

        let ctx = PlatformCommandContext {
            user_id: fixture.user,
            tenant_id: fixture.member_tenant,
            conversation_tenant_id: fixture.channel_tenant,
            channel_type: "telegram".to_owned(),
            args: vec!["yes".to_owned()],
            raw_text: "/group consent yes".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            ambient_group_fallback: true,
            // No such conversation anywhere.
            conversation_id: Some(Uuid::new_v4().to_string()),
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        let err = GroupConsentHandler
            .execute(&ctx)
            .await
            .expect_err("unresolvable conversation must refuse");
        assert!(
            err.to_string().contains("not a member"),
            "refusal should reuse the not-a-member string, got: {err}"
        );

        assert!(
            !member_consent(&resources, fixture.chat_group, fixture.user).await,
            "a refused command must not write consent anywhere"
        );
        assert!(
            !member_consent(&resources, fixture.other_group, fixture.user).await,
            "a refused command must not retarget consent at another group"
        );
    }

    struct CrossTenantGroupFixture {
        user: Uuid,
        member_tenant: TenantId,
        channel_tenant: TenantId,
        chat_group: Uuid,
        other_group: Uuid,
        conversation: String,
    }

    /// Seed the shared-bot topology: one member in tenant A, a bot tenant that
    /// owns the group chat's conversation and coaching group, and a second
    /// group in A that is more recently updated (so `list_groups_for_user`
    /// ordered by `updated_at DESC` would pick it).
    async fn cross_tenant_group_fixture(resources: &ServerContext) -> CrossTenantGroupFixture {
        use pierre_core::models::coaches::{
            CoachCategory, CoachVisibility, CreateSystemCoachRequest,
        };
        use pierre_core::models::groups::{
            CoachingGroup, GroupMember, GroupRespondMode, GroupRole,
        };

        let (user_id, member_tenant_id) =
            create_test_user(resources, "crosstenant-member@test.com").await;
        let channel_tenant_id = TenantId::generate();
        let bot_tenant = Tenant {
            id: channel_tenant_id,
            name: "Bot Tenant".to_owned(),
            slug: format!("bot-tenant-{channel_tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources
            .common
            .repos
            .tenants
            .create(&bot_tenant)
            .await
            .unwrap();

        let coach = resources
            .common
            .repos
            .coaches
            .create_system_coach(
                user_id,
                member_tenant_id,
                &CreateSystemCoachRequest {
                    title: "Cross Tenant Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap();

        let now = Utc::now();
        let chat_group_id = Uuid::new_v4();
        let other_group_id = Uuid::new_v4();
        let mk_group = |id: Uuid, name: &str, owner_tenant: TenantId, updated_at| CoachingGroup {
            id,
            tenant_id: owner_tenant.to_string(),
            name: name.to_owned(),
            description: None,
            coach_id: coach.id.to_string(),
            owner_id: user_id,
            coach_user_id: None,
            peer_data_sharing: true,
            respond_mode: GroupRespondMode::default(),
            max_members: 10,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at,
        };
        let chat_group = mk_group(
            chat_group_id,
            "Bot-tenant chat group",
            channel_tenant_id,
            now,
        );
        // Later `updated_at` — this is what the cross-tenant, unfiltered
        // `list_groups_for_user` fallback would select.
        let other_group = mk_group(
            other_group_id,
            "Member's own group",
            member_tenant_id,
            now + chrono::Duration::seconds(60),
        );
        resources
            .common
            .repos
            .groups
            .create_group(channel_tenant_id, &chat_group)
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .create_group(member_tenant_id, &other_group)
            .await
            .unwrap();

        let mk_member = |group_id: Uuid, member_tenant: TenantId| GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: member_tenant.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        };
        resources
            .common
            .repos
            .groups
            .add_member(&mk_member(chat_group_id, member_tenant_id))
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .add_member(&mk_member(other_group_id, member_tenant_id))
            .await
            .unwrap();

        // The group chat's conversation lives under the BOT tenant — this is
        // what `resolve_linked_session` does for a non-DM.
        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                channel_tenant_id,
                "Telegram group",
                "gpt-4",
                None,
                Some(&chat_group_id.to_string()),
            )
            .await
            .unwrap();

        CrossTenantGroupFixture {
            user: user_id,
            member_tenant: member_tenant_id,
            channel_tenant: channel_tenant_id,
            chat_group: chat_group_id,
            other_group: other_group_id,
            conversation: conversation.id,
        }
    }

    async fn member_consent(resources: &ServerContext, group_id: Uuid, user_id: Uuid) -> bool {
        resources
            .common
            .repos
            .groups
            .list_members(&group_id.to_string())
            .await
            .unwrap()
            .iter()
            .find(|m| m.user_id == user_id)
            .expect("member row")
            .peer_sharing_consent
    }

    #[tokio::test]
    async fn group_coach_command_sets_group_ai_coach() {
        use pierre_commands::group::GroupCoachHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};
        use pierre_core::models::coaches::{
            CoachCategory, CoachVisibility, CreateSystemCoachRequest,
        };
        use pierre_core::models::groups::{
            CoachingGroup, GroupMember, GroupRespondMode, GroupRole,
        };
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;
        let now = chrono::Utc::now();

        let mk_system = |title: &str| CreateSystemCoachRequest {
            title: title.to_owned(),
            description: None,
            system_prompt: "Test prompt".to_owned(),
            category: CoachCategory::Training,
            tags: vec![],
            sample_prompts: vec![],
            visibility: CoachVisibility::Global,
        };
        let initial = resources
            .common
            .repos
            .coaches
            .create_system_coach(user_id, tenant_id, &mk_system("Starter Coach"))
            .await
            .unwrap();
        let target = resources
            .common
            .repos
            .coaches
            .create_system_coach(user_id, tenant_id, &mk_system("5K Marathon"))
            .await
            .unwrap();

        let group_id = Uuid::new_v4();
        let group = CoachingGroup {
            id: group_id,
            tenant_id: tenant_id.to_string(),
            name: "Run Club".to_owned(),
            description: None,
            coach_id: initial.id.to_string(),
            owner_id: user_id,
            coach_user_id: None,
            peer_data_sharing: true,
            respond_mode: GroupRespondMode::default(),
            max_members: 10,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .groups
            .create_group(tenant_id, &group)
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: tenant_id.to_string(),
                role: GroupRole::Owner,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();

        // `/group coach 5k marathon` — args arrive split; match is case-insensitive.
        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec!["5k".to_owned(), "marathon".to_owned()],
            raw_text: "/group coach 5k marathon".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };
        let response = GroupCoachHandler.execute(&ctx).await.unwrap();
        assert!(
            response.text.contains("5K Marathon") && response.text.contains("Run Club"),
            "confirmation should name the group and the new coach, got: {}",
            response.text
        );

        let updated = resources
            .common
            .repos
            .groups
            .get_group(&group_id.to_string(), tenant_id)
            .await
            .unwrap()
            .expect("group exists");
        assert_eq!(
            updated.coach_id,
            target.id.to_string(),
            "group coach_id should now point at the 5K Marathon coach"
        );

        // Unknown name leaves the coach unchanged.
        let ctx_miss = PlatformCommandContext {
            args: vec!["Nonexistent".to_owned()],
            raw_text: "/group coach Nonexistent".to_owned(),
            ..ctx
        };
        let miss = GroupCoachHandler.execute(&ctx_miss).await.unwrap();
        assert!(
            miss.text.to_lowercase().contains("no coach"),
            "unknown coach name should report not found, got: {}",
            miss.text
        );
        let after = resources
            .common
            .repos
            .groups
            .get_group(&group_id.to_string(), tenant_id)
            .await
            .unwrap()
            .expect("group exists");
        assert_eq!(
            after.coach_id,
            target.id.to_string(),
            "coach_id must be unchanged after an unmatched name"
        );
    }

    /// carnet#70: `/group invite coach` could attach a human coach and nothing
    /// could detach one. `GroupService::set_group_coach` existed with `None`
    /// documented as "detach" and zero production callers — the capability was
    /// built and never reachable.
    #[tokio::test]
    async fn group_coach_detach_clears_the_human_coach() {
        use pierre_commands::group::GroupCoachHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};
        use pierre_core::models::coaches::{
            CoachCategory, CoachVisibility, CreateSystemCoachRequest,
        };
        use pierre_core::models::groups::{
            CoachingGroup, GroupMember, GroupRespondMode, GroupRole,
        };
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;
        let now = chrono::Utc::now();

        let persona = resources
            .common
            .repos
            .coaches
            .create_system_coach(
                user_id,
                tenant_id,
                &CreateSystemCoachRequest {
                    title: "Starter Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap();

        // A group that already has a human coach attached — the state
        // `/group invite coach` leaves behind.
        let human_coach = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        resources
            .common
            .repos
            .groups
            .create_group(
                tenant_id,
                &CoachingGroup {
                    id: group_id,
                    tenant_id: tenant_id.to_string(),
                    name: "Run Club".to_owned(),
                    description: None,
                    coach_id: persona.id.to_string(),
                    owner_id: user_id,
                    coach_user_id: Some(human_coach),
                    peer_data_sharing: true,
                    respond_mode: GroupRespondMode::default(),
                    max_members: 10,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: tenant_id.to_string(),
                role: GroupRole::Owner,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec!["detach".to_owned()],
            raw_text: "/group coach detach".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };
        let response = GroupCoachHandler.execute(&ctx).await.unwrap();
        assert_eq!(
            response.text, "Run Club no longer has an attached human coach.",
            "the en confirmation names the group"
        );

        let updated = resources
            .common
            .repos
            .groups
            .get_group(&group_id.to_string(), tenant_id)
            .await
            .unwrap()
            .expect("group exists");
        assert_eq!(
            updated.coach_user_id, None,
            "detach must clear coach_user_id"
        );
        assert_eq!(
            updated.coach_id,
            persona.id.to_string(),
            "detach touches only the human coach; the AI persona is unchanged"
        );
    }

    #[tokio::test]
    async fn test_privacy_status_handler_reads_current_state() {
        use pierre_commands::privacy::PrivacyStatusHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![],
            raw_text: "/privacy".to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        // Default state: consent disabled
        let response = PrivacyStatusHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("disabled"));

        // Flip to enabled and re-run
        resources
            .common
            .repos
            .users
            .update_analytics_consent(user_id, true)
            .await
            .unwrap();
        let response = PrivacyStatusHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("enabled"));
    }

    #[tokio::test]
    async fn test_timezone_handler_persists_valid_iana() {
        use pierre_commands::timezone::TimezoneHandler;
        use pierre_commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;

        // Messaging surfaces never call /api/users/me/timezone, so a fresh
        // Telegram-linked user starts with no timezone on file.
        let before = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            before.timezone.is_none(),
            "fresh messaging user should have no timezone"
        );

        let make_ctx = |args: Vec<String>, raw: &str| PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args,
            raw_text: raw.to_owned(),
            ctx: Arc::<ServerContext>::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(&resources),
        };

        // Valid IANA name persists and is echoed in the confirmation.
        let ctx = make_ctx(
            vec!["America/Toronto".to_owned()],
            "/timezone America/Toronto",
        );
        let response = TimezoneHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("America/Toronto"));
        let stored = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap()
            .timezone;
        assert_eq!(stored.as_deref(), Some("America/Toronto"));

        // Junk argument is rejected without overwriting the stored value.
        let ctx = make_ctx(vec!["Mars/Phobos".to_owned()], "/timezone Mars/Phobos");
        let response = TimezoneHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("Invalid timezone"));
        let unchanged = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap()
            .timezone;
        assert_eq!(unchanged.as_deref(), Some("America/Toronto"));

        // Missing argument is also rejected.
        let ctx = make_ctx(vec![], "/timezone");
        let response = TimezoneHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("Invalid timezone"));
    }

    // ════════════════════════════════════════════════════════════════
    // /group command handlers — populated paths (direct execute())
    // ════════════════════════════════════════════════════════════════
    //
    // The Telegram webhook returns 200 OK and dispatches the command reply
    // asynchronously, so the rendered text never appears in the webhook
    // response — the `test_group_*_command` webhook tests above can only
    // assert the status code, and they run against a user with no groups
    // (the empty / "not a member" branch). To exercise the populated paths
    // and assert on the actual reply body, we invoke each handler directly,
    // matching the `StatusHandler` pattern in `messaging_locale_test`.

    use pierre_commands::group::{
        GroupConsentHandler, GroupInviteHandler, GroupListHandler, GroupMembersHandler,
        GroupStatusHandler,
    };
    use pierre_commands::{CommandHandler, PlatformCommandContext};
    use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
    use pierre_messaging::commands::CommandRegistry;

    /// Create a bare active user (no tenant/provider) for use as an additional
    /// group member. `coaching_group_members.user_id` is a FK to `users(id)`,
    /// so peer members must reference real user rows. `email` is the value
    /// `list_members` surfaces as the member's display name (it joins
    /// `users.email`, not the membership row's stored `display_name`).
    async fn seed_member_user(resources: &ServerContext, email: &str) -> Uuid {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Group Member".to_owned()),
        );
        user.user_status = UserStatus::Active;
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();
        user_id
    }

    /// Persist a coaching-group row directly. The repo `create_group` does
    /// not auto-add the owner as a member (that is `GroupService::create_group`
    /// behavior); membership is always seeded explicitly via `add_group_member`.
    async fn create_group_row(
        resources: &ServerContext,
        tenant_id: TenantId,
        coach_id: &str,
        owner_id: Uuid,
        name: &str,
        peer_data_sharing: bool,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let group = CoachingGroup {
            id,
            tenant_id: tenant_id.to_string(),
            name: name.to_owned(),
            description: None,
            coach_id: coach_id.to_owned(),
            owner_id,
            coach_user_id: None,
            peer_data_sharing,
            respond_mode: GroupRespondMode::default(),
            max_members: 10,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .groups
            .create_group(tenant_id, &group)
            .await
            .unwrap();
        id
    }

    async fn add_group_member(
        resources: &ServerContext,
        group_id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
        role: GroupRole,
        display_name: Option<&str>,
        consent: bool,
    ) {
        let now = Utc::now();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: tenant_id.to_string(),
                role,
                peer_sharing_consent: consent,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: display_name.map(str::to_owned),
            })
            .await
            .unwrap();
    }

    fn group_ctx(
        resources: &Arc<ServerContext>,
        user_id: Uuid,
        tenant_id: TenantId,
        args: Vec<String>,
        raw: &str,
        conversation_id: Option<String>,
    ) -> PlatformCommandContext {
        PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args,
            raw_text: raw.to_owned(),
            ctx: Arc::<ServerContext>::clone(resources),
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            tool_runtime: Arc::<ServerContext>::clone(resources),
        }
    }

    /// `/group` lists the user's groups with member count and the requester's
    /// role label rendered from the localized registry.
    #[tokio::test]
    async fn group_list_handler_renders_populated_group() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "grouplist@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        let bob = seed_member_user(&resources, "bob-list@test.com").await;
        let gid = create_group_row(
            &resources,
            tenant_id,
            &coach_id,
            user_id,
            "Morning Milers",
            true,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Owner,
            None,
            false,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            bob,
            tenant_id,
            GroupRole::Member,
            Some("Bob"),
            false,
        )
        .await;

        let ctx = group_ctx(&resources, user_id, tenant_id, vec![], "/group", None);
        let response = GroupListHandler.execute(&ctx).await.unwrap();

        assert!(
            response.text.contains("Your groups (1):"),
            "expected list header, got: {}",
            response.text
        );
        assert!(response.text.contains("Morning Milers"));
        assert!(
            response.text.contains("2 members"),
            "expected member count, got: {}",
            response.text
        );
        assert!(
            response.text.contains("[owner]"),
            "expected requester role label, got: {}",
            response.text
        );
    }

    /// `/group status` reports member count, active count, and peer-sharing
    /// state for the user's group.
    #[tokio::test]
    async fn group_status_handler_renders_summary() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "groupstatus@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        let bob = seed_member_user(&resources, "bob-status@test.com").await;
        let gid = create_group_row(
            &resources,
            tenant_id,
            &coach_id,
            user_id,
            "Morning Milers",
            true,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Owner,
            None,
            false,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            bob,
            tenant_id,
            GroupRole::Member,
            Some("Bob"),
            false,
        )
        .await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec![],
            "/group status",
            None,
        );
        let response = GroupStatusHandler.execute(&ctx).await.unwrap();

        assert!(response.text.contains("Morning Milers stats"));
        assert!(
            response.text.contains("Members: 2"),
            "expected member count, got: {}",
            response.text
        );
        assert!(response.text.contains("Active: 2"));
        assert!(
            response.text.contains("Peer sharing: on"),
            "peer_data_sharing=true must render `on`, got: {}",
            response.text
        );
    }

    /// `/group members` lists each member with their display name and role,
    /// honoring the localized role labels.
    #[tokio::test]
    async fn group_members_handler_lists_members_with_roles() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "groupmembers@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        let bob = seed_member_user(&resources, "bob-members@test.com").await;
        let gid = create_group_row(
            &resources,
            tenant_id,
            &coach_id,
            user_id,
            "Morning Milers",
            true,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Owner,
            None,
            false,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            bob,
            tenant_id,
            GroupRole::Member,
            None,
            false,
        )
        .await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec![],
            "/group members",
            None,
        );
        let response = GroupMembersHandler.execute(&ctx).await.unwrap();

        // `list_members` surfaces each member's account email as the display
        // name (it joins `users.email`), and the role label is localized.
        assert!(
            response.text.contains("Morning Milers members (2)"),
            "expected members header, got: {}",
            response.text
        );
        assert!(
            response.text.contains("groupmembers@test.com [owner]"),
            "expected owner row with email + role, got: {}",
            response.text
        );
        assert!(
            response.text.contains("bob-members@test.com [member]"),
            "expected member row with email + role, got: {}",
            response.text
        );
    }

    /// `list_members` resolves a member's display name from the joined
    /// `users.email`, NOT the membership row's `display_name` column (left
    /// `None` here). The localized "Unknown" fallback in the handler only
    /// fires for an orphaned membership whose `users` row is missing — a state
    /// the `coaching_group_members.user_id` foreign key prevents — so the
    /// realistic rendering is always the account email.
    #[tokio::test]
    async fn group_members_handler_renders_account_email_as_name() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "groupunknown@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        let gid = create_group_row(
            &resources,
            tenant_id,
            &coach_id,
            user_id,
            "Quiet Group",
            true,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Owner,
            None,
            false,
        )
        .await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec![],
            "/group members",
            None,
        );
        let response = GroupMembersHandler.execute(&ctx).await.unwrap();

        assert!(
            response.text.contains("groupunknown@test.com [owner]"),
            "member name must resolve to the account email, got: {}",
            response.text
        );
    }

    /// `/group invite` is an admin-only action: a plain member is refused
    /// before any invite is generated. This is an authorization boundary —
    /// the check runs regardless of the `tools-groups` feature.
    #[tokio::test]
    async fn group_invite_handler_forbids_non_admin_member() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "groupinviteno@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        // owner_id references a real user (FK); the requester joins as a plain
        // Member so the admin check (which reads the membership role, not
        // owner_id) refuses them.
        let gid = create_group_row(&resources, tenant_id, &coach_id, user_id, "Squad", true).await;
        add_group_member(
            &resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Member,
            None,
            false,
        )
        .await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec![],
            "/group invite",
            None,
        );
        let response = GroupInviteHandler.execute(&ctx).await.unwrap();

        assert!(
            response.text.contains("Only admins and owners"),
            "non-admin invite must be refused, got: {}",
            response.text
        );
    }

    /// `/group invite` issued by an owner generates a real join link + code.
    /// Requires the `tools-groups` feature (the invite branch calls
    /// `group_service().create_invite`); the default test build enables it.
    #[cfg(feature = "tools-groups")]
    #[tokio::test]
    async fn group_invite_handler_generates_link_for_admin() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "groupinviteok@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        let gid = create_group_row(
            &resources,
            tenant_id,
            &coach_id,
            user_id,
            "Morning Milers",
            true,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Owner,
            None,
            false,
        )
        .await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec![],
            "/group invite",
            None,
        );
        let response = GroupInviteHandler.execute(&ctx).await.unwrap();

        assert!(
            response.text.contains("Invite link for Morning Milers"),
            "expected invite body naming the group, got: {}",
            response.text
        );
        assert!(
            response.text.contains("https://app.dravr.ai/groups/join/"),
            "expected join link, got: {}",
            response.text
        );
        assert!(response.text.contains("Code:"));
    }

    /// `/group consent` with an unrecognized argument returns usage help
    /// instead of silently doing nothing (parsed before any group lookup).
    #[tokio::test]
    async fn group_consent_handler_rejects_invalid_arg() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "consentbad@test.com").await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec!["maybe".to_owned()],
            "/group consent maybe",
            None,
        );
        let response = GroupConsentHandler.execute(&ctx).await.unwrap();

        assert!(
            response.text.contains("Usage: /group consent"),
            "invalid arg must return usage, got: {}",
            response.text
        );
    }

    /// `/group consent yes` with no conversation-bound group falls back to the
    /// user's first group and flips that membership's `peer_sharing_consent`.
    #[tokio::test]
    async fn group_consent_handler_falls_back_to_first_group_without_conversation() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "consentfb@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        let gid = create_group_row(
            &resources,
            tenant_id,
            &coach_id,
            user_id,
            "Solo Group",
            true,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Owner,
            None,
            false,
        )
        .await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec!["yes".to_owned()],
            "/group consent yes",
            None,
        );
        let response = GroupConsentHandler.execute(&ctx).await.unwrap();
        assert!(
            response.text.contains("Solo Group"),
            "confirmation should name the resolved group, got: {}",
            response.text
        );

        // Behavioral assertion: the membership flag actually flipped.
        let members = resources
            .common
            .repos
            .groups
            .list_members(&gid.to_string())
            .await
            .unwrap();
        let consent = members
            .iter()
            .find(|m| m.user_id == user_id)
            .expect("membership row")
            .peer_sharing_consent;
        assert!(
            consent,
            "fallback path must flip peer_sharing_consent on the first group"
        );
    }

    /// `/group consent yes` against a conversation bound to a group the user
    /// is NOT a member of affects zero rows → the handler reports "not a
    /// member" rather than silently succeeding.
    #[tokio::test]
    async fn group_consent_handler_errors_when_not_a_member_of_bound_group() {
        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "consent0@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        // The group exists (owner_id references a real user for the FK), but
        // the requester is never added as a member — so the consent update
        // matches zero rows.
        let gid =
            create_group_row(&resources, tenant_id, &coach_id, user_id, "Strangers", true).await;

        // Conversation owned by the requester, bound to that group.
        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                tenant_id,
                "Bound chat",
                "test-model",
                None,
                Some(&gid.to_string()),
            )
            .await
            .unwrap();

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec!["yes".to_owned()],
            "/group consent yes",
            Some(conversation.id.clone()),
        );
        let result = GroupConsentHandler.execute(&ctx).await;
        assert!(
            result.is_err(),
            "consent on a group the user isn't a member of must error (0 rows affected)"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Slash reply privacy routing
    //
    // A slash command issued in any shared room (non-DM) must be answered
    // privately so other members never see the caller's account state. The
    // platform predicate is channel-agnostic — the per-channel private-delivery
    // mechanism (DM / Slack ephemeral / Discord DM) lives in canot. A 1:1 DM is
    // already private, so nothing is redirected.
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn slash_reply_is_private_in_any_shared_room() {
        use pierre_mcp_server::services::messaging_ingress::slash_reply_should_be_private;

        // Non-DM context (group/supergroup/channel) on any platform → private.
        assert!(slash_reply_should_be_private(false, Some("status")));
        assert!(slash_reply_should_be_private(false, Some("group consent")));
        assert!(slash_reply_should_be_private(false, Some("group invite")));
        // No command name (connect card, unknown-command body) keeps the default.
        assert!(slash_reply_should_be_private(false, None));
    }

    #[test]
    fn slash_reply_stays_in_place_for_direct_messages() {
        use pierre_mcp_server::services::messaging_ingress::slash_reply_should_be_private;

        // A 1:1 DM is already private — nothing to redirect.
        assert!(!slash_reply_should_be_private(true, Some("status")));
        assert!(!slash_reply_should_be_private(true, None));
    }

    #[test]
    fn group_setting_changes_are_announced_in_the_room() {
        use pierre_commands::parser::load_command_catalog;
        use pierre_mcp_server::services::messaging_ingress::slash_reply_should_be_private;
        use std::path::Path;

        // Regression (reported live 2026-08-11): `/group respond mentions` in a
        // Telegram group answered the caller privately, so the other members
        // watched the coach go silent with no idea why. A group-wide setting
        // change belongs in the room — for both the respond mode and the
        // group's AI coach persona.
        //
        // The names are read from the real `commands/` catalog rather than
        // written as literals: the value that reaches the visibility rule is
        // the definition's `name:` id (`group-respond`), and a literal spelled
        // as the spaced trigger (`"group respond"`) would match the allowlist
        // in this test while matching nothing in production.
        let commands_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("commands"))
            .expect("repo root resolves from CARGO_MANIFEST_DIR"); // Safe: fixed repo layout
        let defs = load_command_catalog(&commands_dir).definitions;
        assert!(
            !defs.is_empty(),
            "commands/ catalog must load — otherwise this test asserts nothing"
        );

        for trigger in ["/group respond", "/group coach"] {
            let def = defs
                .iter()
                .find(|d| d.command == trigger)
                .unwrap_or_else(|| panic!("no command definition for {trigger}"));
            assert!(
                !slash_reply_should_be_private(false, Some(&def.name)),
                "{trigger} (definition name {:?}) must be announced in the room",
                def.name
            );
        }
    }

    // ════════════════════════════════════════════════════════════════
    // /coach invite — one invite path behind two triggers, and the
    // alias-aware matcher that lets `/coaches <subcommand>` reach it
    // ════════════════════════════════════════════════════════════════

    /// The real `commands/` catalogue loaded into a registry, the way the
    /// server builds it at startup.
    fn real_command_registry() -> CommandRegistry {
        use pierre_commands::parser::load_command_catalog;
        use std::path::Path;

        let commands_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("commands"))
            .expect("repo root resolves from CARGO_MANIFEST_DIR"); // Safe: fixed repo layout
        let defs = load_command_catalog(&commands_dir).definitions;
        assert!(
            !defs.is_empty(),
            "commands/ catalog must load — otherwise this test asserts nothing"
        );
        let mut registry = CommandRegistry::new();
        for def in defs {
            assert!(
                registry.register(def),
                "duplicate command name in catalogue"
            );
        }
        registry
    }

    /// `commands/coach/coach-list.md` aliases `/coach` as `/coaches` and
    /// `/coach list`. The matcher used to greedy-match over commands and
    /// aliases alike, so `/coaches invite` matched the shorter `/coaches`, ran
    /// the list handler and silently dropped `invite`. Every `/coach`
    /// subcommand must be reachable through the alias, with its arguments
    /// intact — which is also why the bare `/coach` is the canonical spelling
    /// and `/coach list` the alias: canot canonicalises an alias by rewriting
    /// it to the definition's command string, and a spaced canonical would
    /// turn `/coaches add @tempo` into `/coach list add @tempo`.
    #[test]
    fn coaches_alias_reaches_every_coach_subcommand() {
        use pierre_messaging::commands::CommandMatcher;

        let registry = real_command_registry();
        let matcher = CommandMatcher::from_registry(&registry);

        for (text, expected_name, expected_args) in [
            ("/coaches invite", "coach-invite", vec![]),
            ("/coaches add @tempo", "coach-add", vec!["@tempo"]),
            ("/coach add @tempo", "coach-add", vec!["@tempo"]),
            ("/coaches remove", "coach-remove", vec![]),
            ("/coaches create", "coach-create", vec![]),
            (
                "/coach create confirm 0123456789abcdef0123456789abcdef",
                "coach-create",
                vec!["confirm", "0123456789abcdef0123456789abcdef"],
            ),
            (
                "/coaches assign 11111111-2222-3333-4444-555555555555 66666666-7777-8888-9999-000000000000",
                "coach-assign",
                vec![
                    "11111111-2222-3333-4444-555555555555",
                    "66666666-7777-8888-9999-000000000000",
                ],
            ),
            ("/coach invite", "coach-invite", vec![]),
            ("/coaches", "coach-list", vec![]),
            ("/coach", "coach-list", vec![]),
            ("/coach list", "coach-list", vec![]),
        ] {
            let parsed = matcher
                .try_match(text, &registry)
                .unwrap_or_else(|| panic!("{text} must match a catalogue command"));
            assert_eq!(parsed.name, expected_name, "{text} dispatched to the wrong handler");
            assert_eq!(parsed.args, expected_args, "{text} lost or gained arguments");
        }
    }

    /// Seed an owner with a group and return `(user_id, tenant_id, group_id)`.
    async fn seed_owner_with_group(
        resources: &ServerContext,
        email: &str,
        group_name: &str,
    ) -> (Uuid, TenantId, Uuid) {
        let (user_id, tenant_id) = create_test_user(resources, email).await;
        let coach_id = seed_coach(resources, user_id, tenant_id, "Coach", "desc").await;
        let gid =
            create_group_row(resources, tenant_id, &coach_id, user_id, group_name, true).await;
        add_group_member(
            resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Owner,
            None,
            false,
        )
        .await;
        (user_id, tenant_id, gid)
    }

    /// Pull the invite code out of a rendered invite body (the `Code:` line).
    fn invite_code_from(body: &str) -> String {
        body.lines()
            .find_map(|line| line.strip_prefix("Code: "))
            .unwrap_or_else(|| panic!("no `Code:` line in invite body: {body}"))
            .trim()
            .to_owned()
    }

    /// `/coach invite` issued by an owner files a *coach*-kind invite for the
    /// conversation's group through `GroupService` — the invite row carries
    /// `kind = coach`, so whoever redeems it is attached as the human coach.
    #[cfg(feature = "tools-groups")]
    #[tokio::test]
    async fn coach_invite_handler_files_a_coach_invite_for_admin() {
        use pierre_commands::coach::CoachInviteHandler;
        use pierre_core::models::groups::GroupInviteKind;

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, gid) =
            seed_owner_with_group(&resources, "coachinviteok@test.com", "Morning Milers").await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec![],
            "/coach invite",
            None,
        );
        let response = CoachInviteHandler.execute(&ctx).await.unwrap();

        assert!(
            response.text.starts_with("Coach invite for Morning Milers"),
            "expected the coach-invite body naming the group, got: {}",
            response.text
        );
        assert!(
            response.text.contains("https://app.dravr.ai/groups/join/"),
            "expected join link, got: {}",
            response.text
        );

        let code = invite_code_from(&response.text);
        let invite = resources
            .common
            .repos
            .groups
            .get_invite_by_code(&code)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("invite {code} must be persisted"));
        assert_eq!(invite.kind, GroupInviteKind::Coach);
        assert_eq!(invite.group_id, gid);
        assert_eq!(invite.created_by, user_id);
    }

    /// `/group invite coach` and `/coach invite` are one implementation: both
    /// produce a coach-kind invite for the same group with the same body.
    #[cfg(feature = "tools-groups")]
    #[tokio::test]
    async fn group_invite_coach_and_coach_invite_share_one_path() {
        use pierre_commands::coach::CoachInviteHandler;
        use pierre_core::models::groups::GroupInviteKind;

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, gid) =
            seed_owner_with_group(&resources, "coachinviteboth@test.com", "Trail Crew").await;

        let via_group = GroupInviteHandler
            .execute(&group_ctx(
                &resources,
                user_id,
                tenant_id,
                vec!["coach".to_owned()],
                "/group invite coach",
                None,
            ))
            .await
            .unwrap();
        let via_coach = CoachInviteHandler
            .execute(&group_ctx(
                &resources,
                user_id,
                tenant_id,
                vec![],
                "/coach invite",
                None,
            ))
            .await
            .unwrap();

        for body in [&via_group.text, &via_coach.text] {
            assert!(
                body.starts_with("Coach invite for Trail Crew"),
                "both triggers render the coach-invite body, got: {body}"
            );
            let invite = resources
                .common
                .repos
                .groups
                .get_invite_by_code(&invite_code_from(body))
                .await
                .unwrap()
                .expect("invite persisted");
            assert_eq!(invite.kind, GroupInviteKind::Coach);
            assert_eq!(invite.group_id, gid);
        }

        // A plain `/group invite` still issues an athlete invite — the coach
        // wording is reserved for the coach kind.
        let member_invite = GroupInviteHandler
            .execute(&group_ctx(
                &resources,
                user_id,
                tenant_id,
                vec![],
                "/group invite",
                None,
            ))
            .await
            .unwrap();
        assert!(
            member_invite.text.starts_with("Invite link for Trail Crew"),
            "got: {}",
            member_invite.text
        );
        let invite = resources
            .common
            .repos
            .groups
            .get_invite_by_code(&invite_code_from(&member_invite.text))
            .await
            .unwrap()
            .expect("invite persisted");
        assert_eq!(invite.kind, GroupInviteKind::Member);
    }

    /// `/coach invite` is admin-only, exactly like `/group invite`: a plain
    /// member is refused before any invite is generated.
    #[tokio::test]
    async fn coach_invite_handler_forbids_non_admin_member() {
        use pierre_commands::coach::CoachInviteHandler;

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "coachinviteno@test.com").await;
        let coach_id = seed_coach(&resources, user_id, tenant_id, "Coach", "desc").await;
        let gid = create_group_row(
            &resources,
            tenant_id,
            &coach_id,
            user_id,
            "Morning Milers",
            true,
        )
        .await;
        add_group_member(
            &resources,
            gid,
            user_id,
            tenant_id,
            GroupRole::Member,
            None,
            false,
        )
        .await;

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec![],
            "/coach invite",
            None,
        );
        let response = CoachInviteHandler.execute(&ctx).await.unwrap();

        assert_eq!(
            response.text, "Only admins and owners can generate invite links.",
            "non-admin coach invite must be refused"
        );
        assert!(
            !response.text.contains("Code:"),
            "no invite may be generated for a refused caller"
        );
    }

    /// End to end through the dispatcher every chat surface uses: `/coaches
    /// invite` typed with the alias reaches the `coach-invite` handler and
    /// files a coach invite — not the list handler with `invite` as an
    /// argument.
    #[cfg(feature = "tools-groups")]
    #[tokio::test]
    async fn dispatching_coaches_invite_runs_the_invite_handler() {
        use pierre_commands::coach::{CoachInviteHandler, CoachListHandler};
        use pierre_commands::dispatch::{try_dispatch, DispatchOutcome, DispatchRequest};
        use pierre_commands::CommandHandlerRegistry;
        use pierre_core::models::groups::GroupInviteKind;
        use pierre_runtime_context::CommandCtx;
        use pierre_tool_runtime::runtime::ToolRuntime;

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id, gid) =
            seed_owner_with_group(&resources, "coachinvitedispatch@test.com", "Dispatch Club")
                .await;

        let command_registry = Arc::new(real_command_registry());
        let mut handlers = CommandHandlerRegistry::new();
        handlers.register("coach-list", Arc::new(CoachListHandler));
        handlers.register("coach-invite", Arc::new(CoachInviteHandler));
        let handlers = Arc::new(handlers);
        let ctx: Arc<dyn CommandCtx> = Arc::<ServerContext>::clone(&resources);
        let tool_runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);

        let outcome = try_dispatch(DispatchRequest {
            ctx: &ctx,
            command_registry: &command_registry,
            command_handler_registry: &handlers,
            user_id,
            tenant_id,
            channel_type: "telegram",
            locale: "en",
            is_direct_message: false,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: tenant_id,
            sender_id: None,
            text: "/coaches invite",
            tool_runtime: &tool_runtime,
        })
        .await
        .unwrap();

        let DispatchOutcome::Executed {
            command_name,
            response,
        } = outcome
        else {
            panic!("/coaches invite must execute a registered command");
        };
        assert_eq!(command_name, "coach-invite");
        assert!(
            response.text.starts_with("Coach invite for Dispatch Club"),
            "got: {}",
            response.text
        );
        let invite = resources
            .common
            .repos
            .groups
            .get_invite_by_code(&invite_code_from(&response.text))
            .await
            .unwrap()
            .expect("invite persisted");
        assert_eq!(invite.kind, GroupInviteKind::Coach);
        assert_eq!(invite.group_id, gid);
    }

    // ════════════════════════════════════════════════════════════════
    // /coach add @handle — the installed coach, by its catalogue handle
    // ════════════════════════════════════════════════════════════════

    /// Publish a catalogue coach under a fresh author and install it for
    /// `user_id`; returns the athlete's installed copy.
    async fn install_recovery_coach(
        resources: &ServerContext,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Coach {
        use crate::helpers::coach_fixtures::{install_catalogue_coach, publish_catalogue_coach};

        let (author_id, author_tenant) =
            create_test_user(resources, &format!("author-{user_id}@test.com")).await;
        let origin = publish_catalogue_coach(
            &resources.common.repos,
            author_id,
            author_tenant,
            "Recovery Coach",
            "You are the recovery coach.",
        )
        .await;
        install_catalogue_coach(&resources.common.repos, origin, user_id, tenant_id).await
    }

    /// A DM conversation for the caller, so the handler has a row to bind.
    async fn dm_conversation(
        resources: &ServerContext,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> String {
        resources
            .common
            .repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                tenant_id,
                "Coach invite DM",
                "test-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id
    }

    /// The coach the conversation row is bound to, read back from the store.
    async fn conversation_coach(
        resources: &ServerContext,
        conversation_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Option<String> {
        resources
            .common
            .repos
            .chat
            .get_conversation(conversation_id, &user_id.to_string(), tenant_id)
            .await
            .unwrap()
            .expect("the conversation exists")
            .coach_id
    }

    /// `/coach add @handle` resolves the caller's installed coach and
    /// attaches it exactly as `/coach add <id>` does: the selection pointer
    /// moves and the conversation the command was typed in is rebound.
    #[tokio::test]
    async fn coach_add_handle_selects_the_installed_coach_and_binds_the_conversation() {
        use pierre_commands::coach::CoachAddHandler;

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "coachinvitehandle@test.com").await;
        let installed = install_recovery_coach(&resources, user_id, tenant_id).await;
        let installed_id = installed.id.to_string();
        assert_eq!(installed.handle.as_deref(), Some("recovery-coach"));
        let conversation_id = dm_conversation(&resources, user_id, tenant_id).await;
        assert_eq!(
            conversation_coach(&resources, &conversation_id, user_id, tenant_id).await,
            None,
            "fixture precondition: no coach bound before the command"
        );

        let ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec!["@recovery-coach".to_owned()],
            "/coach add @recovery-coach",
            Some(conversation_id.clone()),
        );
        let response = CoachAddHandler.execute(&ctx).await.unwrap();

        assert_eq!(response.text, "Coach selected: Recovery Coach.");
        assert_eq!(
            conversation_coach(&resources, &conversation_id, user_id, tenant_id)
                .await
                .as_deref(),
            Some(installed_id.as_str()),
            "the conversation is bound to the caller's installed copy"
        );
        assert_eq!(
            resources
                .common
                .repos
                .tenants
                .get_selected_coach(tenant_id, user_id)
                .await
                .unwrap()
                .as_deref(),
            Some(installed_id.as_str()),
            "the selection pointer moves, as /coach add <id> moves it"
        );
    }

    /// A handle that names no installed coach — unknown, or a catalogue coach
    /// the caller never installed, with or without its `@` — is refused by
    /// name in the caller's locale, and nothing else happens: no pointer, no
    /// rebind. A bare `/coach add` gets the usage line.
    #[tokio::test]
    async fn coach_add_unknown_handle_is_refused_by_name_and_binds_nothing() {
        use crate::helpers::coach_fixtures::publish_catalogue_coach;
        use pierre_commands::coach::CoachAddHandler;

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) =
            create_test_user(&resources, "coachinviteunknown@test.com").await;
        let (author_id, author_tenant) =
            create_test_user(&resources, "coachinviteunknown-author@test.com").await;
        // Published, so `strength-coach` exists in the catalogue — but the
        // caller never installed it.
        publish_catalogue_coach(
            &resources.common.repos,
            author_id,
            author_tenant,
            "Strength Coach",
            "You are the strength coach.",
        )
        .await;
        let conversation_id = dm_conversation(&resources, user_id, tenant_id).await;

        for (typed, expected) in [
            (
                "@nobody-here",
                "No installed coach answers to @nobody-here. Type /coach to see your list, or /discover to install one.",
            ),
            (
                "@strength-coach",
                "No installed coach answers to @strength-coach. Type /coach to see your list, or /discover to install one.",
            ),
            (
                "strength-coach",
                "No installed coach answers to @strength-coach. Type /coach to see your list, or /discover to install one.",
            ),
        ] {
            let ctx = group_ctx(
                &resources,
                user_id,
                tenant_id,
                vec![typed.to_owned()],
                &format!("/coach add {typed}"),
                Some(conversation_id.clone()),
            );
            let response = CoachAddHandler.execute(&ctx).await.unwrap();
            assert_eq!(response.text, expected, "typed {typed}");
        }

        let bare = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec![],
            "/coach add",
            Some(conversation_id.clone()),
        );
        let response = CoachAddHandler.execute(&bare).await.unwrap();
        assert_eq!(
            response.text,
            "Say which coach to add: /coach add @handle. Type /coach to see your list."
        );

        // French, the platform default locale.
        let mut ctx = group_ctx(
            &resources,
            user_id,
            tenant_id,
            vec!["@nobody-here".to_owned()],
            "/coach add @nobody-here",
            Some(conversation_id.clone()),
        );
        ctx.locale = "fr".to_owned();
        let response = CoachAddHandler.execute(&ctx).await.unwrap();
        assert_eq!(
            response.text,
            "Aucun coach installé ne répond à @nobody-here. Tape /coach pour voir ta liste, ou /discover pour l'installer."
        );

        assert_eq!(
            conversation_coach(&resources, &conversation_id, user_id, tenant_id).await,
            None,
            "nothing was bound"
        );
        assert_eq!(
            resources
                .common
                .repos
                .tenants
                .get_selected_coach(tenant_id, user_id)
                .await
                .unwrap(),
            None,
            "nothing was selected"
        );
    }

    /// `/coaches add @handle` — the alias, with the argument — reaches the
    /// add handler with the handle intact, through the dispatcher every chat
    /// surface uses.
    #[tokio::test]
    async fn dispatching_coaches_add_with_a_handle_selects_the_coach() {
        use pierre_commands::coach::{CoachAddHandler, CoachListHandler};
        use pierre_commands::dispatch::{try_dispatch, DispatchOutcome, DispatchRequest};
        use pierre_commands::CommandHandlerRegistry;
        use pierre_runtime_context::CommandCtx;
        use pierre_tool_runtime::runtime::ToolRuntime;

        let resources = create_test_server_resources().await.unwrap();
        let (user_id, tenant_id) = create_test_user(&resources, "coachinvitealias@test.com").await;
        let installed = install_recovery_coach(&resources, user_id, tenant_id).await;
        let installed_id = installed.id.to_string();
        let conversation_id = dm_conversation(&resources, user_id, tenant_id).await;

        let command_registry = Arc::new(real_command_registry());
        let mut handlers = CommandHandlerRegistry::new();
        handlers.register("coach-list", Arc::new(CoachListHandler));
        handlers.register("coach-add", Arc::new(CoachAddHandler));
        let handlers = Arc::new(handlers);
        let ctx: Arc<dyn CommandCtx> = Arc::<ServerContext>::clone(&resources);
        let tool_runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);

        let outcome = try_dispatch(DispatchRequest {
            ctx: &ctx,
            command_registry: &command_registry,
            command_handler_registry: &handlers,
            user_id,
            tenant_id,
            channel_type: "telegram",
            locale: "en",
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: Some(&conversation_id),
            conversation_tenant_id: tenant_id,
            sender_id: None,
            text: "/coaches add @recovery-coach",
            tool_runtime: &tool_runtime,
        })
        .await
        .unwrap();

        let DispatchOutcome::Executed {
            command_name,
            response,
        } = outcome
        else {
            panic!("/coaches add @recovery-coach must execute a registered command");
        };
        assert_eq!(command_name, "coach-add");
        assert_eq!(response.text, "Coach selected: Recovery Coach.");
        assert_eq!(
            conversation_coach(&resources, &conversation_id, user_id, tenant_id)
                .await
                .as_deref(),
            Some(installed_id.as_str()),
            "the alias binds the same conversation the canonical spelling binds"
        );
    }
}
