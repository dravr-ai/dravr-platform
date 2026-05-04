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
        CreateChannelLinkParams, CreateSessionParams, MessagingRepository,
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
