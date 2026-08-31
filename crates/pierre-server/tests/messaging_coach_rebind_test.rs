// ABOUTME: A coach picked mid-session must reach the conversation the athlete is already in
// ABOUTME: Drives two real Telegram webhook turns with an activation between them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

//! A messaging channel holds ONE long-lived conversation per athlete. The coach
//! was bound only when that conversation was created, so activating a different
//! one changed nothing the athlete could see — the previous coach kept
//! answering until `/reset` forged a fresh conversation, which is why `/reset`
//! became the way to change coach, at the cost of the whole thread.
//!
//! Two webhook turns with an activation between them is the smallest fixture
//! that can tell the fix from the bug: turn one opens the conversation, the
//! activation moves `tenant_users.selected_coach_id`, turn two must carry the
//! new coach on the SAME conversation row.

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod coach_rebind_tests {
    use crate::common::create_test_server_resources_with_llm;
    use crate::helpers::axum_test::AxumTestRequest;
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use chrono::Utc;
    use futures_util::stream;
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
        TokenUsage,
    };
    use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
    use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::factory::Database;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// Deterministic offline LLM. The counter marks how many turns completed,
    /// which is what the polls below wait on.
    struct MockLlm {
        calls: Arc<AtomicUsize>,
    }

    impl MockLlm {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
        fn counter(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlm {
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
        fn default_model(&self) -> &'static str {
            "mock-model"
        }
        fn available_models(&self) -> &[String] {
            &[]
        }
        async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ChatResponse {
                content: "Bien reçu.".to_owned(),
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage::new(10, 3, 13)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }
        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(stream::iter(vec![Ok(StreamChunk {
                delta: "Bien reçu.".to_owned(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            })])))
        }
        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    async fn create_user_with_own_tenant(
        resources: &ServerContext,
        email: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Rebind123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Rebind Athlete".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Rebind Tenant {email}"),
            slug: format!("rebind-tenant-{tenant_id}"),
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
        (user_id, tenant_id)
    }

    /// Wait until at least `n` turns have reached the LLM.
    async fn wait_for_turns(calls: &Arc<AtomicUsize>, n: usize) -> bool {
        for _ in 0..60 {
            if calls.load(Ordering::SeqCst) >= n {
                return true;
            }
            sleep(Duration::from_millis(200)).await;
        }
        false
    }

    /// The conversation id the messaging session points at, once it exists.
    async fn session_conversation_id(resources: &ServerContext, user_id: Uuid) -> Option<String> {
        // `messaging_sessions.user_id` is a `uuid` column on PostgreSQL and text
        // on SQLite; the cast lets one bound text id serve both.
        optional_text(
            &resources.coach.database,
            "SELECT pierre_conversation_id FROM messaging_sessions \
             WHERE CAST(user_id AS TEXT) = $1",
            &user_id.to_string(),
        )
        .await
    }

    async fn conversation_coach(
        resources: &ServerContext,
        conversation_id: &str,
    ) -> Option<String> {
        optional_text(
            &resources.coach.database,
            "SELECT coach_id FROM chat_conversations WHERE id = $1",
            conversation_id,
        )
        .await
    }

    /// One nullable text column selected by `sql` with `$1` bound to `bind`,
    /// on whichever backend the test database is.
    async fn optional_text(db: &Database, sql: &str, bind: &str) -> Option<String> {
        let row: Option<(Option<String>,)> = match db {
            Database::SQLite(sqlite) => sqlx::query_as(sql)
                .bind(bind)
                .fetch_optional(sqlite.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(pg) => sqlx::query_as(sql)
                .bind(bind)
                .fetch_optional(pg.pool())
                .await
                .unwrap(),
        };
        row.and_then(|r| r.0)
    }

    #[tokio::test]
    #[serial]
    async fn activating_a_coach_rebinds_the_live_messaging_conversation() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = MockLlm::new();
        let calls = mock.counter();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "rebind_athlete@example.com").await;
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

        let tg_secret = "rebind_tg_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(tg_secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:REBIND_BOT"),
            is_active: true,
        })
        .await
        .unwrap();

        let sender_id = "77";
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: sender_id,
            display_name: Some("Rebind Sender"),
        })
        .await
        .unwrap();

        // Turn 1 — opens the session and its one conversation, with no coach
        // selected yet.
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", tg_secret)
            .json(&json!({
                "update_id": 7001,
                "message": {
                    "message_id": 1,
                    "from": { "id": 77, "first_name": "Rebind" },
                    "chat": { "id": 77, "type": "private" },
                    "text": "Salut"
                }
            }))
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        assert!(
            wait_for_turns(&calls, 1).await,
            "first turn never reached the LLM"
        );

        let conversation_id = session_conversation_id(&resources, user_id)
            .await
            .expect("turn 1 must have opened a conversation");
        assert_eq!(
            conversation_coach(&resources, &conversation_id).await,
            None,
            "fixture precondition: no coach selected before the activation"
        );

        // The athlete picks a coach — exactly what /coaches does.
        let coach = resources
            .common
            .repos
            .coaches
            .create_system_coach(
                user_id,
                tenant_id,
                &CreateSystemCoachRequest {
                    title: "Coach Marathon".to_owned(),
                    description: None,
                    system_prompt: "Tu es un coach marathon.".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap();
        let coach_id = coach.id.to_string();
        resources
            .common
            .repos
            .coaches
            .activate_coach(&coach_id, user_id, tenant_id)
            .await
            .unwrap()
            .expect("activation must resolve the coach");

        // Turn 2 — same session, same conversation, new coach.
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", tg_secret)
            .json(&json!({
                "update_id": 7002,
                "message": {
                    "message_id": 2,
                    "from": { "id": 77, "first_name": "Rebind" },
                    "chat": { "id": 77, "type": "private" },
                    "text": "Analyse ma charge"
                }
            }))
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        assert!(
            wait_for_turns(&calls, 2).await,
            "second turn never reached the LLM"
        );

        // The thread survived: still the same conversation row.
        assert_eq!(
            session_conversation_id(&resources, user_id)
                .await
                .as_deref(),
            Some(conversation_id.as_str()),
            "the rebind must reuse the conversation, not forge a new one — forging is /reset"
        );
        // And it now belongs to the coach the athlete picked. Before the fix this
        // stayed NULL for the life of the session.
        assert_eq!(
            conversation_coach(&resources, &conversation_id).await,
            Some(coach_id.clone()),
            "the selected coach must reach the conversation already in flight"
        );
    }
}
