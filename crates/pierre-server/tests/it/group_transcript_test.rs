// ABOUTME: Integration tests for the surface-neutral group room transcript read model and the room a group thread renders
// ABOUTME: Another member's turns and chatter reach a linked member's room read in order; a withheld entry stays a placeholder
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "client-messaging")]
mod group_transcript_tests {
    use crate::common::create_test_server_resources_with_llm;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::offline_channel::{name_fixture_bot, OfflineSendAdapters};
    use crate::helpers::room_reads;
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use dravr_tronc::dates::Timestamp;
    use embacle::types::ToolCallRequest;
    use futures_util::stream;
    use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_ROOM_REPLY_HELD};
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
        StreamChunk, TokenUsage,
    };
    use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
    use pierre_core::models::groups::{
        CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
        GroupTranscriptEntry, NewGroupTranscriptEntry, TranscriptSpeaker, UpdateGroupRequest,
    };
    use pierre_core::models::messaging::MessageContent;
    use pierre_core::models::{
        AddMessageParams, CarriedRead, ConnectionType, Tenant, TenantId, User, UserStatus,
    };
    use pierre_core::transport::TransportPolicy;
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_database::repositories::SubjectThreads;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::chat::ChatRoutes;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::{json, Value};
    use serial_test::serial;
    use std::collections::BTreeSet;
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
    /// The mock agent reply; must surface as a `agent` transcript entry.
    const AGENT_REPLY: &str = "Bonne base Alice, on structure le plan ensemble.";
    /// What Bob types from the web; must reach a Telegram member's prompt.
    const BOB_MESSAGE: &str = "Semaine chargée: 60 km de course au total";
    /// Carol has not consented — this content must never fan out to others.
    const CAROL_MESSAGE: &str = "je fais juste 5 km demain matin";
    /// Alice's unaddressed room chatter, captured for the room without a turn.
    const ALICE_AMBIENT: &str = "Quelqu'un pour le fractionné de samedi?";
    /// The group's human coach, speaking to the room.
    const COACH_MESSAGE: &str = "montre moi le plan de la semaine";
    /// The agent's reply to the human coach.
    const COACH_AGENT_REPLY: &str = "Voici la semaine: trois sorties, une longue samedi.";
    /// What Carol, who shares with the coach alone, posts in the app's room.
    const CAROL_PRIVATE_POST: &str = "CAROL-PRIVATE-HRV-31 mon genou me fait mal";
    /// The agent's reply to whatever carries Carol's post: her data, restated.
    const CAROL_PRIVATE_REPLY: &str = "CAROL-PRIVATE-ECHO: repos pour le genou, HRV 31.";
    /// What the group's coach writes in the app's room.
    const COACH_INAPP_POST: &str = "COACH-INAPP-POST comment va tout le monde?";
    /// The agent's reply to that, drawn on the coach's view of every member.
    const COACH_INAPP_REPLY: &str = "COACH-INAPP-REPLY: Carol a mal au genou, Alice va bien.";

    /// Deterministic LLM that counts invocations and captures each request's
    /// serialized messages. Mock is test-only: the assertion point here is
    /// the transcript read model and the prompt context, not model output.
    /// Each request that declared tools: its messages, serialized, and the
    /// names of the tools it declared.
    type DeclaredLog = Arc<Mutex<Vec<(String, Vec<String>)>>>;

    struct CapturingLlm {
        reply: String,
        model: String,
        calls: Arc<AtomicUsize>,
        seen_requests: Arc<Mutex<Vec<String>>>,
        /// Each request that declared tools: its messages, serialized, and
        /// the names of the tools it declared.
        declared: DeclaredLog,
        /// A member a turn's first request reads by name with
        /// `get_group_member_activities`, when set; the reply follows the
        /// tool's result.
        reads_member: Arc<Mutex<Option<String>>>,
    }

    impl CapturingLlm {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                reply: reply.into(),
                model: "mock-model".to_owned(),
                calls: Arc::new(AtomicUsize::new(0)),
                seen_requests: Arc::new(Mutex::new(Vec::new())),
                declared: Arc::new(Mutex::new(Vec::new())),
                reads_member: Arc::new(Mutex::new(None)),
            }
        }

        fn member_reader(&self) -> Arc<Mutex<Option<String>>> {
            Arc::clone(&self.reads_member)
        }

        fn call_counter(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }

        fn request_log(&self) -> Arc<Mutex<Vec<String>>> {
            Arc::clone(&self.seen_requests)
        }

        fn declared_log(&self) -> DeclaredLog {
            Arc::clone(&self.declared)
        }

        /// The reply to `request`: Carol's data restated whenever anything
        /// in it carries her post, as a model restates what it read; the
        /// coach's in-app answer to the coach's in-app question; else the
        /// fixed reply.
        fn reply_to(&self, request: &ChatRequest) -> String {
            if request
                .messages
                .iter()
                .any(|m| m.content.contains(CAROL_PRIVATE_POST))
            {
                return CAROL_PRIVATE_REPLY.to_owned();
            }
            let asked = request
                .messages
                .iter()
                .rev()
                .find(|m| m.role == MessageRole::User)
                .is_some_and(|m| m.content.contains(COACH_INAPP_POST));
            if asked {
                return COACH_INAPP_REPLY.to_owned();
            }
            self.reply.clone()
        }

        fn record(&self, request: &ChatRequest) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let serialized = serde_json::to_string(&request.messages).unwrap_or_default();
            if let Some(tools) = request.tools.as_ref().filter(|tools| !tools.is_empty()) {
                self.declared.lock().unwrap().push((
                    serialized.clone(),
                    tools.iter().map(|tool| tool.name.clone()).collect(),
                ));
            }
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
            let reply = self.reply_to(request);
            let answered = request.messages.iter().any(|m| {
                m.content
                    .contains("[Tool Result for get_group_member_activities]")
            });
            let declares = request
                .tools
                .as_ref()
                .is_some_and(|tools| !tools.is_empty());
            let read = self.reads_member.lock().unwrap().clone();
            let tool_calls = read.filter(|_| declares && !answered).map(|member| {
                vec![ToolCallRequest {
                    id: "call-member".to_owned(),
                    function_name: "get_group_member_activities".to_owned(),
                    arguments: json!({ "member": member }),
                }]
            });
            Ok(ChatResponse {
                replay_blocks: None,
                content: if tool_calls.is_some() {
                    String::new()
                } else {
                    reply
                },
                model: self.model.clone(),
                usage: Some(TokenUsage::new(42, 11, 53)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls,
            })
        }
        async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
            self.record(request);
            let chunk = StreamChunk {
                delta: self.reply_to(request),
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
        user.approved_at = Some(Timestamp::now());
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
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
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
        /// Builds the adapters this scenario's webhooks dispatch through, and
        /// holds every message they would have delivered.
        adapters: Arc<OfflineSendAdapters>,
        group_id: Uuid,
        /// The channel/bot tenant every group room is stored under.
        bot_tenant: TenantId,
        /// Alice's OWN tenant, which is not the bot's — the shape that hid
        /// every group room from web and mobile (carnet#248).
        alice_tenant: TenantId,
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
        name_fixture_bot(BOT_ID, "dravr_transcript_bot");

        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (alice_id, alice_tenant) =
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

        // Agent row (FK for the group).
        let agent = resources
            .common
            .repos
            .agents
            .create_system_agent(
                alice_id,
                bot_tenant,
                &CreateSystemAgentRequest {
                    title: "Transcript Coach".to_owned(),
                    description: None,
                    system_prompt: "You are a concise test coach.".to_owned(),
                    category: AgentCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: AgentVisibility::Global,
                },
            )
            .await
            .unwrap();

        // Coaching group pre-bound to the fixture chat, under the BOT tenant
        // (group sessions live there). Mentions mode so unaddressed chatter is
        // captured for the room without dispatching a turn.
        let group_id = Uuid::new_v4();
        let now = Timestamp::now();
        let group = CoachingGroup {
            id: group_id,
            tenant_id: bot_tenant.to_string(),
            name: "Transcript Test Group".to_owned(),
            description: None,
            agent_id: agent.id.to_string(),
            owner_id: alice_id,
            coach_user_id: None,
            peer_data_sharing: true,
            respond_mode: GroupRespondMode::Mentions,
            digest_mode: GroupDigestMode::Off,
            max_members: 10,
            is_active: true,
            channel_type: Some("telegram".to_owned()),
            channel_chat_id: Some(GROUP_CHAT_ID.to_string()),
            chat_connection: None,
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
                    coach_sharing_consent: consent,
                    coach_sharing_consent_at: None,
                    coach_consents: CoachConsents::default(),
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
            bot_tenant,
            alice_tenant,
            tg_secret,
            alice_id,
            bob_id,
            bob_auth,
            carol_id,
            dave_auth,
            adapters: Arc::new(OfflineSendAdapters::default()),
        }
    }

    /// POST a Telegram group-chat webhook update built from `message`.
    async fn post_group_message(scenario: &Scenario, update_id: i64, message: Value) -> StatusCode {
        // The offline factory builds the REAL Telegram adapter and wraps it, so
        // signature verification and payload parsing are the production ones and
        // only the outbound send is captured. Without this the turn attempts a
        // live POST to api.telegram.org, which answers 401 in ~100ms on a
        // developer's machine and hangs on a CI runner that cannot reach it —
        // straight through the 10s budget the waits below allow (run 33462729445).
        let router = MessagingRoutes::routes_with_adapters(
            Arc::clone(&scenario.resources),
            Arc::<OfflineSendAdapters>::clone(&scenario.adapters),
        );
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
    fn addressed_message(message_id: i64, from_id: i64, text: &str) -> Value {
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
    fn ambient_message(message_id: i64, from_id: i64, text: &str) -> Value {
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
            let entries =
                room_reads::visible_to(&scenario.resources.common.repos, &group_id, viewer, 100)
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

    /// GET one page of the transcript route as the given caller, with a raw
    /// query string (`limit=…&before=…`).
    async fn get_transcript_page(
        scenario: &Scenario,
        auth: &str,
        query: &str,
    ) -> (StatusCode, Value) {
        let router = ChatRoutes::routes(Arc::clone(&scenario.resources));
        let resp = AxumTestRequest::get(&format!(
            "/api/chat/groups/{}/transcript?{query}",
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

    /// A bearer token for one fixture user.
    async fn bearer_for(resources: &ServerContext, user_id: Uuid) -> String {
        let user = resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap();
        format!(
            "Bearer {}",
            resources
                .auth
                .auth_manager
                .generate_token(&user, &resources.auth.jwks_manager)
                .unwrap()
        )
    }

    /// An entry the consent rule withholds keeps its place and its speaker,
    /// and carries neither the author nor the words.
    fn assert_withheld_placeholder(entry: &Value, speaker: &str) {
        assert_eq!(
            entry["withheld"], true,
            "the entry is a placeholder: {entry}"
        );
        assert_eq!(entry["speaker"], speaker);
        assert!(
            entry["content"].is_null(),
            "a placeholder carries no words: {entry}"
        );
        assert!(
            entry["author_user_id"].is_null() && entry["author_display_name"].is_null(),
            "a placeholder names no author: {entry}"
        );
        assert!(
            entry["message_id"].is_null(),
            "a placeholder names no source row: {entry}"
        );
        assert_eq!(entry["own"], false);
        assert!(
            entry["id"].as_str().is_some_and(|id| !id.is_empty())
                && entry["created_at"]
                    .as_str()
                    .is_some_and(|at| !at.is_empty()),
            "a placeholder keeps its place in the room: {entry}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn messaging_turn_is_readable_by_web_member_and_consent_withholds_content() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
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
            3,
            "bob sees alice's turn (member + coach) and a placeholder for unconsented carol"
        );
        assert_eq!(entries[0]["speaker"], "member");
        assert_eq!(entries[0]["content"], ALICE_MESSAGE);
        assert_eq!(entries[0]["author_user_id"], scenario.alice_id.to_string());
        assert_eq!(
            entries[0]["author_display_name"], "Alice",
            "an entry names its author by display name, as the roster does"
        );
        assert_eq!(entries[1]["speaker"], "coach");
        assert_eq!(entries[1]["content"], AGENT_REPLY);
        assert_eq!(
            entries[1]["author_user_id"],
            scenario.alice_id.to_string(),
            "the agent's reply is attributed to the member it answered"
        );
        assert_withheld_placeholder(&entries[2], "member");
        assert!(
            !body.to_string().contains(CAROL_MESSAGE),
            "an unconsented member's content must not fan out to others"
        );

        // Taking the network out must not quietly take delivery out with it:
        // the turn still hands its reply to the channel, which is what the live
        // transport used to prove by failing against api.telegram.org.
        let delivered = scenario.adapters.sends();
        assert!(
            !delivered.is_empty(),
            "the turn must still dispatch its reply through the channel"
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
            alice_row["display_name"], "Alice",
            "roster display names come from the members listing source: the \
             display name, the email only when there is none"
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
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
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
        // alice's own member + agent rows join bob's two in the room.
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

    /// A room reaches the list and the thread of a member whose own tenant is
    /// not the one the room is stored under.
    ///
    /// Group sessions live on the channel/bot tenant on purpose — a room's
    /// members may span tenants and its `coaching_group` must resolve to one
    /// row for all of them. Every chat read then filtered on the CALLER's
    /// tenant, so the room was invisible to the very members it belongs to:
    /// measured on deployed dev, all 14 group conversations sat in the bot
    /// tenant and five of their six owners were not members of it (carnet#248).
    #[tokio::test]
    #[serial]
    async fn a_room_reaches_a_member_whose_own_tenant_is_not_the_rooms() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let calls = mock.call_counter();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;

        post_group_message(&scenario, 7501, addressed_message(95, 42, ALICE_MESSAGE)).await;
        assert!(
            wait_for_llm_calls(&calls, 1).await,
            "the group turn must run"
        );

        let chat = scenario.resources.common.repos.chat.as_ref();
        let alice = scenario.alice_id.to_string();

        // The room is stored under the bot tenant, which is NOT alice's.
        assert_ne!(
            scenario.alice_tenant, scenario.bot_tenant,
            "the fixture must reproduce the split tenancy this test is about"
        );

        // Alice's app resolves HER tenant, and must still find the room.
        let page = chat
            .list_conversations(
                &alice,
                scenario.alice_tenant,
                50,
                0,
                TransportPolicy::FirstPartyOnly,
                SubjectThreads::All,
            )
            .await
            .unwrap();
        assert_eq!(
            page.items.len(),
            1,
            "the room must reach the list of a member whose tenant is not the room's"
        );
        let room = &page.items[0];
        assert!(room.group_id.is_some(), "and it must arrive as a group row");
        assert_eq!(
            chat.count_participating_conversations(
                &alice,
                scenario.alice_tenant,
                TransportPolicy::FirstPartyOnly,
                SubjectThreads::All
            )
            .await
            .unwrap(),
            1,
            "the count the list pages against must agree with it"
        );

        // Opening it must work too — a row you cannot read is not visible.
        assert!(
            chat.get_conversation(&room.id, &alice, scenario.alice_tenant)
                .await
                .unwrap()
                .is_some(),
            "the member must be able to open the room from their own tenant"
        );
        let messages = chat
            .get_messages(&room.id, &alice, scenario.alice_tenant)
            .await
            .unwrap();
        assert!(
            messages.iter().any(|m| m.content.contains(ALICE_MESSAGE)),
            "and read what was said in it"
        );

        // The carve-out is groups only, and participation still gates it:
        // bob is in the room's group but has never spoken in the chat, so he
        // has no participant row and must see nothing.
        let bob = scenario.bob_id.to_string();
        let bob_page = chat
            .list_conversations(
                &bob,
                scenario.alice_tenant,
                50,
                0,
                TransportPolicy::FirstPartyOnly,
                SubjectThreads::All,
            )
            .await
            .unwrap();
        assert!(
            bob_page.items.iter().all(|c| c.id != room.id),
            "a non-participant must not reach the room, whatever tenant they ask from"
        );
        assert!(
            chat.get_conversation(&room.id, &bob, scenario.bot_tenant)
                .await
                .unwrap()
                .is_none(),
            "not even from the room's own tenant"
        );
    }
    /// The room a group thread renders, as a linked member reads it: another
    /// linked member's addressed turn, the agent's reply to it and their
    /// unaddressed line, in the order the room heard them and attributed to
    /// that member, with an unconsented member's line kept as a placeholder
    /// (carnet#440 — a group thread used to show only the reader's side).
    #[tokio::test]
    #[serial]
    async fn a_linked_members_room_holds_another_members_turn_chatter_and_a_withheld_placeholder() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let calls = mock.call_counter();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;

        // Bob is the reader, and linked to the room's Telegram chat too.
        let messaging: &dyn MessagingRepository = &*resources.common.repos.messaging;
        messaging
            .create_channel_link(&CreateChannelLinkParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id: scenario.bot_tenant,
                user_id: &scenario.bob_id.to_string(),
                channel_type: "telegram",
                channel_user_id: "44",
                display_name: Some("Bob"),
            })
            .await
            .unwrap();

        // 1. Alice addresses the coach, and the coach answers her — in her own
        //    conversation, fanned out to the room.
        let status =
            post_group_message(&scenario, 7601, addressed_message(40, 42, ALICE_MESSAGE)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            wait_for_llm_calls(&calls, 1).await,
            "the addressed group message must dispatch a turn"
        );
        wait_for_entries(&scenario, scenario.alice_id, 2).await;

        // 2. Alice's unaddressed line: the room only.
        let status =
            post_group_message(&scenario, 7602, ambient_message(41, 42, ALICE_AMBIENT)).await;
        assert_eq!(status, StatusCode::OK);
        wait_for_entries(&scenario, scenario.alice_id, 3).await;

        // 3. Carol, unconsented, speaks too; she reads her own line back, so
        //    the row exists and what bob gets below is withholding, not loss.
        let status =
            post_group_message(&scenario, 7603, ambient_message(42, 43, CAROL_MESSAGE)).await;
        assert_eq!(status, StatusCode::OK);
        let carol_view = wait_for_entries(&scenario, scenario.carol_id, 4).await;
        assert!(carol_view.iter().any(|e| e.content == CAROL_MESSAGE));

        // 4. Bob's room: alice's turn, the reply to her, her chatter, then a
        //    placeholder where carol's line sits.
        let (status, body) = get_transcript(&scenario, &scenario.bob_auth).await;
        assert_eq!(status, StatusCode::OK);
        let entries = body["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 4, "bob's room: {body}");
        let alice = scenario.alice_id.to_string();

        assert_eq!(entries[0]["speaker"], "member");
        assert_eq!(entries[0]["content"], ALICE_MESSAGE);
        assert_eq!(entries[0]["author_user_id"], alice.as_str());
        assert_eq!(entries[0]["author_display_name"], "Alice");
        assert_eq!(entries[0]["own"], false, "alice's turn is not bob's own");
        assert_eq!(entries[0]["withheld"], false);
        assert!(
            entries[0]["message_id"].as_str().is_some(),
            "a turn entry names the chat row it was fanned out from"
        );

        assert_eq!(entries[1]["speaker"], "coach");
        assert_eq!(entries[1]["content"], AGENT_REPLY);
        assert_eq!(
            entries[1]["author_user_id"],
            alice.as_str(),
            "the agent's reply is attributed to the member it answered"
        );
        assert_eq!(entries[1]["own"], false);

        assert_eq!(entries[2]["speaker"], "member");
        assert_eq!(entries[2]["content"], ALICE_AMBIENT);
        assert_eq!(entries[2]["author_user_id"], alice.as_str());
        assert!(
            entries[2]["message_id"].is_null(),
            "ambient chatter names no chat row: its provenance is the channel's own id"
        );

        assert_withheld_placeholder(&entries[3], "member");
        assert!(
            !body.to_string().contains(CAROL_MESSAGE),
            "a withheld entry's words never reach another member"
        );
        let stamps: Vec<&str> = entries
            .iter()
            .map(|e| e["created_at"].as_str().unwrap())
            .collect();
        assert!(
            stamps.windows(2).all(|w| w[0] <= w[1]),
            "the room is oldest first: {stamps:?}"
        );

        // 5. Alice reads the same room: her entries are her own, and each turn
        //    entry names the row her own conversation holds, which is how her
        //    thread shows that row instead of a second copy of it.
        let alice_auth = bearer_for(&resources, scenario.alice_id).await;
        let (status, body) = get_transcript(&scenario, &alice_auth).await;
        assert_eq!(status, StatusCode::OK);
        let entries = body["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 4, "alice's room: {body}");
        assert!(entries[..3].iter().all(|e| e["own"] == true));
        assert_withheld_placeholder(&entries[3], "member");

        let chat = resources.common.repos.chat.as_ref();
        let page = chat
            .list_conversations(
                &alice,
                scenario.alice_tenant,
                50,
                0,
                TransportPolicy::FirstPartyOnly,
                SubjectThreads::All,
            )
            .await
            .unwrap();
        let room_conversation = page
            .items
            .iter()
            .find(|c| c.group_id.is_some())
            .expect("alice's room conversation");
        let own_rows = chat
            .get_messages(&room_conversation.id, &alice, scenario.alice_tenant)
            .await
            .unwrap();
        for (entry, content) in [(&entries[0], ALICE_MESSAGE), (&entries[1], AGENT_REPLY)] {
            let message_id = entry["message_id"]
                .as_str()
                .expect("a turn entry names its row");
            let row = own_rows
                .iter()
                .find(|m| m.id == message_id)
                .expect("the named row is in alice's own conversation");
            assert_eq!(row.content, content);
        }
    }

    /// The room pages back from a cursor: each page holds the entries strictly
    /// before the one the reader already holds, oldest first, until the room
    /// runs out; a malformed cursor is refused rather than read as none.
    #[tokio::test]
    #[serial]
    async fn the_room_pages_back_from_the_oldest_entry_a_reader_holds() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;

        let group_id = scenario.group_id.to_string();
        let tenant = scenario.bot_tenant.to_string();
        for n in 1..=5 {
            let line = format!("ligne {n}");
            resources
                .common
                .repos
                .groups
                .append_transcript_entry(&NewGroupTranscriptEntry {
                    group_id: &group_id,
                    tenant_id: &tenant,
                    author_user_id: scenario.alice_id,
                    speaker: TranscriptSpeaker::Member,
                    content: &line,
                    source_conversation_id: None,
                    source_message_id: None,
                    transport_policy: TransportPolicy::AnyTransport,
                    carried: &[],
                })
                .await
                .unwrap();
            // Distinct instants, so the order under test is the clock's.
            sleep(Duration::from_millis(5)).await;
        }

        let contents = |body: &Value| -> Vec<String> {
            body["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["content"].as_str().unwrap().to_owned())
                .collect()
        };

        let (status, newest) = get_transcript_page(&scenario, &scenario.bob_auth, "limit=2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(contents(&newest), ["ligne 4", "ligne 5"]);

        let cursor = newest["entries"][0]["id"].as_str().unwrap().to_owned();
        let (status, older) = get_transcript_page(
            &scenario,
            &scenario.bob_auth,
            &format!("limit=2&before={cursor}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(contents(&older), ["ligne 2", "ligne 3"]);

        let cursor = older["entries"][0]["id"].as_str().unwrap().to_owned();
        let (status, oldest) = get_transcript_page(
            &scenario,
            &scenario.bob_auth,
            &format!("limit=2&before={cursor}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            contents(&oldest),
            ["ligne 1"],
            "the last page is short: the room ran out"
        );

        let (status, _) =
            get_transcript_page(&scenario, &scenario.bob_auth, "before=not-an-entry-id").await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a malformed cursor must be refused, not read as the newest page"
        );
    }

    /// The group's human coach speaks to the room: their words reach every
    /// member whatever the sharing switches, and keep reaching them once the
    /// coach is detached, since who held the seat is recorded with the entry
    /// (carnet#1511 design call 11). The agent's reply to the coach reaches
    /// a reader only while every member it drew on is readable to them now:
    /// a member who stops sharing hides it from everyone, the coach who
    /// asked included (call 2: one rule for every read). An entry written
    /// before records existed is its author's alone.
    #[tokio::test]
    #[serial]
    async fn the_group_coachs_words_reach_every_member_and_the_agents_reply_follows_what_it_drew_on(
    ) {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let groups = &resources.common.repos.groups;
        let group_id = scenario.group_id.to_string();
        let tenant = scenario.bot_tenant.to_string();

        let (coach_id, _) =
            create_user_with_own_tenant(&resources, "transcript_coach@example.com", "Coach").await;
        assert!(groups
            .set_group_coach_user(&group_id, Some(coach_id), scenario.bot_tenant)
            .await
            .unwrap());
        // The coach's view is read through the coaching gates: coach access
        // and the attachment (carnet#1511 Z4).
        resources
            .common
            .repos
            .users
            .set_manages_roster(coach_id, true)
            .await
            .unwrap();
        let set_peer_switch = |on: bool| {
            let group_id = group_id.clone();
            async move {
                groups
                    .update_group(
                        &group_id,
                        scenario.bot_tenant,
                        &UpdateGroupRequest {
                            name: None,
                            description: None,
                            agent_id: None,
                            peer_data_sharing: Some(on),
                            respond_mode: None,
                            digest_mode: None,
                            is_active: None,
                        },
                    )
                    .await
                    .unwrap()
                    .expect("the bot tenant holds the group");
            }
        };
        set_peer_switch(false).await;

        let alice = [scenario.alice_id];
        let append = |author: Uuid,
                      speaker: TranscriptSpeaker,
                      content: &'static str,
                      carried: &'static [Uuid]| {
            let group_id = group_id.clone();
            let tenant = tenant.clone();
            // Each member read through the coach's view by the coach seated
            // here: the coach reads it, a peer only where it is shared.
            let carried: Vec<CarriedRead> = carried
                .iter()
                .map(|member| CarriedRead::coach(*member, scenario.group_id, coach_id))
                .collect();
            async move {
                groups
                    .append_transcript_entry(&NewGroupTranscriptEntry {
                        group_id: &group_id,
                        tenant_id: &tenant,
                        author_user_id: author,
                        speaker,
                        content,
                        source_conversation_id: None,
                        source_message_id: None,
                        transport_policy: TransportPolicy::AnyTransport,
                        carried: &carried,
                    })
                    .await
                    .unwrap();
                // Distinct instants, so the order under test is the clock's.
                sleep(Duration::from_millis(5)).await;
            }
        };
        let alice: &'static [Uuid] = Box::leak(Box::new(alice));
        append(coach_id, TranscriptSpeaker::Member, COACH_MESSAGE, &[]).await;
        append(coach_id, TranscriptSpeaker::Coach, COACH_AGENT_REPLY, alice).await;

        let carol_auth = bearer_for(&resources, scenario.carol_id).await;
        let coach_auth = bearer_for(&resources, coach_id).await;
        let read_room = |auth: String| {
            let scenario = &scenario;
            async move {
                let (status, room) = get_transcript(scenario, &auth).await;
                assert_eq!(status, StatusCode::OK);
                room["entries"].as_array().unwrap().clone()
            }
        };
        let assert_shown = |entry: &Value, speaker: &str, content: &str| {
            assert_eq!(entry["withheld"], false, "{entry}");
            assert_eq!(entry["speaker"], speaker);
            assert_eq!(entry["content"], content);
            assert_eq!(entry["author_user_id"], coach_id.to_string());
        };

        // Switch off: the coach's words reach Bob and Carol. The reply drew
        // on Alice, whom neither may read with the switch off.
        for auth in [scenario.bob_auth.clone(), carol_auth.clone()] {
            let entries = read_room(auth).await;
            assert_eq!(entries.len(), 2, "{entries:?}");
            assert_shown(&entries[0], "member", COACH_MESSAGE);
            assert_withheld_placeholder(&entries[1], "coach");
        }

        // Switch on and Alice shares with the room: the reply reaches them.
        set_peer_switch(true).await;
        for auth in [scenario.bob_auth.clone(), carol_auth.clone()] {
            let entries = read_room(auth).await;
            assert_shown(&entries[1], "coach", COACH_AGENT_REPLY);
        }
        let context =
            room_reads::visible_to(&resources.common.repos, &group_id, scenario.bob_id, 100)
                .await
                .unwrap();
        assert!(
            context.iter().any(|e| e.content == COACH_AGENT_REPLY),
            "the agent answering Bob reads the reply by the same rule: {context:?}"
        );

        // Alice stops sharing with the room: the reply is hidden from the
        // members; the coach who asked still reads it through the coach's
        // view, since she shares with her coach.
        assert!(groups
            .update_peer_sharing_consent(&group_id, scenario.alice_id, false)
            .await
            .unwrap());
        for auth in [scenario.bob_auth.clone(), carol_auth.clone()] {
            let entries = read_room(auth).await;
            assert_shown(&entries[0], "member", COACH_MESSAGE);
            assert_withheld_placeholder(&entries[1], "coach");
        }
        assert_shown(
            &read_room(coach_auth.clone()).await[1],
            "coach",
            COACH_AGENT_REPLY,
        );
        // Once she stops sharing with the coach too, the coach loses it as
        // well: what it drew on is read by no one now.
        assert!(
            groups
                .update_coach_sharing_consent(&group_id, scenario.alice_id, false)
                .await
                .unwrap(),
            "Alice's coach-sharing consent is withdrawn"
        );
        assert_withheld_placeholder(&read_room(coach_auth.clone()).await[1], "coach");
        let context =
            room_reads::visible_to(&resources.common.repos, &group_id, scenario.bob_id, 100)
                .await
                .unwrap();
        assert!(
            context.iter().all(|e| e.content != COACH_AGENT_REPLY),
            "the agent answering Bob never reads a reply Alice withdrew from: {context:?}"
        );

        // Detached, the coach's words keep the audience they were written
        // with: they still reach Bob.
        assert!(
            groups
                .set_group_coach_user(&group_id, None, scenario.bot_tenant)
                .await
                .unwrap(),
            "the coach seat is emptied"
        );
        let entries = read_room(scenario.bob_auth.clone()).await;
        assert_shown(&entries[0], "member", COACH_MESSAGE);
    }

    /// An entry written before records existed (no `audience_recorded`)
    /// cannot say what it drew on, and no record can be rebuilt for it: its
    /// author alone reads it, whoever shares with the room (carnet#1511 I4).
    #[tokio::test]
    #[serial]
    async fn a_legacy_entry_is_read_by_its_author_alone() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let groups = &resources.common.repos.groups;
        let group_id = scenario.group_id.to_string();
        groups
            .append_transcript_entry(&NewGroupTranscriptEntry {
                group_id: &group_id,
                tenant_id: &scenario.bot_tenant.to_string(),
                author_user_id: scenario.alice_id,
                speaker: TranscriptSpeaker::Member,
                content: ALICE_MESSAGE,
                source_conversation_id: None,
                source_message_id: None,
                transport_policy: TransportPolicy::AnyTransport,
                carried: &[],
            })
            .await
            .unwrap();
        // Recorded, Alice's words reach Bob: she shares with the room.
        let (_, room) = get_transcript(&scenario, &scenario.bob_auth).await;
        assert_eq!(
            room["entries"][0]["content"], ALICE_MESSAGE,
            "premise: {room}"
        );

        written_before_audiences_were_recorded(&resources, scenario.group_id).await;
        let (_, room) = get_transcript(&scenario, &scenario.bob_auth).await;
        assert_withheld_placeholder(&room["entries"][0], "member");
        let alice_auth = bearer_for(&resources, scenario.alice_id).await;
        let (_, room) = get_transcript(&scenario, &alice_auth).await;
        assert_eq!(
            room["entries"][0]["content"], ALICE_MESSAGE,
            "its author reads it"
        );
        let context =
            room_reads::visible_to(&resources.common.repos, &group_id, scenario.bob_id, 100)
                .await
                .unwrap();
        assert!(
            context.is_empty(),
            "the agent answering Bob never reads it: {context:?}"
        );
    }

    /// Append `content` to the scenario's room as `author`'s, drawing on
    /// `carried`, from no chat row.
    async fn append_entry(
        scenario: &Scenario,
        author: Uuid,
        speaker: TranscriptSpeaker,
        content: &str,
        carried: &[Uuid],
    ) {
        // Each member read through the coach's view by the coach seated now,
        // else through the room's peer view.
        let coach = scenario
            .resources
            .common
            .repos
            .groups
            .get_group(&scenario.group_id.to_string(), scenario.bot_tenant)
            .await
            .unwrap()
            .and_then(|group| group.coach_user_id);
        let carried: Vec<CarriedRead> = carried
            .iter()
            .map(|member| {
                coach.map_or_else(
                    || CarriedRead::peer(*member, Some(scenario.group_id)),
                    |coach| CarriedRead::coach(*member, scenario.group_id, coach),
                )
            })
            .collect();
        scenario
            .resources
            .common
            .repos
            .groups
            .append_transcript_entry(&NewGroupTranscriptEntry {
                group_id: &scenario.group_id.to_string(),
                tenant_id: &scenario.bot_tenant.to_string(),
                author_user_id: author,
                speaker,
                content,
                source_conversation_id: None,
                source_message_id: None,
                transport_policy: TransportPolicy::AnyTransport,
                carried: &carried,
            })
            .await
            .unwrap();
        sleep(Duration::from_millis(5)).await;
    }

    /// Whether `viewer` reads an entry holding `content`, as the agent's
    /// read of the room does.
    async fn reads(scenario: &Scenario, viewer: Uuid, content: &str) -> bool {
        room_reads::visible_to(
            &scenario.resources.common.repos,
            &scenario.group_id.to_string(),
            viewer,
            100,
        )
        .await
        .unwrap()
        .iter()
        .any(|entry| entry.content == content)
    }

    /// The agent's reply to the coach, drawn on Carol who shares with her
    /// coach alone, stays with the seat that asked: a coach who takes the
    /// seat after it never reads it through the coach's view, and the coach
    /// who asked reads nothing through it once replaced or detached
    /// (carnet#1511 design call 11; the leak main also has, JF 7106050048).
    #[tokio::test]
    #[serial]
    async fn a_coach_view_reply_stays_with_its_seat_when_the_coach_is_replaced() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let groups = &resources.common.repos.groups;
        let group_id = scenario.group_id.to_string();
        let dan = seat_coach(&scenario).await;
        let reply = "Carol's resting heart rate is up this week.";
        append_entry(
            &scenario,
            dan,
            TranscriptSpeaker::Coach,
            reply,
            &[scenario.carol_id],
        )
        .await;
        assert!(
            reads(&scenario, dan, reply).await,
            "premise: the coach who asked reads it"
        );
        assert!(
            !reads(&scenario, scenario.bob_id, reply).await,
            "premise: Bob never does"
        );

        let (eve, _) =
            create_user_with_own_tenant(&resources, "transcript_eve@example.com", "Eve").await;
        assert!(
            groups
                .set_group_coach_user(&group_id, Some(eve), scenario.bot_tenant)
                .await
                .unwrap(),
            "Eve takes the coach seat"
        );
        assert!(
            !reads(&scenario, eve, reply).await,
            "bug: the coach who took the seat read the earlier seat's coach-only reply"
        );
        assert!(
            !reads(&scenario, dan, reply).await,
            "bug: a replaced coach still reads through the coach's view"
        );
        assert!(
            groups
                .set_group_coach_user(&group_id, None, scenario.bot_tenant)
                .await
                .unwrap(),
            "the coach seat is emptied"
        );
        assert!(
            !reads(&scenario, dan, reply).await,
            "detached, the coach reads nothing through it"
        );
        assert!(
            !reads(&scenario, scenario.bob_id, reply).await,
            "bug: a member reads the coach-only reply"
        );
    }

    /// A member's words written while they were a member stay theirs once
    /// they take the coach seat: the seat is recorded at write, never read
    /// from who coaches now (round-10 RG3).
    #[tokio::test]
    #[serial]
    async fn a_former_members_private_post_stays_private_after_they_take_the_seat() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let carol_said = "My knee has been aching since the trail race.";
        // Carol shares with no one in the room.
        append_entry(
            &scenario,
            scenario.carol_id,
            TranscriptSpeaker::Member,
            carol_said,
            &[],
        )
        .await;
        assert!(
            !reads(&scenario, scenario.bob_id, carol_said).await,
            "premise"
        );
        assert!(
            resources
                .common
                .repos
                .groups
                .set_group_coach_user(
                    &scenario.group_id.to_string(),
                    Some(scenario.carol_id),
                    scenario.bot_tenant
                )
                .await
                .unwrap(),
            "the coach seat moves"
        );
        assert!(
            !reads(&scenario, scenario.bob_id, carol_said).await,
            "bug: a member's private post became the coach speaking to the room"
        );
    }

    /// A member who deletes their room conversation leaves their room words
    /// with the room: an entry is read by its own record, never by whether
    /// its source row still exists (round-10 SW2). Their consent still
    /// decides.
    #[tokio::test]
    #[serial]
    async fn a_deleted_room_thread_keeps_its_entries_for_the_room() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let repos = &resources.common.repos;
        let group_id = scenario.group_id.to_string();
        let bob = scenario.bob_id.to_string();
        // A group room conversation lives under the group's tenant.
        let bob_tenant = scenario.bot_tenant;
        let conversation = repos
            .chat
            .create_conversation(
                &bob,
                bob_tenant,
                "Room",
                "mock-model",
                None,
                Some(&group_id),
            )
            .await
            .unwrap();
        let said = "Long run went well, 24 km at easy pace.";
        let message = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: bob_tenant,
                conversation_id: &conversation.id,
                user_id: &bob,
                role: "user",
                content: said,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap();
        repos
            .groups
            .append_transcript_entry(&NewGroupTranscriptEntry {
                group_id: &group_id,
                tenant_id: &bob_tenant.to_string(),
                author_user_id: scenario.bob_id,
                speaker: TranscriptSpeaker::Member,
                content: said,
                source_conversation_id: Some(&conversation.id),
                source_message_id: Some(&message.id),
                transport_policy: TransportPolicy::AnyTransport,
                carried: &[],
            })
            .await
            .unwrap();
        assert!(reads(&scenario, scenario.alice_id, said).await, "premise");

        assert!(
            repos
                .chat
                .delete_conversation(&conversation.id, &bob, bob_tenant)
                .await
                .unwrap(),
            "Bob's conversation is deleted"
        );
        assert!(
            reads(&scenario, scenario.alice_id, said).await,
            "bug: deleting his thread hid Bob's room words from the room"
        );
        assert!(
            repos
                .groups
                .update_peer_sharing_consent(&group_id, scenario.bob_id, false)
                .await
                .unwrap(),
            "Bob's peer-sharing consent is withdrawn"
        );
        assert!(
            !reads(&scenario, scenario.alice_id, said).await,
            "his consent still decides"
        );
    }

    /// The data export of whoever `auth` signs in, as `GET /api/user/export`
    /// writes it.
    async fn export_of(scenario: &Scenario, auth: &str) -> String {
        let routes =
            pierre_routes_auth::AuthRoutes::routes(scenario.resources.auth_routes_context());
        for _ in 0..200 {
            let response = AxumTestRequest::get("/api/user/export")
                .header("authorization", auth)
                .send(routes.clone())
                .await;
            let body = response.text();
            if body.contains("\"export_busy\"") {
                sleep(Duration::from_millis(50)).await;
                continue;
            }
            return body;
        }
        panic!("the export stayed busy");
    }

    /// carnet#1511 calls 2, 19 and 20, round 11 A6: a member's data export
    /// holds their room entries by the rule their room page reads them by.
    /// Dee coaches the room. Her reply that drew on Alice through the room's
    /// peer view stays hers after Alice stops sharing with the room (call
    /// 20: a withdrawal never hides from its author what the room showed
    /// them). Her reply that drew on Alice through the coach's view leaves
    /// her export with her screen once Alice stops sharing with her coach
    /// (call 2). Her own words stay.
    #[tokio::test]
    #[serial]
    async fn a_members_export_leaves_out_a_room_entry_the_room_page_withholds() {
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let groups = &resources.common.repos.groups;
        let group_id = scenario.group_id.to_string();
        let (dee, dee_tenant) =
            create_user_with_own_tenant(&resources, "transcript_dee@example.com", "Dee").await;
        let now = Timestamp::now();
        groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: scenario.group_id,
                user_id: dee,
                tenant_id: scenario.bot_tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: true,
                coach_sharing_consent: true,
                coach_sharing_consent_at: None,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
        assert!(
            groups
                .set_group_coach_user(&group_id, Some(dee), scenario.bot_tenant)
                .await
                .unwrap(),
            "Dee takes the coach seat"
        );
        resources
            .common
            .repos
            .users
            .set_manages_roster(dee, true)
            .await
            .unwrap();
        let room_reply = "export-check: the room shares Alice's tempo run";
        let reply = "export-check: Alice's tempo run reads well";
        let own = "export-check: my own long run";
        for (speaker, content, carried) in [
            (TranscriptSpeaker::Member, own, vec![]),
            (
                TranscriptSpeaker::Coach,
                room_reply,
                vec![CarriedRead::peer(
                    scenario.alice_id,
                    Some(scenario.group_id),
                )],
            ),
            (
                TranscriptSpeaker::Coach,
                reply,
                vec![CarriedRead::coach(
                    scenario.alice_id,
                    scenario.group_id,
                    dee,
                )],
            ),
        ] {
            groups
                .append_transcript_entry(&NewGroupTranscriptEntry {
                    group_id: &group_id,
                    tenant_id: &dee_tenant.to_string(),
                    author_user_id: dee,
                    speaker,
                    content,
                    source_conversation_id: None,
                    source_message_id: None,
                    transport_policy: TransportPolicy::AnyTransport,
                    carried: &carried,
                })
                .await
                .unwrap();
        }
        let dee_auth = bearer_for(&resources, dee).await;
        let shared = export_of(&scenario, &dee_auth).await;
        assert!(
            shared.contains(own) && shared.contains(room_reply) && shared.contains(reply),
            "premise: {shared}"
        );

        assert!(
            groups
                .update_peer_sharing_consent(&group_id, scenario.alice_id, false)
                .await
                .unwrap(),
            "Alice's peer-sharing consent is withdrawn"
        );
        let peer_withdrawn = export_of(&scenario, &dee_auth).await;
        assert!(
            peer_withdrawn.contains(own) && peer_withdrawn.contains(room_reply),
            "bug: Alice's withdrawal from the room hid from Dee what the room showed her \
             (call 20): {peer_withdrawn}"
        );
        assert!(
            peer_withdrawn.contains(reply),
            "Alice still shares with her coach, so the coach's view stays: {peer_withdrawn}"
        );

        assert!(
            groups
                .update_coach_sharing_consent(&group_id, scenario.alice_id, false)
                .await
                .unwrap(),
            "Alice's coach-sharing consent is withdrawn"
        );
        let withdrawn = export_of(&scenario, &dee_auth).await;
        assert!(
            withdrawn.contains(own),
            "Dee's own words stay in her export"
        );
        assert!(
            !withdrawn.contains(reply),
            "bug: Dee's export kept a reply drawn on Alice through the coach's view after she \
             stopped sharing with her coach"
        );
    }

    /// Mark every entry of `group` as written before a room entry's audience
    /// was recorded (migration 20261123180000): its author's alone.
    async fn written_before_audiences_were_recorded(res: &ServerContext, group: Uuid) {
        const SQL: &str =
            "UPDATE group_transcript_entries SET audience_recorded = FALSE WHERE group_id = $1";
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(SQL)
                    .bind(group.to_string())
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(SQL)
                    .bind(group)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    /// Seat a human coach on the scenario's group, linked in the chat as
    /// sender 44, with Carol sharing her training with the coach alone and
    /// Alice and Bob with the room. Returns the coach's id.
    async fn seat_coach(scenario: &Scenario) -> Uuid {
        let resources = &scenario.resources;
        let groups = &resources.common.repos.groups;
        let group_id = scenario.group_id.to_string();
        let (coach_id, _) =
            create_user_with_own_tenant(resources, "transcript_hold_coach@example.com", "Coach")
                .await;
        resources
            .common
            .repos
            .users
            .set_manages_roster(coach_id, true)
            .await
            .unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: scenario.bot_tenant,
            user_id: &coach_id.to_string(),
            channel_type: "telegram",
            channel_user_id: "44",
            display_name: Some("Coach"),
        })
        .await
        .unwrap();
        assert!(
            groups
                .set_group_coach_user(&group_id, Some(coach_id), scenario.bot_tenant)
                .await
                .unwrap(),
            "the coach takes the coach seat"
        );
        assert!(
            groups
                .update_coach_sharing_consent(&group_id, scenario.carol_id, true)
                .await
                .unwrap(),
            "Carol's coach-sharing consent is granted"
        );
        groups
            .set_coach_assistant_read(&group_id, scenario.carol_id, true, Timestamp::now())
            .await
            .unwrap()
            .expect("a live member");
        coach_id
    }

    /// `auth`'s conversation in the scenario group's room, opened through the
    /// app's route, and `content` posted there through the app's chat route.
    async fn post_in_app(scenario: &Scenario, auth: &str, content: &str) {
        let router = ChatRoutes::routes(Arc::clone(&scenario.resources));
        let room = AxumTestRequest::post(&format!("/api/chat/groups/{}/room", scenario.group_id))
            .header("authorization", auth)
            .send(router.clone())
            .await;
        assert!(
            matches!(room.status_code(), StatusCode::OK | StatusCode::CREATED),
            "the room opens: {}",
            room.status_code()
        );
        let conversation = room.json::<Value>()["id"].as_str().unwrap().to_owned();
        let sent =
            AxumTestRequest::post(&format!("/api/chat/conversations/{conversation}/messages"))
                .header("authorization", auth)
                .json(&json!({ "content": content }))
                .send(router)
                .await;
        assert_eq!(sent.status_code(), StatusCode::OK, "the app's turn runs");
    }

    /// The text bodies the scenario's chat was sent, in order.
    fn sent_texts(scenario: &Scenario) -> Vec<String> {
        scenario
            .adapters
            .sends()
            .into_iter()
            .filter_map(|sent| match sent.content {
                MessageContent::Text { body } => Some(body),
                _ => None,
            })
            .collect()
    }

    /// The memory extraction requests `requests` holds for a turn whose
    /// message was `message`.
    fn extracted(resources: &ServerContext, requests: &[String], message: &str) -> bool {
        let prompt = resources.memory_extraction_prompt();
        let opening: String = prompt.chars().take(60).collect();
        let opening = serde_json::to_string(&opening).unwrap();
        let opening = opening.trim_matches('"');
        requests
            .iter()
            .any(|request| request.contains(opening) && request.contains(message))
    }

    /// Wait for a memory extraction over the turn whose message was
    /// `message`; whether one ran.
    async fn wait_for_extraction(
        resources: &ServerContext,
        requests: &Arc<Mutex<Vec<String>>>,
        message: &str,
    ) -> bool {
        for _ in 0..50 {
            if extracted(resources, &requests.lock().unwrap(), message) {
                return true;
            }
            sleep(Duration::from_millis(100)).await;
        }
        false
    }

    /// A structured write on the sender's own account: relaxed by
    /// room-visible data (carnet#1511 call 12, narrowed; call 16).
    const OWN_ACCOUNT_WRITE: &str = "set_physiology";

    /// The writes that store free text a later prompt reads (carnet#1511
    /// call 16), each declared so on its tool: refused once a turn drew on
    /// any other member's data, as fact extraction is.
    const FREE_TEXT_WRITES: [&str; 4] = [
        "remember_fact",
        "set_goal",
        "commitment_create",
        "save_recipe",
    ];

    /// Whether the turn whose message was `message` was offered `tool`.
    fn offered(declared: &DeclaredLog, message: &str, tool: &str) -> bool {
        declared
            .lock()
            .unwrap()
            .iter()
            .filter(|(request, _)| request.contains(message))
            .any(|(_, tools)| tools.iter().any(|offered| offered == tool))
    }

    /// The members the newest assistant row of `user`'s conversation
    /// `thread` is stored about.
    async fn reply_about(
        resources: &ServerContext,
        thread: &str,
        user: Uuid,
        tenant: TenantId,
    ) -> BTreeSet<Uuid> {
        let chat = &resources.common.repos.chat;
        let reply = chat
            .get_messages(thread, &user.to_string(), tenant)
            .await
            .unwrap()
            .into_iter()
            .rfind(|row| row.role == "assistant")
            .expect("the turn's reply is kept")
            .id;
        chat.list_member_reads(thread, &user.to_string(), tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|read| read.message_id == reply)
            .filter_map(|read| CarriedRead::from_key(&read.key).map(|read| read.member))
            .collect()
    }

    /// `user`'s room conversation in the scenario's group, opened through
    /// the app's route.
    async fn room_of(scenario: &Scenario, auth: &str) -> String {
        let router = ChatRoutes::routes(Arc::clone(&scenario.resources));
        let room = AxumTestRequest::post(&format!("/api/chat/groups/{}/room", scenario.group_id))
            .header("authorization", auth)
            .send(router)
            .await;
        room.json::<Value>()["id"].as_str().unwrap().to_owned()
    }

    /// carnet#1511 call 12, as narrowed by JF 7106050048: stamps always name
    /// every member a row drew on, and room-visible data relaxes only
    /// own-account writes, never fact extraction. Alice and Bob share with
    /// the room, Carol with the coach alone. Bob's in-app room turn carries
    /// Alice's card, and Alice's Telegram group turn Bob's: each is still
    /// offered a write on its sender's own account, but no write that files
    /// a fact, its reply is stored about the peer it carried, and nothing its
    /// sender said is extracted. Once Alice stops sharing, the room no longer
    /// reads Bob's reply that drew on her (round-10 RG1). The human coach
    /// reads each member by the coach's consents, so the agent's reply to
    /// their in-app room turn is stored about every member it carries.
    #[tokio::test]
    #[serial]
    async fn an_ordinary_members_room_turn_writes_her_own_account_but_files_no_fact() {
        let mock = Arc::new(CapturingLlm::new(COACH_AGENT_REPLY));
        let calls = mock.call_counter();
        let requests = mock.request_log();
        let declared = mock.declared_log();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let coach_id = seat_coach(&scenario).await;

        post_in_app(&scenario, &scenario.bob_auth, BOB_MESSAGE).await;
        assert!(
            offered(&declared, BOB_MESSAGE, OWN_ACCOUNT_WRITE),
            "Bob's room turn read only what the room shares, so it may write his own account"
        );
        for write in FREE_TEXT_WRITES {
            assert!(
                !offered(&declared, BOB_MESSAGE, write),
                "bug: Bob's room turn drew on Alice and was offered {write}, which files free text"
            );
        }
        let bob_room = room_of(&scenario, &scenario.bob_auth).await;
        assert_eq!(
            reply_about(&resources, &bob_room, scenario.bob_id, scenario.bot_tenant).await,
            BTreeSet::from([scenario.alice_id]),
            "bug: Bob's reply drew on Alice's card and must be stored about her"
        );
        assert!(
            !wait_for_extraction(&resources, &requests, BOB_MESSAGE).await,
            "bug: what Bob said beside Alice's card was extracted"
        );

        // Carol reads Bob's reply while Alice shares with the room, and no
        // longer once she stops.
        let bob_reply = |entries: Vec<GroupTranscriptEntry>| {
            entries.into_iter().any(|e| {
                e.author_user_id == scenario.bob_id && e.speaker == TranscriptSpeaker::Coach
            })
        };
        let groups = &resources.common.repos.groups;
        let group_id = scenario.group_id.to_string();
        assert!(
            bob_reply(
                room_reads::visible_to(&resources.common.repos, &group_id, scenario.carol_id, 100)
                    .await
                    .unwrap()
            ),
            "premise: Carol reads Bob's reply while Alice shares"
        );
        assert!(
            groups
                .update_peer_sharing_consent(&group_id, scenario.alice_id, false)
                .await
                .unwrap(),
            "Alice's peer-sharing consent is withdrawn"
        );
        assert!(
            !bob_reply(
                room_reads::visible_to(&resources.common.repos, &group_id, scenario.carol_id, 100)
                    .await
                    .unwrap()
            ),
            "bug: Bob's reply drawn on Alice still reaches Carol after Alice stopped sharing"
        );
        // carnet#1511 call 20: a withdrawal never hides from its author what
        // the room showed them. Bob still reads his reply on the room page
        // and in his own thread, where Alice's card was a peer read.
        let (status, room) = get_transcript(&scenario, &scenario.bob_auth).await;
        assert_eq!(status, StatusCode::OK, "the group room is read");
        assert!(
            room["entries"].as_array().unwrap().iter().any(|entry| {
                entry["speaker"] == "coach"
                    && entry["withheld"] == false
                    && entry["content"] == COACH_AGENT_REPLY
            }),
            "bug: Alice's withdrawal hid Bob's own reply from his room page: {room}"
        );
        let thread = AxumTestRequest::get(&format!("/api/chat/conversations/{bob_room}/messages"))
            .header("authorization", &scenario.bob_auth)
            .send(ChatRoutes::routes(Arc::clone(&scenario.resources)))
            .await;
        assert_eq!(thread.status_code(), StatusCode::OK, "Bob's thread is read");
        assert!(
            thread.text().contains(COACH_AGENT_REPLY),
            "bug: Alice's withdrawal hid Bob's own reply from his thread"
        );
        // Shown to its author, the reply is still read by no later turn: a
        // prompt is not a screen (round-10 RG1).
        let bob_again = "Bob: et la semaine prochaine ?";
        post_in_app(&scenario, &scenario.bob_auth, bob_again).await;
        // The turn's own requests, not an extraction over its reply, which
        // quotes the reply it just wrote.
        let extraction: String = resources
            .memory_extraction_prompt()
            .chars()
            .take(60)
            .collect();
        let extraction = serde_json::to_string(&extraction).unwrap();
        let extraction = extraction.trim_matches('"').to_owned();
        let replayed = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.contains(bob_again) && !request.contains(&extraction))
            .any(|request| request.contains(COACH_AGENT_REPLY));
        assert!(
            !replayed,
            "bug: Bob's next turn replayed his reply drawn on Alice after she withdrew"
        );
        assert!(
            groups
                .update_peer_sharing_consent(&group_id, scenario.alice_id, true)
                .await
                .unwrap(),
            "Alice's peer-sharing consent is granted again"
        );

        let alice_says = "Alice: j'ai couru 12 km ce matin";
        let calls_before = calls.load(Ordering::SeqCst);
        let status =
            post_group_message(&scenario, 7401, addressed_message(41, 42, alice_says)).await;
        assert_eq!(status, StatusCode::OK, "Alice's message is accepted");
        assert!(
            wait_for_llm_calls(&calls, calls_before + 1).await,
            "Alice's turn runs"
        );
        assert!(
            offered(&declared, alice_says, OWN_ACCOUNT_WRITE),
            "Alice's group turn read only what the room shares, so it may write her own account"
        );
        for write in FREE_TEXT_WRITES {
            assert!(
                !offered(&declared, alice_says, write),
                "bug: Alice's group turn drew on Bob and was offered {write}, which files free text"
            );
        }
        assert!(
            !wait_for_extraction(&resources, &requests, alice_says).await,
            "bug: what Alice said beside Bob's words was extracted"
        );

        let coach_auth = bearer_for(&resources, coach_id).await;
        post_in_app(&scenario, &coach_auth, COACH_INAPP_POST).await;
        let coach_room = room_of(&scenario, &coach_auth).await;
        assert_eq!(
            reply_about(&resources, &coach_room, coach_id, scenario.bot_tenant).await,
            BTreeSet::from([scenario.alice_id, scenario.bob_id, scenario.carol_id]),
            "the human coach reads every member wider than the room does, Carol's coach-only \
             card included, so the agent's reply to them is about each"
        );
    }

    /// Mark chat row `id` as written before a row's audience was recorded
    /// (migration 20261124130000): a legacy row, whose reads no one can know.
    async fn written_before_rows_were_recorded(res: &ServerContext, id: &str) {
        const SQL: &str = "UPDATE chat_messages SET audience_recorded = FALSE WHERE id = $1";
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(SQL).bind(id).execute(db.pool()).await.unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(SQL).bind(id).execute(db.pool()).await.unwrap();
            }
        }
    }

    /// Write `content` as an agent row of `user`'s `conversation`, then mark
    /// it legacy.
    async fn legacy_row(
        res: &ServerContext,
        conversation: &str,
        user: Uuid,
        tenant: TenantId,
        content: &str,
    ) {
        let row = res
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: tenant,
                conversation_id: conversation,
                user_id: &user.to_string(),
                role: "assistant",
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: Some("web"),
                about_user_id: None,
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap();
        written_before_rows_were_recorded(res, &row.id).await;
    }

    /// The tenant of `user`'s tenants that holds `conversation` for them.
    async fn tenant_of(resources: &ServerContext, conversation: &str, user: Uuid) -> TenantId {
        let repos = &resources.common.repos;
        for tenant in repos.tenants.list_for_user(user).await.unwrap() {
            if repos
                .chat
                .get_conversation(conversation, &user.to_string(), tenant.id)
                .await
                .unwrap()
                .is_some()
            {
                return tenant.id;
            }
        }
        panic!("{conversation} is under none of the user's tenants");
    }

    /// carnet#1511 calls 11 and 17: a chat row written before its audience
    /// was recorded cannot say what it drew on, and nothing is reinterpreted
    /// at read time. A room, every member's, never replays one into a turn:
    /// Bob's room turn is not given the legacy row in his room thread. His
    /// own private thread, which he alone reads, still replays its legacy
    /// row (the still-works seat).
    #[tokio::test]
    #[serial]
    async fn a_room_never_replays_a_legacy_row() {
        const LEGACY_ROOM: &str = "legacy-room: Alice ran 30 km on Sunday";
        const LEGACY_OWN: &str = "legacy-own: your long run went well";
        const ROOM_ASK: &str = "Bob: what should I run tomorrow?";
        const OWN_ASK: &str = "Bob: and in my own thread?";
        let mock = Arc::new(CapturingLlm::new(COACH_AGENT_REPLY));
        let requests = mock.request_log();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let bob_room = room_of(&scenario, &scenario.bob_auth).await;
        let room_tenant = tenant_of(&resources, &bob_room, scenario.bob_id).await;
        legacy_row(
            &resources,
            &bob_room,
            scenario.bob_id,
            room_tenant,
            LEGACY_ROOM,
        )
        .await;
        let router = ChatRoutes::routes(Arc::clone(&scenario.resources));
        let created = AxumTestRequest::post("/api/chat/conversations")
            .header("authorization", &scenario.bob_auth)
            .json(&json!({ "title": "Bob's own" }))
            .send(router.clone())
            .await;
        assert!(
            matches!(created.status_code(), StatusCode::OK | StatusCode::CREATED),
            "Bob's own thread opens"
        );
        let created = created.json::<Value>();
        let own = created["id"].as_str().unwrap().to_owned();
        let own_tenant = tenant_of(&resources, &own, scenario.bob_id).await;
        legacy_row(&resources, &own, scenario.bob_id, own_tenant, LEGACY_OWN).await;

        post_in_app(&scenario, &scenario.bob_auth, ROOM_ASK).await;
        let sent = AxumTestRequest::post(&format!("/api/chat/conversations/{own}/messages"))
            .header("authorization", &scenario.bob_auth)
            .json(&json!({ "content": OWN_ASK }))
            .send(router)
            .await;
        assert_eq!(sent.status_code(), StatusCode::OK, "Bob's own turn runs");

        let given = |ask: &str| {
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request.contains(ask))
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        };
        let room_given = given(ROOM_ASK);
        assert!(!room_given.is_empty(), "premise: Bob's room turn ran");
        assert!(
            !room_given.contains(LEGACY_ROOM),
            "bug: a room replayed a row whose audience was never recorded: {room_given}"
        );
        assert!(
            given(OWN_ASK).contains(LEGACY_OWN),
            "Bob's own thread, which he alone reads, still replays its legacy row"
        );
    }

    /// carnet#1511 call 12: words every member of the room may already read
    /// do not count as a member read. The coach's own words to the room,
    /// quoted in Alice's Telegram group turn, are said where every reader of
    /// the room reads them, so her turn may still write her own account
    /// (round-12 SW2: the coach is no sharer, and was counted beyond the
    /// room by id alone).
    #[tokio::test]
    #[serial]
    async fn the_coachs_words_to_the_room_leave_a_members_own_account_writable() {
        const COACH_WORDS: &str = "Coach: easy week everyone, keep it aerobic";
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let calls = mock.call_counter();
        let requests = mock.request_log();
        let declared = mock.declared_log();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let coach_id = seat_coach(&scenario).await;
        append_entry(
            &scenario,
            coach_id,
            TranscriptSpeaker::Member,
            COACH_WORDS,
            &[],
        )
        .await;

        let alice_says = "Alice: je fais quoi demain ?";
        let calls_before = calls.load(Ordering::SeqCst);
        let status =
            post_group_message(&scenario, 7451, addressed_message(46, 42, alice_says)).await;
        assert_eq!(status, StatusCode::OK, "Alice's message is accepted");
        assert!(
            wait_for_llm_calls(&calls, calls_before + 1).await,
            "Alice's turn runs"
        );
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.contains(alice_says) && request.contains(COACH_WORDS)),
            "premise: Alice's turn quotes the coach's words"
        );
        assert!(
            offered(&declared, alice_says, OWN_ACCOUNT_WRITE),
            "bug: the coach's words to the room withheld a write on Alice's own account"
        );
    }

    /// carnet#1511 calls 8 and 19: the ambient block a group chat's turn
    /// carries quotes room entries, and the turn reads what it quotes: the
    /// author of each line, the coach and an agent reply included, and every
    /// read the quoted entry itself carried. The coach told the room about
    /// Bob's week; Alice's Telegram turn quotes that line. Her reply's room
    /// entry carries the coach's words and Bob's read, so Carol reads it while
    /// Bob shares with the room and no longer once he stops.
    #[tokio::test]
    #[serial]
    async fn a_group_chat_reply_carries_every_read_the_ambient_block_quoted() {
        const COACH_LINE: &str = "Coach: Bob's 60 km week was the club's biggest";
        let mock = Arc::new(CapturingLlm::new(AGENT_REPLY));
        let calls = mock.call_counter();
        let requests = mock.request_log();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let coach_id = seat_coach(&scenario).await;
        append_entry(
            &scenario,
            coach_id,
            TranscriptSpeaker::Member,
            COACH_LINE,
            &[scenario.bob_id],
        )
        .await;

        let alice_says = "Alice: et moi, ma semaine ?";
        let calls_before = calls.load(Ordering::SeqCst);
        let status =
            post_group_message(&scenario, 7501, addressed_message(51, 42, alice_says)).await;
        assert_eq!(status, StatusCode::OK, "Alice's message is accepted");
        assert!(
            wait_for_llm_calls(&calls, calls_before + 1).await,
            "Alice's turn runs"
        );
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.contains(alice_says) && request.contains(COACH_LINE)),
            "premise: Alice's turn quotes the coach's line"
        );

        let repos = &resources.common.repos;
        let group_id = scenario.group_id.to_string();
        let alice_reply = |entries: &[GroupTranscriptEntry]| {
            entries
                .iter()
                .find(|e| {
                    e.author_user_id == scenario.alice_id && e.speaker == TranscriptSpeaker::Coach
                })
                .map(|e| e.id)
        };
        let mut reply = None;
        for _ in 0..50 {
            let entries = room_reads::visible_to(repos, &group_id, scenario.alice_id, 100)
                .await
                .unwrap();
            reply = alice_reply(&entries);
            if reply.is_some() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let reply = reply.expect("Alice's reply reaches the room");
        let carried: Vec<CarriedRead> = repos
            .groups
            .list_transcript_entry_reads(&[reply])
            .await
            .unwrap()
            .into_iter()
            .filter_map(|(_, key)| CarriedRead::from_key(&key))
            .collect();
        assert!(
            carried
                .iter()
                .any(|read| *read == CarriedRead::room_words(coach_id, scenario.group_id)),
            "bug: the reply quoted the coach's words and does not carry them: {carried:?}"
        );
        assert!(
            carried.iter().any(|read| read.member == scenario.bob_id),
            "bug: the reply quoted a line that drew on Bob and does not carry Bob: {carried:?}"
        );

        let carol_reads =
            |entries: Vec<GroupTranscriptEntry>| entries.iter().any(|e| e.id == reply);
        assert!(
            carol_reads(
                room_reads::visible_to(repos, &group_id, scenario.carol_id, 100)
                    .await
                    .unwrap()
            ),
            "premise: Carol reads Alice's reply while Bob shares"
        );
        assert!(
            repos
                .groups
                .update_peer_sharing_consent(&group_id, scenario.bob_id, false)
                .await
                .unwrap(),
            "Bob's peer-sharing consent is withdrawn"
        );
        assert!(
            !carol_reads(
                room_reads::visible_to(repos, &group_id, scenario.carol_id, 100)
                    .await
                    .unwrap()
            ),
            "bug: Alice's reply quoting a line drawn on Bob reached Carol after Bob withdrew"
        );
    }

    /// A solo member's room turn draws on no other member: what they said is
    /// still extracted and they may still file a fact (the still-works seat
    /// of carnet#1511 call 12).
    #[tokio::test]
    #[serial]
    async fn a_room_turn_that_drew_on_no_one_else_still_files_facts() {
        let mock = Arc::new(CapturingLlm::new(COACH_AGENT_REPLY));
        let requests = mock.request_log();
        let declared = mock.declared_log();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let groups = &resources.common.repos.groups;
        let group_id = scenario.group_id.to_string();
        // Alice stops sharing: Bob's turn has no peer's card to read.
        assert!(
            groups
                .update_peer_sharing_consent(&group_id, scenario.alice_id, false)
                .await
                .unwrap(),
            "Alice's peer-sharing consent is withdrawn"
        );

        post_in_app(&scenario, &scenario.bob_auth, BOB_MESSAGE).await;
        for write in FREE_TEXT_WRITES {
            assert!(
                offered(&declared, BOB_MESSAGE, write),
                "Bob's turn drew on no one else, so it may file free text through {write}"
            );
        }
        assert!(
            wait_for_extraction(&resources, &requests, BOB_MESSAGE).await,
            "what Bob said in a turn that drew on no one else is extracted"
        );
    }

    /// How the coach's seat changes after their coach-view reply was written.
    #[derive(Debug, Clone, Copy)]
    enum SeatChange {
        /// Another account takes the seat.
        Replaced,
        /// The group is left with no coach.
        Detached,
    }

    /// carnet#1511 design call 11: a room entry's audience is fixed when it
    /// is written. The coach, who is also a member sharing with the room,
    /// posts in the app's room, and the agent answers from the coach's view,
    /// which reads Carol (she shares with the coach alone). Once the coach is
    /// replaced, and separately once the group is left with no coach, that
    /// reply reaches no one but its author: Bob's in-app room and Alice's
    /// Telegram group turn hold nothing of it, and the chat is posted
    /// nothing of it.
    #[tokio::test]
    #[serial]
    async fn a_coach_view_reply_reaches_no_one_else_once_the_coach_changes() {
        for change in [SeatChange::Replaced, SeatChange::Detached] {
            let mock = Arc::new(CapturingLlm::new(COACH_AGENT_REPLY));
            let calls = mock.call_counter();
            let requests = mock.request_log();
            let resources = create_test_server_resources_with_llm(mock).await.unwrap();
            let scenario = build_scenario(Arc::clone(&resources)).await;
            let coach_id = seat_coach(&scenario).await;
            let groups = &resources.common.repos.groups;
            let group_id = scenario.group_id.to_string();
            let now = Timestamp::now();
            groups
                .add_member(&GroupMember {
                    id: Uuid::new_v4(),
                    group_id: scenario.group_id,
                    user_id: coach_id,
                    tenant_id: scenario.bot_tenant.to_string(),
                    role: GroupRole::Member,
                    peer_sharing_consent: true,
                    coach_sharing_consent: true,
                    coach_sharing_consent_at: None,
                    coach_consents: CoachConsents::default(),
                    consent_given_at: now,
                    joined_at: now,
                    left_at: None,
                    display_name: None,
                })
                .await
                .unwrap();

            post_in_app(
                &scenario,
                &bearer_for(&resources, coach_id).await,
                COACH_INAPP_POST,
            )
            .await;
            let coach_view = wait_for_entries(&scenario, coach_id, 2).await;
            assert!(
                coach_view
                    .iter()
                    .any(|e| e.content.contains(COACH_INAPP_REPLY)),
                "{change:?}: the agent answered the coach from their view"
            );
            let bob_reads = |entries: &[GroupTranscriptEntry]| {
                entries
                    .iter()
                    .any(|e| e.content.contains(COACH_INAPP_REPLY))
            };
            let bob_view =
                room_reads::visible_to(&resources.common.repos, &group_id, scenario.bob_id, 100)
                    .await
                    .unwrap();
            assert!(
                !bob_reads(&bob_view),
                "{change:?}: while the coach is seated, Bob reads nothing of Carol"
            );

            let replaced_by = match change {
                SeatChange::Replaced => {
                    let (other, _) = create_user_with_own_tenant(
                        &resources,
                        "transcript_next_coach@example.com",
                        "Next Coach",
                    )
                    .await;
                    Some(other)
                }
                SeatChange::Detached => None,
            };
            assert!(
                groups
                    .set_group_coach_user(&group_id, replaced_by, scenario.bot_tenant)
                    .await
                    .unwrap(),
                "the coach seat is replaced"
            );

            let bob_view =
                room_reads::visible_to(&resources.common.repos, &group_id, scenario.bob_id, 100)
                    .await
                    .unwrap();
            assert!(
                !bob_reads(&bob_view),
                "{change:?}: the coach-view reply still reaches no one but its author: {:?}",
                bob_view.iter().map(|e| &e.content).collect::<Vec<_>>()
            );
            let (status, page) = get_transcript(&scenario, &scenario.bob_auth).await;
            assert_eq!(status, StatusCode::OK, "{change:?}");
            assert!(
                !page.to_string().contains(COACH_INAPP_REPLY),
                "{change:?}: Bob's in-app room holds nothing of it: {page}"
            );
            if let Some(next) = replaced_by {
                let next_view =
                    room_reads::visible_to(&resources.common.repos, &group_id, next, 100)
                        .await
                        .unwrap();
                assert!(
                    !bob_reads(&next_view),
                    "the next coach reads nothing of the former coach's view"
                );
            }

            let before = requests.lock().unwrap().len();
            let calls_before = calls.load(Ordering::SeqCst);
            let question = "Alice ici: on roule samedi?";
            let status =
                post_group_message(&scenario, 7301, addressed_message(31, 42, question)).await;
            assert_eq!(status, StatusCode::OK, "{change:?}");
            assert!(
                wait_for_llm_calls(&calls, calls_before + 1).await,
                "{change:?}: Alice's turn runs"
            );
            for _ in 0..50 {
                if !sent_texts(&scenario).is_empty() {
                    break;
                }
                sleep(Duration::from_millis(200)).await;
            }
            let prompt = requests.lock().unwrap()[before..]
                .iter()
                .filter(|request| request.contains(question))
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                !prompt.is_empty(),
                "{change:?}: Alice's turn reached the model"
            );
            assert!(
                prompt.contains(COACH_INAPP_POST),
                "{change:?}: the room words are there, so the check below reads a real block: \
                 {prompt}"
            );
            assert!(
                !prompt.contains(COACH_INAPP_REPLY),
                "{change:?}: Alice's group turn carries nothing of the coach's view: {prompt}"
            );
            let sent = sent_texts(&scenario);
            assert!(
                !sent.iter().any(|t| t.contains(COACH_INAPP_REPLY)),
                "{change:?}: the chat is posted nothing of it: {sent:?}"
            );
        }
    }

    /// carnet#1511 design call 6: a group chat is read by everyone in it the
    /// moment a reply is posted, so the rule the app relay applies holds
    /// there too. Carol shares her training with the coach alone. The
    /// coach's turn in the chat carries the room's cards (Alice's and Bob's),
    /// never Carol's, so its reply is posted; a turn that reads Carol by name
    /// is stored about her and stays in the app, the chat told so; and the
    /// next turn, which replays nothing about her into the chat, is posted.
    #[tokio::test]
    #[serial]
    async fn a_group_chat_reply_drawn_on_a_coach_only_member_stays_in_the_app() {
        let mock = Arc::new(CapturingLlm::new(COACH_AGENT_REPLY));
        let calls = mock.call_counter();
        let reads_member = mock.member_reader();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let coach_id = seat_coach(&scenario).await;
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let status = post_group_message(
            &scenario,
            7101,
            addressed_message(20, 44, "Comment va le groupe cette semaine?"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "the coach's message is accepted");
        assert!(wait_for_llm_calls(&calls, 1).await, "the coach's turn runs");
        let mut delivered = false;
        for _ in 0..50 {
            if sent_texts(&scenario).iter().any(|t| t == COACH_AGENT_REPLY) {
                delivered = true;
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        assert!(
            delivered,
            "a reply on the room's cards is posted: {:?}",
            sent_texts(&scenario)
        );
        *reads_member.lock().unwrap() = Some("Carol".to_owned());
        let before = calls.load(Ordering::SeqCst);
        let status = post_group_message(
            &scenario,
            7102,
            addressed_message(21, 44, "Et sa semaine a elle?"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "the message is accepted");
        assert!(
            wait_for_llm_calls(&calls, before + 2).await,
            "the read and the reply run"
        );
        let held = MessagingStringsRegistry::new().get(KEY_ROOM_REPLY_HELD, "fr");
        let mut told = false;
        for _ in 0..50 {
            if sent_texts(&scenario).contains(&held) {
                told = true;
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        let sent = sent_texts(&scenario);
        let session = db
            .get_session_by_channel_identity(
                scenario.bot_tenant,
                "telegram",
                "44",
                Some(&GROUP_CHAT_ID.to_string()),
            )
            .await
            .unwrap()
            .expect("the coach's session in the chat");
        let thread = session["pierre_conversation_id"]
            .as_str()
            .expect("the session's thread")
            .to_owned();
        let chat = &resources.common.repos.chat;
        let replies: Vec<String> = chat
            .get_messages(&thread, &coach_id.to_string(), scenario.bot_tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.role == "assistant")
            .map(|row| row.id)
            .collect();
        assert_eq!(replies.len(), 2, "both replies are kept in the app");
        let reads = chat
            .list_member_reads(&thread, &coach_id.to_string(), scenario.bot_tenant)
            .await
            .unwrap();
        let about = |reply: &str| -> BTreeSet<Uuid> {
            reads
                .iter()
                .filter(|read| read.message_id == reply)
                .filter_map(|read| CarriedRead::from_key(&read.key).map(|read| read.member))
                .collect()
        };
        assert_eq!(
            about(&replies[0]),
            BTreeSet::from([scenario.alice_id, scenario.bob_id]),
            "the posted reply carried the room's cards, Alice's and Bob's, never Carol's"
        );
        assert!(
            about(&replies[1]).contains(&scenario.carol_id),
            "the held reply is stored about Carol"
        );
        // The notice says the reply stays in the app: the coach reads it there,
        // over the route the app reads the thread with.
        let coach_auth = bearer_for(&resources, coach_id).await;
        let page = AxumTestRequest::get(&format!("/api/chat/conversations/{thread}/messages"))
            .header("authorization", &coach_auth)
            .send(ChatRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(
            page.status_code(),
            StatusCode::OK,
            "the conversation's messages are read"
        );
        let page = page.json::<Value>();
        assert!(
            page["messages"]
                .as_array()
                .expect("a window of messages")
                .iter()
                .any(|m| m["id"].as_str() == Some(replies[1].as_str())),
            "the coach reads the held reply in the app: {page}"
        );

        // The next turn reads no member: the held reply, about Carol, is not
        // replayed into a chat that may not read her, so its answer is posted.
        *reads_member.lock().unwrap() = None;
        let before = calls.load(Ordering::SeqCst);
        let status = post_group_message(
            &scenario,
            7103,
            addressed_message(22, 44, "Et pour samedi, on part a quelle heure?"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "the message is accepted");
        assert!(
            wait_for_llm_calls(&calls, before + 1).await,
            "the turn runs"
        );
        let mut posted = false;
        for _ in 0..50 {
            if sent_texts(&scenario)
                .iter()
                .filter(|t| *t == COACH_AGENT_REPLY)
                .count()
                == 2
            {
                posted = true;
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        assert!(
            posted,
            "a held reply does not hold every later one: {:?}",
            sent_texts(&scenario)
        );
        assert!(
            told,
            "the chat is told the reply stays in the app: {sent:?}"
        );
        assert_eq!(
            sent.iter().filter(|t| *t == COACH_AGENT_REPLY).count(),
            1,
            "the reply drawn on Carol is not posted: {sent:?}"
        );
    }

    /// carnet#1511 design call 8: the room words a group chat turn's prompt
    /// carries are the room's, read with what every member may read, never
    /// the sender's view. Carol shares with the coach alone and posts in the
    /// app's room; the coach writes there too and is answered from their view
    /// of every member; Bob, who shares with the room, posts there. The
    /// coach's next turn in the chat carries the coach's own words and Bob's,
    /// and nothing of Carol: neither her post, nor the agent's reply to her,
    /// nor its in-app reply to the coach. Its posted reply holds nothing of
    /// her, and it is stored about the members the room words quote (Bob)
    /// beside the one card the message names (Alice).
    #[tokio::test]
    #[serial]
    async fn a_group_chats_room_words_are_what_every_member_may_read() {
        let mock = Arc::new(CapturingLlm::new(COACH_AGENT_REPLY));
        let calls = mock.call_counter();
        let requests = mock.request_log();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let scenario = build_scenario(Arc::clone(&resources)).await;
        let coach_id = seat_coach(&scenario).await;

        post_in_app(
            &scenario,
            &bearer_for(&resources, scenario.carol_id).await,
            CAROL_PRIVATE_POST,
        )
        .await;
        post_in_app(
            &scenario,
            &bearer_for(&resources, coach_id).await,
            COACH_INAPP_POST,
        )
        .await;
        post_in_app(&scenario, &scenario.bob_auth, BOB_MESSAGE).await;
        // Six room entries: each post and the agent's reply to it.
        let coach_view = wait_for_entries(&scenario, coach_id, 6).await;
        assert!(
            coach_view
                .iter()
                .any(|e| e.content.contains(CAROL_PRIVATE_POST)),
            "the coach's own view reads Carol's post"
        );

        let before = requests.lock().unwrap().len();
        let calls_before = calls.load(Ordering::SeqCst);
        let status = post_group_message(
            &scenario,
            7201,
            addressed_message(30, 44, "Et Alice, elle en est ou?"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "the coach's message is accepted");
        assert!(
            wait_for_llm_calls(&calls, calls_before + 1).await,
            "the coach's turn runs"
        );
        // The chat is sent the turn's answer, whichever it is.
        for _ in 0..50 {
            if !sent_texts(&scenario).is_empty() {
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        let sent = sent_texts(&scenario);
        // The coach's turn's own requests: a late call another turn makes
        // (Carol's own, in the app) is not this prompt.
        let prompt = requests.lock().unwrap()[before..]
            .iter()
            .filter(|request| request.contains("Et Alice, elle en est ou?"))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!prompt.is_empty(), "the coach's turn reached the model");
        assert!(
            prompt.contains("Recent group chat")
                && prompt.contains(COACH_INAPP_POST)
                && prompt.contains(BOB_MESSAGE),
            "the room words carry the coach's own and Bob's: {prompt}"
        );
        for carol in [CAROL_PRIVATE_POST, CAROL_PRIVATE_REPLY, COACH_INAPP_REPLY] {
            assert!(
                !prompt.contains(carol),
                "the chat turn's prompt carries nothing of Carol ({carol}): {prompt}"
            );
            assert!(
                !sent.iter().any(|t| t.contains(carol)),
                "the chat is posted nothing of Carol ({carol}): {sent:?}"
            );
        }
        assert!(
            sent.iter().any(|t| t == COACH_AGENT_REPLY),
            "the agent's reply to the human coach is posted: {sent:?}"
        );

        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        let session = db
            .get_session_by_channel_identity(
                scenario.bot_tenant,
                "telegram",
                "44",
                Some(&GROUP_CHAT_ID.to_string()),
            )
            .await
            .unwrap()
            .expect("the coach's session in the chat");
        let thread = session["pierre_conversation_id"]
            .as_str()
            .expect("the session's thread")
            .to_owned();
        let chat = &resources.common.repos.chat;
        let reply = chat
            .get_messages(&thread, &coach_id.to_string(), scenario.bot_tenant)
            .await
            .unwrap()
            .into_iter()
            .rfind(|row| row.role == "assistant" && row.content == COACH_AGENT_REPLY)
            .expect("the posted reply is kept")
            .id;
        let about: BTreeSet<Uuid> = chat
            .list_member_reads(&thread, &coach_id.to_string(), scenario.bot_tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|read| read.message_id == reply)
            .filter_map(|read| CarriedRead::from_key(&read.key).map(|read| read.member))
            .collect();
        assert_eq!(
            about,
            BTreeSet::from([scenario.alice_id, scenario.bob_id]),
            "the human coach reads members wider than the room does, so the card the message \
             names and the member the room words quote both count (carnet#1511 design call 12)"
        );
    }
}
