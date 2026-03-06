// ABOUTME: End-to-end integration tests for the full messaging channel linking flow
// ABOUTME: Chains: configure channel → webhook (unlinked) → link state → auth → webhook (linked)
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
mod messaging_e2e_tests {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use chrono::{Duration, Utc};
    use hmac::{Hmac, Mac};
    use pierre_database::plugins::{
        CreateLinkStateParams, MessagingRepository, TenantRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerResources;
    use pierre_mcp_server::models::{Tenant, TenantId, User};
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::json;
    use sha2::Sha256;
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    // ════════════════════════════════════════════════════════════════
    // Helpers
    // ════════════════════════════════════════════════════════════════

    /// Compute HMAC-SHA256 and return hex digest
    fn hmac_sha256_hex(secret: &str, data: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(data);
        hex::encode(mac.finalize().into_bytes())
    }

    /// Compute Slack HMAC-SHA256 signature (v0:{timestamp}:{body})
    fn compute_slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        format!("v0={}", hmac_sha256_hex(secret, basestring.as_bytes()))
    }

    /// Compute Messenger HMAC-SHA256 signature (sha256={hex})
    fn compute_messenger_sig(secret: &str, body: &[u8]) -> String {
        format!("sha256={}", hmac_sha256_hex(secret, body))
    }

    /// Compute `WhatsApp` HMAC-SHA256 signature (`sha256={hex}`) — same as Messenger
    fn compute_whatsapp_sig(secret: &str, body: &[u8]) -> String {
        format!("sha256={}", hmac_sha256_hex(secret, body))
    }

