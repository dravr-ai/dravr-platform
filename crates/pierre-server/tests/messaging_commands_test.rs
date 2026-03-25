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
    use pierre_database::plugins::{
        CreateChannelLinkParams, CreateSessionParams, MessagingRepository,
        UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerResources;
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

    async fn create_test_user(resources: &ServerResources, email: &str) -> (Uuid, TenantId) {
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

    async fn setup_linked_user(resources: &ServerResources) -> (axum::Router, Uuid, TenantId) {
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

        // Create messaging session
        let session_id = Uuid::new_v4().to_string();
        let conversation_id = Uuid::new_v4().to_string();
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
    async fn test_unknown_command_falls_through() {
        let resources = create_test_server_resources().await.unwrap();
        let (router, ..) = setup_linked_user(&resources).await;

        // Unrecognized /command should pass through to LLM dispatch
        let (status, body) = send_command(&router, "/unknown_command", 12).await;
        assert_eq!(status, StatusCode::OK);
        // The webhook returns OK but the message is stored for LLM dispatch
        // (not handled as a command)
        let stored = body["messages_stored"].as_i64().unwrap_or(0);
        assert!(stored > 0, "Unrecognized command should be stored for LLM");
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
}
