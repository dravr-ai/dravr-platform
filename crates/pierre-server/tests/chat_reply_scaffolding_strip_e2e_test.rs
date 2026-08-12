// ABOUTME: Full-pipeline e2e guard for c0ce04e34 — after a tool runs, the persisted
// ABOUTME: assistant reply must be non-empty + grounded + free of <tool_result>/<tool_call> scaffolding
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression guard for the production "scaffolding parroting" bug
//! (`c0ce04e34`): in a long conversation the model began echoing raw
//! `<tool_result>` scaffolding back into its final reply, which on the
//! Telegram surface stripped to an empty "Hmm je n'ai pas réussi"
//! response. The fix cleans the durable copy on the write side
//! ([`pierre_chat_pipeline`] stage 19,
//! `strip_simulation_artifacts(&result.content)`) and strips again on the
//! read side. Nothing tested that the *persisted assistant reply* survives
//! a parroted echo, only that a tool was recorded as called
//! (`messaging_eval_pipeline_tool_loop_test.rs` asserts `tools_called`).
//!
//! This test closes that gap end-to-end:
//!
//! 1. A no-network [`StubTool`] is registered via `extra_tools` so the CLI
//!    tool loop can route to it with no auth/provider setup.
//! 2. A stateful [`ParrotingMockProvider`] emits a `<tool_call>` block on
//!    its first `complete()` call (the mock omits `FUNCTION_CALLING`, so the
//!    dispatcher takes the CLI text loop that parses `<tool_call>` blocks)
//!    and, on the final call, a real grounded answer that *also* parrots a
//!    raw `<tool_result>` echo — exactly the prod failure shape.
//! 3. A real Slack webhook drives the full messaging → chat pipeline.
//! 4. The test reads the persisted `chat_messages` assistant row and asserts
//!    the durable reply is non-empty, still carries the grounded marker, and
//!    contains no `<tool_result>` / `<tool_call>` scaffolding.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod reply_scaffolding_strip {
    use crate::common::create_test_server_resources_with_llm_and_tools;
    use crate::helpers::axum_test::AxumTestRequest;
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use chrono::Utc;
    use dravr_tronc::mcp::schema::{Content, Tool, ToolResponse};
    use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
    use futures_util::stream;
    use hmac::{Hmac, Mac};
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
        TokenUsage,
    };
    use pierre_core::models::ConnectionType;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_core::permissions::UserRole;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use pierre_tool_runtime::RuntimeTool;
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

    /// Unique token the model places *outside* the parroted scaffolding so the
    /// test can prove a real answer survived the strip (vs reducing to empty).
    const GROUNDED_MARKER: &str = "GROUNDED_REPLY_OK_c0ce04e34";

    /// Name of the stub tool the mock LLM requests. Isolated from real
    /// registry names so it is unambiguous which tool ran.
    const STUB_TOOL_NAME: &str = "scaffolding_strip_stub_tool";

    /// No-auth, no-network tool. Returns canned JSON so the CLI tool loop
    /// continues instead of erroring on provider/auth requirements. Counts
    /// invocations so the test can prove the loop genuinely executed a tool
    /// (otherwise the strip assertion could pass trivially).
    struct StubTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl McpTool<dyn ToolRuntime> for StubTool {
        fn definition(&self) -> Tool {
            Tool {
                name: STUB_TOOL_NAME.to_owned(),
                description: "Scaffolding-strip stub tool — returns canned data so the tool loop completes without provider/auth setup.".to_owned(),
                input_schema: json!({"type": "object"}),
                annotations: None,
            }
        }

        fn capabilities(&self) -> ToolCapabilities {
            // Deliberately minimal: no auth, no tenant, no provider.
            ToolCapabilities::READS_DATA
        }

        async fn execute(
            &self,
            _state: &Arc<dyn ToolRuntime>,
            _ctx: &ToolContext,
            _args: Value,
        ) -> ToolResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let payload = json!({
                "stub": true,
                "runs_this_week": 3,
                "data": "canned tool result for scaffolding-strip test"
            });
            ToolResponse {
                content: vec![Content::Text {
                    text: payload.to_string(),
                }],
                is_error: false,
                structured_content: Some(payload),
            }
        }
    }

    pierre_tool_runtime::declare_security!(StubTool => empty);

    /// Deterministic provider that reproduces the c0ce04e34 failure shape.
    ///
    /// First `complete()` call returns a `<tool_call>` block referencing
    /// [`STUB_TOOL_NAME`]. The final call returns a genuine grounded answer
    /// that *also* parrots a raw `<tool_result>` echo — the exact thing the
    /// pipeline's write-side strip must remove from the durable reply.
    ///
    /// Capabilities omit `FUNCTION_CALLING` so the dispatcher routes to the
    /// CLI text loop (which parses `<tool_call>` blocks out of plain
    /// `complete()` content); the `Custom` provider wrapper always returns
    /// `function_calls: None`, so the native path can never be reached by a
    /// mock anyway.
    struct ParrotingMockProvider {
        calls: Arc<AtomicUsize>,
    }

    impl ParrotingMockProvider {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for ParrotingMockProvider {
        fn name(&self) -> &'static str {
            "parroting_mock"
        }

        fn display_name(&self) -> &'static str {
            "Parroting Mock LLM (scaffolding-strip e2e)"
        }

        fn capabilities(&self) -> LlmCapabilities {
            // SYSTEM_MESSAGES only — no FUNCTION_CALLING — so the dispatcher
            // takes the CLI text-based tool loop.
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
                // First call: request the stub tool via a <tool_call> block.
                format!(r#"<tool_call>{{"name":"{STUB_TOOL_NAME}","arguments":{{}}}}</tool_call>"#)
            } else {
                // Final call: a real answer that ALSO parrots raw tool-result
                // scaffolding — the production c0ce04e34 failure shape. The
                // pipeline's write-side strip must reduce this to the grounded
                // sentence only.
                format!(
                    "Voici ton résumé: {GROUNDED_MARKER} — tu as complété 3 courses cette semaine. \
                     <tool_result>{{\"stub\":true,\"runs_this_week\":3}}</tool_result>"
                )
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

    /// Compute the Slack webhook signature (`v0=<hex-hmac-sha256>` over
    /// `v0:{ts}:{body}`).
    fn compute_slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// Seed an active admin user + tenant with a synthetic provider connection
    /// so the Slack-webhook turn clears the onboarding gate and reaches the
    /// chat pipeline.
    async fn create_admin_user_in_tenant(
        resources: &Arc<ServerContext>,
        email: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Strip Admin".to_owned()),
        );
        user.is_admin = true;
        user.role = UserRole::Admin;
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Strip Tenant {email}"),
            slug: format!("strip-tenant-{tenant_id}"),
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
            .users
            .update_tenant_id(user_id, tenant_id)
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

    /// Poll `chat_messages` until the pipeline persists an `assistant` row for
    /// the tenant, then return its (already write-side-stripped) content.
    ///
    /// 150 × 200ms = 30s, matching the messaging tool-loop test's budget for
    /// contended CI runners using a mock LLM.
    async fn wait_for_persisted_assistant_reply(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
    ) -> Option<String> {
        let pool = resources
            .coach
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");
        let tenant_str = tenant_id.to_string();

        // chat_messages has no tenant_id column; tenant lives on the parent
        // chat_conversations row, so join through conversation_id.
        for _ in 0..150 {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT m.content \
                 FROM chat_messages m \
                 JOIN chat_conversations c ON m.conversation_id = c.id \
                 WHERE c.tenant_id = ?1 AND m.role = 'assistant' \
                 ORDER BY m.created_at DESC LIMIT 1",
            )
            .bind(&tenant_str)
            .fetch_optional(pool)
            .await
            .unwrap();
            if let Some((content,)) = row {
                return Some(content);
            }
            sleep(Duration::from_millis(200)).await;
        }
        None
    }

    #[tokio::test]
    #[serial]
    async fn persisted_assistant_reply_is_grounded_and_free_of_scaffolding() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");

        let mock = Arc::new(ParrotingMockProvider::new());
        let stub_calls = Arc::new(AtomicUsize::new(0));
        let stub_tool: Arc<dyn RuntimeTool> = Arc::new(StubTool {
            calls: Arc::clone(&stub_calls),
        });
        let resources = create_test_server_resources_with_llm_and_tools(mock, vec![stub_tool])
            .await
            .unwrap();
        let (admin_user_id, tenant_id) =
            create_admin_user_in_tenant(&resources, "scaffolding-strip@example.com").await;
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let signing_secret = "scaffolding_strip_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-scaffolding-strip"),
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

        let slack_sender_id = "U_SCAFFOLDING_STRIP";
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &admin_user_id.to_string(),
            channel_type: "slack",
            channel_user_id: slack_sender_id,
            display_name: Some("Strip Sender"),
        })
        .await
        .unwrap();

        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": slack_sender_id,
                "text": "How many runs did I do this week?",
                "channel": "C_SCAFFOLDING_STRIP",
                "ts": "1700000003.000001"
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

        let reply = wait_for_persisted_assistant_reply(&resources, tenant_id)
            .await
            .expect("pipeline did not persist an assistant chat_messages row within 30s");

        // The tool must have genuinely run — otherwise there would be no
        // scaffolding in play and the strip assertion would pass trivially.
        assert!(
            stub_calls.load(Ordering::SeqCst) >= 1,
            "the CLI tool loop must have invoked the stub tool"
        );

        // c0ce04e34 guard: a real answer survives the write-side strip.
        assert!(
            !reply.trim().is_empty(),
            "persisted assistant reply must not be empty (parroted echo reduced to nothing)"
        );
        assert!(
            reply.contains(GROUNDED_MARKER),
            "grounded answer text must survive the strip, got: {reply:?}"
        );

        // ...and the parroted tool scaffolding must be gone from the durable copy.
        assert!(
            !reply.contains("<tool_result>") && !reply.contains("</tool_result>"),
            "persisted reply must not contain <tool_result> scaffolding, got: {reply:?}"
        );
        assert!(
            !reply.contains("<tool_call>") && !reply.contains("</tool_call>"),
            "persisted reply must not contain <tool_call> scaffolding, got: {reply:?}"
        );
        assert!(
            !reply.contains("\"stub\""),
            "echoed tool-result payload must be stripped from the durable copy, got: {reply:?}"
        );
    }
}
