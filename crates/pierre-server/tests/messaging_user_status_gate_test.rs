// ABOUTME: Integration tests for the user-status gate shared by HTTP middleware and messaging ingress
// ABOUTME: Validates Pending / Suspended messaging users get translated denial replies instead of LLM dispatch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod messaging_user_status_gate_tests {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use chrono::Utc;
    use hmac::{Hmac, Mac};
    use pierre_core::errors::ErrorCode;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::contremaitre::messaging_strings::{
        MessagingStringsRegistry, KEY_ACCOUNT_PENDING, KEY_ACCOUNT_SUSPENDED,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_mcp_server::services::user_status_gate::{
        enforce_user_status, messaging_key_for_status,
    };
    use serde_json::json;
    use sha2::Sha256;
    use std::sync::Arc;
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    // ════════════════════════════════════════════════════════════════
    // Helpers (mirror messaging_otp_linking_test scaffolding)
    // ════════════════════════════════════════════════════════════════

    fn hmac_sha256_hex(secret: &str, data: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(data);
        hex::encode(mac.finalize().into_bytes())
    }

    fn compute_whatsapp_sig(secret: &str, body: &[u8]) -> String {
        format!("sha256={}", hmac_sha256_hex(secret, body))
    }

    /// Create a test user with the requested status and a tenant they own.
    async fn create_user_with_status(
        resources: &ServerContext,
        email: &str,
        password: &str,
        status: UserStatus,
    ) -> (Uuid, TenantId) {
        let password_owned = password.to_owned();
        let password_hash =
            spawn_blocking(move || bcrypt::hash(&password_owned, bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Status User".to_owned()),
        );
        user.user_status = status;
        if status == UserStatus::Active {
            user.approved_by = Some(user.id);
            user.approved_at = Some(Utc::now());
        }

        let user_id = user.id;
        resources.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Status Tenant {email}"),
            slug: format!("status-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        resources.repos.tenants.create(&tenant).await.unwrap();

        (user_id, tenant_id)
    }

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
            api_key: Some("wa_status_test_token"),
            api_secret: None,
            webhook_secret: Some(secret),
            verify_token: None,
            account_id: Some("wa_status_test_business_id"),
            phone_number: Some("15550000001"),
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
    }

    async fn link_channel(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        user_id: Uuid,
        sender_id: &str,
    ) {
        let link_id = Uuid::new_v4().to_string();
        let user_id_str = user_id.to_string();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &link_id,
            tenant_id,
            user_id: &user_id_str,
            channel_type: "whatsapp",
            channel_user_id: sender_id,
            display_name: Some("Linked User"),
        })
        .await
        .unwrap();
    }

    fn whatsapp_text_payload(sender_id: &str, msg_id: &str, text: &str) -> serde_json::Value {
        json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "wa_status_test_business_id",
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

    async fn send_whatsapp_webhook(
        resources: &Arc<ServerContext>,
        secret: &str,
        payload: &serde_json::Value,
    ) -> StatusCode {
        let body_bytes = serde_json::to_vec(payload).unwrap();
        let sig = compute_whatsapp_sig(secret, &body_bytes);
        let router = MessagingRoutes::routes(Arc::clone(resources));

        AxumTestRequest::post("/api/messaging/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", &sig)
            .json(payload)
            .send(router)
            .await
            .status_code()
    }

    /// Count inbound `messaging_messages` stored for the `(tenant, channel, sender)` tuple.
    ///
    /// `store_inbound_message` writes synchronously to `messaging_messages` only
    /// after the user-status gate passes, so a Pending or Suspended user's
    /// traffic produces zero rows while an Active user's first message produces
    /// at least one (the inbound audit row, before LLM dispatch fires).
    async fn count_session_messages(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        channel: &str,
        sender_id: &str,
    ) -> usize {
        let Ok(Some(session)) = db
            .get_session_by_channel_identity(tenant_id, channel, sender_id, None)
            .await
        else {
            return 0;
        };
        let Some(session_id) = session["id"].as_str() else {
            return 0;
        };
        db.get_session_messages(session_id, tenant_id, 100, 0)
            .await
            .map(|m| m.len())
            .unwrap_or(0)
    }

    // ════════════════════════════════════════════════════════════════
    // Unified path: McpAuthMiddleware::authenticate_channel returns an
    // AuthResult identical in shape to the JWT path
    // ════════════════════════════════════════════════════════════════

    /// The cutover assertion: messaging now walks the same `McpAuthMiddleware`
    /// entry point as web/mobile. An active linked user produces an
    /// [`AuthResult`] with `AuthMethod::ChannelLink` carrying the channel
    /// and sender id, plus a resolved `active_tenant_id` and a rate-limit
    /// computed from the user's tier — exactly like `authenticate_jwt_token`.
    #[tokio::test]
    async fn authenticate_channel_returns_channel_link_auth_result_for_active_user() {
        use pierre_auth::auth::AuthMethod;

        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (user_id, tenant_id) = create_user_with_status(
            &resources,
            "channel_auth@example.com",
            "ChannelAuth123!",
            UserStatus::Active,
        )
        .await;

        let sender_id = "15550008001";
        link_channel(db, tenant_id, user_id, sender_id).await;

        let auth_result = resources
            .auth_middleware
            .authenticate_channel(tenant_id, "whatsapp", sender_id)
            .await
            .expect("active linked user should authenticate");

        assert_eq!(auth_result.user_id, user_id);
        match &auth_result.auth_method {
            AuthMethod::ChannelLink {
                channel,
                channel_user_id,
                tier,
            } => {
                assert_eq!(channel, "whatsapp");
                assert_eq!(channel_user_id, sender_id);
                assert!(!tier.is_empty(), "tier must be populated for rate limiting");
            }
            other => panic!("expected AuthMethod::ChannelLink, got {other:?}"),
        }
        assert!(
            auth_result.active_tenant_id.is_some(),
            "active_tenant_id must be resolved on the messaging path same as JWT path"
        );
        assert!(
            !auth_result.rate_limit.is_rate_limited,
            "fresh user should not be rate-limited"
        );
    }

    /// Unlinked sender → `AppError::auth_invalid` with `ErrorCode::AuthInvalid`,
    /// which `resolve_or_prompt` interprets as "send the link prompt".
    #[tokio::test]
    async fn authenticate_channel_returns_auth_invalid_for_unlinked_sender() {
        let resources = create_test_server_resources().await.unwrap();
        let (_, tenant_id) = create_user_with_status(
            &resources,
            "unlinked_owner@example.com",
            "UnlinkedOwner123!",
            UserStatus::Active,
        )
        .await;

        let err = resources
            .auth_middleware
            .authenticate_channel(tenant_id, "whatsapp", "15550008999")
            .await
            .expect_err("no channel link should fail authentication");
        assert_eq!(err.code, ErrorCode::AuthInvalid);
    }

    /// Pending user → `AppError::account_pending`. Same code returned by
    /// the JWT middleware, so frontend/mobile and Telegram surface the same
    /// policy from the same function.
    #[tokio::test]
    async fn authenticate_channel_returns_account_pending_for_pending_user() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (user_id, tenant_id) = create_user_with_status(
            &resources,
            "pending_auth@example.com",
            "PendingAuth123!",
            UserStatus::Pending,
        )
        .await;

        let sender_id = "15550008002";
        link_channel(db, tenant_id, user_id, sender_id).await;

        let err = resources
            .auth_middleware
            .authenticate_channel(tenant_id, "whatsapp", sender_id)
            .await
            .expect_err("pending user should be denied at the auth boundary");
        assert_eq!(err.code, ErrorCode::AccountPending);
    }

    // ════════════════════════════════════════════════════════════════
    // Helper-level: enforce_user_status + messaging_key_for_status
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn enforce_user_status_active_passes() {
        assert!(enforce_user_status(UserStatus::Active).is_ok());
    }

    #[test]
    fn enforce_user_status_pending_returns_account_pending_code() {
        let err = enforce_user_status(UserStatus::Pending).unwrap_err();
        assert_eq!(err.code, ErrorCode::AccountPending);
    }

    #[test]
    fn enforce_user_status_suspended_returns_account_suspended_code() {
        let err = enforce_user_status(UserStatus::Suspended).unwrap_err();
        assert_eq!(err.code, ErrorCode::AccountSuspended);
    }

    #[test]
    fn messaging_key_for_status_maps_pending_and_suspended() {
        assert_eq!(
            messaging_key_for_status(ErrorCode::AccountPending),
            Some(KEY_ACCOUNT_PENDING),
        );
        assert_eq!(
            messaging_key_for_status(ErrorCode::AccountSuspended),
            Some(KEY_ACCOUNT_SUSPENDED),
        );
        assert_eq!(messaging_key_for_status(ErrorCode::InvalidInput), None);
    }

    // ════════════════════════════════════════════════════════════════
    // Registry: pending / suspended translations are wired for every locale
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn registry_has_pending_and_suspended_for_every_compiled_locale() {
        let reg = MessagingStringsRegistry::new();
        for locale in ["fr", "en", "es", "de", "pt"] {
            let pending = reg.get(KEY_ACCOUNT_PENDING, locale);
            let suspended = reg.get(KEY_ACCOUNT_SUSPENDED, locale);
            assert!(
                !pending.trim().is_empty(),
                "registry missing pending copy for {locale}"
            );
            assert!(
                !suspended.trim().is_empty(),
                "registry missing suspended copy for {locale}"
            );
        }
    }

    #[test]
    fn registry_pending_copy_mentions_approval_in_fr_and_en() {
        let reg = MessagingStringsRegistry::new();
        let fr = reg.get(KEY_ACCOUNT_PENDING, "fr").to_lowercase();
        let en = reg.get(KEY_ACCOUNT_PENDING, "en").to_lowercase();
        assert!(
            fr.contains("approbation") || fr.contains("approuv"),
            "FR pending copy should reference approval: {fr}"
        );
        assert!(
            en.contains("approval") || en.contains("approved"),
            "EN pending copy should reference approval: {en}"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // End-to-end gate: Pending linked user does not get stored / dispatched
    // ════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn pending_user_inbound_chat_is_gated_and_not_stored() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (user_id, tenant_id) = create_user_with_status(
            &resources,
            "pending_chat@example.com",
            "PendingChat123!",
            UserStatus::Pending,
        )
        .await;

        let wa_secret = "wa_pending_chat_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550009001";
        link_channel(db, tenant_id, user_id, sender_id).await;

        let payload =
            whatsapp_text_payload(sender_id, "wamid.pending_chat_001", "Can I connect Strava?");
        let status = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        // The gate fires before store_inbound_message and before any LLM dispatch,
        // so no chat_messages row should exist for this user.
        let stored = count_session_messages(db, tenant_id, "whatsapp", sender_id).await;
        assert_eq!(
            stored, 0,
            "Pending user message must not reach chat storage / LLM dispatch"
        );
    }

    #[tokio::test]
    async fn suspended_user_inbound_chat_is_gated_and_not_stored() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (user_id, tenant_id) = create_user_with_status(
            &resources,
            "suspended_chat@example.com",
            "SuspendedChat123!",
            UserStatus::Suspended,
        )
        .await;

        let wa_secret = "wa_suspended_chat_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550009002";
        link_channel(db, tenant_id, user_id, sender_id).await;

        let payload =
            whatsapp_text_payload(sender_id, "wamid.suspended_chat_001", "How's my training?");
        let status = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        let stored = count_session_messages(db, tenant_id, "whatsapp", sender_id).await;
        assert_eq!(
            stored, 0,
            "Suspended user message must not reach chat storage / LLM dispatch"
        );
    }

    /// Once admin flips status from Pending → Active, the same channel link
    /// starts dispatching without any re-linking. Models the recovery path.
    #[tokio::test]
    async fn active_user_inbound_chat_persists_a_message() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let (user_id, tenant_id) = create_user_with_status(
            &resources,
            "active_chat@example.com",
            "ActiveChat123!",
            UserStatus::Active,
        )
        .await;

        let wa_secret = "wa_active_chat_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;

        let sender_id = "15550009003";
        link_channel(db, tenant_id, user_id, sender_id).await;

        let payload = whatsapp_text_payload(sender_id, "wamid.active_chat_001", "Hello Dravr");
        let status = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK);

        let stored = count_session_messages(db, tenant_id, "whatsapp", sender_id).await;
        assert!(
            stored >= 1,
            "Active user message must reach chat storage (got {stored})"
        );
    }
}
