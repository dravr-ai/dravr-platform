// ABOUTME: E2E test for the messaging QuotaGate — a quota-exhausted user's turn is refused
// ABOUTME: pre-dispatch with the localized denial reply, and counters are read under the USER tenant

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod messaging_quota_gate_tests {
    use crate::common::create_test_server_resources;
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use chrono::Utc;
    use hmac::{Hmac, Mac};
    use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_QUOTA_EXCEEDED};
    use pierre_core::models::ConnectionType;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_services::usage_counter::increment_counter;
    use serde_json::json;
    use sha2::Sha256;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    // ════════════════════════════════════════════════════════════════
    // Helpers (mirror messaging_user_status_gate_test scaffolding)
    // ════════════════════════════════════════════════════════════════

    fn compute_whatsapp_sig(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// An Active user with a tenant they own and a synthetic provider, so the
    /// turn passes the status and onboarding gates and reaches the quota gate.
    async fn create_active_user(resources: &ServerContext, email: &str) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("QuotaGate123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();

        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Quota User".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());

        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Quota Tenant {email}"),
            slug: format!("quota-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
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
            api_key: Some("wa_quota_test_token"),
            api_secret: None,
            webhook_secret: Some(secret),
            verify_token: None,
            account_id: Some("wa_quota_test_business_id"),
            phone_number: Some("15550000002"),
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
            display_name: Some("Quota Linked User"),
        })
        .await
        .unwrap();
    }

    fn whatsapp_text_payload(sender_id: &str, msg_id: &str, text: &str) -> serde_json::Value {
        json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "wa_quota_test_business_id",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+15550000002",
                            "phone_number_id": "15550000002"
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

    /// The outbound message bodies stored for the sender's session, newest set
    /// first. The denial reply is persisted (with a retry-queue entry) when
    /// the adapter's live send fails in tests, so the body is observable here
    /// either way.
    async fn outbound_bodies(
        db: &dyn MessagingRepository,
        tenant_id: TenantId,
        sender_id: &str,
    ) -> Vec<String> {
        let Ok(Some(session)) = db
            .get_session_by_channel_identity(tenant_id, "whatsapp", sender_id, None)
            .await
        else {
            return Vec::new();
        };
        let Some(session_id) = session["id"].as_str() else {
            return Vec::new();
        };
        db.get_session_messages(session_id, tenant_id, 100, 0)
            .await
            .map(|rows| {
                rows.into_iter()
                    .filter(|r| r["direction"].as_str() == Some("outbound"))
                    .filter_map(|r| r["content_body"].as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    // ════════════════════════════════════════════════════════════════
    // The gate: a quota-exhausted messaging turn is refused pre-dispatch
    // with the localized denial, never silence (registre#9 / #8)
    // ════════════════════════════════════════════════════════════════

    /// Exhaust the Starter tier's daily message budget (50/day, 1.5x burst →
    /// refusals from 75) under the USER's tenant, then drive a `WhatsApp` turn.
    /// The reply must be the localized quota denial — asserted against the
    /// exact registry string — and no chat turn may be persisted: the gate
    /// runs before the pipeline stores the user message, so a refused turn
    /// leaves chat history untouched.
    #[tokio::test]
    async fn quota_exhausted_turn_gets_localized_denial_not_silence() {
        let resources = create_test_server_resources().await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, tenant_id) =
            create_active_user(&resources, "quota_exhausted@example.com").await;

        let wa_secret = "wa_quota_gate_secret";
        setup_whatsapp_config(db, tenant_id, wa_secret).await;
        let sender_id = "15550009001";
        link_channel(db, tenant_id, user_id, sender_id).await;

        // Burn the whole daily budget under the USER's tenant — the same
        // scope enforcement reads. Recording under the bot tenant was the
        // registre#9 asymmetry this test pins the fix for.
        increment_counter(
            resources.common.repos.usage_counters.as_ref(),
            &tenant_id.to_string(),
            &user_id.to_string(),
            "daily_messages",
            100,
        )
        .await
        .unwrap();

        let payload = whatsapp_text_payload(sender_id, "wamid.quota_001", "coach, am I ready?");
        let status = send_whatsapp_webhook(&resources, wa_secret, &payload).await;
        assert_eq!(status, StatusCode::OK, "webhooks always ack");

        // Dispatch runs async after the webhook ack; poll for the denial.
        let expected = resources
            .mcp
            .messaging_strings_registry
            .get(KEY_QUOTA_EXCEEDED, DEFAULT_LOCALE);
        let mut bodies = Vec::new();
        for _ in 0..40 {
            bodies = outbound_bodies(db, tenant_id, sender_id).await;
            if !bodies.is_empty() {
                break;
            }
            sleep(Duration::from_millis(500)).await;
        }
        assert!(
            bodies.iter().any(|b| b == &expected),
            "the refused turn must answer with the localized quota denial \
             (expected {expected:?}), got outbound bodies: {bodies:?}"
        );

        // The gate runs before the pipeline persists the user message, so a
        // refused turn must leave no chat rows at all.
        let pool = resources.coach.database.sqlite_pool().unwrap();
        let chat_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chat_messages m \
             JOIN chat_conversations c ON m.conversation_id = c.id \
             WHERE c.tenant_id = ?1",
        )
        .bind(tenant_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            chat_rows, 0,
            "a quota-refused turn must not persist any chat message"
        );
    }
}
