// ABOUTME: Integration tests for the in-chat OTP account linking flow in messaging webhooks
// ABOUTME: Covers email validation, OTP verification, cancel, max attempts, and fallback behavior
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
mod messaging_otp_linking_tests {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use chrono::{Duration, Utc};
    use hmac::{Hmac, Mac};
    use pierre_database::plugins::{
        CreateLinkStateParams, MessagingRepository, TenantRepository, UpsertChannelConfigParams,
        UserRepository,
    };
    use pierre_mcp_server::email::ResendEmailService;
    use pierre_mcp_server::mcp::resources::ServerResources;
    use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
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

    /// Compute WhatsApp HMAC-SHA256 signature (`sha256={hex}`)
    fn compute_whatsapp_sig(secret: &str, body: &[u8]) -> String {
        format!("sha256={}", hmac_sha256_hex(secret, body))
    }

    /// SHA-256 hash of an OTP code (matches the hash_otp() function in webhooks.rs)
    fn hash_otp(code: &str) -> String {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(code.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Create a test user with bcrypt-hashed password and `UserStatus::Active`
    async fn create_otp_test_user(
        resources: &ServerResources,
        email: &str,
        password: &str,
    ) -> (Uuid, TenantId) {
        let password_owned = password.to_owned();
        let password_hash =
            spawn_blocking(move || bcrypt::hash(&password_owned, bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(email.to_owned(), password_hash, Some("OTP User".to_owned()));
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());

        let user_id = user.id;
        let user_repo: &dyn UserRepository = &*resources.database;
        user_repo.create(&user).await.unwrap();

        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("OTP Tenant {email}"),
            slug: format!("otp-tenant-{tenant_id}"),
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

    /// Set up a WhatsApp channel config with a known signing secret
    async fn setup_whatsapp_config(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        secret: &str,
    ) {
        let config_id = Uuid::new_v4().to_string();
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &config_id,
            tenant_id,
            channel_type: "whatsapp",
            api_key: Some("wa_otp_test_access_token"),
            api_secret: None,
            webhook_secret: Some(secret),
            verify_token: None,
            account_id: Some("wa_otp_test_business_id"),
            phone_number: Some("15550000001"),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
    }

    /// Build a WhatsApp Cloud API webhook payload for a text message
    fn whatsapp_text_payload(sender_id: &str, msg_id: &str, text: &str) -> serde_json::Value {
        json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "wa_otp_test_business_id",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+15550000001",
                            "phone_number_id": "15550000001"
                        },
                        "messages": [{
                            "from": sender_id,
                            "id": msg_id,
                            "timestamp": "1234567890",
                            "type": "text",
                            "text": { "body": text }
                        }]
                    },
                    "field": "messages"
                }]
            }]
        })
    }

    /// Send a WhatsApp webhook and return the response body as JSON
    async fn send_whatsapp_webhook(
        resources: &Arc<ServerResources>,
        secret: &str,
        payload: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let body_bytes = serde_json::to_vec(payload).unwrap();
        let sig = compute_whatsapp_sig(secret, &body_bytes);
        let router = MessagingRoutes::routes(Arc::clone(resources));

        let resp = AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &sig)
            .json(payload)
            .send(router)
            .await;

        let status = resp.status_code();
        let body: serde_json::Value = resp.json();
        (status, body)
    }

    /// Create an OTP link state in the DB (simulates what start_otp_flow does)
    ///
    /// Creates a link state with `method=otp`, then calls `set_otp_on_link_state`
    /// with an empty email and empty hash to set `otp_step='awaiting_otp'`.
    /// This represents the "awaiting email" sub-state.
    async fn create_otp_link_state_awaiting_email(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        channel_type: &str,
        channel_user_id: &str,
    ) -> String {
        let state_id = Uuid::new_v4().to_string();
        let code = Uuid::new_v4().to_string();
        let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

        let params = CreateLinkStateParams {
            id: &state_id,
            tenant_id,
            user_id: None,
            channel_type,
            code: &code,
            method: "otp",
            channel_user_id: Some(channel_user_id),
            sender_name: Some("OtpTestUser"),
            expires_at: &expires_at,
        };
        db.create_link_state(&params).await.unwrap();

        // Set otp_step to 'awaiting_otp' with empty email (= awaiting email input)
        db.set_otp_on_link_state(&state_id, "", "").await.unwrap();

        state_id
    }

    /// Advance an OTP link state to the "awaiting OTP code" sub-state
    ///
    /// Sets the email and OTP hash on the link state. The OTP code is returned
    /// so tests can verify it or submit the wrong one.
    async fn advance_to_awaiting_otp_code(
        db: &dyn MessagingRepository,
        state_id: &str,
        email: &str,
        otp_code: &str,
    ) {
        let otp_hashed = hash_otp(otp_code);
        db.set_otp_on_link_state(state_id, email, &otp_hashed)
            .await
            .unwrap();
    }

    /// Inject a dummy email service into ServerResources so the OTP flow
    /// triggers (instead of falling back to link-URL).
    ///
    /// The email service uses a fake API key, so actual sends will fail.
    /// This is intentional: we set up OTP state directly in the DB for most tests.
    fn inject_dummy_email_service(resources: &mut Arc<ServerResources>) {
        let inner = Arc::get_mut(resources).expect(
            "Cannot get mutable reference to ServerResources — \
             ensure no clones exist before calling inject_dummy_email_service",
        );
        inner.email_service = Some(Arc::new(ResendEmailService::new(
            "re_fake_test_api_key".to_owned(),
            "Pierre <noreply@test.pierre.dev>".to_owned(),
        )));
    }

    // ════════════════════════════════════════════════════════════════
    // DB Repository: OTP link state CRUD
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_db_otp_link_state_lifecycle() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) =
            create_otp_test_user(&resources, "db_otp@example.com", "DbOtp123!").await;

        let sender_id = "15551111111";
        let channel_type = "whatsapp";

        // Initially: no active OTP state
        let state = db
            .get_active_otp_link_state(tenant_id, channel_type, sender_id)
            .await
            .unwrap();
        assert!(
            state.is_none(),
            "No active OTP state should exist initially"
        );

        // Create OTP link state (awaiting email)
        let state_id =
            create_otp_link_state_awaiting_email(db, tenant_id, channel_type, sender_id).await;

        // Should now be retrievable
        let state = db
            .get_active_otp_link_state(tenant_id, channel_type, sender_id)
            .await
            .unwrap();
        assert!(state.is_some(), "OTP state should be active after creation");
        let state_val = state.unwrap();
        assert_eq!(state_val["otp_step"].as_str(), Some("awaiting_otp"));
        assert_eq!(state_val["email"].as_str().unwrap_or(""), "");
        assert_eq!(state_val["id"].as_str(), Some(state_id.as_str()));

        // Advance to awaiting OTP code
        let otp_code = "123456";
        advance_to_awaiting_otp_code(db, &state_id, "user@example.com", otp_code).await;

        let state = db
            .get_active_otp_link_state(tenant_id, channel_type, sender_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state["email"].as_str(),
            Some("user@example.com"),
            "Email should be set after advancing to OTP code step"
        );
        assert!(
            state["otp_hash"].as_str().is_some_and(|h| !h.is_empty()),
            "OTP hash should be set"
        );
        assert_eq!(state["otp_attempts"].as_i64(), Some(0));

        // Increment attempts
        let count = db.increment_otp_attempts(&state_id).await.unwrap();
        assert_eq!(count, 1, "First attempt should return count 1");

        let count = db.increment_otp_attempts(&state_id).await.unwrap();
        assert_eq!(count, 2, "Second attempt should return count 2");

        // Invalidate
        db.invalidate_otp_link_states(tenant_id, channel_type, sender_id)
            .await
            .unwrap();

        let state = db
            .get_active_otp_link_state(tenant_id, channel_type, sender_id)
            .await
            .unwrap();
        assert!(
            state.is_none(),
            "OTP state should be gone after invalidation"
        );
    }

    #[tokio::test]
    async fn test_db_expired_otp_link_state_not_returned() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) =
            create_otp_test_user(&resources, "db_expired@example.com", "Expired123!").await;

        let sender_id = "15552222222";
        let state_id = Uuid::new_v4().to_string();
        let code = Uuid::new_v4().to_string();
        // Expired 5 minutes ago
        let expires_at = (Utc::now() - Duration::minutes(5)).to_rfc3339();

        let params = CreateLinkStateParams {
            id: &state_id,
            tenant_id,
            user_id: None,
            channel_type: "whatsapp",
            code: &code,
            method: "otp",
            channel_user_id: Some(sender_id),
            sender_name: None,
            expires_at: &expires_at,
        };
        db.create_link_state(&params).await.unwrap();
        db.set_otp_on_link_state(&state_id, "", "").await.unwrap();

        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(state.is_none(), "Expired OTP state should not be returned");
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Unlinked user without email service → link URL
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_unlinked_user_no_email_service_gets_link_url() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) =
            create_otp_test_user(&resources, "nomail@example.com", "NoMail123!").await;

        let wa_secret = "wa_nomail_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        // email_service is None in test resources, so unlinked user should get link URL
        let payload = whatsapp_text_payload("15553333333", "wamid.nomail_001", "Hello Pierre");
        let (status, body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"].as_str(), Some("ok"));
        assert_eq!(
            body["messages_received"].as_u64(),
            Some(1),
            "Webhook should accept the message"
        );

        // The link URL is sent as a background outbound message, not in the webhook response.
        // The webhook handler returns 200 OK with status info.
        // We verify no OTP state was created (since email_service is None, link-URL flow is used)
        let otp_state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", "15553333333")
            .await
            .unwrap();
        assert!(
            otp_state.is_none(),
            "No OTP state should be created when email service is not configured"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Unlinked user with email service → OTP flow starts
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_unlinked_user_with_email_service_starts_otp_flow() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) =
            create_otp_test_user(&resources, "withmail@example.com", "WithMail123!").await;

        let wa_secret = "wa_withmail_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15554444444";
        let payload = whatsapp_text_payload(sender_id, "wamid.withmail_001", "Hello Pierre");
        let (status, body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"].as_str(), Some("ok"));

        // OTP state should have been created by start_otp_flow
        let otp_state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            otp_state.is_some(),
            "OTP state should be created when email service is configured"
        );
        let state_val = otp_state.unwrap();
        assert_eq!(state_val["otp_step"].as_str(), Some("awaiting_otp"));
        // Email should be empty (awaiting email input)
        assert_eq!(
            state_val["email"].as_str().unwrap_or(""),
            "",
            "Email should be empty in the awaiting-email sub-state"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Invalid email format during OTP flow
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_invalid_email_format() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) =
            create_otp_test_user(&resources, "bademail_owner@example.com", "BadEmail123!").await;

        let wa_secret = "wa_bademail_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15555555555";

        // Pre-create OTP state in "awaiting email" sub-state
        create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;

        // Send garbage as email
        let payload = whatsapp_text_payload(sender_id, "wamid.bademail_001", "not-an-email");
        let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // OTP state should still be active (email validation failed, flow continues)
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_some(),
            "OTP state should remain active after invalid email"
        );
        let state_val = state.unwrap();
        assert_eq!(
            state_val["email"].as_str().unwrap_or(""),
            "",
            "Email should still be empty after invalid email input"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Email not found during OTP flow
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_email_not_found() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) = create_otp_test_user(
            &resources,
            "emailnotfound_owner@example.com",
            "NotFound123!",
        )
        .await;

        let wa_secret = "wa_notfound_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15556666666";

        // Pre-create OTP state in "awaiting email" sub-state
        create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;

        // Send a valid-looking email that does not match any user
        let payload =
            whatsapp_text_payload(sender_id, "wamid.notfound_001", "nobody@doesnotexist.com");
        let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // OTP state should still be active (user not found, but flow is not cancelled)
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_some(),
            "OTP state should remain active after email-not-found (user can retry)"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Valid email but email send fails → error message
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_email_send_failure() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let email = "send_fail@example.com";
        let (_, tenant_id) = create_otp_test_user(&resources, email, "SendFail123!").await;

        let wa_secret = "wa_sendfail_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15557777777";

        // Pre-create OTP state in "awaiting email" sub-state
        create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;

        // Send the valid email (the Resend API call will fail with a fake key)
        let payload = whatsapp_text_payload(sender_id, "wamid.sendfail_001", email);
        let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // The email and OTP hash were set on the link state BEFORE the email send attempt
        // (generate_and_send_otp sets the DB first, then sends the email)
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_some(),
            "OTP state should still exist after email send failure"
        );
        let state_val = state.unwrap();
        // The email should have been set on the state (set_otp_on_link_state happens before send)
        assert_eq!(
            state_val["email"].as_str().unwrap_or(""),
            email,
            "Email should be stored even when send fails"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Wrong OTP code → attempt counter increments
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_wrong_code() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let email = "wrongcode@example.com";
        let (_, tenant_id) = create_otp_test_user(&resources, email, "WrongCode123!").await;

        let wa_secret = "wa_wrongcode_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15558888888";

        // Set up OTP state in "awaiting OTP code" sub-state with known OTP
        let state_id =
            create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;
        advance_to_awaiting_otp_code(db, &state_id, email, "654321").await;

        // Send wrong 6-digit code
        let payload = whatsapp_text_payload(sender_id, "wamid.wrong_001", "111111");
        let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // OTP state should still be active with incremented attempts
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_some(),
            "OTP state should remain active after 1 wrong attempt"
        );
        let state_val = state.unwrap();
        assert_eq!(
            state_val["otp_attempts"].as_i64(),
            Some(1),
            "Attempt count should be 1 after one wrong code"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Max OTP attempts exceeded → flow invalidated
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_max_attempts_exceeded() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let email = "maxattempts@example.com";
        let (_, tenant_id) = create_otp_test_user(&resources, email, "MaxAttempts123!").await;

        let wa_secret = "wa_maxattempts_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15559999999";

        // Set up OTP state with known OTP
        let state_id =
            create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;
        advance_to_awaiting_otp_code(db, &state_id, email, "654321").await;

        // Send 3 wrong codes (MAX_OTP_ATTEMPTS = 3)
        for i in 1..=3 {
            let msg_id = format!("wamid.max_{i}");
            let wrong_code = format!("{:06}", 100_000 + i);
            let payload = whatsapp_text_payload(sender_id, &msg_id, &wrong_code);
            let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
            assert_eq!(status, StatusCode::OK, "Webhook should accept attempt {i}");
        }

        // OTP state should be invalidated after 3 wrong attempts
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_none(),
            "OTP state should be invalidated after max attempts"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Non-numeric input during OTP step → no attempt counted
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_non_numeric_input_no_attempt_counted() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let email = "nonnumeric@example.com";
        let (_, tenant_id) = create_otp_test_user(&resources, email, "NonNumeric123!").await;

        let wa_secret = "wa_nonnumeric_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550001111";

        // Set up OTP state in "awaiting OTP code" sub-state
        let state_id =
            create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;
        advance_to_awaiting_otp_code(db, &state_id, email, "654321").await;

        // Send non-numeric text during OTP step
        let payload = whatsapp_text_payload(sender_id, "wamid.nonnumeric_001", "hello");
        let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // OTP state should still be active with 0 attempts (non-numeric is not counted)
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state["otp_attempts"].as_i64(),
            Some(0),
            "Non-numeric input should not increment the attempt counter"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Cancel command during OTP flow → flow cancelled
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_cancel_command() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) =
            create_otp_test_user(&resources, "cancel@example.com", "Cancel123!").await;

        let wa_secret = "wa_cancel_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550002222";

        // Set up OTP state
        create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;

        // Verify state exists
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(state.is_some(), "OTP state should exist before cancel");

        // Send cancel command
        let payload = whatsapp_text_payload(sender_id, "wamid.cancel_001", "cancel");
        let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // OTP state should be invalidated
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_none(),
            "OTP state should be invalidated after cancel command"
        );
    }

    #[tokio::test]
    async fn test_otp_flow_cancel_case_insensitive() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) =
            create_otp_test_user(&resources, "cancelcase@example.com", "CancelCase123!").await;

        let wa_secret = "wa_cancelcase_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550003333";

        create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;

        // Send "CANCEL" (uppercase) — should still cancel
        let payload = whatsapp_text_payload(sender_id, "wamid.cancelcase_001", "CANCEL");
        let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_none(),
            "OTP state should be invalidated by uppercase CANCEL"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Happy path → correct OTP → account linked
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_linking_happy_path() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let email = "happy@example.com";
        let (user_id, tenant_id) = create_otp_test_user(&resources, email, "Happy123!").await;

        let wa_secret = "wa_happy_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550004444";
        let otp_code = "987654";

        // Set up OTP state already at "awaiting OTP code" (bypasses email send)
        let state_id =
            create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;
        advance_to_awaiting_otp_code(db, &state_id, email, otp_code).await;

        // Verify no channel link exists yet
        let link = db
            .get_channel_link(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            link.is_none(),
            "No channel link should exist before OTP verification"
        );

        // Send correct OTP code
        let payload = whatsapp_text_payload(sender_id, "wamid.happy_001", otp_code);
        let (status, _body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // Channel link should now exist
        let link = db
            .get_channel_link(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            link.is_some(),
            "Channel link should be created after correct OTP"
        );
        let link_val = link.unwrap();
        assert_eq!(
            link_val["user_id"].as_str(),
            Some(user_id.to_string().as_str()),
            "Channel link should reference the correct user"
        );

        // OTP state should be invalidated (consumed)
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_none(),
            "OTP state should be invalidated after successful linking"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Already-linked user → straight to LLM dispatch
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_linked_user_bypasses_otp_flow() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let email = "linked@example.com";
        let (user_id, tenant_id) = create_otp_test_user(&resources, email, "Linked123!").await;

        let wa_secret = "wa_linked_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550005555";

        // Pre-create a channel link (user is already linked)
        use pierre_database::plugins::CreateChannelLinkParams;
        let link_id = Uuid::new_v4().to_string();
        let user_id_str = user_id.to_string();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &link_id,
            tenant_id,
            user_id: &user_id_str,
            channel_type: "whatsapp",
            channel_user_id: sender_id,
            display_name: Some("LinkedUser"),
        })
        .await
        .unwrap();

        // Send message from linked user
        let payload = whatsapp_text_payload(sender_id, "wamid.linked_001", "What was my last run?");
        let (status, body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"].as_str(), Some("ok"));
        assert_eq!(body["messages_received"].as_u64(), Some(1));

        // No OTP state should have been created
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(state.is_none(), "Linked user should not trigger OTP flow");
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Expired OTP state → user prompted to start over
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_expired_state_ignored() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let email = "expired_otp@example.com";
        let (_, tenant_id) = create_otp_test_user(&resources, email, "Expired123!").await;

        let wa_secret = "wa_expired_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550006666";

        // Create an expired OTP state (expired 5 minutes ago)
        let state_id = Uuid::new_v4().to_string();
        let code = Uuid::new_v4().to_string();
        let expires_at = (Utc::now() - Duration::minutes(5)).to_rfc3339();

        let params = CreateLinkStateParams {
            id: &state_id,
            tenant_id,
            user_id: None,
            channel_type: "whatsapp",
            code: &code,
            method: "otp",
            channel_user_id: Some(sender_id),
            sender_name: None,
            expires_at: &expires_at,
        };
        db.create_link_state(&params).await.unwrap();
        db.set_otp_on_link_state(&state_id, email, &hash_otp("654321"))
            .await
            .unwrap();

        // Send OTP code for expired state — should not match any active flow
        let payload = whatsapp_text_payload(sender_id, "wamid.expired_001", "654321");
        let (status, body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;

        // Webhook should still succeed (200 OK), but the expired state is ignored.
        // Since no active OTP flow is found and no channel link exists, the user is
        // treated as unlinked and a new OTP flow is started.
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"].as_str(), Some("ok"));

        // A new OTP state should have been created (since email service is available)
        let new_state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            new_state.is_some(),
            "A new OTP flow should start when expired state is ignored"
        );
        let new_state_val = new_state.unwrap();
        // The new state should NOT be the expired one
        assert_ne!(
            new_state_val["id"].as_str().unwrap_or(""),
            state_id.as_str(),
            "New OTP state should have a different ID than the expired one"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Cancel without active OTP flow → normal message
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_cancel_without_active_otp_flow() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let (_, tenant_id) =
            create_otp_test_user(&resources, "noflow_cancel@example.com", "NoFlow123!").await;

        let wa_secret = "wa_noflow_cancel_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550007777";

        // No OTP state exists — sending "cancel" should be treated as a normal
        // unlinked user message (starts new OTP flow)
        let payload = whatsapp_text_payload(sender_id, "wamid.noflow_001", "cancel");
        let (status, body) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"].as_str(), Some("ok"));

        // Since "cancel" was not handled by OTP flow (no active flow), the user is
        // treated as unlinked. With email service available, a new OTP flow should start.
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            state.is_some(),
            "Unlinked user sending 'cancel' without active OTP should start a new OTP flow"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Webhook: Multiple wrong codes then correct code
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_otp_flow_retry_then_correct_code() {
        let mut resources = create_test_server_resources().await.unwrap();
        inject_dummy_email_service(&mut resources);

        let db: &dyn MessagingRepository = &*resources.database;
        let email = "retry_ok@example.com";
        let (_, tenant_id) = create_otp_test_user(&resources, email, "RetryOk123!").await;

        let wa_secret = "wa_retry_ok_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550008888";
        let otp_code = "246810";

        let state_id =
            create_otp_link_state_awaiting_email(db, tenant_id, "whatsapp", sender_id).await;
        advance_to_awaiting_otp_code(db, &state_id, email, otp_code).await;

        // Send 2 wrong codes (still under max attempts of 3)
        for i in 1..=2 {
            let msg_id = format!("wamid.retry_{i}");
            let wrong_code = format!("{:06}", 100_000 + i);
            let payload = whatsapp_text_payload(sender_id, &msg_id, &wrong_code);
            let (status, _) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
            assert_eq!(status, StatusCode::OK);
        }

        // Verify attempts incremented to 2
        let state = db
            .get_active_otp_link_state(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state["otp_attempts"].as_i64(),
            Some(2),
            "Should have 2 attempts after 2 wrong codes"
        );

        // Now send the correct code
        let payload = whatsapp_text_payload(sender_id, "wamid.retry_correct", otp_code);
        let (status, _) = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // Channel link should exist
        let link = db
            .get_channel_link(tenant_id, "whatsapp", sender_id)
            .await
            .unwrap();
        assert!(
            link.is_some(),
            "Channel link should be created after correct OTP on third attempt"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Pure helper function tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn test_hash_otp_deterministic() {
        let hash1 = hash_otp("123456");
        let hash2 = hash_otp("123456");
        assert_eq!(hash1, hash2, "Same input should produce same hash");

        let hash3 = hash_otp("654321");
        assert_ne!(
            hash1, hash3,
            "Different inputs should produce different hashes"
        );
    }

    #[test]
    fn test_hash_otp_is_valid_hex() {
        let hash = hash_otp("999999");
        assert_eq!(hash.len(), 64, "SHA-256 hex should be 64 characters");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "Hash should contain only hex digits"
        );
    }
}
