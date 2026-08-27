// ABOUTME: Integration tests for the group respond-mode gate — mentions mode silences ambient
// ABOUTME: chatter (captured for the room transcript) while addressed turns reach the LLM
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod respond_mode_tests {
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
    use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
    use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_messaging::channels::telegram::transport::TelegramTransport;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// Numeric bot id encoded in the fixture `bot_token` prefix.
    const BOT_ID: i64 = 12_345;
    /// Fixture bot token: the prefix before `:` is the bot id canot derives.
    const BOT_TOKEN: &str = "12345:RESPOND_MODE_BOT";
    /// Shared Telegram supergroup chat id for every test group.
    const GROUP_CHAT_ID: i64 = -100_999_777;

    /// Deterministic LLM that counts invocations and captures each request's
    /// serialized messages, so tests can assert both WHETHER the pipeline ran
    /// (the respond gate) and WHAT context it saw (the ambient transcript).
    /// Mock is test-only: the real provider chain is exercised by the
    /// messaging chat-eval suite; here the LLM boundary is the assertion
    /// point itself.
    struct CapturingLlm {
        reply: String,
        model: String,
        calls: Arc<AtomicUsize>,
        seen_requests: Arc<Mutex<Vec<String>>>,
    }

    impl CapturingLlm {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                reply: reply.into(),
                model: "mock-model".to_owned(),
                calls: Arc::new(AtomicUsize::new(0)),
                seen_requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn call_counter(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }

        fn request_log(&self) -> Arc<Mutex<Vec<String>>> {
            Arc::clone(&self.seen_requests)
        }

        fn record(&self, request: &ChatRequest) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let serialized = serde_json::to_string(&request.messages).unwrap_or_default();
            self.seen_requests.lock().unwrap().push(serialized);
        }
    }

    #[async_trait]
    impl LlmProvider for CapturingLlm {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn display_name(&self) -> &'static str {
            "Capturing mock LLM (tests)"
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
        async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
            self.record(request);
            Ok(ChatResponse {
                content: self.reply.clone(),
                model: self.model.clone(),
                usage: Some(TokenUsage::new(42, 11, 53)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }
        async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
            self.record(request);
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

    /// Create an active user owning a fresh tenant. Mirrors the
    /// cross-tenant-bot test fixture.
    async fn create_user_with_own_tenant(
        resources: &ServerContext,
        email: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("RespondMode123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Respond Mode".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Own Tenant {email}"),
            slug: format!("respond-tenant-{tenant_id}"),
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

    /// Everything a respond-mode scenario needs: bot tenant with a Telegram
    /// config, a linked + provider-connected owner, and a coaching group
    /// bound to the fixture chat in the given respond mode.
    struct Scenario {
        resources: Arc<ServerContext>,
        bot_tenant: TenantId,
        group_id: Uuid,
        tg_secret: &'static str,
    }

    async fn build_scenario(
        resources: Arc<ServerContext>,
        respond_mode: GroupRespondMode,
    ) -> Scenario {
        // Seed canot's process-wide bot-username cache so mention detection
        // never issues a live getMe call from the test suite. The factory
        // builds its own transport from the channel config, but the cache is
        // keyed by bot id (derived from BOT_TOKEN's prefix), so this seeds it
        // for every adapter in the process.
        let _seed = TelegramTransport::with_bot_identity(
            "unused-secret".to_owned(),
            BOT_ID,
            "dravr_respond_mode_bot",
        );

        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (owner_user_id, owner_tenant) =
            create_user_with_own_tenant(&resources, "respond_owner@example.com").await;
        resources
            .common
            .repos
            .provider_connections
            .register_connection(
                owner_user_id,
                owner_tenant,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();

        let (_bot_owner, bot_tenant) =
            create_user_with_own_tenant(&resources, "respond_bot_owner@example.com").await;

        let tg_secret = "respond_mode_tg_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(tg_secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some(BOT_TOKEN),
            is_active: true,
        })
        .await
        .unwrap();

        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            user_id: &owner_user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: "42",
            display_name: Some("Respond Owner"),
        })
        .await
        .unwrap();

        // Coach row (FK for the group). `create_test_server_resources` does
        // not auto-seed coaches.
        let coach = resources
            .common
            .repos
            .coaches
            .create_system_coach(
                owner_user_id,
                bot_tenant,
                &CreateSystemCoachRequest {
                    title: "Respond Mode Coach".to_owned(),
                    description: None,
                    system_prompt: "You are a concise test coach.".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap();

        // Coaching group pre-bound to the fixture chat, under the BOT tenant
        // (group sessions live there). Direct row creation keeps the test
        // independent of the auto-bind coach-selection path.
        let group_id = Uuid::new_v4();
        let now = Utc::now();
        let group = CoachingGroup {
            id: group_id,
            tenant_id: bot_tenant.to_string(),
            name: "Respond Mode Test Group".to_owned(),
            description: None,
            coach_id: coach.id.to_string(),
            owner_id: owner_user_id,
            coach_user_id: None,
            peer_data_sharing: true,
            respond_mode,
            max_members: 10,
            is_active: true,
            channel_type: Some("telegram".to_owned()),
            channel_chat_id: Some(GROUP_CHAT_ID.to_string()),
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .groups
            .create_group(bot_tenant, &group)
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id: owner_user_id,
                tenant_id: bot_tenant.to_string(),
                role: GroupRole::Owner,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();

        Scenario {
            resources,
            bot_tenant,
            group_id,
            tg_secret,
        }
    }

    /// POST a Telegram group-chat webhook update built from `message_fields`.
    async fn post_group_message(
        scenario: &Scenario,
        update_id: i64,
        message: serde_json::Value,
    ) -> StatusCode {
        let router = MessagingRoutes::routes(Arc::clone(&scenario.resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", scenario.tg_secret)
            .json(&json!({ "update_id": update_id, "message": message }))
            .send(router)
            .await;
        resp.status_code()
    }

    fn group_text_message(message_id: i64, from_id: i64, text: &str) -> serde_json::Value {
        json!({
            "message_id": message_id,
            "from": { "id": from_id, "first_name": "Member" },
            "chat": { "id": GROUP_CHAT_ID, "type": "supergroup", "title": "Respond Mode Test Group" },
            "text": text
        })
    }

    async fn wait_for_llm_calls(calls: &Arc<AtomicUsize>, at_least: usize) -> bool {
        for _ in 0..50 {
            if calls.load(Ordering::SeqCst) >= at_least {
                return true;
            }
            sleep(Duration::from_millis(200)).await;
        }
        false
    }

    async fn count_inbound_with_body(pool: &sqlx::SqlitePool, body: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM messaging_messages WHERE direction = 'inbound' AND content_body = ?1",
        )
        .bind(body.to_owned())
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn mentions_mode_silences_ambient_and_injects_transcript_on_addressed_turn() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = Arc::new(CapturingLlm::new("On en parle."));
        let calls = mock.call_counter();
        let requests = mock.request_log();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources), GroupRespondMode::Mentions).await;

        let ambient_text = "nice tempo run everyone, felt easy today";

        // 1. Unaddressed group message from the LINKED owner: no LLM turn, no
        //    reply — but the row is captured for the room transcript.
        let status =
            post_group_message(&scenario, 9101, group_text_message(11, 42, ambient_text)).await;
        assert_eq!(status, StatusCode::OK);
        sleep(Duration::from_millis(1500)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "mentions mode must not dispatch an unaddressed group message to the LLM"
        );

        let pool = scenario
            .resources
            .coach
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");
        assert_eq!(
            count_inbound_with_body(pool, ambient_text).await,
            1,
            "ambient message must be stored for the room transcript"
        );

        // 2. Unaddressed message from an UNLINKED sender: silently dropped —
        //    no link prompt, no stored row, still no LLM turn.
        let unlinked_text = "I am not linked and just chatting";
        let status =
            post_group_message(&scenario, 9102, group_text_message(12, 77, unlinked_text)).await;
        assert_eq!(status, StatusCode::OK);
        sleep(Duration::from_millis(800)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            count_inbound_with_body(pool, unlinked_text).await,
            0,
            "an unlinked sender's ambient chatter must not be stored"
        );

        // 3. ADDRESSED message (a reply to one of the bot's messages) from the
        //    linked owner: the pipeline runs, and its prompt carries the
        //    ambient transcript captured in step 1.
        let status = post_group_message(
            &scenario,
            9103,
            json!({
                "message_id": 13,
                "from": { "id": 42, "first_name": "Member" },
                "chat": { "id": GROUP_CHAT_ID, "type": "supergroup", "title": "Respond Mode Test Group" },
                "text": "what do you think of that session?",
                "reply_to_message": {
                    "message_id": 5,
                    "from": { "id": BOT_ID, "is_bot": true, "first_name": "Dravr" },
                    "text": "Solid week so far."
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            wait_for_llm_calls(&calls, 1).await,
            "an addressed group message must reach the LLM in mentions mode"
        );

        let captured = requests.lock().unwrap().join("\n");
        assert!(
            captured.contains("Recent group chat"),
            "addressed group turn must carry the ambient-transcript block"
        );
        assert!(
            captured.contains(ambient_text),
            "the ambient message stored in step 1 must appear in the transcript"
        );
    }

    #[tokio::test]
    #[serial]
    async fn all_mode_still_answers_unaddressed_group_messages() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = Arc::new(CapturingLlm::new("Bien reçu."));
        let calls = mock.call_counter();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources), GroupRespondMode::All).await;

        let status = post_group_message(
            &scenario,
            9201,
            group_text_message(21, 42, "how is my training load looking?"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            wait_for_llm_calls(&calls, 1).await,
            "all mode (the default) must keep answering unaddressed group messages"
        );
    }

    #[tokio::test]
    #[serial]
    async fn group_respond_command_flips_mode_and_enforces_roles() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = Arc::new(CapturingLlm::new("Jamais appelé."));
        let calls = mock.call_counter();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources), GroupRespondMode::All).await;
        let db: &dyn MessagingRepository = &*scenario.resources.common.repos.messaging;

        // A second linked user who will be auto-enrolled as a plain Member.
        let (member_user_id, member_tenant) =
            create_user_with_own_tenant(&scenario.resources, "respond_member@example.com").await;
        scenario
            .resources
            .common
            .repos
            .provider_connections
            .register_connection(
                member_user_id,
                member_tenant,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: scenario.bot_tenant,
            user_id: &member_user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: "43",
            display_name: Some("Respond Member"),
        })
        .await
        .unwrap();

        let respond_mode_in_db = || async {
            scenario
                .resources
                .common
                .repos
                .groups
                .get_group(&scenario.group_id.to_string(), scenario.bot_tenant)
                .await
                .unwrap()
                .expect("group row must exist")
                .respond_mode
        };

        // A plain Member may not change the mode.
        let status = post_group_message(
            &scenario,
            9301,
            group_text_message(31, 43, "/group respond mentions"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            respond_mode_in_db().await,
            GroupRespondMode::All,
            "a plain member must not be able to flip the respond mode"
        );

        // The Owner flips to mentions.
        let status = post_group_message(
            &scenario,
            9302,
            group_text_message(32, 42, "/group respond mentions"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(respond_mode_in_db().await, GroupRespondMode::Mentions);

        // ESCAPE HATCH: with the group now in mentions mode, the Owner's
        // UNADDRESSED slash command must still be honored — otherwise the
        // mode could never be reverted from inside the chat.
        let status = post_group_message(
            &scenario,
            9303,
            group_text_message(33, 42, "/group respond all"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            respond_mode_in_db().await,
            GroupRespondMode::All,
            "an unaddressed /group respond must work in mentions mode (escape hatch)"
        );

        // Command turns never reach the LLM.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
