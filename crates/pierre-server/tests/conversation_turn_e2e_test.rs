// ABOUTME: Full E2E: Slack webhook → messaging pipeline → llm_usage rows → /internal/conversation-turn
// ABOUTME: Uses MockLlmProvider so the pipeline runs end-to-end without network
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod conversation_turn_e2e_tests {
    use crate::common::create_test_server_resources_with_llm;
    use crate::helpers::axum_test::AxumTestRequest;
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use chrono::Utc;
    use futures_util::stream;
    use hmac::{Hmac, Mac};
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
        TokenUsage,
    };
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
    use pierre_mcp_server::permissions::UserRole;
    use pierre_mcp_server::routes::llm_consumption::LlmConsumptionRoutes;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::{json, Value};
    use serial_test::serial;
    use sha2::Sha256;
    use std::env;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// Deterministic provider that returns a canned response on every
    /// `complete()` call. Injected via
    /// [`ServerContext::with_llm_provider`] so the messaging pipeline
    /// runs end-to-end without reaching the network. Token counts are
    /// non-zero so `LlmCallRecorder` writes a real per-call row.
    struct MockLlmProvider {
        reply: String,
        model: String,
        calls: Arc<AtomicUsize>,
    }

    impl MockLlmProvider {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                reply: reply.into(),
                model: "mock-model".to_owned(),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn call_counter(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlmProvider {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn display_name(&self) -> &'static str {
            "Mock LLM (tests)"
        }

        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::STREAMING
                | LlmCapabilities::FUNCTION_CALLING
                | LlmCapabilities::SYSTEM_MESSAGES
        }

        fn default_model(&self) -> &str {
            &self.model
        }

        fn available_models(&self) -> &[String] {
            &[]
        }

        async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ChatResponse {
                content: self.reply.clone(),
                model: self.model.clone(),
                usage: Some(TokenUsage {
                    prompt_tokens: 42,
                    completion_tokens: 11,
                    total_tokens: 53,
                }),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            let chunk = StreamChunk {
                delta: self.reply.clone(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            };
            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    /// Compute the Slack webhook signature (`v0=<hex-hmac-sha256>` over
    /// `v0:{ts}:{body}`).
    fn compute_slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// Seed an admin user + tenant directly so the test can query the
    /// admin-only `/internal/conversation-turn` endpoint afterwards.
    async fn create_admin_user_in_tenant(
        resources: &Arc<ServerContext>,
        email: &str,
    ) -> (Uuid, TenantId, String) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("E2E Admin".to_owned()),
        );
        user.is_admin = true;
        user.role = UserRole::Admin;
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.repos.users.create(&user).await.unwrap();

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
        resources.repos.tenants.create(&tenant).await.unwrap();
        resources
            .repos
            .users
            .update_tenant_id(user_id, tenant_id)
            .await
            .unwrap();

        let token = resources
            .auth_manager
            .generate_token(&user, &resources.jwks_manager)
            .unwrap();
        (user_id, tenant_id, format!("Bearer {token}"))
    }

    /// Poll the `llm_usage` table until at least one row exists for the
    /// given tenant. Returns the `turn_id` from the first row found.
    ///
    /// The messaging webhook spawns dispatch on a background task, so the
    /// test thread needs to wait for the pipeline to finish. 10s is well
    /// above the mock provider's response time; failures here mean the
    /// pipeline never reached the LLM stage.
    async fn wait_for_llm_usage_row(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
    ) -> Option<Uuid> {
        let pool = resources
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");
        let tenant_str = tenant_id.to_string();

        for _ in 0..50 {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT turn_id FROM llm_usage WHERE tenant_id = ?1 LIMIT 1")
                    .bind(&tenant_str)
                    .fetch_optional(pool)
                    .await
                    .unwrap();
            if let Some((turn_id_str,)) = row {
                return Uuid::parse_str(&turn_id_str).ok();
            }
            sleep(Duration::from_millis(200)).await;
        }
        None
    }

    /// End-to-end contract:
    ///
    /// 1. Slack sends a user message to the webhook.
    /// 2. Canot's webhook adapter stamps a fresh `turn_id` on the
    ///    `IncomingMessage`.
    /// 3. `messaging_ingress::dispatch_and_respond` adopts that id,
    ///    threads it into `TurnInput`, and the chat pipeline uses it for
    ///    every LLM call + the terminal summary.
    /// 4. `LlmCallRecorder` writes one `llm_usage` row per LLM call with
    ///    that `turn_id`.
    /// 5. The admin `/internal/conversation-turn/{turn_id}` endpoint
    ///    returns the rows, proving the full webhook → pipeline → DB →
    ///    observability round-trip works under one id.
    #[tokio::test]
    #[serial]
    async fn slack_webhook_to_conversation_turn_endpoint_round_trip() {
        // The messaging ingress creates a conversation up front via
        // `chat_conversations`, which reads `PIERRE_LLM_MODEL` even though
        // the downstream LLM call goes to our mock. Set it to any model
        // name — the value is stored on the conversation, never dispatched.
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        // Mock LLM so the pipeline runs without touching Gemini/Groq.
        let mock = Arc::new(MockLlmProvider::new("Mock reply from the test provider."));
        let call_counter = mock.call_counter();

        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let (admin_user_id, tenant_id, admin_auth) =
            create_admin_user_in_tenant(&resources, "turn-e2e-admin@example.com").await;
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        // Configure the Slack channel so the webhook's HMAC check passes.
        let signing_secret = "slack_turn_e2e_secret_99";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-turn-e2e-slack"),
            api_secret: None,
            webhook_secret: Some(signing_secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();

        // Bind the Slack sender directly to the admin user so the message
        // routes to the chat pipeline rather than the unlinked-user flow
        // (which would stop at the OTP/link prompt).
        let slack_sender_id = "U_TURN_E2E";
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &admin_user_id.to_string(),
            channel_type: "slack",
            channel_user_id: slack_sender_id,
            display_name: Some("E2E Sender"),
        })
        .await
        .unwrap();

        // Send a real Slack event_callback payload.
        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": slack_sender_id,
                "text": "What was my last run?",
                "channel": "C_TURN_E2E",
                "ts": "1700000000.000001"
            }
        })
        .to_string();
        let timestamp = Utc::now().timestamp().to_string();
        let sig = compute_slack_sig(signing_secret, &timestamp, &body);

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &sig)
            .text(&body)
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        // Wait for the background dispatch to finish writing llm_usage rows.
        let turn_id = wait_for_llm_usage_row(&resources, tenant_id)
            .await
            .expect("messaging pipeline did not write an llm_usage row within 10s");

        assert!(
            call_counter.load(Ordering::SeqCst) >= 1,
            "Mock provider should have been invoked by the pipeline"
        );

        // Query the admin /internal endpoint — this is the observability
        // path that on-call and the messaging-eval harness consume.
        let router = LlmConsumptionRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn_id}"))
            .header("authorization", &admin_auth)
            .send(router)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);
        let body: Value = resp.json::<Value>();
        assert_eq!(body["turn_id"].as_str().unwrap(), turn_id.to_string());
        assert_eq!(body["tenant_id"].as_str().unwrap(), tenant_id.to_string());
        let calls = body["llm_calls"].as_array().expect("llm_calls array");
        assert!(
            !calls.is_empty(),
            "endpoint must return at least one per-call row from the tool loop"
        );
        assert!(
            body["total_tokens"].as_u64().unwrap() > 0,
            "per-call rows carry real token counts from the mock"
        );
    }
}
