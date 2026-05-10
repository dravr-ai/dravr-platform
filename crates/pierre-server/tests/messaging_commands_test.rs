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
    use pierre_database::backends::{
        CreateChannelLinkParams, CreateSessionParams, InsertMessageParams, MessagingRepository,
        UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
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
        resources.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::new();
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
        resources.repos.tenants.create(&tenant).await.unwrap();

        (user_id, tenant_id)
    }

    async fn setup_linked_user(resources: &ServerContext) -> (axum::Router, Uuid, TenantId) {
        let db: &dyn MessagingRepository = &*resources.repos.messaging;
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
        let (router, ..) = setup_linked_user(&resources).await;

        let (status, _) = send_command(&router, "/logout", 4).await;
        assert_eq!(status, StatusCode::OK);
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
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

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
        })
        .await
        .unwrap();

        // Bare "logout" (no slash) is the confirmation reply that triggers the
        // actual unlink — `/logout` only shows the confirmation prompt.
        let (status, _) = send_command(&router, "logout", 100).await;
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
        use pierre_mcp_server::services::commands::privacy::PrivacyOnHandler;
        use pierre_mcp_server::services::commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;

        // Confirm initial state: analytics_consent is false
        let before = resources
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
            resources: Arc::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            conversation_id: None,
        };

        let response = PrivacyOnHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("enabled"));

        // Verify the database reflects the enabled state
        let after = resources
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
        use pierre_mcp_server::services::commands::privacy::PrivacyOffHandler;
        use pierre_mcp_server::services::commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;

        // Seed with consent enabled so we can verify the toggle
        resources
            .repos
            .users
            .update_analytics_consent(user_id, true)
            .await
            .unwrap();
        let before = resources
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
            resources: Arc::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            conversation_id: None,
        };

        let response = PrivacyOffHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("disabled"));

        // Verify the database reflects the disabled state
        let after = resources
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert!(!after.analytics_consent);
    }

    // ════════════════════════════════════════════════════════════════
    // /coach DM vs group regression coverage
    //
    // These tests pin down the DM fix:
    //   - DM: /coach select writes users.default_coach_id, never creates a
    //     group, and the confirmation copy omits "group"/"groupe" wording.
    //   - Group (`is_direct_message = false`) with zero owned groups: old
    //     auto-create path still fires (covered indirectly by the DM path
    //     being distinct — we assert the DM path specifically does NOT
    //     touch groups).
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
        };
        let coach = resources
            .repos
            .coaches
            .create(user_id, tenant_id, &request)
            .await
            .unwrap();
        coach.id.to_string()
    }

    #[tokio::test]
    async fn coach_select_in_dm_sets_users_default_coach() {
        use pierre_mcp_server::services::commands::coach::CoachSelectHandler;
        use pierre_mcp_server::services::commands::{CommandHandler, PlatformCommandContext};

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
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert!(before.default_coach_id.is_none());

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![coach_id.clone()],
            raw_text: format!("/coach select {coach_id}"),
            resources: Arc::clone(&resources),
            locale: "fr".to_owned(),
            is_direct_message: true,
            conversation_id: None,
        };

        let response = CoachSelectHandler.execute(&ctx).await.unwrap();

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

        // Post-condition: default_coach_id persisted on the user row.
        let after = resources
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.default_coach_id.as_deref(), Some(coach_id.as_str()));
    }

    #[tokio::test]
    async fn coach_select_in_dm_clearing_and_reselecting_swaps_coach() {
        use pierre_mcp_server::services::commands::coach::CoachSelectHandler;
        use pierre_mcp_server::services::commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;
        let coach_a = seed_coach(&resources, user_id, tenant_id, "Coach A", "First.").await;
        let coach_b = seed_coach(&resources, user_id, tenant_id, "Coach B", "Second.").await;

        let mk_ctx = |coach: &str| PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![coach.to_owned()],
            raw_text: format!("/coach select {coach}"),
            resources: Arc::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: true,
            conversation_id: None,
        };

        CoachSelectHandler.execute(&mk_ctx(&coach_a)).await.unwrap();
        let after_a = resources
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_a.default_coach_id.as_deref(), Some(coach_a.as_str()));

        CoachSelectHandler.execute(&mk_ctx(&coach_b)).await.unwrap();
        let after_b = resources
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_b.default_coach_id.as_deref(), Some(coach_b.as_str()));
    }

    #[tokio::test]
    async fn coach_list_renders_without_markdown_asterisks() {
        use pierre_mcp_server::services::commands::coach::CoachListHandler;
        use pierre_mcp_server::services::commands::{CommandHandler, PlatformCommandContext};

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
            resources: Arc::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: true,
            conversation_id: None,
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
        use pierre_core::models::coaches::{
            CoachCategory, CoachVisibility, CreateSystemCoachRequest,
        };
        use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRole};
        use pierre_mcp_server::services::commands::group::GroupConsentHandler;
        use pierre_mcp_server::services::commands::{CommandHandler, PlatformCommandContext};
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;
        let user_id_str = user_id.to_string();

        // Coach (required as FK for both groups). Seed a minimal system
        // coach directly via the repo — `create_test_server_resources`
        // does not auto-seed coaches.
        let coach = resources
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
            peer_data_sharing: true,
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
            .repos
            .groups
            .create_group(tenant_id, &chat_group)
            .await
            .unwrap();
        resources
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
            .repos
            .groups
            .add_member(&mk_member(chat_group_id))
            .await
            .unwrap();
        resources
            .repos
            .groups
            .add_member(&mk_member(other_group_id))
            .await
            .unwrap();

        // Conversation explicitly bound to the chat group.
        let conversation = resources
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
            resources: Arc::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            conversation_id: Some(conversation.id.clone()),
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

    #[tokio::test]
    async fn test_privacy_status_handler_reads_current_state() {
        use pierre_mcp_server::services::commands::privacy::PrivacyStatusHandler;
        use pierre_mcp_server::services::commands::{CommandHandler, PlatformCommandContext};

        let resources = create_test_server_resources().await.unwrap();
        let (_router, user_id, tenant_id) = setup_linked_user(&resources).await;

        let ctx = PlatformCommandContext {
            user_id,
            tenant_id,
            channel_type: "telegram".to_owned(),
            args: vec![],
            raw_text: "/privacy".to_owned(),
            resources: Arc::clone(&resources),
            locale: "en".to_owned(),
            is_direct_message: false,
            conversation_id: None,
        };

        // Default state: consent disabled
        let response = PrivacyStatusHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("disabled"));

        // Flip to enabled and re-run
        resources
            .repos
            .users
            .update_analytics_consent(user_id, true)
            .await
            .unwrap();
        let response = PrivacyStatusHandler.execute(&ctx).await.unwrap();
        assert!(response.text.contains("enabled"));
    }
}