    /// Create a test user with bcrypt-hashed password and `UserStatus::Active`
    async fn create_e2e_user(
        resources: &ServerResources,
        email: &str,
        password: &str,
    ) -> (Uuid, TenantId) {
        use pierre_database::plugins::UserRepository;
        use pierre_mcp_server::models::UserStatus;

        let password_owned = password.to_owned();
        let password_hash =
            spawn_blocking(move || bcrypt::hash(&password_owned, bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(email.to_owned(), password_hash, Some("E2E User".to_owned()));
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());

        let user_id = user.id;
        let user_repo: &dyn UserRepository = &*resources.database;
        user_repo.create(&user).await.unwrap();

        // Create a tenant with this user as owner
        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("E2E Tenant {email}"),
            slug: format!("e2e-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let tenant_repo: &dyn TenantRepository = &*resources.database;
        tenant_repo.create(&tenant).await.unwrap();

        (user_id, tenant_id)
    }

    /// Create a webhook-initiated link state (no `user_id`) and return the code
    async fn create_e2e_link_state(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
        sender_name: Option<&str>,
    ) -> String {
        let code = Uuid::new_v4().to_string();
        let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

        let params = CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: None,
            channel_type,
            code: &code,
            method: "channel_initiated",
            channel_user_id: Some(channel_user_id),
            sender_name,
            expires_at: &expires_at,
        };
        db.create_link_state(&params).await.unwrap();
        code
    }

    // ════════════════════════════════════════════════════════════════
    // E2E: Telegram — config → unlinked webhook → link state → auth → linked webhook
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_telegram_e2e_full_flow() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (user_id, tenant_id) =
            create_e2e_user(&resources, "tg_e2e@example.com", "TgPass123!").await;
        let _ = user_id;

        // Step 1: Configure Telegram channel
        let tg_secret = "tg_e2e_webhook_secret";
        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(tg_secret),
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:E2E_BOT_TOKEN"),
            is_active: true,
        })
        .await
        .unwrap();

        let sender_id = "42";

        // Step 2: Send webhook from unlinked user → accepted (200 OK)
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", tg_secret)
            .json(&json!({
                "update_id": 1001,
                "message": {
                    "message_id": 1,
                    "from": { "id": 42, "first_name": "E2eAlice" },
                    "chat": { "id": 42 },
                    "text": "Hello Pierre"
                }
            }))
            .send(router)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        let body: serde_json::Value = resp.json();
        assert_eq!(body["status"].as_str(), Some("ok"));
        assert_eq!(body["messages_received"].as_u64(), Some(1));

        // Step 3: Create link state (simulating what create_link_and_prompt does in background)
        let code =
            create_e2e_link_state(db, tenant_id, "telegram", sender_id, Some("E2eAlice")).await;

        // Step 4: GET the login page
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::get(&format!("/messaging/link/{code}"))
            .send(router)
            .await;

        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(
            html.contains("telegram"),
            "Page should mention channel type"
        );
        assert!(
            html.contains(&code),
            "Page should include the code in a hidden field"
        );

        // Step 5: POST login credentials
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let form_data = [
            ("code", code.as_str()),
            ("email", "tg_e2e@example.com"),
            ("password", "TgPass123!"),
            ("action", "login"),
        ];

        let resp = AxumTestRequest::post("/messaging/link/auth")
            .form(&form_data)
            .send(router)
            .await;

        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(
            html.contains("Linked") || html.contains("linked") || html.contains("success"),
            "Should show success page, got: {}",
            &html[..html.len().min(500)]
        );

        // Step 6: Verify channel link was created in DB
        let link = db
            .get_channel_link(tenant_id, "telegram", sender_id)
            .await
            .unwrap();
        assert!(link.is_some(), "Channel link should exist after auth");

        // Step 7: Send webhook from now-linked user → webhook accepted
        // Note: Session creation may fail in test env (requires PIERRE_LLM_MODEL for
        // create_conversation). We verify the webhook accepts the message and the
        // channel link resolves correctly — full LLM pipeline is tested separately.
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", tg_secret)
            .json(&json!({
                "update_id": 1002,
                "message": {
                    "message_id": 2,
                    "from": { "id": 42, "first_name": "E2eAlice" },
                    "chat": { "id": 42 },
                    "text": "What was my last run?"
                }
            }))
            .send(router)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        let body: serde_json::Value = resp.json();
        assert_eq!(body["status"].as_str(), Some("ok"));
        assert_eq!(
            body["messages_received"].as_u64(),
            Some(1),
            "Linked user's message should be received"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // E2E: Slack — challenge handshake + unlinked message + auth flow
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_slack_e2e_full_flow() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "slack_e2e@example.com", "SlackPass123!").await;

        // Step 1: Configure Slack channel
        let signing_secret = "slack_e2e_signing_secret_99";
        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-e2e-slack-token"),
            api_secret: None,
            webhook_secret: Some(signing_secret),
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        // Step 2: Verify Slack url_verification challenge handshake
        let challenge_body = json!({
            "type": "url_verification",
            "challenge": "e2e_challenge_abc123"
        })
        .to_string();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let sig = compute_slack_sig(signing_secret, &timestamp, &challenge_body);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &sig)
            .json(&json!({
                "type": "url_verification",
                "challenge": "e2e_challenge_abc123"
            }))
            .send(router)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        let body: serde_json::Value = resp.json();
        assert_eq!(
            body["challenge"].as_str(),
            Some("e2e_challenge_abc123"),
            "Slack challenge should echo the challenge value"
        );

        // Step 3: Send a message from an unlinked Slack user
        let slack_msg_body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": "U_E2E_SLACK",
                "text": "Hello from Slack",
                "channel": "C_GENERAL",
                "ts": "1234567890.123456"
            }
        })
        .to_string();
        let timestamp2 = chrono::Utc::now().timestamp().to_string();
        let sig2 = compute_slack_sig(signing_secret, &timestamp2, &slack_msg_body);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp2)
            .header("x-slack-signature", &sig2)
            .json(&json!({
                "type": "event_callback",
                "event": {
                    "type": "message",
                    "user": "U_E2E_SLACK",
                    "text": "Hello from Slack",
                    "channel": "C_GENERAL",
                    "ts": "1234567890.123456"
                }
            }))
            .send(router)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        let body: serde_json::Value = resp.json();
        assert_eq!(body["status"].as_str(), Some("ok"));

        // Step 4: Create link state and authenticate via form
        let sender_id = "U_E2E_SLACK";
        let code =
            create_e2e_link_state(db, tenant_id, "slack", sender_id, Some("SlackUser")).await;

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::get(&format!("/messaging/link/{code}"))
            .send(router)
            .await;
        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(html.contains("slack"), "Page should mention Slack");

        // Step 5: Login
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let form_data = [
            ("code", code.as_str()),
            ("email", "slack_e2e@example.com"),
            ("password", "SlackPass123!"),
            ("action", "login"),
        ];
        let resp = AxumTestRequest::post("/messaging/link/auth")
            .form(&form_data)
            .send(router)
            .await;

        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(
            html.contains("Linked") || html.contains("linked") || html.contains("success"),
            "Slack auth should succeed, got: {}",
            &html[..html.len().min(500)]
        );

        // Step 6: Verify channel link exists
        let link = db
            .get_channel_link(tenant_id, "slack", sender_id)
            .await
            .unwrap();
        assert!(link.is_some(), "Slack channel link should exist");
    }

    // ════════════════════════════════════════════════════════════════
    // E2E: WhatsApp (Meta Cloud API) — unlinked webhook → register → linked webhook
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_whatsapp_e2e_register_flow() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "wa_owner@example.com", "WaOwner123!").await;

        // Step 1: Configure WhatsApp channel (Meta Cloud API fields)
        let wa_secret = "whatsapp_e2e_app_secret";
        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("whatsapp_access_token_e2e"),
            api_secret: None,
            webhook_secret: Some(wa_secret),
            account_id: Some("whatsapp_business_account_id_e2e"),
            phone_number: Some("123456789012345"),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        // Step 2: Send webhook from unlinked WhatsApp user (JSON, Meta HMAC-SHA256)
        let wa_body = json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "whatsapp_business_account_id_e2e",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+15551234567",
                            "phone_number_id": "123456789012345"
                        },
                        "messages": [{
                            "from": "15559876543",
                            "id": "wamid.e2e_msg_001",
                            "timestamp": "1234567890",
                            "type": "text",
                            "text": { "body": "Hi Pierre!" }
                        }]
                    },
                    "field": "messages"
                }]
            }]
        });
        let wa_body_bytes = serde_json::to_vec(&wa_body).unwrap();
        let wa_sig = compute_whatsapp_sig(wa_secret, &wa_body_bytes);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &wa_sig)
            .json(&wa_body)
            .send(router)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        let body: serde_json::Value = resp.json();
        assert_eq!(body["status"].as_str(), Some("ok"));

        // Step 3: Create link state and register a new user
        let sender_id = "15559876543";
        let code =
            create_e2e_link_state(db, tenant_id, "whatsapp", sender_id, Some("WhatsAppUser")).await;

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let form_data = [
            ("code", code.as_str()),
            ("email", "wa_new_user@example.com"),
            ("password", "NewUser123!"),
            ("action", "register"),
            ("display_name", "WhatsApp E2E User"),
        ];

        let resp = AxumTestRequest::post("/messaging/link/auth")
            .form(&form_data)
            .send(router)
            .await;

        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(
            html.contains("Linked") || html.contains("linked") || html.contains("success"),
            "Registration should succeed, got: {}",
            &html[..html.len().min(500)]
        );

        // Step 4: Verify channel link was created
        let link = db
            .get_channel_link(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            link.is_some(),
            "WhatsApp channel link should exist after registration"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // E2E: Messenger — unlinked webhook with sha256= signature
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_messenger_e2e_webhook_and_auth() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "fb_e2e@example.com", "FbPass123!").await;

        // Step 1: Configure Messenger channel
        let app_secret = "messenger_e2e_app_secret";
        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "messenger",
            api_key: Some("messenger_page_access_token_e2e"),
            api_secret: Some(app_secret),
            webhook_secret: Some(app_secret),
            account_id: Some("messenger_page_id_e2e"),
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        // Step 2: Send webhook from unlinked Messenger user
        let fb_body = json!({
            "object": "page",
            "entry": [{
                "id": "messenger_page_id_e2e",
                "time": 1_234_567_890,
                "messaging": [{
                    "sender": { "id": "fb_e2e_user_99" },
                    "recipient": { "id": "messenger_page_id_e2e" },
                    "timestamp": 1_234_567_890,
                    "message": {
                        "mid": "mid.e2e_messenger_001",
                        "text": "Hello from Messenger"
                    }
                }]
            }]
        });
        let fb_body_bytes = serde_json::to_vec(&fb_body).unwrap();
        let fb_sig = compute_messenger_sig(app_secret, &fb_body_bytes);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/messenger")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &fb_sig)
            .json(&fb_body)
            .send(router)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        let body: serde_json::Value = resp.json();
        assert_eq!(body["status"].as_str(), Some("ok"));
        assert_eq!(body["messages_received"].as_u64(), Some(1));

        // Step 3: Create link state and authenticate
        let sender_id = "fb_e2e_user_99";
        let code =
            create_e2e_link_state(db, tenant_id, "messenger", sender_id, Some("FbUser")).await;

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let form_data = [
            ("code", code.as_str()),
            ("email", "fb_e2e@example.com"),
            ("password", "FbPass123!"),
            ("action", "login"),
        ];
        let resp = AxumTestRequest::post("/messaging/link/auth")
            .form(&form_data)
            .send(router)
            .await;

        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(
            html.contains("Linked") || html.contains("linked") || html.contains("success"),
            "Messenger auth should succeed, got: {}",
            &html[..html.len().min(500)]
        );

        // Step 4: Verify channel link
        let link = db
            .get_channel_link(tenant_id, "messenger", sender_id)
            .await
            .unwrap();
        assert!(link.is_some(), "Messenger channel link should exist");
    }

    // ════════════════════════════════════════════════════════════════
    // Security: Invalid signatures rejected across all channels
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_invalid_signature_rejected_telegram() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "tg_sig@example.com", "Pass123!").await;

        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some("correct_secret"),
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:BOT"),
            is_active: true,
        })
        .await
        .unwrap();

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", "wrong_secret")
            .json(&json!({
                "update_id": 999,
                "message": {
                    "message_id": 1,
                    "from": { "id": 1 },
                    "chat": { "id": 1 },
                    "text": "attack"
                }
            }))
            .send(router)
            .await;

        assert_ne!(
            resp.status_code(),
            StatusCode::OK,
            "Wrong Telegram secret should be rejected"
        );
    }

    #[tokio::test]
    async fn test_invalid_signature_rejected_slack() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "slack_sig@example.com", "Pass123!").await;

        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-test"),
            api_secret: None,
            webhook_secret: Some("correct_slack_secret"),
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        let body_str = json!({
            "type": "event_callback",
            "event": {"type": "message", "user": "U1", "text": "hi", "channel": "C1", "ts": "1.1"}
        })
        .to_string();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let bad_sig = compute_slack_sig("wrong_secret", &timestamp, &body_str);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &bad_sig)
            .json(&json!({
                "type": "event_callback",
                "event": {"type": "message", "user": "U1", "text": "hi", "channel": "C1", "ts": "1.1"}
            }))
            .send(router)
            .await;

        assert_ne!(
            resp.status_code(),
            StatusCode::OK,
            "Wrong Slack signature should be rejected"
        );
    }

    #[tokio::test]
    async fn test_invalid_signature_rejected_whatsapp() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "wa_sig@example.com", "Pass123!").await;

        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("whatsapp_access_token"),
            api_secret: None,
            webhook_secret: Some("correct_wa_secret"),
            account_id: Some("wa_business_id"),
            phone_number: Some("123456789012345"),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        let wa_body = json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "15551999999",
                            "id": "wamid.bad",
                            "type": "text",
                            "text": { "body": "hack" }
                        }]
                    }
                }]
            }]
        });
        let wa_body_bytes = serde_json::to_vec(&wa_body).unwrap();
        let bad_sig = compute_whatsapp_sig("wrong_wa_secret", &wa_body_bytes);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &bad_sig)
            .json(&wa_body)
            .send(router)
            .await;

        assert_ne!(
            resp.status_code(),
            StatusCode::OK,
            "Wrong WhatsApp signature should be rejected"
        );
    }

    #[tokio::test]
    async fn test_invalid_signature_rejected_messenger() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "fb_sig@example.com", "Pass123!").await;

        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "messenger",
            api_key: Some("page_token"),
            api_secret: Some("correct_app_secret"),
            webhook_secret: Some("correct_app_secret"),
            account_id: Some("page_id"),
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        let fb_body = json!({
            "object": "page",
            "entry": [{"id": "page_id", "messaging": [{"sender": {"id": "1"}, "recipient": {"id": "page_id"}, "message": {"mid": "m1", "text": "hack"}}]}]
        });
        let fb_body_bytes = serde_json::to_vec(&fb_body).unwrap();
        let bad_sig = compute_messenger_sig("wrong_app_secret", &fb_body_bytes);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/messenger")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &bad_sig)
            .json(&fb_body)
            .send(router)
            .await;

        assert_ne!(
            resp.status_code(),
            StatusCode::OK,
            "Wrong Messenger signature should be rejected"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Security: Consumed code cannot be reused
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_e2e_consumed_code_rejected() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "reuse@example.com", "ReusePass123!").await;

        let code =
            create_e2e_link_state(db, tenant_id, "telegram", "tg-reuse", Some("Reuser")).await;

        // First auth succeeds
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let form_data = [
            ("code", code.as_str()),
            ("email", "reuse@example.com"),
            ("password", "ReusePass123!"),
            ("action", "login"),
        ];
        let resp = AxumTestRequest::post("/messaging/link/auth")
            .form(&form_data)
            .send(router)
            .await;
        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(
            html.contains("Linked") || html.contains("linked") || html.contains("success"),
            "First auth should succeed, got: {}",
            &html[..html.len().min(500)]
        );

        // Second auth with same code should fail
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let form_data2 = [
            ("code", code.as_str()),
            ("email", "reuse@example.com"),
            ("password", "ReusePass123!"),
            ("action", "login"),
        ];
        let resp2 = AxumTestRequest::post("/messaging/link/auth")
            .form(&form_data2)
            .send(router)
            .await;
        assert_eq!(resp2.status(), 200);
        let html2 = resp2.text();
        assert!(
            html2.contains("expired")
                || html2.contains("invalid")
                || html2.contains("already been used"),
            "Second auth should fail, got: {}",
            &html2[..html2.len().min(500)]
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Security: Multi-tenant webhook isolation
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_e2e_cross_tenant_webhook_isolation() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_a, tenant_a) =
            create_e2e_user(&resources, "tenant_a@example.com", "Pass123!").await;

        // Configure Telegram for tenant A
        let secret_a = "tenant_a_tg_secret";
        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id: tenant_a,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(secret_a),
            account_id: None,
            phone_number: None,
            bot_token: Some("A:BOT"),
            is_active: true,
        })
        .await
        .unwrap();

        // Webhook with tenant A's secret should succeed
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", secret_a)
            .json(&json!({
                "update_id": 5001,
                "message": {
                    "message_id": 1,
                    "from": { "id": 111, "first_name": "TenantAUser" },
                    "chat": { "id": 111 },
                    "text": "Message to tenant A"
                }
            }))
            .send(router)
            .await;

        assert_eq!(
            resp.status_code(),
            StatusCode::OK,
            "Tenant A's secret should be accepted"
        );

        // Webhook with unknown secret (no matching config) should fail
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", "unknown_secret")
            .json(&json!({
                "update_id": 5002,
                "message": {
                    "message_id": 2,
                    "from": { "id": 222, "first_name": "UnknownUser" },
                    "chat": { "id": 222 },
                    "text": "Impersonation attempt"
                }
            }))
            .send(router)
            .await;

        assert_ne!(
            resp.status_code(),
            StatusCode::OK,
            "Unknown secret should be rejected (no matching tenant config)"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Edge case: Unconfigured channel
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_e2e_unconfigured_channel_rejected() {
        let resources = create_test_server_resources().await.unwrap();

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", "anything")
            .json(&json!({
                "update_id": 1,
                "message": {
                    "message_id": 1,
                    "from": { "id": 1 },
                    "chat": { "id": 1 },
                    "text": "hello"
                }
            }))
            .send(router)
            .await;

        assert_ne!(
            resp.status_code(),
            StatusCode::OK,
            "Unconfigured channel should reject webhooks"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Edge case: Expired link code
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_meta_webhook_verification_whatsapp() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "wa_verify@example.com", "WaVerify123!").await;

        // Configure WhatsApp channel
        let wa_secret = "my_verify_token_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("wa_access_token"),
            api_secret: None,
            webhook_secret: Some(wa_secret),
            account_id: None,
            phone_number: Some("999888777"),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        let router = MessagingRoutes::routes(Arc::clone(&resources));

        // Valid verification: should echo challenge
        let resp = AxumTestRequest::get(
            "/api/messaging/webhook/whatsapp?hub.mode=subscribe&hub.verify_token=my_verify_token_secret&hub.challenge=challenge_string_12345",
        )
        .send(router)
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.text(), "challenge_string_12345");

        // Wrong verify token: should be rejected
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::get(
            "/api/messaging/webhook/whatsapp?hub.mode=subscribe&hub.verify_token=wrong_token&hub.challenge=challenge_abc",
        )
        .send(router)
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Wrong mode: should be rejected
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::get(
            "/api/messaging/webhook/whatsapp?hub.mode=unsubscribe&hub.verify_token=my_verify_token_secret&hub.challenge=challenge_abc",
        )
        .send(router)
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_meta_webhook_verification_messenger() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "msg_verify@example.com", "MsgVerify123!").await;

        // Configure Messenger channel
        let msg_secret = "messenger_app_secret_verify";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "messenger",
            api_key: Some("page_access_token"),
            api_secret: None,
            webhook_secret: Some(msg_secret),
            account_id: Some("page_id_123"),
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        let router = MessagingRoutes::routes(Arc::clone(&resources));

        // Valid verification
        let resp = AxumTestRequest::get(
            "/api/messaging/webhook/messenger?hub.mode=subscribe&hub.verify_token=messenger_app_secret_verify&hub.challenge=meta_challenge_789",
        )
        .send(router)
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.text(), "meta_challenge_789");
    }

    #[tokio::test]
    async fn test_e2e_expired_link_code_rejected() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;

        let (_user_id, tenant_id) =
            create_e2e_user(&resources, "expired@example.com", "ExpPass123!").await;

        // Create an already-expired link state
        let code = Uuid::new_v4().to_string();
        let expires_at = (Utc::now() - Duration::minutes(1)).to_rfc3339();
        let params = CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: None,
            channel_type: "telegram",
            code: &code,
            method: "channel_initiated",
            channel_user_id: Some("tg-expired"),
            sender_name: None,
            expires_at: &expires_at,
        };
        db.create_link_state(&params).await.unwrap();

        // GET link page should show error
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::get(&format!("/messaging/link/{code}"))
            .send(router)
            .await;
        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(
            html.contains("expired") || html.contains("invalid"),
            "Expired code page should show error, got: {}",
            &html[..html.len().min(500)]
        );

        // POST auth should also show error
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let form_data = [
            ("code", code.as_str()),
            ("email", "expired@example.com"),
            ("password", "ExpPass123!"),
            ("action", "login"),
        ];
        let resp = AxumTestRequest::post("/messaging/link/auth")
            .form(&form_data)
            .send(router)
            .await;
        assert_eq!(resp.status(), 200);
        let html = resp.text();
        assert!(
            html.contains("expired") || html.contains("invalid"),
            "Expired code auth should fail, got: {}",
            &html[..html.len().min(500)]
        );
    }
}
