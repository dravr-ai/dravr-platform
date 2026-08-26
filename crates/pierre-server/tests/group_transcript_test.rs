// ABOUTME: Integration tests for the surface-neutral group room transcript read model
// ABOUTME: Messaging turns readable by web members, web turns visible to messaging, consent withheld
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod group_transcript_tests {
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
    use pierre_core::models::groups::{
        CoachingGroup, GroupMember, GroupRespondMode, GroupRole, GroupTranscriptEntry,
        TranscriptSpeaker,
    };
    use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::chat::ChatRoutes;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_messaging::channels::telegram::transport::TelegramTransport;
    use serde_json::{json, Value};
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// Numeric bot id encoded in the fixture `bot_token` prefix.
    const BOT_ID: i64 = 54_321;
    /// Fixture bot token: the prefix before `:` is the bot id canot derives.
    const BOT_TOKEN: &str = "54321:GROUP_TRANSCRIPT_BOT";
    /// Shared Telegram supergroup chat id for the fixture group.
    const GROUP_CHAT_ID: i64 = -100_888_555;

    /// What Alice types from Telegram; must surface verbatim to a web member.
    const ALICE_MESSAGE: &str = "Je prépare le marathon de Montréal en mai";
    /// The mock coach reply; must surface as a `coach` transcript entry.
    const COACH_REPLY: &str = "Bonne base Alice, on structure le plan ensemble.";
    /// What Bob types from the web; must reach a Telegram member's prompt.
    const BOB_MESSAGE: &str = "Semaine chargée: 60 km de course au total";
    /// Carol has not consented — this content must never fan out to others.
    const CAROL_MESSAGE: &str = "je fais juste 5 km demain matin";

    /// Deterministic LLM that counts invocations and captures each request's
    /// serialized messages. Mock is test-only: the assertion point here is
    /// the transcript read model and the prompt context, not model output.
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

    /// Create an active user owning a fresh tenant, with a provider
    /// connection so the onboarding gate never fires before the turn.
    async fn create_user_with_own_tenant(
        resources: &ServerContext,
        email: &str,
        display_name: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Transcript123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some(display_name.to_owned()),
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
            slug: format!("transcript-tenant-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
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

    /// Everything a transcript scenario needs: bot tenant with a Telegram
    /// config, linked members alice (consented) + carol (not consented), a
    /// web-only member bob (consented), a non-member dave, and the coaching
    /// group bound to the fixture chat.
    struct Scenario {
        resources: Arc<ServerContext>,
        group_id: Uuid,
        tg_secret: &'static str,
        alice_id: Uuid,
        bob_id: Uuid,
        bob_auth: String,
        carol_id: Uuid,
        dave_auth: String,
    }

    async fn build_scenario(resources: Arc<ServerContext>) -> Scenario {
        // Seed canot's process-wide bot-username cache so mention detection
        // never issues a live getMe call from the test suite.
        let _seed = TelegramTransport::with_bot_identity(
            "unused-secret".to_owned(),
            BOT_ID,
            "dravr_transcript_bot",
        );

        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (alice_id, _alice_tenant) =
            create_user_with_own_tenant(&resources, "transcript_alice@example.com", "Alice").await;
        let (bob_id, _bob_tenant) =
            create_user_with_own_tenant(&resources, "transcript_bob@example.com", "Bob").await;
        let (carol_id, _carol_tenant) =
            create_user_with_own_tenant(&resources, "transcript_carol@example.com", "Carol").await;
        let (dave_id, _dave_tenant) =
            create_user_with_own_tenant(&resources, "transcript_dave@example.com", "Dave").await;

        let (_bot_owner, bot_tenant) =
            create_user_with_own_tenant(&resources, "transcript_bot_owner@example.com", "Bot")
                .await;

        let tg_secret = "group_transcript_tg_secret";
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

        for (user_id, sender, name) in [(alice_id, "42", "Alice"), (carol_id, "43", "Carol")] {
            db.create_channel_link(&CreateChannelLinkParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: bot_tenant,
                user_id: &user_id.to_string(),
                channel_type: "telegram",
                channel_user_id: sender,
                display_name: Some(name),
            })
            .await
            .unwrap();
        }

        // Coach row (FK for the group).
        let coach = resources
            .common
            .repos
            .coaches
            .create_system_coach(
                alice_id,
                bot_tenant,
                &CreateSystemCoachRequest {
                    title: "Transcript Coach".to_owned(),
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
        // (group sessions live there). Mentions mode so unaddressed chatter is
        // captured for the room without dispatching a turn.
        let group_id = Uuid::new_v4();
        let now = Utc::now();
        let group = CoachingGroup {
            id: group_id,
            tenant_id: bot_tenant.to_string(),
            name: "Transcript Test Group".to_owned(),
            description: None,
            coach_id: coach.id.to_string(),
            owner_id: alice_id,
            coach_user_id: None,
            peer_data_sharing: true,
            respond_mode: GroupRespondMode::Mentions,
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

        for (user_id, role, consent) in [
            (alice_id, GroupRole::Owner, true),
            (bob_id, GroupRole::Member, true),
            (carol_id, GroupRole::Member, false),
        ] {
            resources
                .common
                .repos
                .groups
                .add_member(&GroupMember {
                    id: Uuid::new_v4(),
                    group_id,
                    user_id,
                    tenant_id: bot_tenant.to_string(),
                    role,
                    peer_sharing_consent: consent,
                    consent_given_at: now,
                    joined_at: now,
                    left_at: None,
                    display_name: None,
                })
                .await
                .unwrap();
        }

        let bob_user = resources
            .common
            .repos
            .users
            .get_global(bob_id)
            .await
            .unwrap()
            .unwrap();
        let bob_auth = format!(
            "Bearer {}",
            resources
                .auth
                .auth_manager
                .generate_token(&bob_user, &resources.auth.jwks_manager)
                .unwrap()
        );
        let dave_user = resources
            .common
            .repos
            .users
            .get_global(dave_id)
            .await
            .unwrap()
            .unwrap();
        let dave_auth = format!(
            "Bearer {}",
            resources
                .auth
                .auth_manager
                .generate_token(&dave_user, &resources.auth.jwks_manager)
                .unwrap()
        );

        Scenario {
            resources,
            group_id,
            tg_secret,
            alice_id,
            bob_id,
            bob_auth,
            carol_id,
            dave_auth,
        }
    }

    /// POST a Telegram group-chat webhook update built from `message`.
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

    /// A group message addressed to the bot (a reply to one of its messages),
    /// which dispatches a turn even in mentions mode.
    fn addressed_message(message_id: i64, from_id: i64, text: &str) -> serde_json::Value {
        json!({
            "message_id": message_id,
            "from": { "id": from_id, "first_name": "Member" },
            "chat": { "id": GROUP_CHAT_ID, "type": "supergroup", "title": "Transcript Test Group" },
            "text": text,
            "reply_to_message": {
                "message_id": 1,
                "from": { "id": BOT_ID, "is_bot": true, "first_name": "Dravr" },
                "text": "Bonjour le groupe."
            }
        })
    }

    /// An unaddressed group message — ambient room chatter.
    fn ambient_message(message_id: i64, from_id: i64, text: &str) -> serde_json::Value {
        json!({
            "message_id": message_id,
            "from": { "id": from_id, "first_name": "Member" },
            "chat": { "id": GROUP_CHAT_ID, "type": "supergroup", "title": "Transcript Test Group" },
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

    /// Poll the repository until the viewer sees at least `at_least` entries
    /// (background dispatch lands them asynchronously). Returns the entries.
    async fn wait_for_entries(
        scenario: &Scenario,
        viewer: Uuid,
        at_least: usize,
    ) -> Vec<GroupTranscriptEntry> {
        let group_id = scenario.group_id.to_string();
        for _ in 0..50 {
            let entries = scenario
                .resources
                .common
                .repos
                .groups
                .list_transcript_visible_to(&group_id, viewer, 100)
                .await
                .unwrap();
            if entries.len() >= at_least {
                return entries;
            }
            sleep(Duration::from_millis(200)).await;
        }
        panic!("transcript never reached {at_least} entries for viewer {viewer}");
    }

    /// GET the transcript route as the given caller.
    async fn get_transcript(scenario: &Scenario, auth: &str) -> (StatusCode, Value) {
        let router = ChatRoutes::routes(Arc::clone(&scenario.resources));
        let resp = AxumTestRequest::get(&format!(
            "/api/chat/groups/{}/transcript",
            scenario.group_id
        ))
        .header("authorization", auth)
        .send(router)
        .await;
        let status = resp.status_code();
        let body = if status == StatusCode::OK {
            resp.json::<Value>()
        } else {
            Value::Null
        };
        (status, body)
    }

    #[tokio::test]
    #[serial]
    async fn messaging_turn_is_readable_by_web_member_and_consent_withholds_content() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = Arc::new(CapturingLlm::new(COACH_REPLY));
        let calls = mock.call_counter();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;

        // 1. Alice, addressed, from Telegram: a full coaching turn runs and
        //    fans both sides of it out to the shared room transcript.
        let status =
            post_group_message(&scenario, 7001, addressed_message(10, 42, ALICE_MESSAGE)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            wait_for_llm_calls(&calls, 1).await,
            "addressed group message must dispatch a turn"
        );
        wait_for_entries(&scenario, scenario.bob_id, 2).await;

        // 2. Carol, unaddressed, not consented: captured for the room, but
        //    her content must not fan out to other members.
        let status =
            post_group_message(&scenario, 7002, ambient_message(11, 43, CAROL_MESSAGE)).await;
        assert_eq!(status, StatusCode::OK);
        // Carol always sees her own entry — proves the row was captured, so
        // its absence for Bob below is withholding, not data loss.
        let carol_view = wait_for_entries(&scenario, scenario.carol_id, 3).await;
        assert!(
            carol_view
                .iter()
                .any(|e| e.content == CAROL_MESSAGE && e.author_user_id == scenario.carol_id),
            "carol must see her own ambient message in the room"
        );

        // 3. Bob — a member who has never touched Telegram — reads the room
        //    over REST and sees Alice's exchange verbatim.
        let (status, body) = get_transcript(&scenario, &scenario.bob_auth).await;
        assert_eq!(status, StatusCode::OK);
        let entries = body["entries"].as_array().unwrap();
        assert_eq!(
            entries.len(),
            2,
            "bob sees alice's turn (member + coach) and nothing from unconsented carol"
        );
        assert_eq!(entries[0]["speaker"], "member");
        assert_eq!(entries[0]["content"], ALICE_MESSAGE);
        assert_eq!(entries[0]["author_user_id"], scenario.alice_id.to_string());
        assert_eq!(entries[1]["speaker"], "coach");
        assert_eq!(entries[1]["content"], COACH_REPLY);
        assert_eq!(
            entries[1]["author_user_id"],
            scenario.alice_id.to_string(),
            "the coach reply is attributed to the member it answered"
        );
        assert!(
            !body.to_string().contains(CAROL_MESSAGE),
            "an unconsented member's content must not fan out to others"
        );

        // 4. Carol's membership stays visible even while her content is
        //    withheld.
        let members = body["members"].as_array().unwrap();
        let carol_row = members
            .iter()
            .find(|m| m["user_id"] == scenario.carol_id.to_string())
            .expect("carol appears in the roster");
        assert_eq!(carol_row["peer_sharing_consent"], false);
        let alice_row = members
            .iter()
            .find(|m| m["user_id"] == scenario.alice_id.to_string())
            .expect("alice appears in the roster");
        assert_eq!(alice_row["peer_sharing_consent"], true);
        assert_eq!(
            alice_row["display_name"], "transcript_alice@example.com",
            "roster display names come from the members listing source"
        );

        // 5. A non-member gets nothing — not even the roster.
        let (status, _) = get_transcript(&scenario, &scenario.dave_auth).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a non-member must be refused the room transcript"
        );
    }

    #[tokio::test]
    #[serial]
    async fn web_turn_reaches_the_room_and_a_messaging_member_prompt() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = Arc::new(CapturingLlm::new(COACH_REPLY));
        let calls = mock.call_counter();
        let requests = mock.request_log();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;

        // 1. Bob opens a group-bound conversation on the web surface and
        //    sends a message through the ordinary chat endpoint.
        let chat_router = ChatRoutes::routes(Arc::clone(&resources));
        let conv_resp = AxumTestRequest::post("/api/chat/conversations")
            .header("authorization", &scenario.bob_auth)
            .json(&json!({
                "title": "Squad room",
                "group_id": scenario.group_id.to_string(),
            }))
            .send(chat_router.clone())
            .await;
        assert_eq!(conv_resp.status_code(), StatusCode::CREATED);
        let conv_id = conv_resp.json::<Value>()["id"].as_str().unwrap().to_owned();

        let msg_resp =
            AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
                .header("authorization", &scenario.bob_auth)
                .json(&json!({ "content": BOB_MESSAGE }))
                .send(chat_router)
                .await;
        assert_eq!(msg_resp.status_code(), StatusCode::OK);

        // 2. Alice — bound through Telegram — can read Bob's web message in
        //    the same room.
        let alice_view = wait_for_entries(&scenario, scenario.alice_id, 2).await;
        let bob_entry = alice_view
            .iter()
            .find(|e| e.author_user_id == scenario.bob_id && e.speaker == TranscriptSpeaker::Member)
            .expect("bob's web message reaches the shared room");
        assert_eq!(bob_entry.content, BOB_MESSAGE);

        // 3. Alice's next addressed Telegram turn carries Bob's web message
        //    as ambient room context in its prompt.
        let status = post_group_message(
            &scenario,
            7101,
            addressed_message(20, 42, "On compare nos semaines?"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            wait_for_llm_calls(&calls, 2).await,
            "alice's addressed turn must dispatch (the web turn was the first call)"
        );
        // The call counter alone can be satisfied by auxiliary LLM calls
        // (memory extraction shares the provider); the turn is only done once
        // alice's own member + coach rows join bob's two in the room.
        wait_for_entries(&scenario, scenario.alice_id, 4).await;
        let captured = requests.lock().unwrap().join("\n");
        assert!(
            captured.contains("Recent group chat"),
            "the addressed group turn must carry the ambient-transcript block"
        );
        assert!(
            captured.contains(BOB_MESSAGE),
            "bob's web message must appear in alice's messaging prompt context"
        );
    }
}
