// ABOUTME: Phase 1 asserters applied to the real /internal/conversation-turn endpoint response
// ABOUTME: Mirrors conversation_turn_e2e_test.rs setup; adds token/latency/tool-called assertions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Phase 1 integration: Phase-1 asserters vs. the real endpoint JSON.
//!
//! The existing [`conversation_turn_e2e_test.rs`] already drives the
//! full Slack webhook → messaging pipeline → `llm_usage` rows →
//! `/internal/conversation-turn/{turn_id}` round trip. This file
//! layers the Phase 1 asserters on top of that same round trip so the
//! pure-function helpers are exercised against a realistic endpoint
//! response rather than only hand-crafted JSON.
//!
//! Three assertions land:
//!
//! - [`assert_latency_ms_at_most`] — endpoint's aggregated latency is
//!   within a generous budget (the mock provider returns instantly).
//! - [`assert_tokens_at_most`] — the canned
//!   `prompt=42 completion=11 total=53` usage numbers the mock returns
//!   sum to well under the budget.
//! - [`assert_tool_called`] is exercised in **inverted mode**: the mock
//!   does not emit `tool_calls`, so `tools_called` is empty, so the
//!   asserter must flag the missing invocation. This is the
//!   hallucination-catching property we care about — it has to fire
//!   loudly when the coach claims an answer without actually calling a
//!   tool.
//!
//! Full tool-using scenarios (asserter happy-path on `tool_called`)
//! require stubbing the MCP executor and are deferred until someone
//! writes a Phase 2 scenario that actually exercises
//! `discover_routes`/`get_activities`/etc. end to end.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod phase_1_integration {
    use crate::common::create_test_server_resources_with_llm;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::messaging_eval::{
        assert_latency_ms_at_most, assert_tokens_at_most, assert_tool_called,
    };
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
    use pierre_database::plugins::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerResources;
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

    /// Deterministic provider returning a fixed reply + canned token
    /// usage. Kept in-file rather than shared with
    /// `conversation_turn_e2e_test.rs` because the sibling test sets
    /// its own call-counter atomic and the duplication is cheap.
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
    }

    #[async_trait]
    impl LlmProvider for MockLlmProvider {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn display_name(&self) -> &'static str {
            "Mock LLM (Phase 1 integration)"
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

    fn compute_slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    async fn create_admin_user_in_tenant(
        resources: &Arc<ServerResources>,
        email: &str,
    ) -> (Uuid, TenantId, String) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Phase 1 Admin".to_owned()),
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
            name: format!("Phase 1 Tenant {email}"),
            slug: format!("phase-1-tenant-{tenant_id}"),
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

    async fn wait_for_llm_usage_row(
        resources: &Arc<ServerResources>,
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

    /// Drive the webhook → pipeline → endpoint round trip, then apply
    /// every Phase 1 asserter to the endpoint JSON. Proves the
    /// asserters compose correctly with the real response shape — not
    /// only with the hand-crafted JSON the unit tests use.
    #[tokio::test]
    #[serial]
    async fn phase_1_asserters_against_real_endpoint_response() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = Arc::new(MockLlmProvider::new("Phase 1 mock reply."));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let (admin_user_id, tenant_id, admin_auth) =
            create_admin_user_in_tenant(&resources, "phase-1-integration@example.com").await;
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let signing_secret = "phase1_slack_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-phase1"),
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

        let slack_sender_id = "U_PHASE1";
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &admin_user_id.to_string(),
            channel_type: "slack",
            channel_user_id: slack_sender_id,
            display_name: Some("Phase 1 Sender"),
        })
        .await
        .unwrap();

        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": slack_sender_id,
                "text": "Quick status check.",
                "channel": "C_PHASE1",
                "ts": "1700000001.000001"
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

        let turn_id = wait_for_llm_usage_row(&resources, tenant_id)
            .await
            .expect("messaging pipeline did not write an llm_usage row within 10s");

        let router = LlmConsumptionRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn_id}"))
            .header("authorization", &admin_auth)
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let endpoint_json: Value = resp.json::<Value>();

        // Phase 1 asserters against the real response:

        // 1. Latency budget — mock returns instantly; a generous
        //    10-second budget must pass.
        assert_latency_ms_at_most(&endpoint_json, 10_000)
            .expect("mock provider latency must be well under 10s");

        // 2. Token budget — mock returns 42+11 = 53 tokens; 1_000 is a
        //    comfortable ceiling.
        assert_tokens_at_most(&endpoint_json, 1_000)
            .expect("mock provider tokens must be well under 1000");

        // 3. Tool-called assertion in inverted mode — the mock did not
        //    emit tool_calls, so tools_called is empty. The asserter
        //    MUST flag this. This is the hallucination-detection
        //    behavior we rely on in real-platform scenarios.
        let err = assert_tool_called(&endpoint_json, "get_activities")
            .expect_err("mock did not invoke any tool; asserter must flag the absence");
        assert_eq!(err.expected, "get_activities");
        assert!(
            err.actual.is_empty(),
            "no tools should have been called; actual = {:?}",
            err.actual
        );
    }
}
