// ABOUTME: Tests for get_roster_overview — the coach's consent-gated roster review from their own thread.
// ABOUTME: Covers the coach attachment, coach consent, sources, room pinning and the conformance roster.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_roster_overview` reads every athlete a coach holds at once, so its
//! scope is security-critical: the athletes are the live members of the
//! groups whose `coach_user_id` is the caller, each read only by their own
//! `coach_sharing_consent`, and a call surfaced from a group's room covers
//! that group alone — the reply there is read by every member (registre#748).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "tools-groups")]
mod roster_overview_tests {
    use crate::common::{
        create_test_server_resources, create_test_server_resources_with_chat_provider,
    };
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use dravr_tronc::mcp::schema::ToolResponse;
    use dravr_tronc::mcp::tool::{McpTool, ToolContext};
    use embacle::types::ToolCallRequest;
    use futures_util::stream;
    use pierre_chat_pipeline::{
        InputSource, PipelineHooks, QuotaState, SurfaceId, SurfaceProfile, SurfaceRequest,
        TurnInput, TurnOrigin,
    };
    use pierre_core::errors::AppError;
    use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
    use pierre_core::models::groups::{
        CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    };
    use pierre_core::models::{
        Activity, ActivityBuilder, ConnectionType, ConversationTurnId, SportType, Tenant, TenantId,
        User, UserStatus,
    };
    use pierre_core::transport::Transport;
    use pierre_llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_providers::ai_scope::{self, Provenance};
    use pierre_tool_runtime::context::{CONVERSATION_ID, TURN_DIRECT_MESSAGE};
    use pierre_tool_runtime::implementations::group_roster::GetRosterOverviewTool;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    async fn seed_user(resources: &ServerContext, name: &str) -> Uuid {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            format!("{}-{}@test.com", name.to_lowercase(), Uuid::new_v4()),
            password_hash,
            Some(name.to_owned()),
        );
        user.user_status = UserStatus::Active;
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();
        user_id
    }

    async fn create_tenant_owned_by(resources: &ServerContext, owner_id: Uuid) -> TenantId {
        let tenant_id = TenantId::generate();
        let now = Utc::now();
        let tenant = Tenant {
            id: tenant_id,
            name: "Roster Tenant".to_owned(),
            slug: format!("roster-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: owner_id,
            created_at: now,
            updated_at: now,
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

    async fn seed_agent(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) -> Uuid {
        resources
            .common
            .repos
            .agents
            .create_system_agent(
                user_id,
                tenant_id,
                &CreateSystemAgentRequest {
                    title: "Group Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: AgentCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: AgentVisibility::Global,
                },
            )
            .await
            .unwrap()
            .id
    }

    /// A group owned by `owner`, with `coach` attached as its human coach.
    async fn create_group(
        resources: &ServerContext,
        tenant_id: TenantId,
        agent_id: Uuid,
        name: &str,
        owner_id: Uuid,
        coach: Option<Uuid>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let group = CoachingGroup {
            id,
            tenant_id: tenant_id.to_string(),
            name: name.to_owned(),
            description: None,
            agent_id: agent_id.to_string(),
            owner_id,
            coach_user_id: coach,
            // The peer switch governs members, never the coach: off here so a
            // test that passes proves the coach read ignores it.
            peer_data_sharing: false,
            respond_mode: GroupRespondMode::default(),
            digest_mode: GroupDigestMode::Off,
            max_members: 20,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at: now,
        };
        resources
            .common
            .repos
            .groups
            .create_group(tenant_id, &group)
            .await
            .unwrap();
        id
    }

    /// Add `user_id` as a member sharing with the coach or not; peer consent is
    /// always off, so only the coach consent can grant the read.
    async fn add_member(
        resources: &ServerContext,
        group_id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
        coach_consent: bool,
    ) {
        let now = Utc::now();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: tenant_id.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: coach_consent,
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

    fn recent_ride(id: &str) -> Activity {
        ActivityBuilder::new(
            id.to_owned(),
            format!("ride {id}"),
            SportType::Ride,
            Utc::now() - Duration::days(2),
            7_200,
            "strava".to_owned(),
        )
        .distance_meters(60_000.0)
        .build()
    }

    /// A Strava connection and one recent ride for `user_id` under their own
    /// tenant.
    async fn seed_ride(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) {
        resources
            .common
            .repos
            .provider_connections
            .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(user_id, &tenant_id, "strava", &[recent_ride("ride-1")])
            .await
            .unwrap();
    }

    /// An athlete with their own tenant.
    async fn seed_athlete(resources: &ServerContext, name: &str) -> (Uuid, TenantId) {
        let user = seed_user(resources, name).await;
        let tenant = create_tenant_owned_by(resources, user).await;
        (user, tenant)
    }

    /// A coach with their own tenant and an agent their groups answer with.
    struct Coach {
        user: Uuid,
        tenant: TenantId,
        agent: Uuid,
    }

    /// A coach holds `manages_roster`: only such a user (or an admin) can be
    /// attached as a group's coach at all.
    async fn seed_coach(resources: &ServerContext, name: &str) -> Coach {
        let user = seed_user(resources, name).await;
        resources
            .common
            .repos
            .users
            .set_manages_roster(user, true)
            .await
            .unwrap();
        let tenant = create_tenant_owned_by(resources, user).await;
        let agent = seed_agent(resources, user, tenant).await;
        Coach {
            user,
            tenant,
            agent,
        }
    }

    async fn overview(resources: &Arc<ServerContext>, coach: &Coach, args: Value) -> Value {
        let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
        let ctx = ToolContext::new()
            .with_user(coach.user.to_string())
            .with_tenant(coach.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let response: ToolResponse = GetRosterOverviewTool.execute(&runtime, &ctx, args).await;
        response
            .structured_content
            .expect("tool result carries structured content")
    }

    /// The overview a web turn in `conversation_id` gets: the route calls a
    /// turn a direct message when its conversation carries no group.
    async fn overview_in(
        resources: &Arc<ServerContext>,
        coach: &Coach,
        conversation_id: String,
    ) -> Value {
        let is_direct_message = resources
            .common
            .repos
            .chat
            .get_conversation(&conversation_id, &coach.user.to_string(), coach.tenant)
            .await
            .unwrap()
            .is_none_or(|conversation| conversation.group_id.is_none());
        overview_from_turn(resources, coach, conversation_id, is_direct_message).await
    }

    /// The overview a turn in `conversation_id` gets, as its turn says who
    /// reads the reply.
    async fn overview_from_turn(
        resources: &Arc<ServerContext>,
        coach: &Coach,
        conversation_id: String,
        is_direct_message: bool,
    ) -> Value {
        TURN_DIRECT_MESSAGE
            .scope(
                Some(is_direct_message),
                CONVERSATION_ID.scope(Some(conversation_id), overview(resources, coach, json!({}))),
            )
            .await
    }

    fn athlete<'a>(payload: &'a Value, name: &str) -> &'a Value {
        payload["athletes"]
            .as_array()
            .expect("athletes array")
            .iter()
            .find(|a| a["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from the roster: {payload:#}"))
    }

    fn names(payload: &Value) -> Vec<String> {
        let mut names: Vec<String> = payload["athletes"]
            .as_array()
            .expect("athletes array")
            .iter()
            .map(|a| a["name"].as_str().unwrap().to_owned())
            .collect();
        names.sort();
        names
    }

    const TURN_MODEL: &str = "roster-turn-model";
    const ROSTER_ASK: &str = "How are my athletes doing?";
    const ROSTER_RESULT: &str = "[Tool Result for get_roster_overview]";

    /// The model of a coach's real turn: offered `get_roster_overview`, it
    /// calls it once, then answers. It keeps every roster result it is handed,
    /// so a test reads what the turn's own tool call returned rather than the
    /// prompt around it.
    struct RosterModel {
        roster_results: Mutex<Vec<String>>,
        models: Vec<String>,
    }

    impl RosterModel {
        fn new() -> Self {
            Self {
                roster_results: Mutex::new(Vec::new()),
                models: vec![TURN_MODEL.to_owned()],
            }
        }

        fn roster_result_count(&self) -> usize {
            self.roster_results.lock().unwrap().len()
        }

        /// The roster results handed to the model from the `from`th one on.
        fn roster_results_since(&self, from: usize) -> Vec<String> {
            self.roster_results.lock().unwrap()[from..].to_vec()
        }
    }

    #[async_trait]
    impl LlmProvider for RosterModel {
        fn name(&self) -> &'static str {
            "roster_turn_mock"
        }
        fn display_name(&self) -> &'static str {
            "Roster turn mock (carnet#1421)"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
        }
        fn default_model(&self) -> &'static str {
            TURN_MODEL
        }
        fn available_models(&self) -> &[String] {
            &self.models
        }

        async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
            let results: Vec<String> = request
                .messages
                .iter()
                .filter(|message| message.content.contains(ROSTER_RESULT))
                .map(|message| message.content.clone())
                .collect();
            let offered = request
                .tools
                .as_ref()
                .is_some_and(|tools| tools.iter().any(|tool| tool.name == "get_roster_overview"));
            let tool_calls = (offered && results.is_empty()).then(|| {
                vec![ToolCallRequest {
                    id: "call-roster".to_owned(),
                    function_name: "get_roster_overview".to_owned(),
                    arguments: json!({}),
                }]
            });
            self.roster_results.lock().unwrap().extend(results);
            let content = if tool_calls.is_some() {
                String::new()
            } else {
                "Noted.".to_owned()
            };
            Ok(ChatResponse {
                content,
                model: TURN_MODEL.to_owned(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls,
            })
        }

        async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
            let response = self.complete(request).await?;
            let chunk = StreamChunk {
                delta: response.content,
                is_final: true,
                finish_reason: response.finish_reason,
            };
            Ok(Box::pin(stream::once(async move { Ok(chunk) })))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    /// The roster results `model` was handed on one coach turn, driven
    /// through `pierre_chat_pipeline::run` in a fresh conversation with no
    /// group, the turn saying who reads the reply. The turn runs over the web
    /// app: the coach tools run only on Dravr's own surfaces.
    async fn roster_on_turn(
        resources: &Arc<ServerContext>,
        model: &RosterModel,
        coach: &Coach,
        is_direct_message: bool,
    ) -> Vec<String> {
        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(
                &coach.user.to_string(),
                coach.tenant,
                "chat",
                TURN_MODEL,
                None,
                None,
            )
            .await
            .unwrap();
        let from = model.roster_result_count();
        let profile = SurfaceProfile::resolve(&SurfaceRequest {
            surface: SurfaceId::Web,
            locale: "en".to_owned(),
            transport: None,
            prose_contract: None,
        });
        let input = TurnInput {
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            conversation_id: conversation.id,
            user_id: coach.user.to_string(),
            conversation_tenant_id: coach.tenant,
            tool_tenant_id: coach.tenant,
            is_direct_message,
            content: ROSTER_ASK.to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quote: None,
            quota: QuotaState::Ok,
            mentioned_agent: None,
            steps_after: None,
        };
        let ctx = resources.chat_pipeline_context();
        ai_scope::serve_over(
            Transport::WebApp,
            ai_scope::tracking(
                Provenance::new(),
                pierre_chat_pipeline::run(&ctx, input, &profile, &PipelineHooks::none()),
            ),
        )
        .await
        .expect("the turn completes");
        model.roster_results_since(from)
    }

    /// Two groups, three athletes: one shared across both groups, one who
    /// revoked the coach's read, one sharing with no source. Each is listed
    /// once, with training only for the sharer.
    #[tokio::test]
    async fn the_coach_reviews_every_athlete_across_their_groups() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;

        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        seed_ride(&resources, alice, alice_tenant).await;
        add_member(&resources, track, alice, alice_tenant, true).await;
        add_member(&resources, trail, alice, alice_tenant, true).await;

        let (bruno, bruno_tenant) = seed_athlete(&resources, "Bruno").await;
        seed_ride(&resources, bruno, bruno_tenant).await;
        add_member(&resources, trail, bruno, bruno_tenant, false).await;

        let (chloe, chloe_tenant) = seed_athlete(&resources, "Chloe").await;
        add_member(&resources, track, chloe, chloe_tenant, true).await;

        let payload = overview(&resources, &coach, json!({})).await;
        assert!(payload.get("error").is_none(), "refused: {payload:#}");
        assert_eq!(names(&payload), ["Alice", "Bruno", "Chloe"]);
        assert_eq!(payload["truncated"], false);
        assert_eq!(payload["groups"].as_array().unwrap().len(), 2);

        let alice = athlete(&payload, "Alice");
        assert_eq!(alice["status"], "shared", "{alice:#}");
        assert_eq!(alice["groups"].as_array().unwrap().len(), 2);
        let training = &alice["training"];
        assert_eq!(
            training["recent_activities"].as_array().unwrap().len(),
            1,
            "the shared athlete's ride is listed: {alice:#}"
        );
        assert!(
            training["days_since_last_activity"].as_i64().is_some(),
            "{alice:#}"
        );

        let bruno = athlete(&payload, "Bruno");
        assert_eq!(bruno["status"], "no_coach_consent");
        assert!(
            bruno.get("training").is_none(),
            "a revoked athlete carries no training at all: {bruno:#}"
        );

        let chloe = athlete(&payload, "Chloe");
        assert_eq!(chloe["status"], "no_source");
        assert!(chloe.get("training").is_none());
    }

    /// An athlete in two of the coach's groups who withdrew the coach's read in
    /// one still shares through the other: each membership grants the same
    /// coach the same read, as `get_group_member_activities` resolves it.
    #[tokio::test]
    async fn sharing_through_one_group_is_enough() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        seed_ride(&resources, alice, alice_tenant).await;
        add_member(&resources, track, alice, alice_tenant, false).await;
        add_member(&resources, trail, alice, alice_tenant, true).await;

        let payload = overview(&resources, &coach, json!({})).await;
        let alice = athlete(&payload, "Alice");
        assert_eq!(alice["status"], "shared", "{alice:#}");
        assert!(alice.get("training").is_some(), "{alice:#}");

        // Pinned to the group where she withdrew, the coach does not read her.
        let payload = overview(&resources, &coach, json!({ "group_id": track.to_string() })).await;
        let alice = athlete(&payload, "Alice");
        assert_eq!(alice["status"], "no_coach_consent", "{alice:#}");
        assert!(alice.get("training").is_none(), "{alice:#}");
    }

    /// The roster the tool lists is exactly the one the tenant-isolation
    /// conformance stage allows a reply to cite, so no listed athlete is
    /// redacted from the reply that names them.
    #[tokio::test]
    async fn the_roster_is_the_set_conformance_allows() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let group = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let mut expected = Vec::new();
        for name in ["Alice", "Bruno"] {
            let (user, tenant) = seed_athlete(&resources, name).await;
            add_member(&resources, group, user, tenant, name == "Alice").await;
            expected.push(user);
        }
        // Someone who left is no longer the coach's athlete.
        let (gone, gone_tenant) = seed_athlete(&resources, "Gone").await;
        add_member(&resources, group, gone, gone_tenant, true).await;
        resources
            .common
            .repos
            .groups
            .remove_member(&group.to_string(), gone)
            .await
            .unwrap();

        let payload = overview(&resources, &coach, json!({})).await;
        assert_eq!(names(&payload), ["Alice", "Bruno"]);

        let mut allowed = resources
            .common
            .repos
            .groups
            .list_athletes_coached_by(coach.user)
            .await
            .unwrap();
        allowed.sort();
        expected.sort();
        assert_eq!(
            allowed, expected,
            "conformance allows exactly the listed athletes"
        );
    }

    /// Owning a group is not coaching it: the roster follows the coach
    /// attachment, like the conformance roster and the plan scope.
    #[tokio::test]
    async fn an_owner_who_is_not_the_groups_coach_has_no_roster() {
        let resources = create_test_server_resources().await.unwrap();
        let owner = seed_coach(&resources, "Owner Omar").await;
        let group = create_group(
            &resources,
            owner.tenant,
            owner.agent,
            "Friends",
            owner.user,
            None,
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, group, alice, alice_tenant, true).await;

        let payload = overview(&resources, &owner, json!({})).await;
        assert_eq!(payload["reason"], "not_a_coach", "{payload:#}");
        assert!(payload.get("athletes").is_none());
    }

    /// The group's coach whose coaching permission was withdrawn after they
    /// were attached reads no roster: the gate is the permission that let them
    /// be attached, re-checked on every read.
    #[tokio::test]
    async fn a_coach_whose_permission_was_withdrawn_has_no_roster() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Carla").await;
        let group = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Squad",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, group, alice, alice_tenant, true).await;
        resources
            .common
            .repos
            .users
            .set_manages_roster(coach.user, false)
            .await
            .unwrap();

        let payload = overview(&resources, &coach, json!({})).await;
        assert_eq!(payload["reason"], "not_a_coach", "{payload:#}");
        assert!(
            payload.get("athletes").is_none(),
            "a withdrawn coach is named no athlete: {payload:#}"
        );
    }

    /// The coach's own DM covers every group; a group's thread covers that
    /// group alone, because every member reads the reply there.
    #[tokio::test]
    async fn a_group_thread_is_pinned_to_its_group() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, track, alice, alice_tenant, true).await;
        let (bruno, bruno_tenant) = seed_athlete(&resources, "Bruno").await;
        add_member(&resources, trail, bruno, bruno_tenant, true).await;

        let chat = &resources.common.repos.chat;
        let dm = chat
            .create_conversation(
                &coach.user.to_string(),
                coach.tenant,
                "dm",
                "gemini-2.0-flash",
                None,
                None,
            )
            .await
            .unwrap();
        let payload = overview_in(&resources, &coach, dm.id).await;
        assert_eq!(names(&payload), ["Alice", "Bruno"], "{payload:#}");

        let track_thread = chat
            .create_conversation(
                &coach.user.to_string(),
                coach.tenant,
                "track",
                "gemini-2.0-flash",
                None,
                Some(&track.to_string()),
            )
            .await
            .unwrap();
        let payload = overview_in(&resources, &coach, track_thread.id).await;
        assert_eq!(
            names(&payload),
            ["Alice"],
            "Trail's athletes must not reach Track's room: {payload:#}"
        );
        assert_eq!(payload["groups"].as_array().unwrap().len(), 1);
    }

    /// In a room whose group the caller does not coach, the roster is refused.
    #[tokio::test]
    async fn a_room_the_caller_does_not_coach_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let mine = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, mine, alice, alice_tenant, true).await;

        let other = seed_coach(&resources, "Coach Other").await;
        let theirs = create_group(
            &resources,
            other.tenant,
            other.agent,
            "Club",
            other.user,
            Some(other.user),
        )
        .await;
        let club_thread = resources
            .common
            .repos
            .chat
            .create_conversation(
                &coach.user.to_string(),
                coach.tenant,
                "club",
                "gemini-2.0-flash",
                None,
                Some(&theirs.to_string()),
            )
            .await
            .unwrap();
        let payload = overview_in(&resources, &coach, club_thread.id).await;
        assert_eq!(payload["reason"], "not_this_groups_coach", "{payload:#}");
        assert!(payload.get("athletes").is_none());
    }

    /// A conversation the caller cannot resolve — a messaging room filed under
    /// the bot tenant — may be read by anyone, so it is refused, not guessed.
    #[tokio::test]
    async fn an_unresolvable_conversation_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let group = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, group, alice, alice_tenant, true).await;

        let bot = seed_user(&resources, "Dravr Bot").await;
        let bot_tenant = create_tenant_owned_by(&resources, bot).await;
        let room = resources
            .common
            .repos
            .chat
            .create_conversation(
                &bot.to_string(),
                bot_tenant,
                "room",
                "gemini-2.0-flash",
                None,
                None,
            )
            .await
            .unwrap();
        let payload = overview_in(&resources, &coach, room.id).await;
        assert_eq!(payload["reason"], "room_unresolved", "{payload:#}");
    }

    /// carnet#1421: a messaging group chat whose group binding failed keeps
    /// no group on its conversation, so the row alone reads as the coach's
    /// own chat. On a turn other people read, the roster of neither group
    /// the coach holds is listed there; the same conversation on a
    /// direct-message turn reviews both.
    #[tokio::test]
    async fn an_ungrouped_shared_chat_is_refused() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, track, alice, alice_tenant, true).await;
        let (bruno, bruno_tenant) = seed_athlete(&resources, "Bruno").await;
        add_member(&resources, trail, bruno, bruno_tenant, true).await;
        let chat = resources
            .common
            .repos
            .chat
            .create_conversation(
                &coach.user.to_string(),
                coach.tenant,
                "chat",
                "gemini-2.0-flash",
                None,
                None,
            )
            .await
            .unwrap();

        let shared = overview_from_turn(&resources, &coach, chat.id.clone(), false).await;
        assert_eq!(
            shared["reason"], "room_unresolved",
            "a chat others read with no group bound to it gets no roster: {shared:#}"
        );
        assert!(
            shared.get("athletes").is_none(),
            "no athlete reaches a shared chat: {shared:#}"
        );

        // A call that names the conversation without saying who reads it is
        // not taken for private.
        let unsaid = CONVERSATION_ID
            .scope(
                Some(chat.id.clone()),
                overview(&resources, &coach, json!({})),
            )
            .await;
        assert_eq!(
            unsaid["reason"], "room_unresolved",
            "a conversation with no turn audience fails closed: {unsaid:#}"
        );
        assert!(
            unsaid.get("athletes").is_none(),
            "no athlete reaches a conversation with no turn audience: {unsaid:#}"
        );

        let direct = overview_from_turn(&resources, &coach, chat.id, true).await;
        assert_eq!(
            names(&direct),
            ["Alice", "Bruno"],
            "the same conversation on a direct-message turn is the coach's own: {direct:#}"
        );
    }

    /// carnet#1421 through a real turn: the chat pipeline hands the turn's
    /// audience to every tool it dispatches. In a conversation with no group,
    /// a turn other people read gets no roster and a direct-message turn
    /// reviews both of the coach's groups.
    #[tokio::test]
    async fn a_turn_tells_the_roster_who_reads_the_reply() {
        let model = Arc::new(RosterModel::new());
        let resources = create_test_server_resources_with_chat_provider(model.clone())
            .await
            .unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let repos = &resources.common.repos;
        // A verified address and a connected source, so no onboarding step
        // answers the turn before the model does.
        repos
            .email_verification
            .mark_verified(coach.user)
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                coach.user,
                coach.tenant,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, track, alice, alice_tenant, true).await;
        let (bruno, bruno_tenant) = seed_athlete(&resources, "Bruno").await;
        add_member(&resources, trail, bruno, bruno_tenant, true).await;

        let shared = roster_on_turn(&resources, &model, &coach, false).await;
        assert!(
            !shared.is_empty(),
            "the model of a turn others read was handed the roster call's result"
        );
        // The model reads a refusal as its message alone, so the room's
        // refusal is told apart from any other error by what it says.
        assert!(
            shared.iter().all(
                |result| result.contains("could not be confirmed as private to you")
                    && !result.contains("Alice")
                    && !result.contains("Bruno")
            ),
            "a turn other people read in a chat with no group gets no roster: {shared:#?}"
        );

        let direct = roster_on_turn(&resources, &model, &coach, true).await;
        assert!(
            direct
                .iter()
                .any(|result| result.contains("Alice") && result.contains("Bruno")),
            "a direct-message turn in a chat with no group reviews both groups: {direct:#?}"
        );
    }

    /// `group_id` narrows the review to one group, and a group the caller does
    /// not coach is refused rather than read.
    #[tokio::test]
    async fn the_group_argument_narrows_and_never_widens() {
        let resources = create_test_server_resources().await.unwrap();
        let coach = seed_coach(&resources, "Coach Lina").await;
        let track = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Track",
            coach.user,
            Some(coach.user),
        )
        .await;
        let trail = create_group(
            &resources,
            coach.tenant,
            coach.agent,
            "Trail",
            coach.user,
            Some(coach.user),
        )
        .await;
        let (alice, alice_tenant) = seed_athlete(&resources, "Alice").await;
        add_member(&resources, track, alice, alice_tenant, true).await;
        let (bruno, bruno_tenant) = seed_athlete(&resources, "Bruno").await;
        add_member(&resources, trail, bruno, bruno_tenant, true).await;

        let payload = overview(&resources, &coach, json!({ "group_id": trail.to_string() })).await;
        assert_eq!(names(&payload), ["Bruno"], "{payload:#}");

        let other = seed_coach(&resources, "Coach Other").await;
        let theirs = create_group(
            &resources,
            other.tenant,
            other.agent,
            "Club",
            other.user,
            Some(other.user),
        )
        .await;
        let payload = overview(
            &resources,
            &coach,
            json!({ "group_id": theirs.to_string() }),
        )
        .await;
        assert_eq!(payload["reason"], "not_your_group", "{payload:#}");
    }
}
