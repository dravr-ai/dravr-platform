// ABOUTME: The registre#9 pin — one turn increments exactly one message counter, on the ATHLETE's tenant
// ABOUTME: Driven on both surfaces, because the bypass existed precisely where only one of them ran

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What messaging bypassing every usage cap looked like, and why it lasted.
//!
//! Enforcement lived in the web handler. The messaging webhook ran its own
//! ladder and simply never called it, so the caps had "one call site" and were
//! unenforced on a whole surface for four months (registre#9). When recording
//! was finally added it went under the *bot's* tenant — the tenant that owns
//! the webhook — while every read was scoped to the athlete's own, so the
//! counters accumulated somewhere nothing looked.
//!
//! Both halves of that are asserted here by value rather than by the existence
//! of a trait:
//!
//! - a Telegram turn, driven through the real webhook with the bot on a
//!   *different* tenant from the athlete, leaves `daily_messages` at exactly 1
//!   under the athlete's tenant and exactly 0 under the bot's;
//! - a web turn through the same
//!   [`pierre_chat_pipeline::turn_service::execute`] leaves it at exactly 1
//!   too.
//!
//! Exactly 1, not "at least 1": a turn that runs two ladders increments twice,
//! and a floor assertion would pass while it did.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod turn_service_quota_tests {
    use crate::common::create_test_server_resources_with_llm;
    use crate::helpers::axum_test::AxumTestRequest;
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use chrono::Utc;
    use futures_util::stream;
    use pierre_chat_pipeline::{
        CommandPersistence, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile, SurfaceRequest,
        TurnOrigin, TurnRequest,
    };
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
        TokenUsage,
    };
    use pierre_core::models::ConnectionType;
    use pierre_core::models::{ConversationTurnId, Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::factory::Database;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_runtime_context::default_admin_config;
    use pierre_services::usage_counter::UsageCounterService;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// Deterministic coach: one short reply, real token counts, no tools.
    struct CountingMockProvider;

    #[async_trait]
    impl LlmProvider for CountingMockProvider {
        fn name(&self) -> &'static str {
            "counting_mock"
        }
        fn display_name(&self) -> &'static str {
            "Counting Mock LLM (quota pin)"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::SYSTEM_MESSAGES
        }
        fn default_model(&self) -> &'static str {
            "mock-model"
        }
        fn available_models(&self) -> &[String] {
            &[]
        }

        async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
            Ok(ChatResponse {
                content: "Ta semaine est bien dosée: garde le volume et dors davantage.".to_owned(),
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage::new(25, 15, 40)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            let chunk = StreamChunk {
                delta: String::new(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            };
            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    /// Create an active athlete owning their own tenant, with a synthetic
    /// provider connection so the turn clears the onboarding gate.
    async fn create_athlete(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("QuotaPin123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Quota Pin".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = create_tenant(resources, user_id, &format!("athlete-{email}")).await;
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

    /// Create a tenant owned by `owner`.
    async fn create_tenant(resources: &Arc<ServerContext>, owner: Uuid, label: &str) -> TenantId {
        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Tenant {label}"),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner,
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
        tenant_id
    }

    /// Read one counter's current value for `(tenant, user)`.
    async fn counter(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
        user_id: Uuid,
        counter_type: &str,
    ) -> i64 {
        UsageCounterService::new(
            resources.common.repos.usage_counters.as_ref(),
            default_admin_config(),
        )
        .get_current(&tenant_id.to_string(), &user_id.to_string(), counter_type)
        .await
        .unwrap()
    }

    /// Poll until the counter reaches `expected`, returning the last value
    /// read. Turn accounting settles after the assistant row is persisted, so
    /// the row's presence does not yet mean the spend is recorded.
    async fn wait_for_counter(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
        user_id: Uuid,
        counter_type: &str,
        expected: i64,
    ) -> i64 {
        let mut latest = 0;
        for _ in 0..150 {
            latest = counter(resources, tenant_id, user_id, counter_type).await;
            if latest == expected {
                return latest;
            }
            sleep(Duration::from_millis(200)).await;
        }
        latest
    }

    /// Poll `chat_messages` until the turn's assistant row lands under
    /// `tenant_id`, so the counter assertions read a finished turn.
    async fn wait_for_assistant_row(resources: &Arc<ServerContext>, tenant_id: TenantId) -> bool {
        const SQL: &str = "SELECT COUNT(*) FROM chat_messages m \
                 JOIN chat_conversations c ON m.conversation_id = c.id \
                 WHERE c.tenant_id = $1 AND m.role = 'assistant'";
        let tenant = tenant_id.to_string();
        for _ in 0..150 {
            let count: i64 = match resources.coach.database.as_ref() {
                Database::SQLite(db) => sqlx::query_scalar(SQL)
                    .bind(&tenant)
                    .fetch_one(db.pool())
                    .await
                    .unwrap(),
                #[cfg(feature = "postgresql")]
                Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                    .bind(&tenant)
                    .fetch_one(db.pool())
                    .await
                    .unwrap(),
            };
            if count > 0 {
                return true;
            }
            sleep(Duration::from_millis(200)).await;
        }
        false
    }

    /// A Telegram turn spends the ATHLETE's budget, never the bot's.
    ///
    /// The bot's webhook, config and channel link all live on a tenant the
    /// athlete does not belong to — the shared-bot topology registre#9 hid in.
    #[tokio::test]
    #[serial]
    async fn telegram_turn_increments_one_message_on_the_athletes_tenant() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");

        let resources = create_test_server_resources_with_llm(Arc::new(CountingMockProvider))
            .await
            .unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (athlete_id, athlete_tenant) =
            create_athlete(&resources, "telegram-quota-pin@example.com").await;
        let (bot_owner_id, _) = create_athlete(&resources, "telegram-bot-owner@example.com").await;
        let bot_tenant = create_tenant(&resources, bot_owner_id, "bot").await;
        assert_ne!(
            bot_tenant, athlete_tenant,
            "the pin is meaningless unless the bot and the athlete are on different tenants"
        );

        let secret = "telegram_quota_pin_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            channel_type: "telegram",
            api_key: Some("tg-quota-pin"),
            api_secret: None,
            webhook_secret: Some(secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("tg-bot-token"),
            is_active: true,
        })
        .await
        .unwrap();

        let chat_id = "778899";
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            user_id: &athlete_id.to_string(),
            channel_type: "telegram",
            channel_user_id: chat_id,
            display_name: Some("Quota Pin"),
        })
        .await
        .unwrap();

        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let response = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", secret)
            .json(&json!({
                "update_id": 5150,
                "message": {
                    "message_id": 1,
                    "date": Utc::now().timestamp(),
                    "chat": { "id": chat_id.parse::<i64>().unwrap(), "type": "private" },
                    "from": { "id": chat_id.parse::<i64>().unwrap(), "is_bot": false },
                    "text": "Comment se présente ma semaine d'entraînement?"
                }
            }))
            .send(router)
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "webhooks always ack"
        );

        assert!(
            wait_for_assistant_row(&resources, athlete_tenant).await,
            "the Telegram turn never produced an assistant row within 30s"
        );

        assert_eq!(
            wait_for_counter(&resources, athlete_tenant, athlete_id, "daily_messages", 1).await,
            1,
            "one Telegram turn must spend exactly one daily message on the athlete's own tenant"
        );
        assert_eq!(
            wait_for_counter(&resources, athlete_tenant, athlete_id, "weekly_messages", 1).await,
            1,
            "and exactly one weekly message on the same tenant"
        );
        assert_eq!(
            counter(&resources, bot_tenant, athlete_id, "daily_messages").await,
            0,
            "nothing may land on the bot's tenant — a budget nothing reads is a budget nothing \
             enforces (registre#9)"
        );
        assert_eq!(
            counter(&resources, bot_tenant, athlete_id, "weekly_messages").await,
            0,
            "same for the weekly counter"
        );
    }

    /// A web turn spends the same counter, through the same service.
    #[tokio::test]
    #[serial]
    async fn web_turn_increments_one_message_on_the_athletes_tenant() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");

        let resources = create_test_server_resources_with_llm(Arc::new(CountingMockProvider))
            .await
            .unwrap();
        let (athlete_id, athlete_tenant) =
            create_athlete(&resources, "web-quota-pin@example.com").await;

        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(
                &athlete_id.to_string(),
                athlete_tenant,
                "quota pin",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap();

        // The same profile the web handler resolves: no transport behind it.
        let profile = SurfaceProfile::resolve(&SurfaceRequest {
            surface: SurfaceId::Web,
            locale: "fr".to_owned(),
            transport: None,
            prose_contract: None,
        });
        let ctx = resources.chat_pipeline_context();
        let served = pierre_chat_pipeline::execute(
            &ctx,
            TurnRequest {
                origin: TurnOrigin::Athlete,
                conversation_id: conversation.id.clone(),
                user_id: athlete_id,
                conversation_tenant_id: athlete_tenant,
                tool_tenant_id: athlete_tenant,
                content: "Comment se présente ma semaine d'entraînement?".to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                channel_type: "web",
                is_direct_message: true,
                ambient_group_fallback: false,
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                hooks: PipelineHooks::none(),
            },
            &profile,
        )
        .await
        .expect("the web turn must be admitted and served");

        let ServedTurn::Pipeline(envelope) = served else {
            panic!("a plain-prose web turn must run the pipeline");
        };
        assert!(
            !envelope.assistant.prose().trim().is_empty(),
            "the served turn must carry the coach's reply"
        );
        assert_eq!(
            envelope.locale, "fr",
            "the turn's own locale rides back out on the envelope"
        );

        assert_eq!(
            counter(&resources, athlete_tenant, athlete_id, "daily_messages").await,
            1,
            "one web turn must spend exactly one daily message"
        );
        assert_eq!(
            counter(&resources, athlete_tenant, athlete_id, "weekly_messages").await,
            1,
            "and exactly one weekly message"
        );
        assert_eq!(
            counter(&resources, athlete_tenant, athlete_id, "daily_tokens").await,
            40,
            "the provider's own token counts are what the budget is charged"
        );
    }
}
