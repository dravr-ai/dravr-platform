// ABOUTME: Full-pipeline integration test for assert_tool_called — real tool invocation, not seeded
// ABOUTME: Closes the "bypass" gap in messaging_eval_tool_called_happy_path_test.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Full-pipeline happy-path integration for
//! [`assert_tool_called`](helpers::messaging_eval::assert_tool_called).
//!
//! This test closes the bypass in
//! `messaging_eval_tool_called_happy_path_test.rs`. That file seeds
//! an `llm_usage` summary row directly because wiring a real tool-
//! invocation scenario required a test-only hook on
//! [`ServerResourcesOptions`] and an in-test [`StubTool`] that the
//! mock LLM could reference. Both of those now exist, so this test
//! exercises the real path:
//!
//! 1. A [`StubTool`] is registered via `extra_tools` so the tool
//!    dispatcher can route to a no-auth no-network handler.
//! 2. A stateful [`MockLlmProvider`] returns a `<tool_call>` block on
//!    its first `complete()` call (routing through the CLI tool
//!    loop — the mock deliberately does **not** advertise
//!    `FUNCTION_CALLING` so the native path is skipped) and a plain
//!    reply on every subsequent call.
//! 3. The real chat pipeline executes the tool via the registry,
//!    writes the per-call + terminal-summary `llm_usage` rows with
//!    `tools_called` populated from the pipeline (not the test).
//! 4. The endpoint returns the summary and `assert_tool_called`
//!    asserts the stub's name is grounded in the response.
//!
//! Together with the unit test (hand-crafted JSON), the Phase 1
//! inverted-mode integration (no tools invoked), and this one (real
//! tool invocation), the asserter's behaviour is tested from three
//! independent angles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod pipeline_tool_loop {
    use crate::common::create_test_server_resources_with_llm_and_tools;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::messaging_eval::assert_tool_called;
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use chrono::Utc;
    use futures_util::stream;
    use hmac::{Hmac, Mac};
    use pierre_core::errors::{AppError, AppResult};
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
        TokenUsage,
    };
    use pierre_database::plugins::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerResources;
    use pierre_mcp_server::mcp::schema::JsonSchema;
    use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
    use pierre_mcp_server::permissions::UserRole;
    use pierre_mcp_server::routes::llm_consumption::LlmConsumptionRoutes;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_mcp_server::tools::traits::{McpTool, ToolCapabilities};
    use pierre_mcp_server::tools::{ToolExecutionContext, ToolResult};
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

    /// Name of the stub tool the mock LLM requests. Isolated from the
    /// real tool-registry names to avoid ambiguity about which tool
    /// actually ran.
    const STUB_TOOL_NAME: &str = "messaging_eval_stub_tool";

    /// No-auth no-network tool used exclusively by this integration
    /// test. Returns canned JSON so the tool loop continues instead of
    /// erroring out on provider/auth requirements.
    struct StubTool;

    #[async_trait]
    impl McpTool for StubTool {
        fn name(&self) -> &'static str {
            STUB_TOOL_NAME
        }

        fn description(&self) -> &'static str {
            "Messaging-eval stub tool — returns canned data so the tool loop completes without provider/auth setup."
        }

        fn input_schema(&self) -> JsonSchema {
            JsonSchema {
                schema_type: "object".to_owned(),
                properties: None,
                required: None,
            }
        }

        fn capabilities(&self) -> ToolCapabilities {
            // Deliberately minimal: no auth, no tenant, no provider.
            // The tool loop should be able to invoke this without any
            // external setup.
            ToolCapabilities::READS_DATA
        }

        async fn execute(
            &self,
            _args: Value,
            _context: &ToolExecutionContext,
        ) -> AppResult<ToolResult> {
            Ok(ToolResult::ok(json!({
                "stub": true,
                "data": "canned tool result for messaging-eval pipeline test"
            })))
        }
    }

    /// Deterministic provider with per-call behaviour.
    ///
    /// First `complete()` call returns a response whose `content` is a
    /// `<tool_call>` block referencing [`STUB_TOOL_NAME`]. Subsequent
    /// calls return a plain-text reply so the CLI tool loop terminates.
    ///
    /// Capabilities deliberately omit `FUNCTION_CALLING` — the
    /// pipeline's dispatcher would otherwise route to
    /// `run_api_tool_loop`, which calls
    /// `complete_with_tools`, and the `Custom` provider variant (the
    /// wrapper our mock lives behind) always returns
    /// `function_calls: None` regardless of what `complete()` returns.
    /// Without `FUNCTION_CALLING` the dispatcher falls through to
    /// `run_cli_tool_loop`, which parses `<tool_call>` blocks out of
    /// the plain `complete()` content — exactly what the mock emits.
    struct MockLlmProviderWithToolCalls {
        calls: Arc<AtomicUsize>,
    }

    impl MockLlmProviderWithToolCalls {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlmProviderWithToolCalls {
        fn name(&self) -> &'static str {
            "mock_with_tool_calls"
        }

        fn display_name(&self) -> &'static str {
            "Mock LLM (pipeline tool loop)"
        }

        fn capabilities(&self) -> LlmCapabilities {
            // SYSTEM_MESSAGES only — no FUNCTION_CALLING, no
            // SDK_TOOL_CALLING — so the dispatcher picks the CLI
            // text-based tool loop (the path that parses <tool_call>
            // blocks out of response content).
            LlmCapabilities::SYSTEM_MESSAGES
        }

        fn default_model(&self) -> &'static str {
            "mock-model"
        }

        fn available_models(&self) -> &[String] {
            &[]
        }

        async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let content = if n == 0 {
                // First call: emit a <tool_call> block. CLI tool loop
                // parses this and routes to the stub via the registry.
                format!(r#"<tool_call>{{"name":"{STUB_TOOL_NAME}","arguments":{{}}}}</tool_call>"#)
            } else {
                // Subsequent calls: plain text reply ends the loop.
                "All set — stub tool ran successfully.".to_owned()
            };
            Ok(ChatResponse {
                content,
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage {
                    prompt_tokens: 30,
                    completion_tokens: 15,
                    total_tokens: 45,
                }),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            let chunk = StreamChunk {
                delta: "streaming not used by CLI tool loop".to_owned(),
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
            Some("Pipeline Admin".to_owned()),
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
            name: format!("Pipeline Tenant {email}"),
            slug: format!("pipeline-tenant-{tenant_id}"),
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

    /// Poll the `llm_usage` table until a per-call row carrying
    /// `tools_called` exists for the given tenant. Phase 1 removed the
    /// legacy zero-token `turn_summary` writer; the per-call row that
    /// dispatched a tool now carries `tools_called` directly.
    async fn wait_for_summary_row_turn_id(
        resources: &Arc<ServerResources>,
        tenant_id: TenantId,
    ) -> Option<Uuid> {
        let pool = resources
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");
        let tenant_str = tenant_id.to_string();

        for _ in 0..50 {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT turn_id FROM llm_usage WHERE tenant_id = ?1 AND tools_called IS NOT NULL AND tools_called != '[]' ORDER BY created_at DESC LIMIT 1",
            )
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

    #[tokio::test]
    #[serial]
    async fn pipeline_invocation_populates_tools_called_on_summary_row() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");

        let mock = Arc::new(MockLlmProviderWithToolCalls::new());
        let stub_tool: Arc<dyn McpTool> = Arc::new(StubTool);
        let resources = create_test_server_resources_with_llm_and_tools(mock, vec![stub_tool])
            .await
            .unwrap();
        let (admin_user_id, tenant_id, admin_auth) =
            create_admin_user_in_tenant(&resources, "pipeline-tool-loop@example.com").await;
        let db: &dyn MessagingRepository = &*resources.repos.messaging;

        let signing_secret = "pipeline_tool_loop_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-pipeline-tool-loop"),
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

        let slack_sender_id = "U_PIPELINE_TOOL_LOOP";
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &admin_user_id.to_string(),
            channel_type: "slack",
            channel_user_id: slack_sender_id,
            display_name: Some("Pipeline Sender"),
        })
        .await
        .unwrap();

        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": slack_sender_id,
                "text": "Call the stub tool please.",
                "channel": "C_PIPELINE_TOOL_LOOP",
                "ts": "1700000002.000001"
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

        // Wait for the pipeline to complete its tool loop and write
        // the summary row. The summary is what carries tools_called.
        let turn_id = wait_for_summary_row_turn_id(&resources, tenant_id)
            .await
            .expect("pipeline did not emit a turn_summary row within 10s");

        let router = LlmConsumptionRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn_id}"))
            .header("authorization", &admin_auth)
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let endpoint_json: Value = resp.json::<Value>();

        // The happy-path assertion: the asserter returns Ok because
        // the pipeline actually invoked the stub tool via the tool
        // loop, not because the test seeded a row directly.
        assert_tool_called(&endpoint_json, STUB_TOOL_NAME).expect(
            "pipeline must have invoked the stub tool and the summary row must record the name",
        );
    }
}
