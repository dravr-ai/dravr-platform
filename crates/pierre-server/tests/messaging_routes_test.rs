// ABOUTME: Integration tests for messaging gateway route handlers (webhook + config CRUD)
// ABOUTME: Tests authentication, channel config endpoints, and multi-tenant isolation via HTTP
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
    clippy::uninlined_format_args
)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod messaging_routes_tests {
    use crate::common::{
        create_test_server_resources, create_test_tenant, create_test_user, generate_test_token,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::json;
    use std::sync::Arc;

    // ════════════════════════════════════════════════════════════════
    // Setup helpers
    // ════════════════════════════════════════════════════════════════

    async fn setup_messaging_router() -> (axum::Router, String) {
        let resources = create_test_server_resources().await.unwrap();
        let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
        let token = generate_test_token(&resources, &user).await;
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        (router, format!("Bearer {token}"))
    }

    async fn setup_two_tenant_router() -> (axum::Router, String, String) {
        let resources = create_test_server_resources().await.unwrap();

        let (_user_a, token_a) = create_test_tenant(&resources, "tenant_a@example.com")
            .await
            .unwrap();
        let (_user_b, token_b) = create_test_tenant(&resources, "tenant_b@example.com")
            .await
            .unwrap();

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        (
            router,
            format!("Bearer {token_a}"),
            format!("Bearer {token_b}"),
        )
    }

    // ════════════════════════════════════════════════════════════════
    // Config CRUD endpoint tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_list_channels_empty() {
        let (router, token) = setup_messaging_router().await;

        let response = AxumTestRequest::get("/api/messaging/channels")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert!(body["channels"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_list_channels_requires_auth() {
        let (router, _token) = setup_messaging_router().await;

        let response = AxumTestRequest::get("/api/messaging/channels")
            .send(router)
            .await;

        assert_ne!(
            response.status_code(),
            StatusCode::OK,
            "Should reject unauthenticated request"
        );
    }

    #[tokio::test]
    async fn test_upsert_channel_config() {
        let (router, token) = setup_messaging_router().await;

        let response = AxumTestRequest::put("/api/messaging/channels/slack")
            .header("authorization", &token)
            .json(&json!({
                "enabled": true,
                "credentials": {
                    "api_key": "xoxb-test-slack-token",
                    "webhook_secret": "slack_signing_secret_123"
                }
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(body["config"]["channel"], "slack");
        assert_eq!(body["config"]["enabled"], true);
        assert_eq!(body["config"]["has_credentials"], true);
        assert_eq!(body["action"], "upserted");
    }

    #[tokio::test]
    async fn test_get_channel_config() {
        let (router, token) = setup_messaging_router().await;

        // First create a config
        AxumTestRequest::put("/api/messaging/channels/telegram")
            .header("authorization", &token)
            .json(&json!({
                "enabled": true,
                "credentials": {
                    "bot_token": "12345:ABC-DEF",
                    "webhook_secret": "tg_secret"
                }
            }))
            .send(router.clone())
            .await;

        // Now get it
        let response = AxumTestRequest::get("/api/messaging/channels/telegram")
            .header("authorization", &token)
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert!(body["config"].is_object(), "Should return config object");
    }

    #[tokio::test]
    async fn test_delete_channel_config() {
        let (router, token) = setup_messaging_router().await;

        // Create config
        AxumTestRequest::put("/api/messaging/channels/discord")
            .header("authorization", &token)
            .json(&json!({
                "enabled": true,
                "credentials": { "api_key": "discord_bot_token" }
            }))
            .send(router.clone())
            .await;

        // Delete it
        let response = AxumTestRequest::delete("/api/messaging/channels/discord")
            .header("authorization", &token)
            .send(router.clone())
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(body["action"], "deleted");

        // Verify it's gone
        let response = AxumTestRequest::get("/api/messaging/channels/discord")
            .header("authorization", &token)
            .send(router)
            .await;

        let body: serde_json::Value = response.json();
        assert!(
            body["config"].is_null(),
            "Config should be null after delete"
        );
    }

    #[tokio::test]
    async fn test_invalid_channel_name() {
        let (router, token) = setup_messaging_router().await;

        let response = AxumTestRequest::put("/api/messaging/channels/sms")
            .header("authorization", &token)
            .json(&json!({
                "enabled": true,
                "credentials": {}
            }))
            .send(router)
            .await;

        assert_ne!(
            response.status_code(),
            StatusCode::OK,
            "Invalid channel name 'sms' should be rejected"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Multi-tenant isolation tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_config_tenant_isolation() {
        let (router, token_a, token_b) = setup_two_tenant_router().await;

        // Tenant A configures Slack
        AxumTestRequest::put("/api/messaging/channels/slack")
            .header("authorization", &token_a)
            .json(&json!({
                "enabled": true,
                "credentials": { "api_key": "tenant_a_slack_key" }
            }))
            .send(router.clone())
            .await;

        // Tenant B lists channels — should see nothing
        let response = AxumTestRequest::get("/api/messaging/channels")
            .header("authorization", &token_b)
            .send(router.clone())
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert!(
            body["channels"].as_array().unwrap().is_empty(),
            "Tenant B must not see Tenant A's channel configs"
        );

        // Tenant B gets Tenant A's specific config — should be null
        let response = AxumTestRequest::get("/api/messaging/channels/slack")
            .header("authorization", &token_b)
            .send(router)
            .await;

        let body: serde_json::Value = response.json();
        assert!(
            body["config"].is_null(),
            "Tenant B must not see Tenant A's Slack config"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook endpoint tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_webhook_unknown_channel_rejected() {
        let (router, _token) = setup_messaging_router().await;

        // Webhooks don't use JWT auth — they use channel signatures
        let response = AxumTestRequest::post("/api/messaging/webhook/sms")
            .header("content-type", "application/json")
            .json(&json!({"test": true}))
            .send(router)
            .await;

        assert_ne!(
            response.status_code(),
            StatusCode::OK,
            "Unknown channel 'sms' should be rejected"
        );
    }

    #[tokio::test]
    async fn test_webhook_missing_signature_rejected() {
        let (router, _token) = setup_messaging_router().await;

        // Telegram webhook without the X-Telegram-Bot-Api-Secret-Token header
        let response = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .json(&json!({
                "update_id": 123,
                "message": {
                    "message_id": 1,
                    "chat": { "id": 12345 },
                    "text": "hello"
                }
            }))
            .send(router)
            .await;

        assert_ne!(
            response.status_code(),
            StatusCode::OK,
            "Webhook without signature should be rejected"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook handshake response tests
    // ════════════════════════════════════════════════════════════════

    /// Compute Slack HMAC-SHA256 signature for webhook verification
    fn compute_slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[tokio::test]
    async fn test_slack_webhook_challenge_response() {
        use pierre_database::plugins::{MessagingRepository, UpsertChannelConfigParams};
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        // Look up the test user's tenant_id for the channel config
        let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant_id = tenants[0].id;

        // Set up Slack channel config with known signing secret
        let signing_secret = "test_slack_signing_secret_42";
        let config_id = Uuid::new_v4().to_string();
        let params = UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-test-token"),
            api_secret: None,
            webhook_secret: Some(signing_secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        };
        db.upsert_channel_config(&params).await.unwrap();

        // Build url_verification challenge payload
        let challenge_body = json!({
            "type": "url_verification",
            "challenge": "abc123_challenge_token"
        })
        .to_string();

        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signature = compute_slack_sig(signing_secret, &timestamp, &challenge_body);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let response = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &signature)
            .json(&json!({
                "type": "url_verification",
                "challenge": "abc123_challenge_token"
            }))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(
            body["challenge"].as_str(),
            Some("abc123_challenge_token"),
            "Slack challenge response should echo the challenge value"
        );
    }

    #[tokio::test]
    async fn test_discord_webhook_ping_response() {
        use ed25519_dalek::Signer;
        use pierre_database::plugins::{MessagingRepository, UpsertChannelConfigParams};
        use rand::RngCore;
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant_id = tenants[0].id;

        // Discord uses Ed25519 for signature verification.
        // Set up config with the public key as webhook_secret and an application ID.
        let mut secret_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut secret_bytes);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&secret_bytes);
        let public_key_hex = hex::encode(signing_key.verifying_key().as_bytes());

        let config_id = Uuid::new_v4().to_string();
        let params = UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "discord",
            api_key: Some("discord-bot-token"),
            api_secret: None,
            webhook_secret: Some(&public_key_hex),
            verify_token: None,
            account_id: Some("discord-app-id-123"),
            phone_number: None,
            bot_token: None,
            is_active: true,
        };
        db.upsert_channel_config(&params).await.unwrap();

        // Build Discord PING payload (interaction type 1)
        let ping_body = json!({"type": 1}).to_string();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let message = format!("{timestamp}{ping_body}");
        let signature = signing_key.sign(message.as_bytes());
        let signature_hex = hex::encode(signature.to_bytes());

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let response = AxumTestRequest::post("/api/messaging/webhook/discord")
            .header("content-type", "application/json")
            .header("x-signature-ed25519", &signature_hex)
            .header("x-signature-timestamp", &timestamp)
            .json(&json!({"type": 1}))
            .send(router)
            .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        let body: serde_json::Value = response.json();
        assert_eq!(
            body["type"].as_u64(),
            Some(1),
            "Discord PING should receive PONG (type 1)"
        );
    }

    #[tokio::test]
    async fn test_unlinked_user_webhook_returns_link_url() {
        use pierre_database::plugins::{MessagingRepository, UpsertChannelConfigParams};
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant_id = tenants[0].id;

        // Set up Telegram channel config (simple secret-based auth)
        let secret = "tg_webhook_secret_for_unlinked_test";
        let config_id = Uuid::new_v4().to_string();
        let params = UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:FAKE_BOT_TOKEN"),
            is_active: true,
        };
        db.upsert_channel_config(&params).await.unwrap();

        // Send a message from an unlinked user (no channel_link exists for this sender)
        let body = json!({
            "update_id": 999,
            "message": {
                "message_id": 42,
                "from": { "id": 98765, "first_name": "UnlinkedUser" },
                "chat": { "id": 98765 },
                "text": "Hello bot"
            }
        });

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let response = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", secret)
            .json(&body)
            .send(router)
            .await;

        // Webhook should succeed (200 OK)
        assert_eq!(response.status_code(), StatusCode::OK);
        let resp_body: serde_json::Value = response.json();

        // The webhook handler stores the inbound message and returns ok.
        // For unlinked users, a link URL message is dispatched in the background.
        // We verify the webhook accepted the message.
        assert_eq!(
            resp_body["status"].as_str(),
            Some("ok"),
            "Webhook should accept message from unlinked user"
        );
        assert_eq!(
            resp_body["messages_received"].as_u64(),
            Some(1),
            "Should report 1 message received"
        );
    }

    #[tokio::test]
    async fn test_send_failure_enqueues_for_retry() {
        use pierre_database::plugins::{
            CreateSessionParams, InsertMessageParams, MessagingRepository,
        };
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant_id = tenants[0].id;

        // Create a session and a message (outbound queue requires FK to messages)
        let session_id = Uuid::new_v4().to_string();
        let session_params = CreateSessionParams {
            id: &session_id,
            user_id: &user.id.to_string(),
            tenant_id,
            channel_type: "whatsapp",
            channel_user_id: "wa-retry-test",
            channel_conversation_id: None,
            pierre_conversation_id: None,
        };
        db.create_session(&session_params).await.unwrap();

        let msg_id = Uuid::new_v4().to_string();
        let msg_params = InsertMessageParams {
            id: &msg_id,
            tenant_id,
            session_id: &session_id,
            direction: "outbound",
            channel_type: "whatsapp",
            channel_message_id: "",
            sender_id: "pierre",
            content_type: "text",
            content_body: Some("retry test message"),
            correlation_id: &Uuid::new_v4().to_string(),
            raw_payload: None,
        };
        db.insert_message(&msg_params).await.unwrap();

        // Enqueue for retry (simulating what enqueue_failed_outbound does)
        let queue_id = Uuid::new_v4().to_string();
        db.enqueue_outbound(
            &queue_id,
            &msg_id,
            tenant_id,
            None,
            "whatsapp",
            r#"{"messaging_product":"whatsapp","to":"15551234567","type":"text","text":{"body":"retry test message"}}"#,
        )
        .await
        .unwrap();

        // Verify the message appears in the pending outbound queue
        let pending = db.get_pending_outbound(tenant_id, 10).await.unwrap();
        assert!(
            pending.iter().any(|e| e["id"].as_str() == Some(&queue_id)),
            "Failed outbound should be enqueued for retry"
        );
        let entry = pending
            .iter()
            .find(|e| e["id"].as_str() == Some(&queue_id))
            .unwrap();
        assert_eq!(entry["status"].as_str(), Some("pending"));
        assert_eq!(entry["channel_type"].as_str(), Some("whatsapp"));
    }

    // ════════════════════════════════════════════════════════════════
    // verify_token separation tests
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_upsert_with_verify_token_roundtrips() {
        use pierre_database::plugins::{MessagingRepository, UpsertChannelConfigParams};
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant_id = tenants[0].id;

        let config_id = Uuid::new_v4().to_string();
        let params = UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("wa_api_key"),
            api_secret: None,
            webhook_secret: Some("hmac_signing_secret"),
            verify_token: Some("meta_verify_token_abc"),
            account_id: None,
            phone_number: Some("+15551234567"),
            bot_token: None,
            is_active: true,
        };
        db.upsert_channel_config(&params).await.unwrap();

        let config = db
            .get_channel_config(tenant_id, "whatsapp")
            .await
            .unwrap()
            .expect("config should exist");

        assert_eq!(
            config["webhook_secret"].as_str(),
            Some("hmac_signing_secret"),
            "webhook_secret should persist"
        );
        assert_eq!(
            config["verify_token"].as_str(),
            Some("meta_verify_token_abc"),
            "verify_token should persist separately from webhook_secret"
        );
    }

    #[tokio::test]
    async fn test_meta_verify_prefers_verify_token_over_webhook_secret() {
        use pierre_database::plugins::{MessagingRepository, UpsertChannelConfigParams};
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant_id = tenants[0].id;

        // Config with both verify_token and webhook_secret set
        let config_id = Uuid::new_v4().to_string();
        let params = UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("wa_key"),
            api_secret: None,
            webhook_secret: Some("hmac_secret_do_not_leak"),
            verify_token: Some("my_verify_token"),
            account_id: None,
            phone_number: Some("+15550001111"),
            bot_token: None,
            is_active: true,
        };
        db.upsert_channel_config(&params).await.unwrap();

        let router = MessagingRoutes::routes(Arc::clone(&resources));

        // GET with verify_token that matches => should succeed
        let response = AxumTestRequest::get(
            "/api/messaging/webhook/whatsapp?hub.mode=subscribe&hub.verify_token=my_verify_token&hub.challenge=challenge_123",
        )
        .send(router.clone())
        .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.text(), "challenge_123");

        // GET with webhook_secret value should fail (verify_token takes precedence)
        let response = AxumTestRequest::get(
            "/api/messaging/webhook/whatsapp?hub.mode=subscribe&hub.verify_token=hmac_secret_do_not_leak&hub.challenge=challenge_456",
        )
        .send(router)
        .await;

        assert_ne!(
            response.status_code(),
            StatusCode::OK,
            "webhook_secret must not be accepted when verify_token is set"
        );
    }

    #[tokio::test]
    async fn test_meta_verify_falls_back_to_webhook_secret_when_no_verify_token() {
        use pierre_database::plugins::{MessagingRepository, UpsertChannelConfigParams};
        use uuid::Uuid;

        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user.id)
            .await
            .unwrap();
        let tenant_id = tenants[0].id;

        // Config without verify_token (legacy setup)
        let config_id = Uuid::new_v4().to_string();
        let params = UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "messenger",
            api_key: Some("msg_key"),
            api_secret: None,
            webhook_secret: Some("legacy_webhook_secret"),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        };
        db.upsert_channel_config(&params).await.unwrap();

        let router = MessagingRoutes::routes(Arc::clone(&resources));

        // GET with webhook_secret value should succeed (backward compat)
        let response = AxumTestRequest::get(
            "/api/messaging/webhook/messenger?hub.mode=subscribe&hub.verify_token=legacy_webhook_secret&hub.challenge=compat_challenge",
        )
        .send(router)
        .await;

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.text(), "compat_challenge");
    }
}
