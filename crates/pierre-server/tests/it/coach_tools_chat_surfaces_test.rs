// ABOUTME: An API key's chat and the A2A text turn offer a human coach's assistant the coach tools, as /mcp does
// ABOUTME: One role per turn: its catalogue and its calls agree; an athlete is offered none; an admin's app tools need Admin access

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Coach tools on the chat turns served outside the app (carnet#1511, plan site 5)
//!
//! Every surface decides who is offered a tool through the one access check
//! `/mcp`, the A2A card and dispatch read (`tool_access::withheld`). A chat
//! turn served over an API key, or the A2A text turn, is decided for the
//! caller's role, read once by the turn service: a human coach's assistant is
//! offered the coach tools there too, and sees only the members who allowed
//! it; an athlete is offered none and is refused one; an admin is offered the
//! tools kept in Dravr's own app only when the grant holds `admin`, and never
//! an admin tool, which chat offers no one.
//!
//! Each case runs a real turn through `pierre_chat_pipeline::execute`, the
//! entry both surfaces call, with a model that records the tools declared to
//! it and calls the roster overview once, so the declarations, the prompt's
//! tool index and the dispatch of the call are all the turn's own.

#[cfg(feature = "tools-groups")]
mod coach_tools_chat_surfaces_tests {
    use crate::common::{
        create_test_server_resources_with_chat_provider, create_test_user_with_plan,
        delegated_test_token_for_tenant, issue_test_api_key,
    };
    use crate::helpers::coach_roster_fixtures::{
        coach, coached_group, group_agent, join, person, seed_activities, Person,
    };
    use crate::helpers::room_reads;

    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration as StdDuration, Instant};
    use tokio::time::sleep;

    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use embacle::types::ToolCallRequest;
    use futures_util::stream;
    use pierre_chat_pipeline::{
        dispatch_slash, CommandPersistence, InputSource, PipelineHooks, SlashRequest, SurfaceId,
        SurfaceProfile, SurfaceRequest, TurnOrigin, TurnRequest,
    };
    use pierre_contremaitre::messaging_strings::{
        MessagingStringsRegistry, KEY_AGENT_CREATE_EMPTY, KEY_GUARDIAN_CONFIRM_ELSEWHERE,
    };
    use pierre_core::constants::oauth::providers::INTERVALS_ICU;
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
        StreamChunk,
    };
    use pierre_core::models::a2a::user_task_key;
    use pierre_core::models::groups::{CoachConsents, GroupMember, GroupRole, UpdateGroupRequest};
    use pierre_core::models::{
        Activity, ActivityBuilder, AddMessageParams, AgentListItem, CarriedRead,
        ConversationRecord, ConversationSubject, ConversationTurnId, DataSource, DeviceType,
        ListAgentsFilter, MessageRecord, ReadWay, SportType, StoredSleepSession, TenantId,
        UserPhysiologicalProfile, COMMAND_FINISH_REASON, DIVIDER_FINISH_REASON,
    };
    use pierre_core::permissions::scopes::OAuthScope;
    use pierre_core::transport::Transport;
    use pierre_core::transport::TransportPolicy;
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_database::repositories::{
        CreateSessionParams, InsertCompactionBlockParams, InsertMessageParams,
        NewSubjectConversation,
    };
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::viz::{VizTarget, VizToken};
    use pierre_providers::ai_scope::{self, Provenance};
    use pierre_routes_auth::AuthRoutes;
    use pierre_services::call_room::CallRoom;
    use pierre_services::coach_roster::coaches_group;
    use pierre_tool_runtime::coach_seat::TurnSeat;
    use pierre_tool_runtime::context::TURN_DIRECT_MESSAGE;
    use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
    use pierre_tool_runtime::runtime::ToolRuntime;
    use pierre_tool_runtime::tool_access::CallerRole;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    /// The three coach tools (carnet#1511).
    const COACH_TOOLS: [&str; 3] = [
        "get_group_member_activities",
        "get_group_member_plan_status",
        "get_roster_overview",
    ];

    /// The two coaching-platform tools, kept in Dravr's own app.
    const APP_TOOLS: [&str; 2] = ["get_provider_roster", "propose_athlete_link"];

    /// The tool the model calls once per turn.
    const ROSTER: &str = "get_roster_overview";

    /// The grant the A2A coaching skill requires, what a read-only Claude
    /// connection holds.
    const READ_ONLY: [OAuthScope; 2] = [OAuthScope::FitnessRead, OAuthScope::ProfileRead];

    const REPLY: &str = "Here is your group.";

    /// What the model reads back when a coach tool is refused for the role
    /// (`tool_access::coach_role_refusal`).
    const ROLE_REFUSED: &str = "is for a human coach who leads an active group";

    /// What the model reads back when it calls a tool the turn did not
    /// offer (`function_dispatch::not_offered`, carnet#1511 design call 13):
    /// the refusal of a name the registry does not hold.
    const NOT_OFFERED: &str = "not found. Available tools";

    /// The argument a coach names one coached athlete with (carnet#1511,
    /// Q16).
    const ATHLETE: &str = "athlete";

    /// What the model reads back when a chat turn that takes no `athlete`
    /// is sent one (`read_subject::athlete_argument_refusal`).
    const ATHLETE_ARGUMENT_REFUSED: &str = "takes no `athlete` here";

    /// What the model reads back when a name is sent where others may read
    /// the reply (`read_subject::room_refusal`).
    const ROOM_REFUSED: &str = "Name an athlete from your own conversation with Dravr";

    /// What the model reads back when a turn that read another member
    /// writes the caller's own account (`coach_seat::after_member_read_refusal`).
    const MEMBER_READ_REFUSED: &str = "this conversation read another athlete's data";

    /// Alex's ride uploaded to Dravr, the one an external read by name
    /// reaches.
    const ALEX_UPLOAD: &str = "alex-upload-1";

    /// Alex's display name, as a member card names them.
    const ALEX_NAME: &str = "Alex Athlete";

    /// A summary of the coach's private read, as the compactor writes one.
    const SUMMARY_MARKER: &str = "summary-of-the-read-of-alex";

    /// A model that records the tools declared to each turn request, and
    /// those declared with the `athlete` argument, and makes one call (the
    /// roster overview unless a test sets another) whether it was declared
    /// or not, so a call the turn did not offer reaches dispatch too.
    struct RosterModel {
        declared: Mutex<Vec<BTreeSet<String>>>,
        naming: Mutex<Vec<BTreeSet<String>>>,
        call: Mutex<(String, Value)>,
        /// A second call the turn makes once the first is answered, when set.
        then: Mutex<Option<(String, Value)>>,
        conversations: Mutex<Vec<String>>,
        /// Whether the final reply relays the conversation, tool result
        /// included, as a model passing a read on would.
        relay: Mutex<bool>,
        /// Every turn request whole, system prompt included: the history,
        /// the summaries, the memory block and the names index a turn was
        /// given.
        requests: Mutex<Vec<String>>,
        /// Every request that declared no tool, whole: the detached learning
        /// passes (fact extraction, advice capture).
        learning: Mutex<Vec<String>>,
        /// Whether a request that declares no tool is answered with an agent
        /// proposal quoting it, as `/agent create` asks for one: its
        /// description and system prompt are what the model was sent.
        propose: Mutex<bool>,
        /// Whether a turn request is answered, with no tool call, by the
        /// lines of its system prompt that name Alex: a reply restating the
        /// member cards the platform put into the prompt.
        echo_cards: Mutex<bool>,
        models: Vec<String>,
    }

    impl RosterModel {
        fn new() -> Self {
            Self {
                declared: Mutex::new(Vec::new()),
                naming: Mutex::new(Vec::new()),
                call: Mutex::new((ROSTER.to_owned(), json!({}))),
                then: Mutex::new(None),
                conversations: Mutex::new(Vec::new()),
                relay: Mutex::new(false),
                requests: Mutex::new(Vec::new()),
                learning: Mutex::new(Vec::new()),
                propose: Mutex::new(false),
                echo_cards: Mutex::new(false),
                models: vec!["mock-model".to_owned()],
            }
        }

        /// The tools declared to the turn's first request.
        fn declared_first(&self) -> BTreeSet<String> {
            self.declared
                .lock()
                .unwrap()
                .first()
                .cloned()
                .unwrap_or_default()
        }

        /// The tools declared to the turn's first request with the `athlete`
        /// argument.
        fn naming_first(&self) -> BTreeSet<String> {
            self.naming
                .lock()
                .unwrap()
                .first()
                .cloned()
                .unwrap_or_default()
        }

        /// Every non-system message the model was sent, joined: the roster
        /// tool's result is in it.
        fn conversation(&self) -> String {
            self.conversations.lock().unwrap().join("\n")
        }
    }

    #[async_trait]
    impl LlmProvider for RosterModel {
        fn name(&self) -> &'static str {
            "coach_tools_chat_surfaces_mock"
        }
        fn display_name(&self) -> &'static str {
            "Coach tools chat surfaces mock (carnet#1511)"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
        }
        fn default_model(&self) -> &'static str {
            "mock-model"
        }
        fn available_models(&self) -> &[String] {
            &self.models
        }

        async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
            let whole = request
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            // The detached learning passes declare no tool: only the turn's
            // own requests are read.
            let Some(tools) = request.tools.as_ref().filter(|tools| !tools.is_empty()) else {
                self.learning.lock().unwrap().push(whole.clone());
                if *self.propose.lock().unwrap() {
                    let proposal = json!({
                        "title": "Echo",
                        "description": whole,
                        "system_prompt": whole,
                        "category": "training",
                        "tags": ["echo"],
                    });
                    return Ok(reply(proposal.to_string(), None));
                }
                return Ok(reply(String::new(), None));
            };
            self.requests.lock().unwrap().push(whole);
            self.declared
                .lock()
                .unwrap()
                .push(tools.iter().map(|tool| tool.name.clone()).collect());
            self.naming.lock().unwrap().push(
                tools
                    .iter()
                    .filter(|tool| {
                        tool.parameters.as_ref().is_some_and(|parameters| {
                            parameters["properties"].get(ATHLETE).is_some()
                        })
                    })
                    .map(|tool| tool.name.clone())
                    .collect(),
            );
            if *self.echo_cards.lock().unwrap() {
                let cards = request
                    .messages
                    .iter()
                    .filter(|m| matches!(m.role, MessageRole::System))
                    .flat_map(|m| m.content.lines())
                    .filter(|line| line.contains(ALEX_NAME))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(reply(cards, None));
            }
            let (name, arguments) = self.call.lock().unwrap().clone();
            let conversation = request
                .messages
                .iter()
                .filter(|m| !matches!(m.role, MessageRole::System))
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            // Answered on this turn: the room may replay an earlier turn's
            // answer to the same tool, which leaves this turn's call unmade.
            let marker = format!("[Tool Result for {name}]");
            let own_turn = request
                .messages
                .iter()
                .rposition(|m| {
                    matches!(m.role, MessageRole::User) && !m.content.starts_with("[Tool Result")
                })
                .unwrap_or(0);
            let answered = request.messages[own_turn..]
                .iter()
                .any(|m| m.content.contains(&marker));
            let relayed = format!("{REPLY}\n{conversation}");
            self.conversations.lock().unwrap().push(conversation);
            // The second call is made once: after the first call's result,
            // and before its own. A second call to the same tool is made
            // until the turn holds two results for it.
            let then = self.then.lock().unwrap().clone().filter(|(then, _)| {
                let marker = format!("[Tool Result for {then}]");
                let made = request.messages[own_turn..]
                    .iter()
                    .map(|m| m.content.matches(&marker).count())
                    .sum::<usize>();
                made < if *then == name { 2 } else { 1 }
            });
            if let (true, Some((then, arguments))) = (answered, then) {
                return Ok(reply(
                    String::new(),
                    Some(vec![ToolCallRequest {
                        id: "call-then".to_owned(),
                        function_name: then,
                        arguments,
                    }]),
                ));
            }
            if answered {
                let relay = *self.relay.lock().unwrap();
                return Ok(reply(if relay { relayed } else { REPLY.to_owned() }, None));
            }
            Ok(reply(
                String::new(),
                Some(vec![ToolCallRequest {
                    id: "call-once".to_owned(),
                    function_name: name,
                    arguments,
                }]),
            ))
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

    fn reply(content: String, tool_calls: Option<Vec<ToolCallRequest>>) -> ChatResponse {
        ChatResponse {
            replay_blocks: None,
            content,
            model: "mock-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls,
        }
    }

    /// A ride from Intervals.icu `days_ago`, whose terms let it reach an
    /// assistant outside Dravr.
    fn intervals_ride(id: &str, days_ago: i64) -> Activity {
        ActivityBuilder::new(
            id.to_owned(),
            format!("ride {id}"),
            SportType::Ride,
            Timestamp::now() - Elapsed::days(days_ago),
            3_600,
            INTERVALS_ICU.to_owned(),
        )
        .distance_meters(30_000.0)
        .build()
    }

    /// A coach of one group whose two members share with the coach, each
    /// with a ride an outside assistant may read: Alex has not allowed the
    /// coach's assistant, Bea has. Every caller has a source connected.
    struct Squad {
        res: Arc<ServerContext>,
        model: Arc<RosterModel>,
        casey: Person,
        alex: Person,
        bea: Person,
        group: Uuid,
        agent: String,
    }

    async fn squad() -> Squad {
        let model = Arc::new(RosterModel::new());
        let res = create_test_server_resources_with_chat_provider(model.clone())
            .await
            .unwrap();
        let casey = coach(&res, "casey@chat-surfaces.test", "Casey Coach").await;
        let alex = person(&res, "alex@chat-surfaces.test", "Alex Athlete").await;
        let bea = person(&res, "bea@chat-surfaces.test", "Bea Bright").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, true).await;
        join(&res, group, bea.id, bea.tenant, true).await;
        res.common
            .repos
            .groups
            .set_coach_assistant_read(&group.to_string(), bea.id, true, Timestamp::now())
            .await
            .unwrap()
            .expect("a live member");
        for (who, id) in [(&casey, "casey-1"), (&alex, "alex-1"), (&bea, "bea-1")] {
            seed_activities(&res, who, INTERVALS_ICU, &[intervals_ride(id, 2)]).await;
            res.common
                .repos
                .email_verification
                .mark_verified(who.id)
                .await
                .unwrap();
        }
        Squad {
            res,
            model,
            casey,
            alex,
            bea,
            group,
            agent,
        }
    }

    /// What one turn was offered and what its one call answered.
    struct Turn {
        declared: BTreeSet<String>,
        naming: BTreeSet<String>,
        conversation: String,
    }

    /// Run one turn for `who` over `transport` under `grant`, as the API-key
    /// chat route and the A2A text turn enter it.
    async fn turn(squad: &Squad, who: &Person, transport: Transport, grant: &[OAuthScope]) -> Turn {
        let conversation_id = squad
            .res
            .common
            .repos
            .chat
            .create_conversation(
                &who.id.to_string(),
                who.tenant,
                "Coach tools",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        turn_in(squad, who, transport, grant, conversation_id, true).await
    }

    /// [`turn`] in the conversation `conversation_id`, entered as a direct
    /// message or not. The API-key chat route marks a turn a direct message
    /// only on a thread read by its owner alone ([`route_marks_direct`]); a
    /// caller here passes what the route would mark.
    async fn turn_in(
        squad: &Squad,
        who: &Person,
        transport: Transport,
        grant: &[OAuthScope],
        conversation_id: String,
        is_direct_message: bool,
    ) -> Turn {
        let res = &squad.res;
        let surface = match transport {
            Transport::A2a => SurfaceId::A2a,
            Transport::Messaging => SurfaceId::Telegram,
            _ => SurfaceId::Web,
        };
        squad.model.declared.lock().unwrap().clear();
        squad.model.naming.lock().unwrap().clear();
        squad.model.conversations.lock().unwrap().clear();
        pierre_chat_pipeline::execute(
            &res.chat_pipeline_context(),
            TurnRequest {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                conversation_id: conversation_id.clone(),
                user_id: who.id,
                conversation_tenant_id: who.tenant,
                tool_tenant_id: who.tenant,
                content: "How is my group doing?".to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                quote: None,
                channel_type: surface.wire_channel(),
                transport,
                is_direct_message,
                ambient_group_fallback: false,
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                hooks: PipelineHooks::none(),
                grant: grant.to_vec(),
                caller: None,
            },
            &SurfaceProfile::resolve(&SurfaceRequest {
                surface,
                locale: "en".to_owned(),
                transport: None,
                prose_contract: None,
            }),
        )
        .await
        .expect("the turn is served");
        Turn {
            declared: squad.model.declared_first(),
            naming: squad.model.naming_first(),
            conversation: squad.model.conversation(),
        }
    }

    /// [`turn`], with the model calling `tool` with `arguments` instead of
    /// the roster overview.
    async fn turn_calling(
        squad: &Squad,
        who: &Person,
        transport: Transport,
        grant: &[OAuthScope],
        (tool, arguments): (&str, Value),
    ) -> Turn {
        *squad.model.call.lock().unwrap() = (tool.to_owned(), arguments);
        let served = turn(squad, who, transport, grant).await;
        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
        served
    }

    /// Alex's training as a ride uploaded to Dravr, read from the cache: the
    /// fixture's provider connection holds no token to read with.
    async fn upload_alex_ride(squad: &Squad) {
        let repos = &squad.res.common.repos;
        repos
            .provider_connections
            .remove_connection(squad.alex.id, squad.alex.tenant, INTERVALS_ICU)
            .await
            .unwrap();
        let upload = ActivityBuilder::new(
            ALEX_UPLOAD.to_owned(),
            format!("ride {ALEX_UPLOAD}"),
            SportType::Ride,
            Timestamp::now() - Elapsed::days(3),
            3_600,
            "upload".to_owned(),
        )
        .distance_meters(30_000.0)
        .build();
        repos
            .activity_cache
            .upsert_activities(squad.alex.id, &squad.alex.tenant, "upload", &[upload])
            .await
            .unwrap();
    }

    /// In a room the room rule refuses every name in, `turn` was offered
    /// `athlete` on no tool at all: the listing leaves out what dispatch would
    /// refuse (carnet#1422), a plan tool's own `athlete` included.
    fn assert_offers_no_refused_name(turn: &Turn, room: &str) {
        assert!(
            turn.naming.is_empty(),
            "{room}: no tool is offered a name it would be refused: {:?}",
            turn.naming
        );
    }

    /// A read naming Alex, a member of the coach's squad who shares.
    fn read_naming_alex() -> (String, Value) {
        ("get_activities".to_owned(), json!({ "athlete": "Alex" }))
    }

    /// A read naming Bea, the member who allowed the coach's assistant.
    fn read_naming_bea() -> (&'static str, Value) {
        ("get_activities", json!({ "athlete": "Bea" }))
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    /// The two external chat surfaces, each with the grant its caller holds:
    /// the A2A text turn a read-only Claude connection, an API key the
    /// account's own grant.
    fn external_turns() -> [(Transport, Vec<OAuthScope>); 2] {
        [
            (Transport::A2a, READ_ONLY.to_vec()),
            (Transport::ApiKey, OAuthScope::self_grant()),
        ]
    }

    /// A human coach's A2A text turn and API-key chat are offered the coach
    /// tools, as `/mcp` lists them to the coach, and the roster call shows
    /// only the member who allowed the coach's assistant; the other is a
    /// count, never named.
    #[tokio::test]
    async fn a_coachs_a2a_text_turn_and_api_key_chat_are_offered_the_coach_tools() {
        let squad = squad().await;
        for (transport, grant) in external_turns() {
            let turn = turn(&squad, &squad.casey, transport, &grant).await;
            assert!(
                set(&COACH_TOOLS).is_subset(&turn.declared),
                "{transport:?}: missing {:?}",
                set(&COACH_TOOLS)
                    .difference(&turn.declared)
                    .collect::<Vec<_>>()
            );
            assert!(
                turn.declared.is_disjoint(&set(&APP_TOOLS)),
                "{transport:?}: a coach is not offered the tools kept in the app: {:?}",
                turn.declared
                    .intersection(&set(&APP_TOOLS))
                    .collect::<Vec<_>>()
            );
            assert!(
                turn.conversation.contains("Bea Bright"),
                "{transport:?}: the member who allowed the assistant is named: {}",
                turn.conversation
            );
            assert!(
                !turn.conversation.contains("Alex Athlete"),
                "{transport:?}: a member who did not allow it is never named: {}",
                turn.conversation
            );
            assert!(
                turn.conversation
                    .contains("1 member has not allowed AI tools"),
                "{transport:?}: the member left out is counted: {}",
                turn.conversation
            );
        }
    }

    /// An athlete's A2A text turn and API-key chat are offered no coach tool,
    /// and the call the model makes anyway is refused in band as a tool the
    /// turn did not offer.
    #[tokio::test]
    async fn an_athletes_a2a_text_turn_and_api_key_chat_are_offered_no_coach_tool() {
        let squad = squad().await;
        for (transport, grant) in external_turns() {
            let turn = turn(&squad, &squad.alex, transport, &grant).await;
            assert!(
                turn.declared.contains("get_activities"),
                "{transport:?}: {:?}",
                turn.declared
            );
            assert!(
                turn.declared.is_disjoint(&set(&COACH_TOOLS)),
                "{transport:?}: {:?}",
                turn.declared
                    .intersection(&set(&COACH_TOOLS))
                    .collect::<Vec<_>>()
            );
            assert!(
                turn.conversation.contains(NOT_OFFERED),
                "{transport:?}: the call is refused as not offered: {}",
                turn.conversation
            );
            assert!(
                !turn.conversation.contains("Bea Bright"),
                "{transport:?}: {}",
                turn.conversation
            );
        }
    }

    /// An admin's chat turn served outside the app is offered the coach
    /// tools with or without a group, the tools kept in the app only when
    /// the grant holds `admin`, and never an admin tool: chat offers no one
    /// those.
    #[tokio::test]
    async fn an_admins_external_chat_is_offered_the_app_tools_only_with_admin_access() {
        let squad = squad().await;
        let res = &squad.res;
        let ada = person(res, "ada@chat-surfaces.test", "Ada Admin").await;
        res.common
            .repos
            .users
            .set_admin_status(ada.id, true)
            .await
            .unwrap();
        seed_activities(res, &ada, INTERVALS_ICU, &[intervals_ride("ada-1", 2)]).await;
        res.common
            .repos
            .email_verification
            .mark_verified(ada.id)
            .await
            .unwrap();
        let admin_tools: BTreeSet<String> = res
            .mcp
            .tool_registry
            .admin_tool_schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect();
        assert!(!admin_tools.is_empty(), "the registry holds admin tools");

        let with_admin: Vec<OAuthScope> = READ_ONLY
            .iter()
            .copied()
            .chain([OAuthScope::Admin])
            .collect();
        let cases = [
            (Transport::A2a, READ_ONLY.to_vec(), false),
            (Transport::A2a, with_admin, true),
            // An API key carries the account's own grant, `admin` included
            // for an admin (Q25).
            (Transport::ApiKey, OAuthScope::self_grant(), true),
        ];
        for (transport, grant, admin_access) in cases {
            let turn = turn(&squad, &ada, transport, &grant).await;
            assert!(
                set(&COACH_TOOLS).is_subset(&turn.declared),
                "{transport:?} {grant:?}: {:?}",
                set(&COACH_TOOLS)
                    .difference(&turn.declared)
                    .collect::<Vec<_>>()
            );
            // The read one on a read-only grant with Admin access; the write
            // one too on a grant that writes.
            assert_eq!(
                turn.declared.contains("get_provider_roster"),
                admin_access,
                "{transport:?} {grant:?}: the tools kept in the app need Admin access: {:?}",
                turn.declared
            );
            // Its every call names an athlete, which an admin who coaches
            // no one is never offered: it is not offered either (carnet#1511).
            assert!(
                !turn.declared.contains("propose_athlete_link"),
                "{transport:?} {grant:?}: {:?}",
                turn.declared
            );
            assert!(
                turn.declared.is_disjoint(&admin_tools),
                "{transport:?} {grant:?}: chat offers no admin tool: {:?}",
                turn.declared.intersection(&admin_tools).collect::<Vec<_>>()
            );
            assert!(
                !turn.conversation.contains(ROLE_REFUSED),
                "{transport:?}: an admin's roster call runs: {}",
                turn.conversation
            );
        }
    }

    /// carnet#1511: `propose_athlete_link` cannot be called without the
    /// `athlete` it links, so it is offered exactly where that argument is:
    /// to a human coach in the app. An admin who coaches no one, over A2A or
    /// an API key, and a plain athlete in the app or on a messaging chat are
    /// neither offered it nor served a call naming one; the coach in the app
    /// is offered it with the argument.
    #[tokio::test]
    async fn a_tool_that_needs_an_athlete_is_offered_and_run_only_where_one_is_offered() {
        let squad = squad().await;
        let res = &squad.res;
        let ada = person(res, "ada@chat-surfaces.test", "Ada Admin").await;
        res.common
            .repos
            .users
            .set_admin_status(ada.id, true)
            .await
            .unwrap();
        res.common
            .repos
            .email_verification
            .mark_verified(ada.id)
            .await
            .unwrap();
        let with_admin: Vec<OAuthScope> = OAuthScope::self_grant()
            .into_iter()
            .chain([OAuthScope::Admin])
            .collect();
        let link = ("propose_athlete_link", json!({ ATHLETE: "Alex" }));
        // Outside the app a write names no one for anyone. In the app,
        // someone who coaches no one has no one to name. Either way the turn
        // does not offer it, and a call made anyway is refused as such.
        let cases = [
            (&ada, Transport::A2a, with_admin.clone()),
            (&ada, Transport::ApiKey, OAuthScope::self_grant()),
            (&ada, Transport::WebApp, with_admin),
            (&squad.bea, Transport::WebApp, OAuthScope::self_grant()),
            (&squad.bea, Transport::Messaging, OAuthScope::self_grant()),
        ];
        for (who, transport, grant) in cases {
            let turn = turn_calling(&squad, who, transport, &grant, link.clone()).await;
            assert!(
                !turn.declared.contains("propose_athlete_link"),
                "{transport:?}: not offered to who names no one: {:?}",
                turn.declared
            );
            assert!(
                turn.conversation.contains(NOT_OFFERED)
                    && !turn.conversation.contains("pending_confirmation"),
                "{transport:?}: and refused when called anyway: {}",
                turn.conversation
            );
        }
        let coach = turn(
            &squad,
            &squad.casey,
            Transport::WebApp,
            &OAuthScope::self_grant(),
        )
        .await;
        assert!(
            coach.declared.contains("propose_athlete_link")
                && coach.naming.contains("propose_athlete_link"),
            "the coach in the app is offered it, with the athlete: {:?}",
            coach.naming
        );
    }

    /// carnet#1511 design call 3: a chat turn runs no tool its catalogue
    /// did not offer. Chat offers no admin tool to anyone, an admin
    /// included, so an admin's turn in the app and over an API key that
    /// calls one anyway is refused at dispatch, as the catalogue left it out.
    #[tokio::test]
    async fn an_admins_chat_turn_runs_no_admin_tool_it_was_not_offered() {
        let squad = squad().await;
        let res = &squad.res;
        let ada = person(res, "ada@chat-surfaces.test", "Ada Admin").await;
        res.common
            .repos
            .users
            .set_admin_status(ada.id, true)
            .await
            .unwrap();
        res.common
            .repos
            .email_verification
            .mark_verified(ada.id)
            .await
            .unwrap();
        let admin_tool = ("admin_list_system_agents", json!({}));
        for transport in [Transport::WebApp, Transport::ApiKey] {
            let turn = turn_calling(
                &squad,
                &ada,
                transport,
                &OAuthScope::self_grant(),
                admin_tool.clone(),
            )
            .await;
            assert!(
                !turn.declared.contains(admin_tool.0),
                "{transport:?}: chat offers no admin tool: {:?}",
                turn.declared
            );
            assert!(
                turn.conversation.contains(NOT_OFFERED)
                    && !turn.conversation.contains("\"agents\""),
                "{transport:?}: and refuses one called anyway: {}",
                turn.conversation
            );
        }
    }

    /// carnet#1511 design call 13: a chat turn runs no tool its catalogue
    /// did not offer, whatever left it out. A plain athlete's turn, in the
    /// app and over an API key, that calls a tool outside the chat
    /// categories (`list_agents`, `calculate_personalized_zones`) or an admin
    /// tool is refused at dispatch as not offered, and nothing the tool would
    /// have answered reaches the model.
    #[tokio::test]
    async fn a_chat_turn_runs_no_tool_it_did_not_offer() {
        let squad = squad().await;
        let calls = [
            ("list_agents", json!({}), "\"agents\""),
            ("calculate_personalized_zones", json!({}), "pace_zones"),
            ("admin_list_system_agents", json!({}), "\"agents\""),
        ];
        for transport in [Transport::WebApp, Transport::ApiKey] {
            for (tool, arguments, answer) in &calls {
                let turn = turn_calling(
                    &squad,
                    &squad.bea,
                    transport,
                    &OAuthScope::self_grant(),
                    (tool, arguments.clone()),
                )
                .await;
                assert!(
                    !turn.declared.contains(*tool),
                    "{transport:?}: {tool} is not offered: {:?}",
                    turn.declared
                );
                assert!(
                    turn.conversation
                        .contains(&format!("Tool '{tool}' {NOT_OFFERED}")),
                    "{transport:?}: {tool} called anyway is refused as not offered: {}",
                    turn.conversation
                );
                assert!(
                    !turn.conversation.contains(answer),
                    "{transport:?}: {tool} never ran: {}",
                    turn.conversation
                );
            }
        }
    }

    /// The listing follows the account, not a stale turn: a coach whose
    /// coach access is taken back is offered no coach tool on their next
    /// turn, and their call is refused.
    #[tokio::test]
    async fn a_coach_whose_access_is_taken_back_is_offered_none_on_the_next_turn() {
        let squad = squad().await;
        squad
            .res
            .common
            .repos
            .users
            .set_manages_roster(squad.casey.id, false)
            .await
            .unwrap();
        let turn = turn(
            &squad,
            &squad.casey,
            Transport::ApiKey,
            &OAuthScope::self_grant(),
        )
        .await;
        assert!(
            turn.declared.is_disjoint(&set(&COACH_TOOLS)),
            "{:?}",
            turn.declared
        );
        assert!(
            turn.conversation.contains(NOT_OFFERED),
            "{}",
            turn.conversation
        );
    }

    /// Dravr's own app is unchanged: a coach's web turn is offered the coach
    /// tools and reads every member who shares, allowed or not.
    #[tokio::test]
    async fn a_coachs_web_turn_still_reads_every_member_who_shares() {
        let squad = squad().await;
        let turn = turn(
            &squad,
            &squad.casey,
            Transport::WebApp,
            &OAuthScope::self_grant(),
        )
        .await;
        assert!(
            set(&COACH_TOOLS).is_subset(&turn.declared),
            "a coach's web turn is offered the coach tools: {:?}",
            turn.declared
        );
        assert!(
            turn.conversation.contains("Alex Athlete") && turn.conversation.contains("Bea Bright"),
            "{}",
            turn.conversation
        );
        assert!(
            !turn.naming.contains("get_activities"),
            "in the app a read never takes `athlete`: the coach's thread names the athlete: {:?}",
            turn.naming
        );
    }

    /// Q16 on the chat surfaces: a non-coach's A2A text turn and API-key
    /// chat are offered `athlete` on no tool, as `/mcp` lists it, and a call
    /// that names one anyway is refused as `/mcp` refuses it: no member the
    /// caller coaches answers to the name.
    #[tokio::test]
    async fn a_non_coachs_external_chat_offers_no_athlete_argument_and_refuses_one() {
        let squad = squad().await;
        for (transport, grant) in external_turns() {
            let turn =
                turn_calling(&squad, &squad.alex, transport, &grant, read_naming_bea()).await;
            assert!(
                turn.declared.contains("get_activities"),
                "{transport:?}: the read itself is offered, only its `athlete` is not: {:?}",
                turn.declared
            );
            assert!(
                turn.naming.is_empty(),
                "{transport:?}: no tool takes `athlete` for a non-coach: {:?}",
                turn.naming
            );
            assert!(
                turn.conversation
                    .contains("No athlete matching 'Bea' shares their training with you"),
                "{transport:?}: the name is refused as /mcp refuses it (no_match): {}",
                turn.conversation
            );
            assert!(
                !turn.conversation.contains("ride bea-1"),
                "{transport:?}: Bea's ride is never read: {}",
                turn.conversation
            );
        }
    }

    /// Q16 on the chat surfaces: a human coach's A2A text turn and API-key
    /// chat are offered `athlete` on the reads, as `/mcp` lists it to them,
    /// and a read naming a member who shares with the coach reads that
    /// member, whether or not they allowed the coach's assistant: that opt-in
    /// governs the roster, never a read by name.
    #[tokio::test]
    async fn a_coachs_external_chat_offers_the_athlete_argument_and_reads_the_named_member() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        for (transport, grant) in external_turns() {
            let turn = turn_calling(
                &squad,
                &squad.casey,
                transport,
                &grant,
                ("get_activities", json!({ "athlete": "Alex" })),
            )
            .await;
            assert!(
                turn.naming.contains("get_activities"),
                "{transport:?}: a coach's read takes `athlete`: {:?}",
                turn.naming
            );
            assert!(
                turn.conversation.contains(ALEX_UPLOAD),
                "{transport:?}: the named member's ride is read: {}",
                turn.conversation
            );
            assert!(
                !turn.conversation.contains("ride casey-1"),
                "{transport:?}: never the coach's own: {}",
                turn.conversation
            );
        }
    }

    /// In Dravr's own app the dispatch refuses a name: a coach's web turn
    /// that names an athlete is refused, and reads no one.
    #[tokio::test]
    async fn a_coachs_web_turn_that_names_an_athlete_is_refused() {
        let squad = squad().await;
        let turn = turn_calling(
            &squad,
            &squad.casey,
            Transport::WebApp,
            &OAuthScope::self_grant(),
            ("get_activities", json!({ "athlete": "Alex" })),
        )
        .await;
        assert!(
            turn.conversation.contains(ATHLETE_ARGUMENT_REFUSED),
            "the app's dispatch refuses a named athlete: {}",
            turn.conversation
        );
        assert!(
            !turn.conversation.contains("ride alex-1"),
            "the named athlete is never read: {}",
            turn.conversation
        );
    }

    /// A coach's thread about one athlete names its athlete, served in the
    /// app or outside it: an API-key or A2A turn there is offered no
    /// `athlete` on a data read, and a call naming another member is refused
    /// rather than silently answered with the thread's athlete.
    #[tokio::test]
    async fn a_coachs_external_turn_in_an_athlete_thread_takes_no_athlete_argument() {
        let squad = squad().await;
        let thread = squad
            .res
            .common
            .repos
            .chat
            .open_subject_conversation(&NewSubjectConversation {
                user_id: &squad.casey.id.to_string(),
                tenant_id: squad.casey.tenant,
                title: "Bea Bright",
                model: "mock-model",
                agent_id: &squad.agent,
                subject: ConversationSubject {
                    athlete_user_id: squad.bea.id,
                    group_id: squad.group,
                },
            })
            .await
            .unwrap()
            .conversation
            .id;
        for (transport, grant) in external_turns() {
            *squad.model.call.lock().unwrap() =
                ("get_activities".to_owned(), json!({ "athlete": "Alex" }));
            let turn = turn_in(
                &squad,
                &squad.casey,
                transport,
                &grant,
                thread.clone(),
                true,
            )
            .await;
            *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
            // The plan tools keep their own `athlete`, which they resolve
            // themselves; no data read takes one.
            assert!(
                !turn.naming.contains("get_activities"),
                "{transport:?}: the thread names its athlete: {:?}",
                turn.naming
            );
            assert!(
                turn.conversation.contains(ATHLETE_ARGUMENT_REFUSED),
                "{transport:?}: a name in the athlete thread is refused: {}",
                turn.conversation
            );
            assert!(
                !turn.conversation.contains("ride bea-1")
                    && !turn.conversation.contains("ride alex-1"),
                "{transport:?}: no one is read for a name the thread does not take: {}",
                turn.conversation
            );
        }
    }

    /// The room rule on a read by name (carnet#1422's classifier, the one
    /// `plan_scope`'s gate 1 and the coach tools ask): an
    /// API-key chat turn in the room of a group the coach is a plain member
    /// of is refused a name, so nothing about a member of another group is
    /// read or fanned out to that room, where its coach, who does not coach
    /// that member, would read it.
    #[tokio::test]
    async fn a_coachs_api_key_turn_in_another_groups_room_reads_no_named_athlete() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let dana = coach(res, "dana@chat-surfaces.test", "Dana Coach").await;
        let dana_agent = group_agent(res, &dana).await;
        let bees = coached_group(res, dana.tenant, dana.id, &dana_agent, "Bees").await;
        join(res, bees, squad.casey.id, squad.casey.tenant, true).await;
        let room = repos
            .chat
            .create_conversation(
                &squad.casey.id.to_string(),
                squad.casey.tenant,
                "Bees",
                "mock-model",
                Some(&dana_agent),
                Some(&bees.to_string()),
            )
            .await
            .unwrap()
            .id;
        *squad.model.relay.lock().unwrap() = true;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let turn = turn_in(
            &squad,
            &squad.casey,
            Transport::ApiKey,
            &OAuthScope::self_grant(),
            room,
            false,
        )
        .await;
        assert!(
            turn.conversation.contains(ROOM_REFUSED),
            "a name in the room of a group the caller does not coach is refused: {}",
            turn.conversation
        );
        assert_offers_no_refused_name(&turn, "the Bees' room");
        assert!(
            !turn.conversation.contains(ALEX_UPLOAD),
            "Alex's ride is never read in the Bees' room: {}",
            turn.conversation
        );
        let transcript = room_reads::visible_to(repos, &bees.to_string(), dana.id, 50)
            .await
            .unwrap();
        assert!(
            transcript.iter().any(|entry| entry.content.contains(REPLY)),
            "the reply reached the room, so the check below reads a real fan-out: {:?}",
            transcript.iter().map(|e| &e.content).collect::<Vec<_>>()
        );
        assert!(
            transcript
                .iter()
                .all(|entry| !entry.content.contains(ALEX_UPLOAD)),
            "nothing about Alex reaches the room Dana reads: {:?}",
            transcript.iter().map(|e| &e.content).collect::<Vec<_>>()
        );
    }

    /// The room rule's other two rooms: in the room of the group the coach
    /// coaches, the coach seat offers no read of one athlete, so a call made
    /// anyway is refused as not offered; in a messaging group chat whose group binding failed, which
    /// keeps no group and which the route does not mark a direct message
    /// (carnet#1422), a name is refused. Neither reads Alex.
    #[tokio::test]
    async fn a_coachs_api_key_turn_names_only_where_the_room_allows() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let repos = &squad.res.common.repos;
        let own_room = repos
            .chat
            .create_conversation(
                &squad.casey.id.to_string(),
                squad.casey.tenant,
                "Squad",
                "mock-model",
                Some(&squad.agent),
                Some(&squad.group.to_string()),
            )
            .await
            .unwrap()
            .id;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let in_own_room = turn_in(
            &squad,
            &squad.casey,
            Transport::ApiKey,
            &OAuthScope::self_grant(),
            own_room,
            false,
        )
        .await;
        assert!(
            in_own_room.conversation.contains(NOT_OFFERED)
                && !in_own_room.conversation.contains(ALEX_UPLOAD),
            "in the room of the group the coach coaches, a read of one athlete is not \
             offered: {}",
            in_own_room.conversation
        );
        let unconfirmed = unbound_group_chat(&squad).await;
        let marked = route_marks_direct(&squad, &unconfirmed).await;
        assert!(
            !marked,
            "the route marks no messaging group chat a direct message (carnet#1422)"
        );
        let in_unconfirmed = turn_in(
            &squad,
            &squad.casey,
            Transport::ApiKey,
            &OAuthScope::self_grant(),
            unconfirmed,
            marked,
        )
        .await;
        assert!(
            in_unconfirmed.conversation.contains(ROOM_REFUSED)
                && !in_unconfirmed.conversation.contains(ALEX_UPLOAD),
            "a chat not confirmed private reads no one by name: {}",
            in_unconfirmed.conversation
        );
        assert_offers_no_refused_name(&in_own_room, "the coach's own room");
        assert_offers_no_refused_name(&in_unconfirmed, "an unbound group chat");
    }

    /// The coach's row of a Telegram group chat whose group binding failed,
    /// as the messaging ingress opens it (`open_new_session`): the coach's
    /// own conversation, bound to no group and no agent, that the chat's
    /// session writes into, the session recorded as a group chat's. Its
    /// tenant stands for the bot's, which the route reads it under when the
    /// two coincide.
    async fn unbound_group_chat(squad: &Squad) -> String {
        let repos = &squad.res.common.repos;
        let casey = squad.casey.id.to_string();
        let room = repos
            .chat
            .create_conversation(
                &casey,
                squad.casey.tenant,
                "Group chat",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        repos
            .messaging
            .create_session(&CreateSessionParams {
                id: &Uuid::new_v4().to_string(),
                user_id: &casey,
                tenant_id: squad.casey.tenant,
                channel_type: "telegram",
                channel_user_id: "tg-casey",
                channel_conversation_id: Some("-100123"),
                pierre_conversation_id: Some(&room),
            })
            .await
            .unwrap();
        record_chat(squad, &room, false).await;
        room
    }

    /// Record the chat of the session writing into `conversation_id` as the
    /// ingress records it on every turn.
    async fn record_chat(squad: &Squad, conversation_id: &str, is_direct_message: bool) {
        let repos = &squad.res.common.repos;
        let session = repos
            .messaging
            .get_session_by_pierre_conversation_id(squad.casey.tenant, conversation_id)
            .await
            .unwrap()
            .expect("a session writes into the conversation");
        repos
            .messaging
            .record_session_chat(
                squad.casey.tenant,
                session["id"].as_str().unwrap(),
                is_direct_message,
            )
            .await
            .unwrap();
    }

    /// Whether the chat route marks a turn on `conversation_id` a direct
    /// message, by the route's own rule.
    async fn route_marks_direct(squad: &Squad, conversation_id: &str) -> bool {
        let repos = &squad.res.common.repos;
        let row = repos
            .chat
            .get_conversation(
                conversation_id,
                &squad.casey.id.to_string(),
                squad.casey.tenant,
            )
            .await
            .unwrap()
            .expect("the coach's conversation");
        CallRoom::of_conversation(repos, &row, true)
            .await
            .is_private()
    }

    /// One message to `conversation_id` over the chat route of the served
    /// app, authorized by `api_key`: the status and the body as text.
    async fn post_over_api_key(
        app: &Router,
        api_key: &str,
        conversation_id: &str,
    ) -> (StatusCode, String) {
        post_text_over_api_key(app, api_key, conversation_id, "How is Alex doing?").await
    }

    /// [`post_over_api_key`] with `content` as the message: a slash command
    /// is typed this way too.
    async fn post_text_over_api_key(
        app: &Router,
        api_key: &str,
        conversation_id: &str,
        content: &str,
    ) -> (StatusCode, String) {
        let request = Request::post(format!(
            "/api/chat/conversations/{conversation_id}/messages"
        ))
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("authorization", api_key)
        .body(Body::from(json!({ "content": content }).to_string()))
        .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Every row of `conversation_id`'s history, joined.
    async fn history(squad: &Squad, conversation_id: &str) -> String {
        squad
            .res
            .common
            .repos
            .chat
            .get_messages(
                conversation_id,
                &squad.casey.id.to_string(),
                squad.casey.tenant,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.content)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// carnet#1422 over the real API-key route: a coach's message in the
    /// row of a messaging group chat whose group binding failed is not
    /// served as a direct message, so a read naming Alex is refused there
    /// and nothing about Alex reaches the reply or the room's history, which
    /// the chat's next turn replays to everyone in it. The coach tools are
    /// refused there too. In the coach's own API-key conversation, bound to
    /// no chat, the same read reads Alex.
    #[tokio::test]
    async fn a_coachs_api_key_message_in_an_unbound_group_chat_names_no_athlete() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let app = ProviderToolRouter::build_http_app(&squad.res);
        let api_key = issue_test_api_key(&squad.res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        *squad.model.call.lock().unwrap() = read_naming_alex();

        let own = squad
            .res
            .common
            .repos
            .chat
            .create_conversation(
                &squad.casey.id.to_string(),
                squad.casey.tenant,
                "Coach script",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(ALEX_UPLOAD),
            "in the coach's own conversation a read by name reads Alex: {body}"
        );

        let room = unbound_group_chat(&squad).await;
        squad.model.conversations.lock().unwrap().clear();
        let (status, body) = post_over_api_key(&app, &api_key, &room).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            squad.model.conversation().contains(ROOM_REFUSED),
            "the name is refused in the group chat: {}",
            squad.model.conversation()
        );
        assert!(
            !body.contains(ALEX_UPLOAD),
            "Alex's ride is not in the reply: {body}"
        );
        let stored = history(&squad, &room).await;
        assert!(
            stored.contains(REPLY),
            "the turn was written to the room, so the check below reads it: {stored}"
        );
        assert!(
            !stored.contains(ALEX_UPLOAD),
            "Alex's ride is not in the room's history: {stored}"
        );

        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
        squad.model.conversations.lock().unwrap().clear();
        let (status, body) = post_over_api_key(&app, &api_key, &room).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            squad.model.conversation().contains(NOT_OFFERED),
            "the roster is not offered in the group chat: {}",
            squad.model.conversation()
        );
        assert!(
            !history(&squad, &room).await.contains("Bea Bright"),
            "no member is named into the room"
        );
    }
    /// The row the coach's session of the Telegram group chat writes into
    /// now, as the ingress resolves it each turn.
    async fn group_chat_row(squad: &Squad) -> String {
        squad
            .res
            .common
            .repos
            .messaging
            .get_session_by_channel_identity(
                squad.casey.tenant,
                "telegram",
                "tg-casey",
                Some("-100123"),
            )
            .await
            .unwrap()
            .expect("the group chat's session")["pierre_conversation_id"]
            .as_str()
            .expect("the session writes into a conversation")
            .to_owned()
    }

    /// Point the coach's session of the Telegram group chat at
    /// `conversation_id`, as `/reset` and the session self-heal repoint it.
    async fn repoint_group_chat(squad: &Squad, conversation_id: &str) {
        let repos = &squad.res.common.repos;
        let session = repos
            .messaging
            .get_session_by_channel_identity(
                squad.casey.tenant,
                "telegram",
                "tg-casey",
                Some("-100123"),
            )
            .await
            .unwrap()
            .expect("the group chat's session");
        repos
            .messaging
            .set_session_conversation(session["id"].as_str().unwrap(), conversation_id)
            .await
            .unwrap();
    }

    /// The coach's next message in the Telegram group chat, served as the
    /// messaging ingress serves a group chat's turn (`dispatch.rs`: not a
    /// direct message), on the row its session writes into: everything the
    /// model was given, system prompt included. The model relays what it was
    /// given, so the reply the whole chat reads says no more than this.
    async fn group_chat_turn(squad: &Squad, conversation_id: &str) -> String {
        let res = &squad.res;
        squad.model.requests.lock().unwrap().clear();
        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
        pierre_chat_pipeline::execute(
            &res.chat_pipeline_context(),
            TurnRequest {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                conversation_id: conversation_id.to_owned(),
                user_id: squad.casey.id,
                conversation_tenant_id: squad.casey.tenant,
                tool_tenant_id: squad.casey.tenant,
                content: "How is everyone getting on?".to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                quote: None,
                channel_type: SurfaceId::Telegram.wire_channel(),
                transport: Transport::Messaging,
                is_direct_message: false,
                ambient_group_fallback: false,
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                hooks: PipelineHooks::none(),
                grant: OAuthScope::self_grant(),
                caller: None,
            },
            &SurfaceProfile::resolve(&SurfaceRequest {
                surface: SurfaceId::Telegram,
                locale: "en".to_owned(),
                transport: None,
                prose_contract: None,
            }),
        )
        .await
        .expect("the group chat's turn is served");
        squad.model.requests.lock().unwrap().join("\n")
    }

    /// The round-3 chain over real routes (carnet#1422): `/reset` typed over
    /// the API key in the row of a group chat whose group binding failed,
    /// Home's look at the personal thread, a read naming Alex in the row the
    /// group chat's session now writes into, then the coach's next message
    /// in the group chat. Nothing about Alex reaches what that turn was
    /// given or the reply the chat reads, whichever step refuses it.
    #[tokio::test]
    async fn reset_then_home_then_a_named_read_never_reaches_the_group_chat() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let app = ProviderToolRouter::build_http_app(&squad.res);
        let api_key = issue_test_api_key(&squad.res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        let room = unbound_group_chat(&squad).await;

        let (status, body) = post_text_over_api_key(&app, &api_key, &room, "/reset").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let fresh = group_chat_row(&squad).await;
        assert_ne!(fresh, room, "the reset repointed the group chat's session");

        let home = Request::get("/api/chat/personal")
            .header("authorization", &api_key)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(home).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Home answers over the API key"
        );

        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &fresh).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let given = group_chat_turn(&squad, &fresh).await;
        assert!(
            given.contains("How is everyone getting on?"),
            "the group chat's turn was captured: {given}"
        );
        assert!(
            !given.contains(ALEX_UPLOAD),
            "the group chat's turn, and so the reply the chat reads, holds nothing about \
             Alex: {given}"
        );
    }

    /// The replay sink on its own (carnet#1422): a read naming Alex made
    /// where the coach alone read it, in their own API-key conversation,
    /// stays in that row's history. Once a group chat's session writes into
    /// the row, the chat's next turn is given none of it; nor is the room of
    /// a group the coach is a plain member of, once the row is bound to it.
    #[tokio::test]
    async fn a_private_named_read_is_never_replayed_once_others_read_the_row() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let dana = coach(res, "dana@chat-surfaces.test", "Dana Coach").await;
        let dana_agent = group_agent(res, &dana).await;
        let bees = coached_group(res, dana.tenant, dana.id, &dana_agent, "Bees").await;
        join(res, bees, squad.casey.id, squad.casey.tenant, true).await;
        // The group chat whose binding failed, writing elsewhere for now.
        unbound_group_chat(&squad).await;

        let own = |title: &'static str| async move {
            repos
                .chat
                .create_conversation(
                    &squad.casey.id.to_string(),
                    squad.casey.tenant,
                    title,
                    "mock-model",
                    None,
                    None,
                )
                .await
                .unwrap()
                .id
        };

        // A messaging session repointed at the row.
        let repointed = own("Coach script").await;
        let (status, body) = post_over_api_key(&app, &api_key, &repointed).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            history(&squad, &repointed).await.contains(ALEX_UPLOAD),
            "the private read is in the row's history, so the replay below is tested"
        );
        repoint_group_chat(&squad, &repointed).await;
        let given = group_chat_turn(&squad, &repointed).await;
        assert!(
            given.contains("How is everyone getting on?") && !given.contains(ALEX_UPLOAD),
            "a group chat writing into the row is given nothing about Alex: {given}"
        );

        // The row bound to a group the coach is a plain member of.
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let bound = own("Coach notes").await;
        let (status, body) = post_over_api_key(&app, &api_key, &bound).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            history(&squad, &bound).await.contains(ALEX_UPLOAD),
            "the private read is in the bound row's history"
        );
        repos
            .chat
            .set_conversation_group_id(&bound, Some(&bees.to_string()), squad.casey.tenant)
            .await
            .unwrap();
        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
        squad.model.requests.lock().unwrap().clear();
        let in_bees = turn_in(
            &squad,
            &squad.casey,
            Transport::ApiKey,
            &OAuthScope::self_grant(),
            bound,
            false,
        )
        .await;
        let given = squad.model.requests.lock().unwrap().join("\n");
        assert!(
            !given.contains(ALEX_UPLOAD) && !in_bees.conversation.contains(ALEX_UPLOAD),
            "the Bees' room is given nothing about Alex: {given}"
        );
    }

    /// The extraction sink (carnet#1070's rule, on the read by name): a
    /// turn that read a named athlete is about that athlete, so nothing it
    /// says is filed on the coach, whose dossier every later turn renders,
    /// a room's included. The coach's turn that reads no one is the
    /// control: it is still extracted.
    #[tokio::test]
    async fn a_turn_that_read_a_named_athlete_files_nothing_on_the_coach() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let extraction = squad.res.chat_pipeline_context().memory_extraction_prompt;
        assert!(!extraction.is_empty(), "the fixture runs fact extraction");
        *squad.model.relay.lock().unwrap() = true;
        let named = turn_calling(
            &squad,
            &squad.casey,
            Transport::ApiKey,
            &OAuthScope::self_grant(),
            ("get_activities", json!({ "athlete": "Alex" })),
        )
        .await;
        assert!(
            named.conversation.contains(ALEX_UPLOAD),
            "the named read ran, so the turn had Alex to file: {}",
            named.conversation
        );
        // The control: a turn that reads no one, in the coach's own
        // conversation.
        turn_calling(
            &squad,
            &squad.casey,
            Transport::ApiKey,
            &OAuthScope::self_grant(),
            ("get_connection_status", json!({})),
        )
        .await;
        let extracted = |marker: &str| {
            squad
                .model
                .learning
                .lock()
                .unwrap()
                .iter()
                .filter(|pass| pass.starts_with(&extraction) && pass.contains(marker))
                .count()
        };
        let deadline = Instant::now() + StdDuration::from_secs(10);
        while extracted("") == 0 && Instant::now() < deadline {
            sleep(StdDuration::from_millis(20)).await;
        }
        assert!(
            extracted("") > 0,
            "the coach's turn that read no one is still extracted"
        );
        // The named turn's pass was spawned first: by now it would have run.
        sleep(StdDuration::from_millis(200)).await;
        assert_eq!(
            extracted(ALEX_UPLOAD),
            0,
            "no fact is extracted on the coach from a turn that read Alex"
        );
    }
    /// The one classifier on the chat route (carnet#1422): a thread a DM's
    /// messaging session still writes into, older than the personal thread,
    /// stays a direct message in the app and over an API key, so the coach
    /// keeps their plan context and a read by name there reads Alex. A
    /// personal thread a group chat's session writes into is a room, whatever
    /// its personal flag: a name is refused there.
    #[tokio::test]
    async fn the_route_serves_a_dms_thread_as_direct_and_a_group_chats_as_a_room() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let casey = squad.casey.id.to_string();
        let session = |chat: &'static str, conversation: String| {
            let casey = casey.clone();
            async move {
                repos
                    .messaging
                    .create_session(&CreateSessionParams {
                        id: &Uuid::new_v4().to_string(),
                        user_id: &casey,
                        tenant_id: squad.casey.tenant,
                        channel_type: "telegram",
                        channel_user_id: "tg-casey",
                        channel_conversation_id: Some(chat),
                        pierre_conversation_id: Some(&conversation),
                    })
                    .await
                    .unwrap();
            }
        };

        // A DM's thread from before the personal thread.
        let dm_thread = repos
            .chat
            .create_conversation(&casey, squad.casey.tenant, "DM", "mock-model", None, None)
            .await
            .unwrap()
            .id;
        session("tg-casey", dm_thread.clone()).await;
        record_chat(&squad, &dm_thread, true).await;
        assert!(
            route_marks_direct(&squad, &dm_thread).await,
            "a DM's thread is a direct message"
        );
        let (status, body) = post_over_api_key(&app, &api_key, &dm_thread).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(ALEX_UPLOAD),
            "in the coach's DM thread a read by name reads Alex: {body}"
        );

        // The personal thread, with a group chat's session pointing at it.
        let personal = repos
            .chat
            .create_conversation(&casey, squad.casey.tenant, "Home", "mock-model", None, None)
            .await
            .unwrap()
            .id;
        let home = Request::get("/api/chat/personal")
            .header("authorization", &api_key)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(home).await.unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(
            String::from_utf8_lossy(&bytes).contains(&personal),
            "Home adopted the newest own thread as personal"
        );
        session("-100123", personal.clone()).await;
        record_chat(&squad, &personal, false).await;
        assert!(
            !route_marks_direct(&squad, &personal).await,
            "a thread a group chat writes into is never a direct message"
        );
        squad.model.conversations.lock().unwrap().clear();
        let (status, body) = post_over_api_key(&app, &api_key, &personal).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            squad.model.conversation().contains(ROOM_REFUSED) && !body.contains(ALEX_UPLOAD),
            "a name is refused in it: {body}"
        );
    }

    /// The state the backfill and an old adoption leave (carnet#1422): a
    /// row designated the coach's personal thread before Home read the
    /// sessions, that an ungrouped Telegram group chat's session writes
    /// into, the session's chat not on record. It is a room, not the coach's
    /// direct message: a read by name is refused there over an API key and
    /// nothing about Alex reaches the reply. A legacy Discord DM's session,
    /// which the backfill cannot decide, reads as a room the same way until
    /// its next DM turn records its chat, and is the coach's own from then on.
    #[tokio::test]
    async fn a_personal_row_an_unrecorded_session_writes_into_is_a_room() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let casey = squad.casey.id.to_string();
        let personal = repos
            .chat
            .create_conversation(&casey, squad.casey.tenant, "Home", "mock-model", None, None)
            .await
            .unwrap()
            .id;
        assert!(
            repos
                .chat
                .designate_personal_conversation(&personal, &casey, squad.casey.tenant)
                .await
                .unwrap(),
            "the row was designated personal while no session wrote into it"
        );
        let session = |channel: &'static str, chat: &'static str| {
            let (casey, personal) = (casey.clone(), personal.clone());
            async move {
                repos
                    .messaging
                    .create_session(&CreateSessionParams {
                        id: &Uuid::new_v4().to_string(),
                        user_id: &casey,
                        tenant_id: squad.casey.tenant,
                        channel_type: channel,
                        channel_user_id: "casey",
                        channel_conversation_id: Some(chat),
                        pierre_conversation_id: Some(&personal),
                    })
                    .await
                    .unwrap();
            }
        };
        session("telegram", "-100123").await;
        assert!(
            casey_row(&squad, &personal).await.personal,
            "a messaging session leaves the row personal"
        );
        assert!(
            !route_marks_direct(&squad, &personal).await,
            "a personal row an unrecorded session writes into is not a direct message"
        );
        squad.model.conversations.lock().unwrap().clear();
        let (status, body) = post_over_api_key(&app, &api_key, &personal).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            squad.model.conversation().contains(ROOM_REFUSED),
            "the name is refused in the group chat's row: {}",
            squad.model.conversation()
        );
        assert!(
            !body.contains(ALEX_UPLOAD) && !history(&squad, &personal).await.contains(ALEX_UPLOAD),
            "nothing about Alex reaches the reply or the row: {body}"
        );

        // A legacy Discord DM's thread: a room until its next DM turn records
        // the chat, the coach's own from then on.
        let dm = repos
            .chat
            .create_conversation(&casey, squad.casey.tenant, "DM", "mock-model", None, None)
            .await
            .unwrap()
            .id;
        repos
            .messaging
            .create_session(&CreateSessionParams {
                id: &Uuid::new_v4().to_string(),
                user_id: &casey,
                tenant_id: squad.casey.tenant,
                channel_type: "discord",
                channel_user_id: "casey-discord",
                channel_conversation_id: Some("987654321"),
                pierre_conversation_id: Some(&dm),
            })
            .await
            .unwrap();
        assert!(
            !route_marks_direct(&squad, &dm).await,
            "before its next DM turn, a Discord session's thread is served as a room"
        );
        record_chat(&squad, &dm, true).await;
        assert!(
            route_marks_direct(&squad, &dm).await,
            "once its DM turn records the chat, the thread is the coach's own again"
        );
    }

    /// The row a conversation id names, read for the coach.
    async fn casey_row(squad: &Squad, conversation_id: &str) -> ConversationRecord {
        squad
            .res
            .common
            .repos
            .chat
            .get_conversation(
                conversation_id,
                &squad.casey.id.to_string(),
                squad.casey.tenant,
            )
            .await
            .unwrap()
            .expect("the coach's conversation")
    }

    /// Home's look at the coach's personal thread, over the API key: the
    /// designated thread's id, if any.
    async fn home_thread(app: &Router, api_key: &str) -> Option<String> {
        let home = Request::get("/api/chat/personal")
            .header("authorization", api_key)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(home).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Home answers over the API key"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        body["conversation"]["id"].as_str().map(ToOwned::to_owned)
    }

    /// `/reset` typed over the chat route in a group chat's row forges the
    /// fresh row on the group chat's own channel, never the surface's
    /// "web": a web row is what Home adopts as the personal thread.
    #[tokio::test]
    async fn reset_in_a_group_chats_row_keeps_the_chats_channel() {
        let squad = squad().await;
        let app = ProviderToolRouter::build_http_app(&squad.res);
        let api_key = issue_test_api_key(&squad.res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        let room = unbound_group_chat(&squad).await;
        let (status, body) = post_text_over_api_key(&app, &api_key, &room, "/reset").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let fresh = group_chat_row(&squad).await;
        assert_ne!(fresh, room, "the reset repointed the group chat's session");
        assert_eq!(
            casey_row(&squad, &fresh).await.channel_type,
            "telegram",
            "the group chat's fresh row stays on its channel"
        );
    }

    /// Home adopts no row a group chat's messaging session writes into as
    /// the personal thread, whatever its channel: the coach's newest own
    /// thread that no group chat writes into is adopted instead.
    #[tokio::test]
    async fn home_never_adopts_a_row_a_group_chat_writes_into() {
        let squad = squad().await;
        let repos = &squad.res.common.repos;
        let app = ProviderToolRouter::build_http_app(&squad.res);
        let api_key = issue_test_api_key(&squad.res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        let casey = squad.casey.id.to_string();
        let own = repos
            .chat
            .create_conversation(&casey, squad.casey.tenant, "Own", "mock-model", None, None)
            .await
            .unwrap()
            .id;
        // The group chat's row, newer, on the web channel as a reset typed
        // in the app stamped it before the fix above.
        let room = unbound_group_chat(&squad).await;
        assert_eq!(
            casey_row(&squad, &room).await.channel_type,
            "web",
            "the group chat's row is stamped on the web channel"
        );
        let adopted = home_thread(&app, &api_key).await;
        assert_eq!(
            adopted.as_deref(),
            Some(own.as_str()),
            "Home adopts the coach's own thread, never the group chat's"
        );
        assert!(
            !casey_row(&squad, &room).await.personal,
            "the group chat's row is never marked personal"
        );
    }
    /// What a tool answers when the room or the caller's role refuses the
    /// call: the coach tools' `call_room`, the room rule on a name, the
    /// coach seat of the group's room, which reads no one's data but through
    /// the group tools, and a coach-only tool's refusal of someone who
    /// coaches no group or whose coach access is off (carnet#1511 I7).
    const ROOM_GATE_REFUSALS: [&str; 11] = [
        // A tool the turn did not offer is refused before any gate
        // (carnet#1511 design call 13).
        NOT_OFFERED,
        "is not available here",
        "could not be confirmed",
        "You are not this group's coach",
        "a room is read by everyone",
        "a room would publish it",
        ROOM_REFUSED,
        // A name from someone who coaches no one matches no one (Q16).
        "No athlete matching",
        // A tool whose every call names an athlete, for someone who coaches
        // no one (carnet#1511).
        ROLE_REFUSED,
        // The roster and a member's plan status for someone who coaches no
        // group, and for a coach whose coach access was taken back.
        "You are not the human coach of any Dravr group",
        "Your coach access is off",
    ];

    /// In each of `rooms` (`(name, conversation, marked direct)`), `who`'s
    /// turn, over an API key, in the app and in a messaging chat, is offered
    /// each coach tool exactly where the tool's own gate takes the call, and
    /// `athlete` on each plan tool likewise: a coach where the caller alone
    /// reads, anyone else nowhere (Q16).
    async fn assert_offered_is_accepted(
        squad: &Squad,
        who: &Person,
        rooms: &[(&str, String, bool)],
    ) {
        let whole = [
            (ROSTER, json!({})),
            ("get_group_member_activities", json!({ "member": "Bea" })),
            ("get_group_member_plan_status", json!({ "member": "Bea" })),
        ];
        // Kept in Dravr's own app: offered on its surfaces alone, the link
        // by the athlete it names.
        let in_app = [
            ("get_provider_roster", json!({}), false),
            ("propose_athlete_link", json!({ "athlete": "Bea" }), true),
        ];
        let naming = [
            ("get_training_plan", json!({ "athlete": "Bea" })),
            ("get_planned_workouts", json!({ "athlete": "Bea" })),
            (
                "recommend_plan_flavour",
                json!({ "athlete": "Bea", "hours_per_week": 6.0, "sessions_per_week": 4 }),
            ),
        ];
        for transport in [Transport::ApiKey, Transport::WebApp, Transport::Messaging] {
            for (room, conversation, direct) in rooms {
                assert_room_offers_what_it_accepts(
                    squad,
                    who,
                    transport,
                    (room, conversation, *direct),
                    whole
                        .iter()
                        .map(|(tool, arguments)| (*tool, arguments, false))
                        .chain(
                            in_app
                                .iter()
                                .filter(|_| transport.is_first_party())
                                .map(|(tool, arguments, by_name)| (*tool, arguments, *by_name)),
                        )
                        .chain(
                            naming
                                .iter()
                                .map(|(tool, arguments)| (*tool, arguments, true)),
                        ),
                )
                .await;
            }
        }
        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
    }

    /// Clear `user`'s usage counters, on either backend.
    async fn reset_usage_counters(res: &ServerContext, user: Uuid) {
        let sql = "DELETE FROM usage_counters WHERE user_id = $1";
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(sql)
                    .bind(user.to_string())
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(sql)
                    .bind(user.to_string())
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    /// [`assert_offered_is_accepted`] for one room over one transport, each
    /// of `calls` (`(tool, arguments, by name)`) made once.
    async fn assert_room_offers_what_it_accepts<'a>(
        squad: &Squad,
        who: &Person,
        transport: Transport,
        (room, conversation, direct): (&str, &str, bool),
        calls: impl Iterator<Item = (&'a str, &'a Value, bool)>,
    ) {
        // Each room's turns count toward the day's message quota afresh, as
        // they would on separate days: the sweep is turn after turn. Only
        // `who`'s own counters, so no other test's quota moves.
        reset_usage_counters(&squad.res, who.id).await;
        for (tool, arguments, by_name) in calls {
            *squad.model.call.lock().unwrap() = (tool.to_owned(), arguments.clone());
            let served = turn_in(
                squad,
                who,
                transport,
                &OAuthScope::self_grant(),
                conversation.to_owned(),
                direct,
            )
            .await;
            let offered = if by_name {
                served.naming.contains(tool)
            } else {
                served.declared.contains(tool)
            };
            // The turn's own answer is the newest: the room replays the
            // answers of the turns before it, over another transport too.
            let answer = served
                .conversation
                .rsplit_once(&format!("[Tool Result for {tool}]"))
                .map_or_else(
                    || {
                        panic!(
                            "{transport:?} {room}: {tool} answered: {}",
                            served.conversation
                        )
                    },
                    |(_, answer)| answer.to_owned(),
                );
            // A tool the turn withheld is refused by name before any gate,
            // which says nothing of the gate: ask the gate itself, through
            // an executor bound as the turn's, so a listing that withholds
            // what its gate would take fails here (carnet#1511 I7).
            let answer = if answer.contains(NOT_OFFERED) {
                assert!(
                    !offered,
                    "{transport:?} {room}: {tool} was offered yet refused as not offered: {answer}"
                );
                gate_answer(
                    squad,
                    who,
                    transport,
                    (conversation, direct),
                    tool,
                    arguments,
                )
                .await
            } else {
                answer
            };
            let accepted = !ROOM_GATE_REFUSALS
                .iter()
                .any(|refusal| answer.contains(refusal));
            assert_eq!(
                offered, accepted,
                "{transport:?} {room}: {tool} offered={offered} but its gate \
                 accepted={accepted}: {answer}"
            );
        }
    }

    /// What the gates answer `who` calling `tool` with `arguments` in
    /// `conversation` over `transport`, past the turn's name check: an
    /// executor bound as the pipeline binds the turn's (its role, room,
    /// seat and direct-message flag), called directly.
    async fn gate_answer(
        squad: &Squad,
        who: &Person,
        transport: Transport,
        (conversation, direct): (&str, bool),
        tool: &str,
        arguments: &Value,
    ) -> String {
        let repos = &squad.res.common.repos;
        let group = repos
            .chat
            .get_conversation(conversation, &who.id.to_string(), who.tenant)
            .await
            .unwrap()
            .and_then(|row| row.group_id);
        let seat = match group {
            Some(group) if coaches_group(repos, who.id, &group).await.unwrap() => TurnSeat::Coach,
            _ => TurnSeat::Subject,
        };
        let runtime: Arc<dyn ToolRuntime> = squad.res.clone();
        let executor = UniversalExecutor::new(runtime)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(transport)
            .with_turn_role(CallerRole::resolve(repos, who.id).await)
            .with_conversation_id(conversation.to_owned())
            .with_conversation_tenant(who.tenant.as_uuid())
            .with_turn_token(Uuid::new_v4().to_string())
            .with_seat(seat);
        let response = TURN_DIRECT_MESSAGE
            .scope(Some(direct), async {
                executor
                    .execute_tool(UniversalRequest {
                        tool_name: tool.to_owned(),
                        parameters: arguments.clone(),
                        user_id: who.id.to_string(),
                        protocol: "test".to_owned(),
                        tenant_id: Some(who.tenant.to_string()),
                    })
                    .await
            })
            .await;
        match response {
            Ok(response) => format!(
                "{} {}",
                response.error.unwrap_or_default(),
                response.result.unwrap_or_default()
            ),
            Err(error) => error.to_string(),
        }
    }

    /// The listing follows the room, for whole tools and for `athlete`
    /// (carnet#1422): in every room kind a coach's turn, over an API key, in
    /// the app and in a messaging chat, is offered a coach tool, a tool kept
    /// in the app, and `athlete` on a plan tool, exactly where the tool's own
    /// room gate takes the call; an admin who coaches nothing and a plain
    /// athlete are offered `athlete` nowhere (Q16). A plain athlete and a
    /// coach whose coach access was taken back are swept on the coach tools
    /// too, so a role refusal counts as a refusal (carnet#1511 I7). No
    /// exception: whatever is offered is taken, whatever is withheld would
    /// be refused. A member's activities answer a name they cannot read with
    /// a miss, which is the tool's answer, not a refusal of the call.
    #[tokio::test]
    async fn every_room_offers_exactly_what_its_gates_accept() {
        let squad = squad().await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let casey = squad.casey.id.to_string();
        let dana = coach(res, "dana@chat-surfaces.test", "Dana Coach").await;
        let dana_agent = group_agent(res, &dana).await;
        let bees = coached_group(res, dana.tenant, dana.id, &dana_agent, "Bees").await;
        join(res, bees, squad.casey.id, squad.casey.tenant, true).await;
        let row = |title: &'static str, agent: Option<String>, group: Option<String>| {
            let casey = casey.clone();
            async move {
                repos
                    .chat
                    .create_conversation(
                        &casey,
                        squad.casey.tenant,
                        title,
                        "mock-model",
                        agent.as_deref(),
                        group.as_deref(),
                    )
                    .await
                    .unwrap()
                    .id
            }
        };
        let private = row("Own", None, None).await;
        let coached = row(
            "Squad",
            Some(squad.agent.clone()),
            Some(squad.group.to_string()),
        )
        .await;
        let member = row("Bees", Some(dana_agent.clone()), Some(bees.to_string())).await;
        let unconfirmed = unbound_group_chat(&squad).await;
        let rooms = [
            ("private", private, true),
            ("the coached group's room", coached, false),
            ("another group's room", member, false),
            ("an unbound group chat", unconfirmed, false),
        ];
        assert_offered_is_accepted(&squad, &squad.casey, &rooms).await;
        let rooms_of_casey = rooms;

        // An admin who coaches nothing: offered the coach tools in private
        // alone, and refused them in any group's room.
        let ada = person(res, "ada@chat-surfaces.test", "Ada Admin").await;
        repos.users.set_admin_status(ada.id, true).await.unwrap();
        seed_activities(res, &ada, INTERVALS_ICU, &[intervals_ride("ada-1", 2)]).await;
        repos
            .email_verification
            .mark_verified(ada.id)
            .await
            .unwrap();
        let ada_tenant = ada.tenant;
        let ada_row = |title: &'static str, group: Option<String>| {
            let ada = ada.id.to_string();
            async move {
                repos
                    .chat
                    .create_conversation(
                        &ada,
                        ada_tenant,
                        title,
                        "mock-model",
                        None,
                        group.as_deref(),
                    )
                    .await
                    .unwrap()
                    .id
            }
        };
        let rooms = [
            (
                "an admin's own conversation",
                ada_row("Own", None).await,
                true,
            ),
            (
                "a group's room an admin reads",
                ada_row("Squad", Some(squad.group.to_string())).await,
                false,
            ),
        ];
        assert_offered_is_accepted(&squad, &ada, &rooms).await;

        // A plain athlete, who coaches no one and is no admin: offered no
        // plan tool's `athlete` on any transport, in the app included, and
        // a name they send anyway matches no one (Q16).
        let bea_own = repos
            .chat
            .create_conversation(
                &squad.bea.id.to_string(),
                squad.bea.tenant,
                "Own",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let naming = [
            ("get_training_plan", json!({ "athlete": "Alex" })),
            ("get_planned_workouts", json!({ "athlete": "Alex" })),
        ];
        for transport in [Transport::ApiKey, Transport::WebApp, Transport::Messaging] {
            assert_room_offers_what_it_accepts(
                &squad,
                &squad.bea,
                transport,
                ("a plain athlete's own conversation", &bea_own, true),
                naming
                    .iter()
                    .map(|(tool, arguments)| (*tool, arguments, true)),
            )
            .await;
        }

        // The same plain athlete, offered each coach tool exactly where its
        // gate takes her call, in the app too: the roster and a member's
        // plan status, which serve a coach alone, nowhere; a member's
        // activities, which serve the group's peers, in her own
        // conversation and in her group's room (carnet#1511 I7).
        let bea_room = repos
            .chat
            .create_conversation(
                &squad.bea.id.to_string(),
                squad.bea.tenant,
                "Squad",
                "mock-model",
                Some(&squad.agent),
                Some(&squad.group.to_string()),
            )
            .await
            .unwrap()
            .id;
        let whole = [
            (ROSTER, json!({})),
            ("get_group_member_activities", json!({ "member": "Alex" })),
            ("get_group_member_plan_status", json!({ "member": "Alex" })),
        ];
        for transport in [Transport::ApiKey, Transport::WebApp, Transport::Messaging] {
            for (room, conversation, direct) in [
                ("a plain athlete's own conversation", &bea_own, true),
                ("a plain athlete's group room", &bea_room, false),
            ] {
                assert_room_offers_what_it_accepts(
                    &squad,
                    &squad.bea,
                    transport,
                    (room, conversation, direct),
                    whole
                        .iter()
                        .map(|(tool, arguments)| (*tool, arguments, false)),
                )
                .await;
            }
        }

        // A coach whose coach access was taken back is offered neither the
        // roster nor a member's plan status, in the app too, and reads the
        // room of the group they are still attached to as any member does:
        // never as its coach (the one coach rule, carnet#1511 I7).
        repos
            .users
            .set_manages_roster(squad.casey.id, false)
            .await
            .unwrap();
        let coach_only = [
            (ROSTER, json!({})),
            ("get_group_member_plan_status", json!({ "member": "Bea" })),
        ];
        for transport in [Transport::ApiKey, Transport::WebApp, Transport::Messaging] {
            for (room, conversation, direct) in &rooms_of_casey {
                assert_room_offers_what_it_accepts(
                    &squad,
                    &squad.casey,
                    transport,
                    (room, conversation, *direct),
                    coach_only
                        .iter()
                        .map(|(tool, arguments)| (*tool, arguments, false)),
                )
                .await;
            }
        }
        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
    }

    /// The coach's own API-key conversation, bound to no chat.
    async fn own_conversation(squad: &Squad, title: &str) -> String {
        squad
            .res
            .common
            .repos
            .chat
            .create_conversation(
                &squad.casey.id.to_string(),
                squad.casey.tenant,
                title,
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id
    }

    /// A follow-up in `conversation_id` that reads no member: the model
    /// checks the coach's own connections and relays what it was given, the
    /// replayed history included, as a reply restating an earlier read does.
    async fn restate_over_api_key(
        squad: &Squad,
        app: &Router,
        api_key: &str,
        conversation_id: &str,
    ) -> String {
        *squad.model.call.lock().unwrap() = ("get_connection_status".to_owned(), json!({}));
        let (status, body) =
            post_text_over_api_key(app, api_key, conversation_id, "Remind me what we found?").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// carnet#1422, the restatement: a read naming Alex where the coach
    /// alone reads it, then a follow-up that reads no one and restates it
    /// from the replayed history. Once a group chat's session writes into the
    /// row, the chat's next turn is given nothing about Alex, the restatement
    /// included: the conversation remembers what it read, so every row from
    /// the first read on is withheld from a room that may not read Alex. The
    /// restating reply names Alex as its own read, and nothing the coach said
    /// in either turn is filed on the coach
    /// (`a_follow_up_restating_a_read_files_nothing_on_the_coach`).
    #[tokio::test]
    async fn a_restated_private_read_never_reaches_a_room() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        unbound_group_chat(&squad).await;
        let own = own_conversation(&squad, "Coach script").await;

        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(ALEX_UPLOAD),
            "the private read reads Alex: {body}"
        );
        let restated = restate_over_api_key(&squad, &app, &api_key, &own).await;
        assert!(
            restated.contains(ALEX_UPLOAD),
            "the follow-up restates Alex's ride from the replayed history: {restated}"
        );
        let rows = res
            .common
            .repos
            .chat
            .get_messages(&own, &squad.casey.id.to_string(), squad.casey.tenant)
            .await
            .unwrap();
        let reply = rows
            .iter()
            .rev()
            .find(|row| row.role == "assistant")
            .expect("the follow-up's reply");
        assert_eq!(
            carried_members(&squad, &own, &reply.id).await,
            BTreeSet::from([squad.alex.id]),
            "the restating reply carries Alex, as the read it restates"
        );
        // A restatement a build before carnet#1422 wrote, stamped about no
        // one: what the conversation read still holds it back.
        res.common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: squad.casey.tenant,
                conversation_id: &own,
                user_id: &squad.casey.id.to_string(),
                role: "assistant",
                content: &format!("Earlier you asked about {ALEX_UPLOAD}."),
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

        repoint_group_chat(&squad, &own).await;
        let given = group_chat_turn(&squad, &own).await;
        assert!(
            given.contains("How is everyone getting on?") && !given.contains(ALEX_UPLOAD),
            "the group chat is given nothing about Alex, the restatements included: {given}"
        );
    }

    /// carnet#1422, a read of several members: a roster naming Alex and Bea
    /// where the coach alone reads it is stored as about both, never about
    /// nobody, and the follow-up that restates it too. A group chat writing
    /// into the row is then given neither name.
    #[tokio::test]
    async fn a_private_roster_read_of_several_members_never_reaches_a_room() {
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        unbound_group_chat(&squad).await;
        res.common
            .repos
            .groups
            .set_coach_assistant_read(
                &squad.group.to_string(),
                squad.alex.id,
                true,
                Timestamp::now(),
            )
            .await
            .unwrap()
            .expect("a live member");
        let own = own_conversation(&squad, "Coach roster").await;

        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
        let (status, body) =
            post_text_over_api_key(&app, &api_key, &own, "How is my group doing?").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains("Bea Bright") && body.contains("Alex Athlete"),
            "the private roster names both members: {body}"
        );
        let restated = restate_over_api_key(&squad, &app, &api_key, &own).await;
        assert!(restated.contains("Bea Bright"), "{restated}");
        let reads = res
            .common
            .repos
            .chat
            .list_member_reads(&own, &squad.casey.id.to_string(), squad.casey.tenant)
            .await
            .unwrap();
        for member in [squad.alex.id, squad.bea.id] {
            assert!(
                reads.iter().any(|read| CarriedRead::from_key(&read.key)
                    .is_some_and(|read| read.member == member)),
                "the roster's rows are about {member}, never about nobody: {reads:?}"
            );
        }

        repoint_group_chat(&squad, &own).await;
        let given = group_chat_turn(&squad, &own).await;
        assert!(
            given.contains("How is everyone getting on?")
                && !given.contains("Bea Bright")
                && !given.contains("bea-1")
                && !given.contains("Alex Athlete"),
            "the group chat is given no member the roster named: {given}"
        );
    }

    /// carnet#1422, compaction: a summary covering the private read and its
    /// restatement replays with them in the coach's own conversation, and
    /// is withheld with them from a group chat writing into the row.
    #[tokio::test]
    async fn a_summary_of_a_private_read_never_reaches_a_room() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        unbound_group_chat(&squad).await;
        let own = own_conversation(&squad, "Coach summary").await;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        restate_over_api_key(&squad, &app, &api_key, &own).await;
        let rows = repos
            .chat
            .get_messages(&own, &squad.casey.id.to_string(), squad.casey.tenant)
            .await
            .unwrap();
        // The summary the compactor would write over every row so far, its
        // last the restating reply, about nobody: the reply's summary names
        // no member of its own.
        repos
            .memory
            .insert_compaction_block(&InsertCompactionBlockParams {
                tenant_id: squad.casey.tenant,
                conversation_id: &own,
                summary: SUMMARY_MARKER,
                summary_tokens: 10,
                original_tokens: 100,
                first_message_id: &rows.first().unwrap().id,
                last_message_id: &rows.last().unwrap().id,
                transport_policy: TransportPolicy::AnyTransport,
                about_user_id: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap();

        // The control: the coach's own next turn is given the summary.
        squad.model.requests.lock().unwrap().clear();
        restate_over_api_key(&squad, &app, &api_key, &own).await;
        let own_given = squad.model.requests.lock().unwrap().join("\n");
        assert!(
            own_given.contains(SUMMARY_MARKER),
            "the coach's own conversation replays the summary: {own_given}"
        );

        repoint_group_chat(&squad, &own).await;
        let given = group_chat_turn(&squad, &own).await;
        assert!(
            given.contains("How is everyone getting on?")
                && !given.contains(SUMMARY_MARKER)
                && !given.contains(ALEX_UPLOAD),
            "the group chat is given no summary of the read: {given}"
        );
    }

    /// carnet#1422, extraction: nothing a follow-up restating a private read
    /// says is filed on the coach, as nothing the read's own turn says is
    /// (carnet#1070's rule). A conversation that never read anyone is still
    /// extracted.
    #[tokio::test]
    async fn a_follow_up_restating_a_read_files_nothing_on_the_coach() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let extraction = res.chat_pipeline_context().memory_extraction_prompt;
        assert!(!extraction.is_empty(), "the fixture runs fact extraction");
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        let own = own_conversation(&squad, "Coach script").await;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let restated = restate_over_api_key(&squad, &app, &api_key, &own).await;
        assert!(
            restated.contains(ALEX_UPLOAD),
            "the follow-up restates Alex, so it had Alex to file: {restated}"
        );

        // The control: a conversation that never read anyone.
        let fresh = own_conversation(&squad, "Coach notes").await;
        restate_over_api_key(&squad, &app, &api_key, &fresh).await;
        let passes = |marker: &str| {
            squad
                .model
                .learning
                .lock()
                .unwrap()
                .iter()
                .filter(|pass| pass.starts_with(&extraction) && pass.contains(marker))
                .count()
        };
        let deadline = Instant::now() + StdDuration::from_secs(10);
        while passes("") == 0 && Instant::now() < deadline {
            sleep(StdDuration::from_millis(20)).await;
        }
        assert!(
            passes("") > 0,
            "a conversation that read no one is extracted"
        );
        // Both of the read's turns were spawned first: by now they would have run.
        sleep(StdDuration::from_millis(200)).await;
        assert_eq!(
            passes(ALEX_UPLOAD),
            0,
            "no fact is extracted on the coach from a turn that restates Alex"
        );
    }

    /// What the coach's turn in `conversation_id` over an API key is given,
    /// the turn a direct message or not: every request whole.
    async fn given_over_api_key(
        squad: &Squad,
        conversation_id: &str,
        is_direct_message: bool,
    ) -> String {
        *squad.model.call.lock().unwrap() = ("get_connection_status".to_owned(), json!({}));
        squad.model.requests.lock().unwrap().clear();
        turn_in(
            squad,
            &squad.casey,
            Transport::ApiKey,
            &OAuthScope::self_grant(),
            conversation_id.to_owned(),
            is_direct_message,
        )
        .await;
        squad.model.requests.lock().unwrap().join("\n")
    }

    /// carnet#1422, the row's audience changing under a private read and its
    /// restatement. Bound to the group the coach coaches, whose consent lets
    /// the coach read Alex, the room is given the read: the absences below
    /// are seen against it. Unbound again, a group chat writing into the row
    /// is given nothing about Alex, while the coach's own turn there is given
    /// the read again. Once Alex leaves the group, no turn is given anything
    /// about Alex, the restatement included.
    #[tokio::test]
    async fn a_private_read_follows_the_rows_audience_as_it_changes() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        unbound_group_chat(&squad).await;
        let row = own_conversation(&squad, "Coach script").await;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &row).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let restated = restate_over_api_key(&squad, &app, &api_key, &row).await;
        assert!(restated.contains(ALEX_UPLOAD), "{restated}");

        // Bound to the group the coach coaches.
        repos
            .chat
            .set_conversation_group_id(&row, Some(&squad.group.to_string()), squad.casey.tenant)
            .await
            .unwrap();
        let coached = given_over_api_key(&squad, &row, false).await;
        assert!(
            coached.contains(ALEX_UPLOAD),
            "the coached group's room reads what its consent lets the coach read: {coached}"
        );

        // Unbound: a group chat writing into the row, then the coach alone.
        repos
            .chat
            .set_conversation_group_id(&row, None, squad.casey.tenant)
            .await
            .unwrap();
        repoint_group_chat(&squad, &row).await;
        let chat = group_chat_turn(&squad, &row).await;
        assert!(
            chat.contains("How is everyone getting on?") && !chat.contains(ALEX_UPLOAD),
            "a group chat writing into the unbound row is given nothing about Alex: {chat}"
        );
        let elsewhere = own_conversation(&squad, "Elsewhere").await;
        repoint_group_chat(&squad, &elsewhere).await;
        let own = given_over_api_key(&squad, &row, true).await;
        assert!(
            own.contains(ALEX_UPLOAD),
            "the coach's own turn in the row reads the read again: {own}"
        );

        // A membership change: Alex leaves the group.
        assert!(
            repos
                .groups
                .remove_member(&squad.group.to_string(), squad.alex.id)
                .await
                .unwrap(),
            "Alex is removed from the group"
        );
        let after = given_over_api_key(&squad, &row, true).await;
        assert!(
            !after.contains(ALEX_UPLOAD),
            "once Alex left, no turn is given the read or its restatement: {after}"
        );
    }

    /// Dana, a user in the coach's own tenant who coaches no one and is in
    /// no group, with a token scoped to that tenant, as a participant the
    /// coach adds reads the thread.
    async fn colleague(squad: &Squad) -> Person {
        let res = &squad.res;
        let (id, user, _) = create_test_user_with_plan(
            &res.agent.database,
            "dana@chat-surfaces.test",
            "professional",
        )
        .await
        .unwrap();
        res.common
            .repos
            .users
            .update_tenant_id(id, squad.casey.tenant)
            .await
            .unwrap();
        let token = res
            .auth
            .auth_manager
            .generate_token_with_tenant(
                &user,
                &res.auth.jwks_manager,
                Some(squad.casey.tenant.to_string()),
            )
            .unwrap();
        Person {
            id,
            tenant: squad.casey.tenant,
            auth: format!("Bearer {token}"),
        }
    }

    /// `method` `path` on the served app as `auth`, with `body` when given:
    /// the status and the body as text.
    async fn call_as(
        app: &Router,
        auth: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, String) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("authorization", auth)
            .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The transcript of `conversation_id` as `auth` reads it over the
    /// route.
    async fn transcript_as(app: &Router, auth: &str, conversation_id: &str) -> String {
        let (status, body) = call_as(
            app,
            auth,
            "GET",
            &format!("/api/chat/conversations/{conversation_id}/messages"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// carnet#1422, a shared thread: a coach's own conversation with a
    /// colleague added to it is read by that colleague, so it is never
    /// private. A read naming Alex made while the coach alone read the
    /// thread is not served to the colleague added afterwards, in the
    /// transcript or its preview, while the coach still reads it; and once
    /// the colleague is in, a read naming Alex and the coach roster are
    /// refused there, over the API key and over the app.
    #[tokio::test]
    async fn a_participant_added_to_a_coachs_thread_reads_nothing_about_a_member() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        let dana = colleague(&squad).await;
        let own = own_conversation(&squad, "Coach script").await;

        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(ALEX_UPLOAD),
            "the coach alone reads the thread, so the read names Alex: {body}"
        );
        // A short reply about Alex as the newest row, the one a preview shows.
        res.common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: squad.casey.tenant,
                conversation_id: &own,
                user_id: &squad.casey.id.to_string(),
                role: "assistant",
                content: &format!("Alex rode {ALEX_UPLOAD}."),
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: Some("web"),
                about_user_id: Some(squad.alex.id),
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap();

        let (status, body) = call_as(
            &app,
            &squad.casey.auth,
            "POST",
            &format!("/api/chat/conversations/{own}/participants"),
            Some(json!({ "user_id": dana.id.to_string() })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert!(
            !route_marks_direct(&squad, &own).await,
            "a thread someone else participates in is never private"
        );

        let read_by_dana = transcript_as(&app, &dana.auth, &own).await;
        assert!(
            read_by_dana.contains("How is Alex doing?") && !read_by_dana.contains(ALEX_UPLOAD),
            "the colleague is served nothing the coach read about Alex: {read_by_dana}"
        );
        let (status, listed) =
            call_as(&app, &dana.auth, "GET", "/api/chat/conversations", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(
            listed.contains(&own) && !listed.contains(ALEX_UPLOAD),
            "nor in the thread's preview: {listed}"
        );
        let read_by_casey = transcript_as(&app, &squad.casey.auth, &own).await;
        assert!(
            read_by_casey.contains(ALEX_UPLOAD),
            "the coach still reads what they may read: {read_by_casey}"
        );

        *squad.model.call.lock().unwrap() = read_naming_alex();
        squad.model.conversations.lock().unwrap().clear();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let conversation = squad.model.conversation();
        assert!(
            conversation.contains(ROOM_REFUSED) && !conversation.contains(ALEX_UPLOAD),
            "a read naming Alex is refused once the colleague is in: {conversation}"
        );

        for auth in [api_key.as_str(), squad.casey.auth.as_str()] {
            *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
            squad.model.conversations.lock().unwrap().clear();
            let (status, body) =
                post_text_over_api_key(&app, auth, &own, "How is my group doing?").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let conversation = squad.model.conversation();
            assert!(
                !conversation.contains("Bea Bright") && !body.contains("Bea Bright"),
                "the roster is not read where the colleague reads the reply: {conversation}"
            );
        }
        let read_by_dana = transcript_as(&app, &dana.auth, &own).await;
        assert!(
            !read_by_dana.contains(ALEX_UPLOAD) && !read_by_dana.contains("Bea Bright"),
            "the colleague reads nothing about a member: {read_by_dana}"
        );
    }

    /// Alex's sleep the coach's wellness read is served: a night length no
    /// other fixture writes.
    const ALEX_SLEEP_SECONDS: u32 = 26_113;

    /// File `nights` nights of Alex's sleep, synced from Intervals.icu.
    async fn file_alex_sleep(squad: &Squad, nights: i64) {
        let repos = &squad.res.common.repos;
        let alex = &squad.alex;
        let source = repos
            .data_sources
            .upsert_data_source(
                &alex.tenant,
                &DataSource {
                    id: String::new(),
                    user_id: alex.id.to_string(),
                    provider: INTERVALS_ICU.to_owned(),
                    device_model: None,
                    software_version: None,
                    source: None,
                    device_type: DeviceType::Band,
                    original_source_name: None,
                },
            )
            .await
            .unwrap();
        let now = Timestamp::now();
        for night in 0..nights {
            let woke = now - Elapsed::days(night) - Elapsed::minutes(5);
            repos
                .sleep
                .upsert_sleep_session(
                    &alex.tenant,
                    &StoredSleepSession {
                        id: format!("icu-sleep-{}", Uuid::new_v4()),
                        user_id: alex.id.to_string(),
                        data_source_id: source.clone(),
                        is_nap: false,
                        start_datetime: woke - Elapsed::seconds(i64::from(ALEX_SLEEP_SECONDS)),
                        end_datetime: woke,
                        total_sleep_seconds: Some(ALEX_SLEEP_SECONDS),
                        deep_sleep_seconds: None,
                        light_sleep_seconds: None,
                        rem_sleep_seconds: None,
                        awake_seconds: None,
                        sleep_efficiency: None,
                        avg_heart_rate: None,
                        min_heart_rate: None,
                        avg_hrv: Some(87.125),
                        sleep_score: None,
                        avg_spo2: None,
                        stages: Vec::new(),
                        source_name: INTERVALS_ICU.to_owned(),
                    },
                    now,
                )
                .await
                .unwrap();
        }
    }

    /// carnet#1511 call 19, the round-11 room-gate blocker: a stored read
    /// records how it was made and its tier, so a reader is admitted by the
    /// way it was read, never by whether they may read the member at all.
    /// The coach reads Alex's training, then her wellness, by name over an
    /// API key in their own thread; Bree, a member of the same group whom
    /// Alex shares her training with, is then added to the thread. Bree reads
    /// the training read (Alex's peer sharing covers it) but never the
    /// wellness read, which stays the coach's under Alex's wellness grant;
    /// the coach still reads both. Once the thread is bound to the group,
    /// the room's next turn replays the wellness read, and the reply it
    /// writes carries it: Bea's room page never shows it.
    #[tokio::test]
    async fn a_coachs_wellness_read_stays_the_coachs_where_a_peer_reads_the_thread() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        file_alex_sleep(&squad, 3).await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let group = squad.group.to_string();
        let group_tenant = repos
            .groups
            .list_groups_coached_by(squad.casey.id)
            .await
            .unwrap()
            .into_iter()
            .find(|g| g.id == squad.group)
            .map(|g| TenantId::parse_str(&g.tenant_id).unwrap())
            .unwrap();
        repos
            .groups
            .update_group(
                &group,
                group_tenant,
                &UpdateGroupRequest {
                    name: None,
                    description: None,
                    agent_id: None,
                    peer_data_sharing: Some(true),
                    respond_mode: None,
                    digest_mode: None,
                    is_active: None,
                },
            )
            .await
            .unwrap()
            .expect("the group");
        assert!(
            repos
                .groups
                .update_peer_sharing_consent(&group, squad.alex.id, true)
                .await
                .unwrap(),
            "Alex's peer-sharing consent is recorded"
        );
        repos
            .groups
            .set_coach_consents(&group, squad.alex.id, None, Some(true), Timestamp::now())
            .await
            .unwrap();
        let bree = colleague(&squad).await;
        join(res, squad.group, bree.id, bree.tenant, true).await;

        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        let own = own_conversation(&squad, "Coach script").await;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        *squad.model.call.lock().unwrap() = (
            "get_sleep_sessions".to_owned(),
            json!({ "athlete": "Alex" }),
        );
        let (status, body) =
            post_text_over_api_key(&app, &api_key, &own, "How did Alex sleep?").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let sleep = ALEX_SLEEP_SECONDS.to_string();
        let read_by_casey = transcript_as(&app, &squad.casey.auth, &own).await;
        assert!(
            read_by_casey.contains(ALEX_UPLOAD) && read_by_casey.contains(&sleep),
            "premise: the coach read Alex's training and her sleep: {read_by_casey}"
        );

        let (status, body) = call_as(
            &app,
            &squad.casey.auth,
            "POST",
            &format!("/api/chat/conversations/{own}/participants"),
            Some(json!({ "user_id": bree.id.to_string() })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let read_by_bree = transcript_as(&app, &bree.auth, &own).await;
        assert!(
            read_by_bree.contains(ALEX_UPLOAD),
            "Alex shares her training with the group, so a member reads the training read: \
             {read_by_bree}"
        );
        assert!(
            !read_by_bree.contains(&sleep),
            "bug: a member read Alex's wellness, which she shares with her coach alone: \
             {read_by_bree}"
        );
        let read_by_casey = transcript_as(&app, &squad.casey.auth, &own).await;
        assert!(
            read_by_casey.contains(&sleep),
            "the coach still reads her own wellness read: {read_by_casey}"
        );
        let (status, listed) =
            call_as(&app, &bree.auth, "GET", "/api/chat/conversations", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(!listed.contains(&sleep), "nor in the preview: {listed}");

        // Bound to the group, the room replays the wellness read into the
        // coach's next turn, and the reply carries it: Bea's room page never
        // reads it. (A route that binds a thread holding a coaching turn is
        // not known; the bind is the repository's, as the verifier's was.)
        assert!(
            repos
                .chat
                .remove_participant(&own, squad.casey.tenant, &bree.id.to_string())
                .await
                .unwrap(),
            "Bree is removed from the conversation"
        );
        repos
            .chat
            .set_conversation_group_id(&own, Some(&group), squad.casey.tenant)
            .await
            .unwrap();
        *squad.model.call.lock().unwrap() = ("get_connection_status".to_owned(), json!({}));
        let (status, body) =
            post_text_over_api_key(&app, &api_key, &own, "Remind me what we found?").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(&sleep),
            "premise: the room's turn replayed the wellness read for its coach: {body}"
        );
        let (status, room) = call_as(
            &app,
            &squad.bea.auth,
            "GET",
            &format!("/api/chat/groups/{group}/transcript"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{room}");
        assert!(
            !room.contains(&sleep),
            "bug: the room page served a member Alex's wellness the coach read: {room}"
        );
    }

    /// carnet#1511 R2: a turn that replays another participant's own words
    /// reads them, as a tool's read is read. Dana, added to the coach's
    /// thread, writes there first; her own turn reads no one else, so what
    /// she said is extracted on her. The coach's next turn replays her row:
    /// its reply carries her words as a thread read, and nothing the coach
    /// said there is extracted on the coach.
    #[tokio::test]
    async fn a_turn_replaying_another_participants_words_files_nothing_on_its_sender() {
        const DANA_SAYS: &str = "dana-r2: my left knee aches after 30 km";
        const CASEY_SAYS: &str = "casey-r2: I prefer morning sessions";
        let squad = squad().await;
        let res = &squad.res;
        let extraction = res.chat_pipeline_context().memory_extraction_prompt;
        assert!(!extraction.is_empty(), "the fixture runs fact extraction");
        let app = ProviderToolRouter::build_http_app(res);
        let dana = colleague(&squad).await;
        let own = own_conversation(&squad, "Shared thread").await;
        let (status, body) = call_as(
            &app,
            &squad.casey.auth,
            "POST",
            &format!("/api/chat/conversations/{own}/participants"),
            Some(json!({ "user_id": dana.id.to_string() })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        *squad.model.call.lock().unwrap() = ("get_connection_status".to_owned(), json!({}));
        let (status, body) = post_text_over_api_key(&app, &dana.auth, &own, DANA_SAYS).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) =
            post_text_over_api_key(&app, &squad.casey.auth, &own, CASEY_SAYS).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let repos = &res.common.repos;
        let casey = squad.casey.id.to_string();
        let reply = repos
            .chat
            .get_messages(&own, &casey, squad.casey.tenant)
            .await
            .unwrap()
            .into_iter()
            .rfind(|row| row.role == "assistant")
            .expect("the coach's turn replied")
            .id;
        let carried: BTreeSet<String> = repos
            .chat
            .list_member_reads(&own, &casey, squad.casey.tenant)
            .await
            .unwrap()
            .into_iter()
            .filter(|read| read.message_id == reply)
            .map(|read| read.key)
            .collect();
        assert!(
            carried.contains(&CarriedRead::thread(dana.id).key()),
            "bug: the agent's reply on the human coach's turn replayed Dana's words without carrying them: {carried:?}"
        );

        let extracted = |marker: &str| {
            squad
                .model
                .learning
                .lock()
                .unwrap()
                .iter()
                .filter(|pass| pass.starts_with(&extraction) && pass.contains(marker))
                .count()
        };
        let deadline = Instant::now() + StdDuration::from_secs(10);
        while extracted(DANA_SAYS) == 0 && Instant::now() < deadline {
            sleep(StdDuration::from_millis(20)).await;
        }
        assert!(
            extracted(DANA_SAYS) > 0,
            "premise: Dana's own turn, which read no one else, is extracted"
        );
        sleep(StdDuration::from_millis(200)).await;
        assert_eq!(
            extracted(CASEY_SAYS),
            0,
            "bug: the coach's turn replayed Dana's words and was extracted on the coach"
        );
    }

    /// The members row `row` of `conversation` carries a read of, as the
    /// coach reads the thread (carnet#1511 call 19: the link rows are the one
    /// carrier; a new row names no one in `about_user_id`).
    async fn carried_members(squad: &Squad, conversation: &str, row: &str) -> BTreeSet<Uuid> {
        squad
            .res
            .common
            .repos
            .chat
            .list_member_reads(
                conversation,
                &squad.casey.id.to_string(),
                squad.casey.tenant,
            )
            .await
            .unwrap()
            .into_iter()
            .filter(|read| read.message_id == row)
            .filter_map(|read| CarriedRead::from_key(&read.key).map(|read| read.member))
            .collect()
    }

    /// The coach's own FTP as stored.
    async fn coach_ftp(squad: &Squad) -> Option<u32> {
        squad
            .res
            .common
            .repos
            .user_physiological_profile
            .get_user_physiological_profile(squad.casey.tenant, squad.casey.id)
            .await
            .unwrap()
            .and_then(|profile| profile.ftp_watts)
    }

    /// carnet#1422, writes after a read: a turn that read another member is
    /// about that member, so a write that would land on the caller's own
    /// account is refused there, and withheld from the listing of every later
    /// turn that replays the read. The model reads Alex, then calls
    /// `set_physiology` with Alex's FTP: refused, and the coach's own
    /// physiology is unchanged; on the follow-up, which replays the read, the
    /// write is not offered and is refused again. Without a read the same
    /// write lands on the coach.
    #[tokio::test]
    async fn a_turn_that_read_a_member_writes_nothing_on_the_coachs_account() {
        const WRITE: &str = "set_physiology";
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();

        let control = own_conversation(&squad, "My own numbers").await;
        *squad.model.call.lock().unwrap() = (WRITE.to_owned(), json!({ "ftp_watts": 250 }));
        squad.model.declared.lock().unwrap().clear();
        let (status, body) =
            post_text_over_api_key(&app, &api_key, &control, "My FTP is 250").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            squad.model.declared_first().contains(WRITE),
            "a turn that read no one is offered the write"
        );
        assert_eq!(
            coach_ftp(&squad).await,
            Some(250),
            "without a read the write lands"
        );

        let own = own_conversation(&squad, "Coach script").await;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        *squad.model.then.lock().unwrap() = Some((WRITE.to_owned(), json!({ "ftp_watts": 310 })));
        squad.model.conversations.lock().unwrap().clear();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let conversation = squad.model.conversation();
        assert!(
            conversation.contains(ALEX_UPLOAD) && conversation.contains(MEMBER_READ_REFUSED),
            "the write after the read is refused: {conversation}"
        );
        assert_eq!(
            coach_ftp(&squad).await,
            Some(250),
            "Alex's FTP is not the coach's"
        );

        *squad.model.call.lock().unwrap() = (WRITE.to_owned(), json!({ "ftp_watts": 320 }));
        *squad.model.then.lock().unwrap() = None;
        squad.model.declared.lock().unwrap().clear();
        squad.model.conversations.lock().unwrap().clear();
        let (status, body) =
            post_text_over_api_key(&app, &api_key, &own, "Save that as my FTP").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            !squad.model.declared_first().contains(WRITE),
            "the follow-up replays the read, so the write is not offered"
        );
        let conversation = squad.model.conversation();
        assert!(
            conversation.contains(MEMBER_READ_REFUSED),
            "and it is refused if called anyway: {conversation}"
        );
        assert_eq!(
            coach_ftp(&squad).await,
            Some(250),
            "the coach's FTP is unchanged"
        );
    }

    /// Make every write naming a member on a row of `conversation_id` fail,
    /// on either backend. Scoped to the one conversation, so no other test's
    /// rows are refused.
    async fn refuse_member_rows(res: &ServerContext, conversation_id: &str) {
        let name = format!("fault_members_{}", conversation_id.replace('-', "_"));
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(&format!(
                    "CREATE TRIGGER {name} BEFORE INSERT ON chat_message_members \
                     WHEN (SELECT conversation_id FROM chat_messages WHERE id = NEW.message_id) \
                       = '{conversation_id}' \
                     BEGIN SELECT RAISE(ABORT, 'member row refused by the test'); END"
                ))
                .execute(db.pool())
                .await
                .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(&format!(
                    "CREATE OR REPLACE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql \
                     AS $$ BEGIN \
                       IF (SELECT conversation_id FROM chat_messages WHERE id = NEW.message_id) \
                          = '{conversation_id}' THEN \
                         RAISE EXCEPTION 'member row refused by the test'; \
                       END IF; \
                       RETURN NEW; \
                     END $$"
                ))
                .execute(db.pool())
                .await
                .unwrap();
                sqlx::query(&format!(
                    "CREATE TRIGGER {name} BEFORE INSERT ON chat_message_members \
                     FOR EACH ROW EXECUTE FUNCTION {name}()"
                ))
                .execute(db.pool())
                .await
                .unwrap();
            }
        }
    }

    /// carnet#1422, a reply about several members is stored with them or
    /// not at all: with the write naming its members refused, the roster's
    /// reply about Alex and Bea is not stored about nobody, where any room
    /// the thread became would replay it.
    #[tokio::test]
    async fn a_reply_about_several_members_is_never_stored_about_nobody() {
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        res.common
            .repos
            .groups
            .set_coach_assistant_read(
                &squad.group.to_string(),
                squad.alex.id,
                true,
                Timestamp::now(),
            )
            .await
            .unwrap()
            .expect("a live member");
        let own = own_conversation(&squad, "Coach roster").await;
        refuse_member_rows(res, &own).await;

        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
        let (status, body) =
            post_text_over_api_key(&app, &api_key, &own, "How is my group doing?").await;
        assert_ne!(
            status,
            StatusCode::OK,
            "the reply could not be stored: {body}"
        );
        let rows = res
            .common
            .repos
            .chat
            .get_messages(&own, &squad.casey.id.to_string(), squad.casey.tenant)
            .await
            .unwrap();
        let mut about_nobody = Vec::new();
        for row in &rows {
            if row.role == "assistant"
                && (row.content.contains("Bea Bright") || row.content.contains("Alex Athlete"))
                && carried_members(&squad, &own, &row.id).await.is_empty()
            {
                about_nobody.push(row.content.clone());
            }
        }
        assert!(
            about_nobody.is_empty(),
            "a reply naming both members is stored about nobody: {about_nobody:?}"
        );
    }

    /// A coach's thread about one athlete reads that athlete on every turn,
    /// and its plan tools act on them with `athlete` left out: the rule that
    /// withholds an own-account write after a member read is the seat's own
    /// there, so a plan write after a read of the thread's athlete is not
    /// refused as a write on the coach, in the turn or on the follow-up.
    #[tokio::test]
    async fn an_athlete_threads_plan_write_after_its_read_is_not_withheld() {
        let squad = squad().await;
        let thread = squad
            .res
            .common
            .repos
            .chat
            .open_subject_conversation(&NewSubjectConversation {
                user_id: &squad.casey.id.to_string(),
                tenant_id: squad.casey.tenant,
                title: "Bea Bright",
                model: "mock-model",
                agent_id: &squad.agent,
                subject: ConversationSubject {
                    athlete_user_id: squad.bea.id,
                    group_id: squad.group,
                },
            })
            .await
            .unwrap()
            .conversation
            .id;
        *squad.model.relay.lock().unwrap() = true;
        *squad.model.call.lock().unwrap() = ("get_activities".to_owned(), json!({}));
        *squad.model.then.lock().unwrap() =
            Some(("save_training_plan".to_owned(), json!({ "weeks": [] })));
        let read = turn_in(
            &squad,
            &squad.casey,
            Transport::WebApp,
            &OAuthScope::self_grant(),
            thread.clone(),
            true,
        )
        .await;
        *squad.model.then.lock().unwrap() = None;
        assert!(
            read.conversation
                .contains("[Tool Result for get_activities]"),
            "premise: the turn reads the thread's athlete: {}",
            read.conversation
        );
        assert!(
            read.conversation
                .contains("[Tool Result for save_training_plan]")
                && !read.conversation.contains(MEMBER_READ_REFUSED),
            "the plan write acts on the thread's athlete: {}",
            read.conversation
        );
        *squad.model.call.lock().unwrap() =
            ("save_training_plan".to_owned(), json!({ "weeks": [] }));
        let follow_up = turn_in(
            &squad,
            &squad.casey,
            Transport::WebApp,
            &OAuthScope::self_grant(),
            thread,
            true,
        )
        .await;
        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
        assert!(
            follow_up.declared.contains("save_training_plan")
                && !follow_up.conversation.contains(MEMBER_READ_REFUSED),
            "the follow-up still offers and takes the athlete's plan write: {}",
            follow_up.conversation
        );
    }

    /// The coach's own thread with a private read naming Alex and a short
    /// reply about Alex as its newest row, then colleague Dana added to it
    /// over the route: the app, the coach's API key, Dana and the thread.
    async fn shared_thread_with_alex(squad: &Squad) -> (Router, String, Person, String) {
        upload_alex_ride(squad).await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        let dana = colleague(squad).await;
        let own = own_conversation(squad, "Coach script").await;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(ALEX_UPLOAD),
            "the coach alone reads the thread, so the read names Alex: {body}"
        );
        add_row_about_alex(squad, &own, &format!("Alex rode {ALEX_UPLOAD}."), None).await;
        let (status, body) = call_as(
            &app,
            &squad.casey.auth,
            "POST",
            &format!("/api/chat/conversations/{own}/participants"),
            Some(json!({ "user_id": dana.id.to_string() })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        (app, api_key, dana, own)
    }

    /// An assistant row of the coach's about Alex in `conversation_id`, with
    /// `blocks` when given: its id.
    async fn add_row_about_alex(
        squad: &Squad,
        conversation_id: &str,
        content: &str,
        blocks: Option<&str>,
    ) -> String {
        squad
            .res
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: squad.casey.tenant,
                conversation_id,
                user_id: &squad.casey.id.to_string(),
                role: "assistant",
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: blocks,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: Some("web"),
                about_user_id: Some(squad.alex.id),
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap()
            .id
    }

    /// The single-use token of the `/agent create` proposal in `body`.
    fn proposal_token(body: &str) -> String {
        let marker = "/agent create confirm ";
        let at = body
            .find(marker)
            .unwrap_or_else(|| panic!("no proposal: {body}"))
            + marker.len();
        body[at..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect()
    }

    /// The stored `/agent create` proposal rows of `conversation_id`, as the
    /// store holds them.
    async fn stored_proposals(squad: &Squad, conversation_id: &str) -> Vec<MessageRecord> {
        squad
            .res
            .common
            .repos
            .chat
            .get_messages(
                conversation_id,
                &squad.casey.id.to_string(),
                squad.casey.tenant,
            )
            .await
            .unwrap()
            .into_iter()
            .filter(|row| {
                row.role == "assistant"
                    && row.finish_reason.as_deref() == Some(COMMAND_FINISH_REASON)
                    && row.content.contains("/agent create confirm")
            })
            .collect()
    }

    /// What Dana says in the coach's shared thread: her own words, which no
    /// member rule withholds, yet never durable text in anyone's draft.
    const DANA_TYPED: &str = "dana-durable: I train before work";

    /// Dana says [`DANA_TYPED`] in the coach's shared thread `own`.
    async fn dana_says(squad: &Squad, app: &Router, dana: &Person, own: &str) {
        *squad.model.call.lock().unwrap() = ("get_connection_status".to_owned(), json!({}));
        let (status, body) = post_text_over_api_key(app, &dana.auth, own, DANA_TYPED).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// carnet#1511 A20, `/agent create` in a shared thread: a durable draft
    /// reads only its caller's own words in a conversation they own. Dana,
    /// a participant added to the coach's thread, drafts from it: the thread
    /// is not hers, so nothing in it is hers to file, her own words
    /// included. The model is never asked, she is told there is nothing to
    /// draft from, and no proposal is stored.
    #[tokio::test]
    async fn a_participants_agent_draft_reads_nothing_of_anothers_thread() {
        let squad = squad().await;
        let (app, _api_key, dana, own) = shared_thread_with_alex(&squad).await;
        dana_says(&squad, &app, &dana, &own).await;
        *squad.model.propose.lock().unwrap() = true;
        squad.model.learning.lock().unwrap().clear();

        let (status, body) = post_text_over_api_key(&app, &dana.auth, &own, "/agent create").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            squad.model.learning.lock().unwrap().is_empty(),
            "bug: a draft was asked for from a thread that is not Dana's"
        );
        let empty = MessagingStringsRegistry::new().render(KEY_AGENT_CREATE_EMPTY, "fr", &[]);
        assert!(
            body.contains(&empty),
            "Dana is told there is nothing to draft from: {body}"
        );
        assert!(
            !body.contains(ALEX_UPLOAD) && !body.contains(COACH_TYPED),
            "nothing of the thread reaches her: {body}"
        );
        assert!(
            stored_proposals(&squad, &own).await.is_empty(),
            "no proposal is stored"
        );
    }

    /// The coach's own words the draft reads in [`shared_thread_with_alex`]:
    /// a typed row, which no member rule withholds.
    const COACH_TYPED: &str = "How is Alex doing?";

    /// What the model was sent to draft the coach's agent, cleared first.
    fn drafted_from(squad: &Squad) -> String {
        squad.model.learning.lock().unwrap().join("\n")
    }

    /// carnet#1511 A8 and A20: a row written before its record existed
    /// cannot say what it drew on, so no durable text is built from it. The
    /// coach's own thread holds a legacy agent row and a row written since:
    /// the draft reads only the latter.
    #[tokio::test]
    async fn an_agent_draft_never_reads_a_legacy_row() {
        const LEGACY: &str = "legacy-durable: your long run went well";
        const RECORDED: &str = "recorded-durable: I want to race a marathon";
        const SQL: &str = "UPDATE chat_messages SET audience_recorded = FALSE WHERE id = $1";
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let own = own_conversation(&squad, "Before and after").await;
        let legacy = coach_row(&squad, &own, "assistant", LEGACY, false).await;
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(SQL)
                    .bind(&legacy)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(SQL)
                    .bind(&legacy)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
        coach_row(&squad, &own, "user", RECORDED, false).await;
        *squad.model.propose.lock().unwrap() = true;
        squad.model.learning.lock().unwrap().clear();

        let (status, body) =
            post_text_over_api_key(&app, &squad.casey.auth, &own, "/agent create").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let drafted = drafted_from(&squad);
        assert!(
            drafted.contains(RECORDED),
            "the draft reads the coach's recorded words: {drafted}"
        );
        assert!(
            !drafted.contains(LEGACY),
            "bug: a legacy row reached a durable draft: {drafted}"
        );
    }

    /// carnet#1511 design call 1 (carnet#1070): an agent's system prompt
    /// outlives the thread and can be bound to a group's room, so the coach's
    /// own draft quotes no member, even one the coach may read now, nor what
    /// another participant said in the thread (A20). The draft still reads
    /// what the coach typed, and the proposal it stores is about no one.
    #[tokio::test]
    async fn a_coachs_agent_draft_quotes_no_member_the_coach_read() {
        let squad = squad().await;
        let (app, _api_key, dana, own) = shared_thread_with_alex(&squad).await;
        dana_says(&squad, &app, &dana, &own).await;
        *squad.model.propose.lock().unwrap() = true;
        squad.model.learning.lock().unwrap().clear();

        let (status, body) =
            post_text_over_api_key(&app, &squad.casey.auth, &own, "/agent create").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let drafted = drafted_from(&squad);
        assert!(
            drafted.contains(COACH_TYPED),
            "the draft reads what the coach typed: {drafted}"
        );
        assert!(
            !drafted.contains(ALEX_UPLOAD) && !body.contains(ALEX_UPLOAD),
            "nothing the coach read about Alex reaches the model or the card: {drafted}"
        );
        assert!(
            !drafted.contains(DANA_TYPED),
            "bug: another participant's words entered the coach's draft (A20): {drafted}"
        );
        let stored = stored_proposals(&squad, &own).await;
        assert_eq!(stored.len(), 1, "the proposal is stored once");
        assert_eq!(
            stored[0].about_user_id, None,
            "the proposal quotes no member, so it is about no one"
        );
    }

    /// carnet#1511 design call 1, a command run outside any turn: a
    /// messaging surface dispatches `/agent create` straight to the command.
    /// Its draft quotes no member either.
    #[tokio::test]
    async fn a_command_off_any_turn_drafts_nothing_about_a_member() {
        let squad = squad().await;
        let (_app, _api_key, _dana, own) = shared_thread_with_alex(&squad).await;
        *squad.model.propose.lock().unwrap() = true;
        squad.model.learning.lock().unwrap().clear();
        let pipeline = squad.res.chat_pipeline_context();
        let command = dispatch_slash(
            &pipeline,
            &SlashRequest {
                user_id: squad.casey.id,
                tenant_id: squad.casey.tenant,
                conversation_id: &own,
                conversation_tenant_id: squad.casey.tenant,
                channel_type: "telegram",
                transport: Transport::Messaging,
                surface: SurfaceId::Telegram,
                renders_buttons: true,
                locale: "en",
                is_direct_message: true,
                ambient_group_fallback: true,
                persistence: CommandPersistence::Always,
                sender_id: None,
                text: "/agent create",
                input_source: InputSource::Typed,
                origin: TurnOrigin::Athlete,
            },
        )
        .await
        .unwrap()
        .expect("/agent create is a catalogued command");
        let drafted = drafted_from(&squad);
        assert!(
            drafted.contains(COACH_TYPED),
            "the draft reads what the coach typed: {drafted}"
        );
        assert!(
            !drafted.contains(ALEX_UPLOAD) && !command.text.contains(ALEX_UPLOAD),
            "nothing the coach read about Alex reaches the model or the card: {}",
            command.text
        );
        let stored = stored_proposals(&squad, &own).await;
        assert_eq!(stored.len(), 1, "the proposal is stored once");
        assert_eq!(stored[0].about_user_id, None, "it is about no one");
    }

    /// The agents titled as the echo proposal names them that `who` owns.
    async fn echo_agents(squad: &Squad, who: &Person) -> Vec<AgentListItem> {
        squad
            .res
            .common
            .repos
            .agents
            .list(who.id, who.tenant, &ListAgentsFilter::with_defaults())
            .await
            .unwrap()
            .into_iter()
            .filter(|item| item.agent.title == "Echo")
            .collect()
    }

    /// The Squad's agent id, as the group row holds it.
    async fn squad_agent(squad: &Squad) -> String {
        squad
            .res
            .common
            .repos
            .groups
            .get_group(&squad.group.to_string(), squad.casey.tenant)
            .await
            .unwrap()
            .expect("the Squad")
            .agent_id
    }

    /// Casey as the Squad's owner, with a conversation bound to its room.
    async fn caseys_squad_room(squad: &Squad) -> String {
        let repos = &squad.res.common.repos;
        let now = Timestamp::now();
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: squad.group,
                user_id: squad.casey.id,
                tenant_id: squad.casey.tenant.to_string(),
                role: GroupRole::Owner,
                peer_sharing_consent: false,
                coach_sharing_consent: false,
                coach_sharing_consent_at: None,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
        let room = own_conversation(squad, "Squad room").await;
        repos
            .chat
            .set_conversation_group_id(&room, Some(&squad.group.to_string()), squad.casey.tenant)
            .await
            .unwrap();
        room
    }

    /// carnet#1511 design call 1, the round-6 verifier's sequence over the
    /// real routes: the coach reads Alex by name in a private thread, drafts
    /// an agent there, and confirms the draft from the Squad's room. The
    /// confirm is refused there, spends nothing and creates no agent: the
    /// draft is confirmed only where it was drafted. Confirmed in its own
    /// thread, the agent's prompt quotes nothing about Alex, and bound to
    /// the Squad over `PUT /api/groups/{id}`, the room's turn is given its
    /// prompt and nothing about Alex.
    #[tokio::test]
    async fn an_agent_drafted_after_a_private_read_carries_nothing_into_a_groups_room() {
        let squad = squad().await;
        upload_alex_ride(&squad).await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let api_key = issue_test_api_key(&res.agent.database, squad.casey.id, "coach script")
            .await
            .unwrap();
        *squad.model.relay.lock().unwrap() = true;
        let own = own_conversation(&squad, "Coach script").await;
        *squad.model.call.lock().unwrap() = read_naming_alex();
        let (status, body) = post_over_api_key(&app, &api_key, &own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(ALEX_UPLOAD),
            "the coach alone reads the thread, so the read names Alex: {body}"
        );
        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));

        *squad.model.propose.lock().unwrap() = true;
        squad.model.learning.lock().unwrap().clear();
        let (status, card) =
            post_text_over_api_key(&app, &squad.casey.auth, &own, "/agent create").await;
        assert_eq!(status, StatusCode::OK, "{card}");
        let drafted = drafted_from(&squad);
        assert!(
            drafted.contains(COACH_TYPED),
            "the draft reads what the coach typed: {drafted}"
        );
        assert!(
            !drafted.contains(ALEX_UPLOAD) && !card.contains(ALEX_UPLOAD),
            "nothing the coach read about Alex reaches the model or the card: {card}"
        );
        let token = proposal_token(&card);

        // Confirmed from the Squad's room: refused, saying where the draft is
        // confirmed, with no agent made and the Squad's agent unchanged.
        let room = caseys_squad_room(&squad).await;
        let before = squad_agent(&squad).await;
        let (status, refused) = post_text_over_api_key(
            &app,
            &squad.casey.auth,
            &room,
            &format!("/agent create confirm {token}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{refused}");
        // The fixture's accounts read French, the default locale.
        let elsewhere =
            MessagingStringsRegistry::new().render(KEY_GUARDIAN_CONFIRM_ELSEWHERE, "fr", &[]);
        assert!(
            refused.contains(&elsewhere),
            "a confirm from another conversation says where to confirm: {refused}"
        );
        assert!(
            echo_agents(&squad, &squad.casey).await.is_empty(),
            "no agent is created from another conversation"
        );
        assert_eq!(
            squad_agent(&squad).await,
            before,
            "the Squad keeps its agent"
        );
        assert!(
            res.common
                .repos
                .agent_drafts
                .pending_action(
                    &token,
                    &squad.casey.id.to_string(),
                    &squad.casey.tenant.to_string()
                )
                .await
                .unwrap()
                .is_some(),
            "the refused confirm spends nothing: the draft still waits"
        );

        // Confirmed where it was drafted: the agent exists, quoting no one.
        let (status, done) = post_text_over_api_key(
            &app,
            &squad.casey.auth,
            &own,
            &format!("/agent create confirm {token}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{done}");
        let created = echo_agents(&squad, &squad.casey).await;
        assert_eq!(
            created.len(),
            1,
            "the agent is created in its thread: {done}"
        );
        let agent = &created[0].agent;
        assert!(
            agent.system_prompt.contains(COACH_TYPED) && !agent.system_prompt.contains(ALEX_UPLOAD),
            "the agent's prompt is drawn from the coach's words, never Alex's ride: {}",
            agent.system_prompt
        );

        // Bound to the Squad over the group route, the second way an agent
        // reaches a room: the room's turn runs under its prompt and is
        // given nothing about Alex.
        let (status, body) = call_as(
            &app,
            &squad.casey.auth,
            "PUT",
            &format!("/api/groups/{}", squad.group),
            Some(json!({ "agent_id": agent.id.to_string() })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let group_agent = squad_agent(&squad).await;
        assert_eq!(
            group_agent,
            agent.id.to_string(),
            "the group route binds the agent to the Squad"
        );
        // A room carries the group's agent from when it is opened.
        res.common
            .repos
            .chat
            .set_conversation_agent_id(&room, Some(&group_agent), squad.casey.tenant)
            .await
            .unwrap();
        squad.model.requests.lock().unwrap().clear();
        let (status, body) =
            post_text_over_api_key(&app, &squad.casey.auth, &room, "What should we do today?")
                .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let given = squad.model.requests.lock().unwrap().join("\n");
        assert!(
            given.contains(COACH_TYPED),
            "the room's turn runs under the new agent's prompt: {given}"
        );
        assert!(
            !given.contains(ALEX_UPLOAD) && !body.contains(ALEX_UPLOAD),
            "the Squad's room is given nothing about Alex: {given}"
        );
    }

    /// carnet#1422, the one classifier for a screen read too: a row a
    /// messaging group chat writes into, bound to no group, is never read as
    /// the coach's private thread. Before a group chat's session points at
    /// it, the coach reads the row about Alex there; once one does, the
    /// transcript and its preview withhold it from the coach as well, as the
    /// chat's turns do: no group's consent says who may be read there.
    #[tokio::test]
    async fn a_row_a_group_chat_writes_into_is_never_read_as_private() {
        let squad = squad().await;
        let app = ProviderToolRouter::build_http_app(&squad.res);
        let thread = own_conversation(&squad, "Legacy").await;
        let marker = format!("Alex rode {ALEX_UPLOAD}.");
        add_row_about_alex(&squad, &thread, &marker, None).await;
        let before = transcript_as(&app, &squad.casey.auth, &thread).await;
        assert!(
            before.contains(&marker),
            "the coach alone reads the thread and may read Alex: {before}"
        );

        squad
            .res
            .common
            .repos
            .messaging
            .create_session(&CreateSessionParams {
                id: &Uuid::new_v4().to_string(),
                user_id: &squad.casey.id.to_string(),
                tenant_id: squad.casey.tenant,
                channel_type: "telegram",
                channel_user_id: "tg-casey",
                channel_conversation_id: Some("-100777"),
                pierre_conversation_id: Some(&thread),
            })
            .await
            .unwrap();
        record_chat(&squad, &thread, false).await;
        let after = transcript_as(&app, &squad.casey.auth, &thread).await;
        assert!(
            !after.contains(&marker),
            "a group chat writes into the row, so no member is read there: {after}"
        );
        let (status, list) = call_as(
            &app,
            &squad.casey.auth,
            "GET",
            "/api/chat/conversations",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{list}");
        assert!(
            !list.contains(ALEX_UPLOAD),
            "nor in the thread's preview: {list}"
        );
    }

    /// carnet#1422, a chart's token: a chart on a row about Alex is pressed
    /// for a reader who may read Alex, and for a colleague in the same thread
    /// it is not found, as GET messages does not serve them the row.
    #[tokio::test]
    async fn a_chart_about_a_member_is_found_only_for_a_reader_who_may_read_them() {
        let squad = squad().await;
        let (app, _api_key, dana, own) = shared_thread_with_alex(&squad).await;
        let blocks = r#"[{"type":"chart","kind":"line","source_tool":"analyze_training_load","x":{"label":"Date","type":"time"},"series":[{"label":"CTL","points":[["2026-07-01",42.0],["2026-07-02",43.1]]}]}]"#;
        let message_id = add_row_about_alex(&squad, &own, "Alex's load.", Some(blocks)).await;
        let fetch = |reader: Uuid| {
            let token = VizToken::for_block(
                VizTarget {
                    conversation_id: own.clone(),
                    user_id: reader.to_string(),
                    tenant_id: squad.casey.tenant,
                    message_id: message_id.clone(),
                },
                0,
                "dark",
                "en",
            )
            .mint(&squad.res.auth.admin_jwt_secret);
            let app = app.clone();
            async move {
                let request = Request::get(format!("/api/viz/{token}.png"))
                    .body(Body::empty())
                    .unwrap();
                app.oneshot(request).await.unwrap().status()
            }
        };
        // The test server presses no image, so a block found fails at the
        // press, never as not found.
        let for_casey = fetch(squad.casey.id).await;
        assert_ne!(
            for_casey,
            StatusCode::NOT_FOUND,
            "the coach's token finds the chart about Alex"
        );
        assert_eq!(
            fetch(dana.id).await,
            StatusCode::NOT_FOUND,
            "a colleague's token finds nothing about Alex"
        );
    }

    /// carnet#1422, the preview by the one rule: the coach alone reads a
    /// thread whose newest row is about Alex. While Alex shares with the
    /// coach the preview shows it; once Alex stops, it is blank, as the
    /// transcript withholds the row.
    #[tokio::test]
    async fn a_preview_about_a_member_who_stopped_sharing_is_blank_for_the_coach() {
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let own = own_conversation(&squad, "Coach notes").await;
        add_row_about_alex(&squad, &own, &format!("Alex rode {ALEX_UPLOAD}."), None).await;
        let (status, listed) = call_as(
            &app,
            &squad.casey.auth,
            "GET",
            "/api/chat/conversations",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(
            listed.contains(ALEX_UPLOAD),
            "the coach reads Alex while Alex shares: {listed}"
        );

        res.common
            .repos
            .groups
            .update_coach_sharing_consent(&squad.group.to_string(), squad.alex.id, false)
            .await
            .unwrap();
        let (status, listed) = call_as(
            &app,
            &squad.casey.auth,
            "GET",
            "/api/chat/conversations",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(
            listed.contains(&own) && !listed.contains(ALEX_UPLOAD),
            "once Alex stops sharing, the preview shows nothing about Alex: {listed}"
        );
        assert!(
            !transcript_as(&app, &squad.casey.auth, &own)
                .await
                .contains(ALEX_UPLOAD),
            "as the transcript withholds the row"
        );
    }

    /// A row of the coach's in `conversation_id`: `role`, `content`, about
    /// Alex when `about_alex`. Its id; a beat apart from the row before, so
    /// the rows keep the order they were written in.
    async fn coach_row(
        squad: &Squad,
        conversation_id: &str,
        role: &str,
        content: &str,
        about_alex: bool,
    ) -> String {
        sleep(StdDuration::from_millis(5)).await;
        squad
            .res
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: squad.casey.tenant,
                conversation_id,
                user_id: &squad.casey.id.to_string(),
                role,
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: Some("web"),
                about_user_id: about_alex.then_some(squad.alex.id),
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap()
            .id
    }

    /// The coach's own data export, as `GET /api/user/export` writes it.
    async fn coach_export(squad: &Squad) -> String {
        export_as(squad, &squad.casey.auth).await
    }

    /// The data export of whoever `auth` signs in, as `GET /api/user/export`
    /// writes it.
    async fn export_as(squad: &Squad, auth: &str) -> String {
        let routes = AuthRoutes::routes(squad.res.auth_routes_context());
        // The server writes a few exports at once, process-wide: a busy
        // refusal is retried, as it asks.
        for _ in 0..200 {
            let (status, body) = call_as(&routes, auth, "GET", "/api/user/export", None).await;
            if body.contains("\"export_busy\"") {
                sleep(StdDuration::from_millis(50)).await;
                continue;
            }
            assert_eq!(status, StatusCode::OK, "{body}");
            return body;
        }
        panic!("the export stayed busy");
    }

    /// carnet#1511 design call 2: a coach's data export applies the rule
    /// the transcript is served by. While Alex shares, the export holds the
    /// row about Alex, the reply after it, and the summary covering them;
    /// once Alex stops sharing, it holds none of them, the summary whose
    /// last row the coach typed included, while what the coach typed stays.
    #[tokio::test]
    async fn a_coachs_export_holds_nothing_about_a_member_who_stopped_sharing() {
        let squad = squad().await;
        let res = &squad.res;
        let own = own_conversation(&squad, "Coach notes").await;
        let asked = coach_row(&squad, &own, "user", "coach-typed-first", false).await;
        coach_row(
            &squad,
            &own,
            "assistant",
            &format!("Alex rode {ALEX_UPLOAD}."),
            true,
        )
        .await;
        coach_row(&squad, &own, "assistant", "reply-restating-the-read", false).await;
        let typed = coach_row(&squad, &own, "user", "coach-typed-later", false).await;
        res.common
            .repos
            .memory
            .insert_compaction_block(&InsertCompactionBlockParams {
                tenant_id: squad.casey.tenant,
                conversation_id: &own,
                summary: SUMMARY_MARKER,
                summary_tokens: 10,
                original_tokens: 100,
                first_message_id: &asked,
                last_message_id: &typed,
                transport_policy: TransportPolicy::AnyTransport,
                about_user_id: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap();

        let shared = coach_export(&squad).await;
        for held in [ALEX_UPLOAD, "reply-restating-the-read", SUMMARY_MARKER] {
            assert!(
                shared.contains(held),
                "while Alex shares, the export holds {held}"
            );
        }

        res.common
            .repos
            .groups
            .update_coach_sharing_consent(&squad.group.to_string(), squad.alex.id, false)
            .await
            .unwrap();
        let withdrawn = coach_export(&squad).await;
        for gone in [ALEX_UPLOAD, "reply-restating-the-read", SUMMARY_MARKER] {
            assert!(
                !withdrawn.contains(gone),
                "once Alex stops sharing, the export holds no {gone}"
            );
        }
        for kept in ["coach-typed-first", "coach-typed-later"] {
            assert!(
                withdrawn.contains(kept),
                "what the coach typed stays: {kept}"
            );
        }
        assert!(
            withdrawn.contains("\"failed_tables\":[]"),
            "the rule decided every row"
        );
    }

    /// carnet#1511 design call 5: a group's room is read by the group's
    /// members, each under their own consent, so it takes no participant
    /// added by hand: someone added would read what the members shared with
    /// the group or its coach alone. The coach's own thread still takes one.
    #[tokio::test]
    async fn a_groups_room_takes_no_participant_added_by_hand() {
        let squad = squad().await;
        let app = ProviderToolRouter::build_http_app(&squad.res);
        let dana = colleague(&squad).await;
        let room = caseys_squad_room(&squad).await;

        let (status, body) = call_as(
            &app,
            &squad.casey.auth,
            "POST",
            &format!("/api/chat/conversations/{room}/participants"),
            Some(json!({ "user_id": dana.id.to_string() })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a group's room is refused: {body}"
        );
        assert!(body.contains("invite them to the group"), "{body}");
        let (status, _) = call_as(
            &app,
            &dana.auth,
            "GET",
            &format!("/api/chat/conversations/{room}/messages"),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "the colleague reads nothing there"
        );

        let own = own_conversation(&squad, "Coach script").await;
        let (status, body) = call_as(
            &app,
            &squad.casey.auth,
            "POST",
            &format!("/api/chat/conversations/{own}/participants"),
            Some(json!({ "user_id": dana.id.to_string() })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "the coach's own thread takes one: {body}"
        );
    }

    /// The members `message_id` is stored about beside its row
    /// (`chat_message_members`). The column is a UUID on PostgreSQL and
    /// TEXT on SQLite, so it is read back as text on both.
    async fn stored_members(res: &ServerContext, message_id: &str) -> BTreeSet<String> {
        const MEMBERS_SQL: &str =
            "SELECT CAST(member_user_id AS TEXT) FROM chat_message_members WHERE message_id = $1";
        let rows: Vec<String> = match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => sqlx::query_scalar(MEMBERS_SQL)
                .bind(message_id)
                .fetch_all(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => sqlx::query_scalar(MEMBERS_SQL)
                .bind(message_id)
                .fetch_all(db.pool())
                .await
                .unwrap(),
        };
        rows.into_iter().collect()
    }

    /// carnet#1511 design call 4, a coach's room turn over the real route:
    /// the cards the platform puts into the prompt are Alex's and Bea's
    /// coach view (Alex shares with the coach alone). The reply restating
    /// Alex's card is stored about both members, so someone who reaches the
    /// row without being a member reads nothing of it, while the coach does,
    /// and `/agent create` drafted in the room afterwards quotes nothing
    /// about Alex. The model calls no tool: the cards are the only read.
    #[tokio::test]
    async fn a_coach_room_reply_drawn_from_the_cards_is_about_the_members_carried() {
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let room = caseys_squad_room(&squad).await;
        // A participant of the room's row who is no member of the group: a
        // row shared before the route refused group rooms.
        let dana = colleague(&squad).await;
        res.common
            .repos
            .chat
            .add_participant(
                &room,
                squad.casey.tenant,
                &dana.id.to_string(),
                &squad.casey.id.to_string(),
            )
            .await
            .unwrap();
        *squad.model.echo_cards.lock().unwrap() = true;
        squad.model.requests.lock().unwrap().clear();

        let typed = "How is my group doing this week?";
        let (status, body) = post_text_over_api_key(&app, &squad.casey.auth, &room, typed).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains(ALEX_NAME),
            "the coach's room turn carries Alex's coach view: {body}"
        );
        let reply = res
            .common
            .repos
            .chat
            .get_messages(&room, &squad.casey.id.to_string(), squad.casey.tenant)
            .await
            .unwrap()
            .into_iter()
            .rfind(|row| row.role == "assistant")
            .expect("the reply is stored");
        assert!(reply.content.contains(ALEX_NAME), "{}", reply.content);
        let about = stored_members(res, &reply.id).await;
        assert!(
            about.contains(&squad.alex.id.to_string()) && about.contains(&squad.bea.id.to_string()),
            "the reply is stored about every member whose card it carried: {about:?}"
        );

        let read_by_casey = transcript_as(&app, &squad.casey.auth, &room).await;
        assert!(
            read_by_casey.contains(ALEX_NAME),
            "the coach reads the reply: {read_by_casey}"
        );
        let read_by_dana = transcript_as(&app, &dana.auth, &room).await;
        assert!(
            read_by_dana.contains(typed) && !read_by_dana.contains(ALEX_NAME),
            "a non-member who reaches the row reads nothing of Alex's card: {read_by_dana}"
        );

        *squad.model.echo_cards.lock().unwrap() = false;
        *squad.model.propose.lock().unwrap() = true;
        squad.model.learning.lock().unwrap().clear();
        let (status, card) =
            post_text_over_api_key(&app, &squad.casey.auth, &room, "/agent create").await;
        assert_eq!(status, StatusCode::OK, "{card}");
        let drafted = drafted_from(&squad);
        assert!(
            drafted.contains(typed),
            "the draft reads what the coach typed: {drafted}"
        );
        assert!(
            !drafted.contains(ALEX_NAME) && !card.contains(ALEX_NAME),
            "the draft quotes nothing about Alex: {drafted}"
        );
    }

    /// carnet#1511 design call 4, a coach's thread about one athlete: the
    /// athlete's roster line reaches the model whether or not a tool reads
    /// them, so the reply restating it is stored about the athlete, and
    /// `/agent create` drafted in the thread quotes nothing about them.
    #[tokio::test]
    async fn an_athlete_thread_reply_drawn_from_the_roster_line_is_about_the_athlete() {
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let thread = res
            .common
            .repos
            .chat
            .open_subject_conversation(&NewSubjectConversation {
                user_id: &squad.casey.id.to_string(),
                tenant_id: squad.casey.tenant,
                title: ALEX_NAME,
                model: "mock-model",
                agent_id: &squad.agent,
                subject: ConversationSubject {
                    athlete_user_id: squad.alex.id,
                    group_id: squad.group,
                },
            })
            .await
            .unwrap()
            .conversation
            .id;
        *squad.model.echo_cards.lock().unwrap() = true;
        let typed = "Plan next week please";
        let (status, body) = post_text_over_api_key(&app, &squad.casey.auth, &thread, typed).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let reply = res
            .common
            .repos
            .chat
            .get_messages(&thread, &squad.casey.id.to_string(), squad.casey.tenant)
            .await
            .unwrap()
            .into_iter()
            .rfind(|row| row.role == "assistant")
            .expect("the reply is stored");
        assert!(
            reply.content.contains(ALEX_NAME),
            "the thread carries Alex's roster line: {}",
            reply.content
        );
        assert_eq!(
            carried_members(&squad, &thread, &reply.id).await,
            BTreeSet::from([squad.alex.id]),
            "the reply restating the roster line carries Alex"
        );

        *squad.model.echo_cards.lock().unwrap() = false;
        *squad.model.propose.lock().unwrap() = true;
        squad.model.learning.lock().unwrap().clear();
        let (status, card) =
            post_text_over_api_key(&app, &squad.casey.auth, &thread, "/agent create").await;
        assert_eq!(status, StatusCode::OK, "{card}");
        let drafted = drafted_from(&squad);
        assert!(
            drafted.contains(typed),
            "the draft reads what the coach typed: {drafted}"
        );
        let roster_line: Vec<&str> = reply
            .content
            .lines()
            .filter(|line| line.contains(ALEX_NAME))
            .collect();
        assert!(!roster_line.is_empty(), "{}", reply.content);
        assert!(
            roster_line.iter().all(|line| !drafted.contains(line)),
            "the draft quotes nothing of Alex's roster line {roster_line:?}: {drafted}"
        );
    }

    /// One row of the coach's messaging session `session` saying `body`,
    /// `inbound` or outbound, copying the reply `reply` when it names one.
    async fn channel_row(
        squad: &Squad,
        session: &str,
        inbound: bool,
        body: &str,
        reply: Option<&str>,
    ) {
        let id = Uuid::new_v4().to_string();
        squad
            .res
            .common
            .repos
            .messaging
            .insert_message(&InsertMessageParams {
                id: &id,
                tenant_id: squad.casey.tenant,
                session_id: session,
                direction: if inbound { "inbound" } else { "outbound" },
                channel_type: "telegram",
                channel_message_id: &id,
                sender_id: "casey-on-telegram",
                content_type: "text",
                content_body: Some(body),
                correlation_id: &id,
                raw_payload: None,
                chat_message_id: reply,
                proactive_purpose: None,
            })
            .await
            .unwrap();
    }

    /// An A2A task of the coach's carrying `marker`, in `context`, its
    /// reads recorded when given: each member read by the coach through the
    /// coach's view in the squad, as the A2A server stamps a task's reads
    /// (carnet#1511 A7).
    async fn coach_task(squad: &Squad, marker: &str, context: &str, members: Option<&[Uuid]>) {
        let a2a = &squad.res.common.repos.a2a;
        let task = a2a
            .create_task(
                &user_task_key(squad.casey.id),
                None,
                "message",
                &json!({ "parts": [{ "text": marker }] }),
                Some(context),
            )
            .await
            .unwrap();
        if let Some(members) = members {
            let reads: Vec<CarriedRead> = members
                .iter()
                .map(|member| CarriedRead::coach(*member, squad.group, squad.casey.id))
                .collect();
            a2a.record_task_members(&task, &serde_json::to_string(&reads).unwrap())
                .await
                .unwrap();
        }
    }

    /// carnet#1511 design calls 7 and 10: the export applies the per-reader
    /// rule to the copies a messaging channel and an A2A task hold, as to
    /// the conversation's own rows. While Alex shares with the coach, the
    /// outbound row copying the reply about Alex, a notice naming no reply,
    /// the tool task that read Alex and the text turn's tasks in the thread
    /// are exported; once Alex stops sharing, none of them is but the text
    /// task that ended before the row about Alex, while what the coach sent,
    /// a copy of a reply about no one and a task that read no one stay. A
    /// tool task with no record of whose data it holds (even one filed under
    /// the thread's id) cannot be decided and is always left out, never
    /// failed.
    #[tokio::test]
    async fn a_coachs_export_holds_no_channel_or_task_copy_about_a_member_who_stopped_sharing() {
        let squad = squad().await;
        let res = &squad.res;
        let own = own_conversation(&squad, "Coach notes").await;
        coach_row(&squad, &own, "user", "coach-asked-first", false).await;
        let plain = coach_row(&squad, &own, "assistant", "reply-about-no-one", false).await;
        // A text turn's task that ended before the row about Alex was
        // stored: it neither produced nor followed that row.
        coach_task(&squad, "early-text-task-of-the-thread", &own, None).await;
        sleep(StdDuration::from_millis(20)).await;
        let about_alex = coach_row(
            &squad,
            &own,
            "assistant",
            &format!("Alex rode {ALEX_UPLOAD}."),
            true,
        )
        .await;
        let session = Uuid::new_v4().to_string();
        res.common
            .repos
            .messaging
            .create_session(&CreateSessionParams {
                id: &session,
                user_id: &squad.casey.id.to_string(),
                tenant_id: squad.casey.tenant,
                channel_type: "telegram",
                channel_user_id: "casey-on-telegram",
                channel_conversation_id: Some("casey-dm"),
                pierre_conversation_id: Some(&own),
            })
            .await
            .unwrap();
        // The coach's own DM with the bot: the thread stays theirs alone.
        record_chat(&squad, &own, true).await;
        channel_row(&squad, &session, true, "coach-sent-on-telegram", None).await;
        channel_row(
            &squad,
            &session,
            false,
            "channel-copy-of-plain",
            Some(&plain),
        )
        .await;
        channel_row(
            &squad,
            &session,
            false,
            "channel-copy-of-alex",
            Some(&about_alex),
        )
        .await;
        channel_row(
            &squad,
            &session,
            false,
            "channel-notice-naming-no-reply",
            None,
        )
        .await;
        let elsewhere = Uuid::new_v4().to_string();
        coach_task(
            &squad,
            "task-that-read-alex",
            &elsewhere,
            Some(&[squad.alex.id]),
        )
        .await;
        coach_task(&squad, "task-that-read-no-one", &elsewhere, Some(&[])).await;
        coach_task(&squad, "task-in-the-thread", &own, None).await;
        coach_task(&squad, "task-from-before-the-record", &elsewhere, None).await;
        // A tool call a client filed under the thread's id, with no record of
        // whose data it read: the thread does not speak for its result.
        res.common
            .repos
            .a2a
            .create_task(
                &user_task_key(squad.casey.id),
                None,
                "message",
                &json!({ "parts": [{ "data": {
                    "tool_name": "get_group_member_activities",
                    "parameters": { "member": "tool-task-in-the-thread-unrecorded" }
                } }] }),
                Some(&own),
            )
            .await
            .unwrap();

        let undecided = [
            "task-from-before-the-record",
            "tool-task-in-the-thread-unrecorded",
        ];
        let shared = coach_export(&squad).await;
        for held in [
            "coach-sent-on-telegram",
            "channel-copy-of-plain",
            "channel-copy-of-alex",
            "channel-notice-naming-no-reply",
            "task-that-read-alex",
            "task-that-read-no-one",
            "task-in-the-thread",
            "early-text-task-of-the-thread",
        ] {
            assert!(
                shared.contains(held),
                "while Alex shares, the export holds {held}"
            );
        }
        for gone in undecided {
            assert!(
                !shared.contains(gone),
                "what cannot be decided is left out: {gone}"
            );
        }
        assert!(shared.contains("\"failed_tables\":[]"), "nothing failed");

        res.common
            .repos
            .groups
            .update_coach_sharing_consent(&squad.group.to_string(), squad.alex.id, false)
            .await
            .unwrap();
        let withdrawn = coach_export(&squad).await;
        for gone in [
            "channel-copy-of-alex",
            "channel-notice-naming-no-reply",
            "task-that-read-alex",
            "task-in-the-thread",
        ]
        .into_iter()
        .chain(undecided)
        {
            assert!(
                !withdrawn.contains(gone),
                "once Alex stops sharing, the export holds no {gone}"
            );
        }
        for kept in [
            "coach-sent-on-telegram",
            "channel-copy-of-plain",
            "task-that-read-no-one",
            "early-text-task-of-the-thread",
        ] {
            assert!(
                withdrawn.contains(kept),
                "what is about no one stays: {kept}"
            );
        }
        assert!(
            withdrawn.contains("\"failed_tables\":[]"),
            "the rule decided every row"
        );
    }

    /// carnet#1511 design call 10: an athlete who coaches no one and reads
    /// no member keeps, in their own export, the messages the bot sent them
    /// that name no reply: a one-time code and a notice, as their session's
    /// conversation withholds nothing from them.
    #[tokio::test]
    async fn a_plain_users_export_keeps_their_codes_and_notices() {
        let squad = squad().await;
        let res = &squad.res;
        let bea = &squad.bea;
        let own = res
            .common
            .repos
            .chat
            .create_conversation(
                &bea.id.to_string(),
                bea.tenant,
                "Own",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let session = Uuid::new_v4().to_string();
        let messaging = &res.common.repos.messaging;
        messaging
            .create_session(&CreateSessionParams {
                id: &session,
                user_id: &bea.id.to_string(),
                tenant_id: bea.tenant,
                channel_type: "telegram",
                channel_user_id: "bea-on-telegram",
                channel_conversation_id: Some("bea-dm"),
                pierre_conversation_id: Some(&own),
            })
            .await
            .unwrap();
        for (inbound, body) in [
            (true, "bea-asked-for-a-code"),
            (false, "bea-one-time-code-482913"),
            (false, "bea-notice-your-week-is-ready"),
        ] {
            let id = Uuid::new_v4().to_string();
            messaging
                .insert_message(&InsertMessageParams {
                    id: &id,
                    tenant_id: bea.tenant,
                    session_id: &session,
                    direction: if inbound { "inbound" } else { "outbound" },
                    channel_type: "telegram",
                    channel_message_id: &id,
                    sender_id: if inbound { "bea-on-telegram" } else { "pierre" },
                    content_type: "text",
                    content_body: Some(body),
                    correlation_id: &id,
                    raw_payload: None,
                    chat_message_id: None,
                    proactive_purpose: None,
                })
                .await
                .unwrap();
        }

        let exported = export_as(&squad, &bea.auth).await;
        for kept in [
            "bea-asked-for-a-code",
            "bea-one-time-code-482913",
            "bea-notice-your-week-is-ready",
        ] {
            assert!(exported.contains(kept), "Bea's export keeps {kept}");
        }
        assert!(exported.contains("\"failed_tables\":[]"), "nothing failed");
    }

    /// carnet#1511 design call 7, an A2A tool task over the real route: the
    /// coach's agent reads Bea by name with a direct tool call, and the task
    /// records that it read Bea. The coach's export holds the task while
    /// Bea shares with the coach, and leaves it out once she stops.
    #[tokio::test]
    async fn an_a2a_tool_task_that_read_a_member_follows_their_sharing_in_the_export() {
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let grant = format!(
            "Bearer {}",
            delegated_test_token_for_tenant(
                res,
                squad.casey.id,
                Some(squad.casey.tenant.to_string())
            )
        );
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SendMessage",
            "params": {
                "message": {
                    "messageId": format!("m-{}", Uuid::new_v4()),
                    "role": "ROLE_USER",
                    "parts": [{ "data": {
                        "tool_name": "get_group_member_activities",
                        "parameters": { "member": "Bea Bright" }
                    } }]
                }
            },
        });
        let request = Request::post("/a2a/jsonrpc")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("A2A-Version", "1.0")
            .header("authorization", &grant)
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let answer: Value = serde_json::from_slice(&bytes).unwrap();
        let task = answer["result"]["task"].clone();
        assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED", "{answer}");
        assert!(
            task.to_string().contains("bea-1"),
            "the task read Bea's ride: {task}"
        );
        let task_id = task["id"].as_str().expect("the task's id").to_owned();

        let shared = coach_export(&squad).await;
        assert!(
            shared.contains(&task_id),
            "while Bea shares, the export holds the task"
        );
        res.common
            .repos
            .groups
            .update_coach_sharing_consent(&squad.group.to_string(), squad.bea.id, false)
            .await
            .unwrap();
        let withdrawn = coach_export(&squad).await;
        assert!(
            !withdrawn.contains(&task_id) && !withdrawn.contains("bea-1"),
            "once Bea stops sharing, the export holds no task that read her"
        );
        assert!(
            withdrawn.contains("\"failed_tables\":[]"),
            "the rule decided every row"
        );
    }

    /// A task from before the record of `who`, carrying `input`, filed
    /// under no conversation: its id.
    async fn legacy_task(squad: &Squad, who: Uuid, input: Value) -> String {
        squad
            .res
            .common
            .repos
            .a2a
            .create_task(&user_task_key(who), None, "message", &input, None)
            .await
            .unwrap()
    }

    /// A legacy tool task's input: one call of `tool` with `parameters`,
    /// beside `marker` as text so the export shows the task.
    fn tool_input(marker: &str, tool: &str, parameters: &Value) -> Value {
        json!({ "parts": [
            { "text": marker },
            { "data": { "tool_name": tool, "parameters": parameters } }
        ] })
    }

    /// The tail every export that left nothing out ends with, as on main.
    const WHOLE_EXPORT_TAIL: &str =
        "\"complete\":true,\"limit_reached\":null,\"failed_tables\":[],\"skipped_tables\":[]}";

    /// carnet#1511 design §6.2 rule 2 and §6.4: an athlete who never had
    /// another member's data within reach keeps every task from before the
    /// record, a tool call naming someone and a text turn included, and their
    /// export ends exactly as on main: no withheld table, complete.
    #[tokio::test]
    async fn a_plain_athletes_export_keeps_every_legacy_task_and_ends_as_on_main() {
        let squad = squad().await;
        let dana = colleague(&squad).await;
        let mut markers = Vec::new();
        for (marker, input) in [
            (
                "dana-legacy-own-read",
                tool_input("dana-legacy-own-read", "get_activities", &json!({})),
            ),
            (
                "dana-legacy-named-read",
                tool_input(
                    "dana-legacy-named-read",
                    "get_activities",
                    &json!({ "athlete": "Alex" }),
                ),
            ),
            (
                "dana-legacy-text-turn",
                json!({ "parts": [{ "text": "dana-legacy-text-turn" }] }),
            ),
        ] {
            legacy_task(&squad, dana.id, input).await;
            markers.push(marker);
        }
        let exported = export_as(&squad, &dana.auth).await;
        for marker in markers {
            assert!(exported.contains(marker), "Dana's export keeps {marker}");
        }
        assert!(
            exported.trim_end().ends_with(WHOLE_EXPORT_TAIL)
                && !exported.contains("withheld_tables"),
            "a plain athlete's export ends as on main: {}",
            &exported[exported.len().saturating_sub(160)..]
        );
    }

    /// carnet#1511 design §6.2 and §6.4: a coach once had members' data
    /// within reach, so a task of theirs from before the record is kept only
    /// when it provably holds their own: a call of a tool that reads its
    /// caller alone, naming no one. The same tool naming an athlete, a tool
    /// that may read a coached athlete, and a text turn no thread speaks for
    /// are left out, and the export names the table and says it is not
    /// complete.
    #[tokio::test]
    async fn a_coachs_legacy_task_is_kept_only_when_it_holds_their_own_data() {
        let squad = squad().await;
        let casey = squad.casey.id;
        let kept = "casey-legacy-own-connection-check";
        legacy_task(
            &squad,
            casey,
            tool_input(kept, "get_connection_status", &json!({})),
        )
        .await;
        let left_out = [
            "casey-legacy-connection-check-naming-alex",
            "casey-legacy-activities-read",
            "casey-legacy-text-turn",
        ];
        legacy_task(
            &squad,
            casey,
            tool_input(
                left_out[0],
                "get_connection_status",
                &json!({ "athlete": "Alex" }),
            ),
        )
        .await;
        legacy_task(
            &squad,
            casey,
            tool_input(left_out[1], "get_activities", &json!({})),
        )
        .await;
        legacy_task(&squad, casey, json!({ "parts": [{ "text": left_out[2] }] })).await;

        let exported = coach_export(&squad).await;
        assert!(exported.contains(kept), "the coach keeps their own task");
        for gone in left_out {
            assert!(
                !exported.contains(gone),
                "a legacy task that may hold a member's data is left out: {gone}"
            );
        }
        let tail = &exported[exported
            .rfind("\"withheld_tables\"")
            .expect("a withheld table")..];
        assert!(
            tail.contains("\"a2a_tasks\"") && tail.contains("\"complete\":false"),
            "the export names the table before complete and is not complete: {tail}"
        );
        assert!(
            tail.contains("\"failed_tables\":[]"),
            "nothing failed: {tail}"
        );
    }

    /// carnet#1511 design A7 (call 19), over the real A2A route: a tool task
    /// records each read with how it was made, as a chat row does. The
    /// coach's named read of Alex's sleep is recorded as the coach's own
    /// wellness read under the group, so the coach's export holds the task
    /// while Alex grants wellness reads, and leaves it out once she takes
    /// that grant back, though she still shares her training with the coach.
    #[tokio::test]
    async fn an_a2a_wellness_read_is_the_coachs_and_follows_the_wellness_grant() {
        let squad = squad().await;
        file_alex_sleep(&squad, 2).await;
        let res = &squad.res;
        let group = squad.group.to_string();
        res.common
            .repos
            .groups
            .set_coach_consents(&group, squad.alex.id, None, Some(true), Timestamp::now())
            .await
            .unwrap();
        let app = ProviderToolRouter::build_http_app(res);
        let grant = format!(
            "Bearer {}",
            delegated_test_token_for_tenant(
                res,
                squad.casey.id,
                Some(squad.casey.tenant.to_string())
            )
        );
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SendMessage",
            "params": {
                "message": {
                    "messageId": format!("m-{}", Uuid::new_v4()),
                    "role": "ROLE_USER",
                    "parts": [{ "data": {
                        "tool_name": "get_sleep_sessions",
                        "parameters": { "athlete": "Alex" }
                    } }]
                }
            },
        });
        let request = Request::post("/a2a/jsonrpc")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("A2A-Version", "1.0")
            .header("authorization", &grant)
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let answer: Value = serde_json::from_slice(&bytes).unwrap();
        let task = answer["result"]["task"].clone();
        assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED", "{answer}");
        assert!(
            task.to_string().contains(&ALEX_SLEEP_SECONDS.to_string()),
            "the task read Alex's sleep: {task}"
        );
        let task_id = task["id"].as_str().expect("the task's id").to_owned();

        let shared = coach_export(&squad).await;
        let document: Value = serde_json::from_str(&shared).unwrap();
        let row = document["tables"]["a2a_tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["task_id"] == task_id.as_str())
            .expect("while Alex grants wellness reads, the export holds the task")
            .clone();
        let reads: Vec<CarriedRead> =
            serde_json::from_str(row["about_members"].as_str().expect("the task's record"))
                .unwrap();
        let alex: Vec<&CarriedRead> = reads.iter().filter(|r| r.member == squad.alex.id).collect();
        assert_eq!(alex.len(), 1, "one read of Alex: {reads:?}");
        assert_eq!(
            (
                alex[0].way,
                alex[0].group,
                alex[0].coach,
                alex[0].class.as_str()
            ),
            (
                ReadWay::Coach,
                Some(squad.group),
                Some(squad.casey.id),
                "wellness"
            ),
            "the read is the coach's wellness read under the group: {reads:?}"
        );

        res.common
            .repos
            .groups
            .set_coach_consents(&group, squad.alex.id, None, Some(false), Timestamp::now())
            .await
            .unwrap();
        let withdrawn = coach_export(&squad).await;
        assert!(
            !withdrawn.contains(&task_id),
            "once Alex takes the wellness grant back, the export holds no task that read it"
        );
        let tail = &withdrawn[withdrawn
            .rfind("\"withheld_tables\"")
            .expect("a withheld table")..];
        assert!(
            tail.contains("\"a2a_tasks\"") && tail.contains("\"complete\":false"),
            "the export says what it left out: {tail}"
        );
    }

    // ------------------------------------------------------------------
    // Round 13, the last fix pass (carnet#1511 calls 21 to 23).
    // ------------------------------------------------------------------

    /// One JSON-RPC call on the A2A binding as `grant`: the envelope.
    async fn a2a_rpc(app: &Router, grant: &str, method: &str, params: Value) -> Value {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let request = Request::post("/a2a/jsonrpc")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("A2A-Version", "1.0")
            .header("authorization", grant)
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Round-12 room-gate major (design call 2): a stored A2A task is one
    /// more read of the member its call read. The coach's agent reads Bea by
    /// name; while Bea shares with the coach, `GetTask` and `ListTasks` serve
    /// the result, and once she stops, both serve the task without it, as
    /// the export, the screen and the replay withhold it.
    #[tokio::test]
    async fn a_stored_a2a_task_withholds_a_member_who_stopped_sharing() {
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let grant = format!(
            "Bearer {}",
            delegated_test_token_for_tenant(
                res,
                squad.casey.id,
                Some(squad.casey.tenant.to_string())
            )
        );
        let sent = a2a_rpc(
            &app,
            &grant,
            "SendMessage",
            json!({ "message": {
                "messageId": format!("m-{}", Uuid::new_v4()),
                "role": "ROLE_USER",
                "parts": [{ "data": {
                    "tool_name": "get_group_member_activities",
                    "parameters": { "member": "Bea Bright" }
                } }]
            } }),
        )
        .await;
        let task = sent["result"]["task"].clone();
        assert!(
            task.to_string().contains("bea-1"),
            "the task read Bea: {sent}"
        );
        let task_id = task["id"].as_str().expect("the task's id").to_owned();

        let read = a2a_rpc(&app, &grant, "GetTask", json!({ "id": task_id })).await;
        assert!(
            read.to_string().contains("bea-1"),
            "while Bea shares, GetTask serves the result: {read}"
        );
        res.common
            .repos
            .groups
            .update_coach_sharing_consent(&squad.group.to_string(), squad.bea.id, false)
            .await
            .unwrap();
        let read = a2a_rpc(&app, &grant, "GetTask", json!({ "id": task_id })).await;
        assert_eq!(
            read["result"]["id"],
            task_id.as_str(),
            "the task itself is served: {read}"
        );
        assert!(
            !read.to_string().contains("bea-1"),
            "bug: GetTask served a member who stopped sharing: {read}"
        );
        let listed = a2a_rpc(
            &app,
            &grant,
            "ListTasks",
            json!({ "includeArtifacts": true }),
        )
        .await;
        assert!(
            listed.to_string().contains(&task_id) && !listed.to_string().contains("bea-1"),
            "bug: ListTasks served a member who stopped sharing: {listed}"
        );
    }

    /// Round-12 room-gate major (call 21): a model quick reply is written by
    /// the model from the reply that offered it, and a tap stores it as the
    /// sender's own words, which no rule withholds. In a thread whose rows
    /// read another member, a tapped one is answered as a suggestion that no
    /// longer resolves: no row of its words is stored and no model runs.
    #[tokio::test]
    async fn a_tapped_model_quick_reply_in_a_thread_that_read_a_member_is_dropped() {
        const WORDS: &str = "Cut Alex's intervals after her 4h sleep";
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let own = own_conversation(&squad, "Alex's week").await;
        coach_row(&squad, &own, "user", "How did Alex sleep?", false).await;
        coach_row(&squad, &own, "assistant", "Alex slept 4h, HRV 40.", true).await;

        let (status, body) =
            post_text_over_api_key(&app, &squad.casey.auth, &own, &format!("qr:0:{WORDS}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            !history(&squad, &own).await.contains(WORDS),
            "bug: a tapped model quick reply drawn from Alex's data was stored as the coach's \
             own words"
        );

        // A thread that read no one keeps the tap.
        let plain = own_conversation(&squad, "My week").await;
        coach_row(
            &squad,
            &plain,
            "assistant",
            "Your week looks steady.",
            false,
        )
        .await;
        let (status, body) =
            post_text_over_api_key(&app, &squad.casey.auth, &plain, "qr:0:Easy spin").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            history(&squad, &plain).await.contains("Easy spin"),
            "a quick reply in a thread that read no member is the coach's own words"
        );
    }

    /// Round-13 still-works major: the tap is decided by the reads of the
    /// topic the quick reply was offered in, as the offer is. Once a divider
    /// opened a new topic, a quick reply offered by a turn that drew on no
    /// one is the coach's own words, as on main, although an earlier topic
    /// of the same thread read Alex.
    #[tokio::test]
    async fn a_tapped_model_quick_reply_after_a_divider_is_taken() {
        let squad = squad().await;
        let res = &squad.res;
        let app = ProviderToolRouter::build_http_app(res);
        let own = own_conversation(&squad, "Alex then me").await;
        coach_row(&squad, &own, "user", "How did Alex sleep?", false).await;
        coach_row(&squad, &own, "assistant", "Alex slept 4h, HRV 40.", true).await;
        sleep(StdDuration::from_millis(5)).await;
        res.common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: squad.casey.tenant,
                conversation_id: &own,
                user_id: &squad.casey.id.to_string(),
                role: "system",
                content: "",
                token_count: None,
                finish_reason: Some(DIVIDER_FINISH_REASON),
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
        coach_row(&squad, &own, "user", "Plan my long run", false).await;
        coach_row(&squad, &own, "assistant", "Two hours, easy.", false).await;

        let (status, body) =
            post_text_over_api_key(&app, &squad.casey.auth, &own, "qr:0:Make it 90 minutes").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            history(&squad, &own).await.contains("Make it 90 minutes"),
            "bug: a quick reply offered after a divider, by a turn that read no one, was dropped"
        );
    }

    /// What `who` calling `tool` with `arguments` in their own conversation
    /// `conversation` in the app is answered, from an executor built inside
    /// the turn's direct-message scope, as the pipeline builds a turn's (it
    /// reads the flag when it is built), under the running provenance.
    async fn private_call(
        squad: &Squad,
        who: &Person,
        conversation: &str,
        tool: &str,
        arguments: &Value,
    ) -> String {
        let repos = &squad.res.common.repos;
        let role = CallerRole::resolve(repos, who.id).await;
        let runtime: Arc<dyn ToolRuntime> = squad.res.clone();
        let response = TURN_DIRECT_MESSAGE
            .scope(Some(true), async {
                UniversalExecutor::new(runtime)
                    .with_scopes(OAuthScope::self_grant())
                    .with_transport(Transport::WebApp)
                    .with_turn_role(role)
                    .with_conversation_id(conversation.to_owned())
                    .with_conversation_tenant(who.tenant.as_uuid())
                    .with_turn_token(Uuid::new_v4().to_string())
                    .with_seat(TurnSeat::Subject)
                    .execute_tool(UniversalRequest {
                        tool_name: tool.to_owned(),
                        parameters: arguments.clone(),
                        user_id: who.id.to_string(),
                        protocol: "test".to_owned(),
                        tenant_id: Some(who.tenant.to_string()),
                    })
                    .await
            })
            .await;
        match response {
            Ok(response) => format!(
                "{} {}",
                response.error.unwrap_or_default(),
                response.result.unwrap_or_default()
            ),
            Err(error) => error.to_string(),
        }
    }

    /// carnet#1511 call 24: free text lands on an athlete's account only
    /// from a topic that drew on no member besides the caller and that
    /// athlete. In the coach's own conversation a session prescribed for
    /// Alex is refused after a read of Bea, whether made on this turn or
    /// replayed from an earlier one ("roster, then prescribe for Alex"), and
    /// the refusal points at Alex's thread or a new topic. A read of Alex
    /// herself refuses nothing.
    #[tokio::test]
    async fn a_free_text_write_onto_one_athlete_after_reading_another_is_refused() {
        let squad = squad().await;
        let own = own_conversation(&squad, "Planning").await;
        let prescribe =
            json!({ "athlete": "Alex", "date": "2026-12-01", "template_slug": "long_run_z2" });
        let bea = CarriedRead::coach(squad.bea.id, squad.group, squad.casey.id);
        let after_bea = ai_scope::tracking(Provenance::new(), async {
            ai_scope::note_member_read(bea.clone());
            private_call(&squad, &squad.casey, &own, "prescribe_workout", &prescribe).await
        })
        .await;
        assert!(
            after_bea.contains("member_read_withheld"),
            "bug: a session for Alex was written from a turn that read Bea: {after_bea}"
        );
        let after_replayed_bea = ai_scope::tracking(Provenance::new(), async {
            ai_scope::note_replayed_member_read(bea.clone());
            private_call(&squad, &squad.casey, &own, "prescribe_workout", &prescribe).await
        })
        .await;
        assert!(
            after_replayed_bea.contains("member_read_withheld"),
            "bug: a read of Bea replayed from an earlier turn of the topic let a session for \
             Alex through: {after_replayed_bea}"
        );
        assert!(
            after_replayed_bea.contains("thread") && after_replayed_bea.contains("new topic"),
            "bug: the refusal names no remedy (Alex's thread, or a new topic): \
             {after_replayed_bea}"
        );
        let after_alex = ai_scope::tracking(Provenance::new(), async {
            ai_scope::note_member_read(CarriedRead::coach(
                squad.alex.id,
                squad.group,
                squad.casey.id,
            ));
            private_call(&squad, &squad.casey, &own, "prescribe_workout", &prescribe).await
        })
        .await;
        assert!(
            !after_alex.contains("member_read_withheld"),
            "bug: a read of Alex herself refused a session for Alex: {after_alex}"
        );
    }

    /// carnet#1511 call 26: a write is never a read of its own target. In
    /// the coach's own conversation, two sessions prescribed for Alex in one
    /// turn are both taken, and so is one for Bea after them.
    #[tokio::test]
    async fn repeated_writes_onto_one_athlete_in_one_turn_are_all_taken() {
        let squad = squad().await;
        let own = own_conversation(&squad, "Week planning").await;
        let provenance = Provenance::new();
        let answers = ai_scope::tracking(provenance.clone(), async {
            let mut answers = Vec::new();
            for (athlete, date) in [
                ("Alex", "2026-12-01"),
                ("Alex", "2026-12-03"),
                ("Bea", "2026-12-02"),
            ] {
                let arguments =
                    json!({ "athlete": athlete, "date": date, "template_slug": "long_run_z2" });
                answers.push(
                    private_call(&squad, &squad.casey, &own, "prescribe_workout", &arguments).await,
                );
            }
            answers
        })
        .await;
        assert!(
            provenance
                .carried_reads()
                .iter()
                .any(|read| read.member == squad.alex.id),
            "premise: the first write reached Alex's plan scope and is stamped about her"
        );
        for answer in answers {
            assert!(
                !answer.contains("member_read_withheld"),
                "bug: a write onto one athlete refused a later write in the same turn: {answer}"
            );
        }
    }

    /// Alex holds a saved FTP and threshold heart rate, under her tenant and
    /// the coach's: what a session prescribed for her is anchored to.
    async fn alex_holds_a_profile(squad: &Squad) {
        let mut held = UserPhysiologicalProfile::new(squad.alex.id, SportType::Run);
        held.ftp_watts = Some(280);
        held.threshold_hr = Some(172);
        let mut tenants = vec![squad.alex.tenant];
        if squad.casey.tenant != squad.alex.tenant {
            tenants.push(squad.casey.tenant);
        }
        for tenant in tenants {
            squad
                .res
                .common
                .repos
                .user_physiological_profile
                .upsert_user_physiological_profile(tenant, squad.alex.id, &held)
                .await
                .unwrap();
        }
    }

    /// A session prescribed for `athlete` on `date`, as the model calls it.
    fn prescribe_for(athlete: &str, date: &str) -> (String, Value) {
        (
            "prescribe_workout".to_owned(),
            json!({ ATHLETE: athlete, "date": date, "template_slug": "long_run_z2" }),
        )
    }

    /// The coach's turn in their own conversation `own` in the app, the
    /// model calling `first` and then `then`, if any: what the model read
    /// on the turn's last request.
    async fn coach_turn_calling(
        squad: &Squad,
        own: &str,
        first: (String, Value),
        then: Option<(String, Value)>,
    ) -> String {
        *squad.model.call.lock().unwrap() = first;
        *squad.model.then.lock().unwrap() = then;
        turn_in(
            squad,
            &squad.casey,
            Transport::WebApp,
            &OAuthScope::self_grant(),
            own.to_owned(),
            true,
        )
        .await;
        *squad.model.call.lock().unwrap() = (ROSTER.to_owned(), json!({}));
        *squad.model.then.lock().unwrap() = None;
        squad
            .model
            .conversations
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    /// How many results of `tool` the model read on the turn itself in
    /// `conversation`: after the turn's own message, the history it replays
    /// left out.
    fn results_of(conversation: &str, tool: &str) -> usize {
        conversation
            .rsplit("How is my group doing?")
            .next()
            .unwrap_or_default()
            .matches(&format!("[Tool Result for {tool}]"))
            .count()
    }

    /// carnet#1511 call 35 (supersedes 31): a coach's own write onto an
    /// athlete is no read of them, even when it admits their stored
    /// physiology for its own computation. Prescribing for Alex, who holds
    /// a saved FTP, anchors her session to it; a session for Bea in the same
    /// turn is still taken. The turn is stamped about Alex, so its rows keep
    /// her audience, but it draws on no one.
    #[tokio::test]
    async fn a_write_that_admits_its_targets_physiology_draws_on_no_one() {
        let squad = squad().await;
        alex_holds_a_profile(&squad).await;
        let own = own_conversation(&squad, "Week planning").await;
        let provenance = Provenance::new();
        let answers = ai_scope::tracking(provenance.clone(), async {
            let mut answers = Vec::new();
            for (athlete, date) in [("Alex", "2026-12-01"), ("Bea", "2026-12-02")] {
                let (tool, arguments) = prescribe_for(athlete, date);
                answers.push(private_call(&squad, &squad.casey, &own, &tool, &arguments).await);
            }
            answers
        })
        .await;
        for answer in &answers {
            assert!(
                !answer.contains("member_read_withheld"),
                "bug: a write that admitted Alex's stored physiology refused the next write \
                 in the turn: {answer}"
            );
        }
        assert!(
            !provenance.members_drawn_on().contains(&squad.alex.id),
            "bug: a write onto Alex that admitted her physiology drew on her: {}",
            answers[0]
        );
        assert!(
            provenance
                .carried_reads()
                .iter()
                .any(|read| read.member == squad.alex.id),
            "bug: the write onto Alex no longer stamps the turn about her: {}",
            answers[0]
        );
    }

    /// carnet#1511 call 35, end to end in one turn: the coach asks, in
    /// their own conversation in the app, for a session for Alex (who holds
    /// a saved FTP) and one for Bea. The model prescribes both in one turn;
    /// both are taken.
    #[tokio::test]
    async fn a_coach_prescribes_for_two_athletes_in_one_turn() {
        let squad = squad().await;
        alex_holds_a_profile(&squad).await;
        let own = own_conversation(&squad, "Two sessions").await;
        let conversation = coach_turn_calling(
            &squad,
            &own,
            prescribe_for("Alex", "2026-12-01"),
            Some(prescribe_for("Bea", "2026-12-02")),
        )
        .await;
        assert_eq!(
            results_of(&conversation, "prescribe_workout"),
            2,
            "premise: the model prescribed for Alex and then for Bea: {conversation}"
        );
        assert!(
            !conversation.contains("member_read_withheld")
                && !conversation.contains("it read another group member's data"),
            "bug: the session for Bea was refused after the session for Alex: {conversation}"
        );
    }

    /// carnet#1511 call 35, end to end over two turns of one topic: a
    /// session for Alex in turn 1, then one for Bea in turn 2. Turn 2
    /// replays turn 1's rows, which carry the write onto Alex as written,
    /// never as a read of her, so the session for Bea is taken. Alex's rows
    /// keep her audience: the coach still reads them.
    #[tokio::test]
    async fn a_coach_prescribes_for_another_athlete_in_the_next_turn() {
        let squad = squad().await;
        alex_holds_a_profile(&squad).await;
        let own = own_conversation(&squad, "Two turns").await;
        let first =
            coach_turn_calling(&squad, &own, prescribe_for("Alex", "2026-12-01"), None).await;
        assert_eq!(
            results_of(&first, "prescribe_workout"),
            1,
            "premise: turn 1 prescribed for Alex: {first}"
        );
        assert!(
            !first.contains("member_read_withheld"),
            "premise: the session for Alex is taken: {first}"
        );
        let second =
            coach_turn_calling(&squad, &own, prescribe_for("Bea", "2026-12-02"), None).await;
        assert_eq!(
            results_of(&second, "prescribe_workout"),
            1,
            "premise: turn 2 prescribed for Bea: {second}"
        );
        assert!(
            !second.contains("member_read_withheld")
                && !second.contains("it read another group member's data")
                && !second.contains("'prescribe_workout' not found"),
            "bug: a write onto Alex replayed from turn 1 refused the session for Bea: {second}"
        );
        assert!(
            squad.model.declared_first().contains("prescribe_workout"),
            "bug: turn 2 did not offer prescribe_workout after a write onto Alex"
        );
        let keys: Vec<String> = squad
            .res
            .common
            .repos
            .chat
            .list_member_reads(&own, &squad.casey.id.to_string(), squad.casey.tenant)
            .await
            .unwrap()
            .into_iter()
            .map(|read| read.key)
            .collect();
        let alex_written = format!("written|{}|", squad.alex.id);
        assert!(
            keys.iter().any(|key| key.starts_with(&alex_written)),
            "bug: turn 1's rows do not carry the write onto Alex as written: {keys:?}"
        );
        assert!(
            history(&squad, &own).await.contains(REPLY),
            "the coach still reads both turns' replies"
        );
    }

    /// carnet#1511 calls 24 and 35, end to end: a real read of the other
    /// members (the roster overview, which reads Bea) followed by a
    /// free-text session for Alex in the same turn is still refused; only
    /// the coach's own writes stopped counting as reads.
    #[tokio::test]
    async fn a_session_for_alex_after_a_real_read_of_bea_is_refused() {
        let squad = squad().await;
        alex_holds_a_profile(&squad).await;
        let own = own_conversation(&squad, "Read then write").await;
        let conversation = coach_turn_calling(
            &squad,
            &own,
            (ROSTER.to_owned(), json!({})),
            Some(prescribe_for("Alex", "2026-12-01")),
        )
        .await;
        assert!(
            results_of(&conversation, ROSTER) == 1 && conversation.contains("Bea"),
            "premise: the roster read Bea: {conversation}"
        );
        assert_eq!(
            results_of(&conversation, "prescribe_workout"),
            1,
            "premise: the model read the roster and then prescribed for Alex: {conversation}"
        );
        assert!(
            conversation.contains("it read another group member's data"),
            "bug: a session for Alex was written after a real read of Bea: {conversation}"
        );
    }

    /// carnet#1511 call 30: the athlete-thread seat needs live coach access
    /// at turn start. With it, the coach's thread about Bea withholds the
    /// coach's own writes; once the coach's access is taken back, the
    /// thread is the coach's private seat, and offers what their own
    /// conversation does instead of athlete tools every call would refuse.
    #[tokio::test]
    async fn an_athlete_thread_without_coach_access_is_the_coachs_private_seat() {
        let squad = squad().await;
        let thread = squad
            .res
            .common
            .repos
            .chat
            .open_subject_conversation(&NewSubjectConversation {
                user_id: &squad.casey.id.to_string(),
                tenant_id: squad.casey.tenant,
                title: "Bea Bright",
                model: "mock-model",
                agent_id: &squad.agent,
                subject: ConversationSubject {
                    athlete_user_id: squad.bea.id,
                    group_id: squad.group,
                },
            })
            .await
            .unwrap()
            .conversation
            .id;
        let grant = OAuthScope::self_grant();
        let live = turn_in(
            &squad,
            &squad.casey,
            Transport::WebApp,
            &grant,
            thread.clone(),
            true,
        )
        .await;
        assert!(
            !live.declared.contains("remember_fact"),
            "premise: the athlete thread withholds the coach's own writes: {:?}",
            live.declared
        );
        squad
            .res
            .common
            .repos
            .users
            .set_manages_roster(squad.casey.id, false)
            .await
            .unwrap();
        let revoked = turn_in(
            &squad,
            &squad.casey,
            Transport::WebApp,
            &grant,
            thread.clone(),
            true,
        )
        .await;
        assert!(
            revoked.declared.contains("remember_fact"),
            "bug: a thread whose coach access is gone still holds the athlete-thread seat: {:?}",
            revoked.declared
        );
        // Offered is accepted: a tool bound to the thread's athlete is
        // refused on every call once the coach no longer reaches her, so it
        // is not offered either.
        for bound in [
            "get_training_plan",
            "save_training_plan",
            "prescribe_workout",
            "get_planned_workouts",
            "save_workout_to_library",
        ] {
            if !revoked.declared.contains(bound) {
                continue;
            }
            let answer = private_call(&squad, &squad.casey, &thread, bound, &json!({})).await;
            assert!(
                !answer.contains("coach access is off"),
                "bug: {bound} is offered in a thread whose every call to it is refused: {answer}"
            );
        }
        assert!(
            live.declared.contains("save_training_plan"),
            "premise: the live thread offers the plan tools: {:?}",
            live.declared
        );
        // The loop above checks only what is offered; on the fixed seat the
        // tools bound to the unreached athlete are withheld outright.
        for bound in ["save_training_plan", "prescribe_workout"] {
            assert!(
                !revoked.declared.contains(bound),
                "bug: {bound} is offered for an athlete the coach no longer reaches: {:?}",
                revoked.declared
            );
        }
    }

    /// Round-12 still-works major (call 20): a member's own pre-deploy entry
    /// is theirs, as it was. Bea's private reply from before carried reads
    /// existed names Alex; she reads it on her screen and in her export
    /// although Alex shares nothing with her, and still once she has left the
    /// group.
    #[tokio::test]
    async fn a_readers_own_legacy_row_stays_theirs() {
        const LEGACY: &str = "legacy-peer-reply: Alex rode 60 km";
        const SQL: &str =
            "UPDATE chat_messages SET audience_recorded = FALSE, author_user_id = NULL WHERE id = $1";
        let squad = squad().await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let app = ProviderToolRouter::build_http_app(res);
        let bea = &squad.bea;
        let own = repos
            .chat
            .create_conversation(
                &bea.id.to_string(),
                bea.tenant,
                "Own",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let legacy = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: bea.tenant,
                conversation_id: &own,
                user_id: &bea.id.to_string(),
                role: "assistant",
                content: LEGACY,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: Some("web"),
                about_user_id: Some(squad.alex.id),
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap()
            .id;
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(SQL)
                    .bind(&legacy)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(SQL)
                    .bind(&legacy)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
        assert!(
            transcript_as(&app, &bea.auth, &own).await.contains(LEGACY),
            "bug: Bea's own legacy reply was withheld from her screen"
        );
        repos
            .groups
            .remove_member(&squad.group.to_string(), bea.id)
            .await
            .unwrap();
        assert!(
            transcript_as(&app, &bea.auth, &own).await.contains(LEGACY),
            "bug: Bea's own legacy reply left her screen when she left the group"
        );
        assert!(
            export_as(&squad, &bea.auth).await.contains(LEGACY),
            "bug: Bea's own legacy reply left her export"
        );
    }

    /// Round-12 still-works major: `/agent create` drafts from a plain
    /// athlete's own turns from before the deploy, as on main. Pia coaches
    /// no one and belongs to no group, so nothing in her legacy rows can be
    /// anyone else's.
    #[tokio::test]
    async fn a_plain_athletes_agent_draft_reads_their_own_legacy_turns() {
        const LEGACY: &str = "legacy-own: I want to ride a gran fondo in June";
        const SQL: &str = "UPDATE chat_messages SET audience_recorded = FALSE WHERE id = $1";
        let squad = squad().await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let app = ProviderToolRouter::build_http_app(res);
        let pia = person(res, "pia@chat-surfaces.test", "Pia Plain").await;
        repos
            .email_verification
            .mark_verified(pia.id)
            .await
            .unwrap();
        let own = repos
            .chat
            .create_conversation(
                &pia.id.to_string(),
                pia.tenant,
                "Own",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let legacy = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: pia.tenant,
                conversation_id: &own,
                user_id: &pia.id.to_string(),
                role: "user",
                content: LEGACY,
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
            .unwrap()
            .id;
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(SQL)
                    .bind(&legacy)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(SQL)
                    .bind(&legacy)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
        *squad.model.propose.lock().unwrap() = true;
        squad.model.learning.lock().unwrap().clear();

        let (status, body) = post_text_over_api_key(&app, &pia.auth, &own, "/agent create").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let drafted = drafted_from(&squad);
        assert!(
            drafted.contains(LEGACY),
            "bug: a plain athlete's own legacy turn did not reach their draft: {drafted} {body}"
        );
    }

    /// Round-12 still-works major: a plain athlete who deletes the
    /// conversation their Telegram session wrote to keeps, in their export,
    /// the bot's codes, notices and reply copies, as on main: the session
    /// now names no conversation and the reply is in none, but nothing in
    /// them can be anyone else's. The export ends as on main.
    #[tokio::test]
    async fn a_plain_athletes_export_after_deleting_a_messaging_thread_ends_as_on_main() {
        let squad = squad().await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let pia = person(res, "pia-export@chat-surfaces.test", "Pia Plain").await;
        let own = repos
            .chat
            .create_conversation(
                &pia.id.to_string(),
                pia.tenant,
                "Telegram",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let reply = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: pia.tenant,
                conversation_id: &own,
                user_id: &pia.id.to_string(),
                role: "assistant",
                content: "pia-reply-in-the-thread",
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: Some("telegram"),
                about_user_id: None,
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap()
            .id;
        let session = Uuid::new_v4().to_string();
        repos
            .messaging
            .create_session(&CreateSessionParams {
                id: &session,
                user_id: &pia.id.to_string(),
                tenant_id: pia.tenant,
                channel_type: "telegram",
                channel_user_id: "pia-on-telegram",
                channel_conversation_id: Some("pia-dm"),
                pierre_conversation_id: Some(&own),
            })
            .await
            .unwrap();
        for (body, copies) in [
            ("pia-one-time-code-482913", None),
            ("pia-notice-your-week-is-ready", None),
            ("pia-copy-of-the-reply", Some(reply.as_str())),
        ] {
            let id = Uuid::new_v4().to_string();
            repos
                .messaging
                .insert_message(&InsertMessageParams {
                    id: &id,
                    tenant_id: pia.tenant,
                    session_id: &session,
                    direction: "outbound",
                    channel_type: "telegram",
                    channel_message_id: &id,
                    sender_id: "pierre",
                    content_type: "text",
                    content_body: Some(body),
                    correlation_id: &id,
                    raw_payload: None,
                    chat_message_id: copies,
                    proactive_purpose: None,
                })
                .await
                .unwrap();
        }
        assert!(
            repos
                .chat
                .delete_conversation(&own, &pia.id.to_string(), pia.tenant)
                .await
                .unwrap(),
            "Pia's conversation is deleted"
        );

        let exported = export_as(&squad, &pia.auth).await;
        for kept in [
            "pia-one-time-code-482913",
            "pia-notice-your-week-is-ready",
            "pia-copy-of-the-reply",
        ] {
            assert!(exported.contains(kept), "bug: Pia's export left out {kept}");
        }
        assert!(
            exported.ends_with(WHOLE_EXPORT_TAIL),
            "bug: Pia's export does not end as on main: {}",
            &exported[exported.len().saturating_sub(300)..]
        );
    }

    /// Call 22 (design §8, Q24): an admin keeps the coach tools in Dravr's
    /// own app and on Messaging, as over MCP and A2A. An admin who coaches no
    /// group is offered the roster and a member's plan status in their own
    /// conversation and reads the "no group" answer from each: listing and
    /// dispatch agree.
    #[tokio::test]
    async fn an_admin_keeps_the_coach_tools_in_the_app_and_reads_no_group() {
        let squad = squad().await;
        let res = &squad.res;
        let repos = &res.common.repos;
        let ada = person(res, "ada-app@chat-surfaces.test", "Ada Admin").await;
        repos.users.set_admin_status(ada.id, true).await.unwrap();
        seed_activities(res, &ada, INTERVALS_ICU, &[intervals_ride("ada-app-1", 2)]).await;
        repos
            .email_verification
            .mark_verified(ada.id)
            .await
            .unwrap();
        for transport in [Transport::WebApp, Transport::Messaging] {
            for (tool, arguments) in [
                (ROSTER, json!({})),
                ("get_group_member_plan_status", json!({ "member": "Bea" })),
            ] {
                let served = turn_calling(
                    &squad,
                    &ada,
                    transport,
                    &OAuthScope::self_grant(),
                    (tool, arguments),
                )
                .await;
                assert!(
                    served.declared.contains(tool),
                    "bug: {transport:?}: an admin is not offered {tool} in the app"
                );
                let answer = served
                    .conversation
                    .rsplit_once(&format!("[Tool Result for {tool}]"))
                    .map_or_default(|(_, answer)| answer.to_owned());
                assert!(
                    answer.contains("You lead no active group"),
                    "{transport:?}: {tool} answers an admin with no group the no-group text: {answer}"
                );
            }
        }
    }

    /// Call 23: a member's peer sharing covers their activities alone. The
    /// coach's read of Alex's plan is a coach-way read no peer view covers,
    /// so no member of the room reads a reply drawn from it, whatever Alex
    /// shares with the group.
    #[tokio::test]
    async fn a_coachs_read_of_a_members_plan_is_no_peer_views() {
        let squad = squad().await;
        let own = own_conversation(&squad, "Alex's plan").await;
        let reads = ai_scope::tracking(Provenance::new(), async {
            let answer = private_call(
                &squad,
                &squad.casey,
                &own,
                "get_training_plan",
                &json!({ "athlete": "Alex" }),
            )
            .await;
            (
                answer,
                ai_scope::current_provenance()
                    .map_or_default(|provenance| provenance.carried_reads()),
            )
        })
        .await;
        let (answer, reads) = reads;
        let of_alex: Vec<&CarriedRead> = reads
            .iter()
            .filter(|read| read.member == squad.alex.id)
            .collect();
        assert!(!of_alex.is_empty(), "the plan read noted Alex: {answer}");
        // A row carries the strictest of a member's reads: the plan read
        // outweighs any read of her activities the plan tool made beside it.
        let stamped = CarriedRead::strictest_per_member(of_alex.iter().copied());
        assert!(
            stamped.iter().all(|read| !read.peer_view_covers()),
            "bug: a coach's read of Alex's plan is covered by her peer sharing: {of_alex:?}"
        );
    }

    /// Write `content` as an agent row of the coach's conversation `own`
    /// about `about`, as main wrote it before rows carried their reads
    /// (migration 20261124130000): no record, no author, and a member named
    /// only when the turn read exactly one (`ai_scope::sole` on main).
    async fn coach_legacy_reply(squad: &Squad, own: &str, about: Option<Uuid>, content: &str) {
        const SQL: &str =
            "UPDATE chat_messages SET audience_recorded = FALSE, author_user_id = NULL WHERE id = $1";
        let res = &squad.res;
        let row = res
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: squad.casey.tenant,
                conversation_id: own,
                user_id: &squad.casey.id.to_string(),
                role: "assistant",
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: Some("web"),
                about_user_id: about,
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap()
            .id;
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(SQL)
                    .bind(&row)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(SQL)
                    .bind(&row)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
        sleep(StdDuration::from_millis(5)).await;
    }

    /// A topic divider ("+" or `/reset`) in the coach's conversation `own`.
    async fn coach_divider(squad: &Squad, own: &str) {
        squad
            .res
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: squad.casey.tenant,
                conversation_id: own,
                user_id: &squad.casey.id.to_string(),
                role: "system",
                content: "",
                token_count: None,
                finish_reason: Some(DIVIDER_FINISH_REASON),
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
        sleep(StdDuration::from_millis(5)).await;
    }

    const LEGACY_ABOUT_BEA: &str = "legacy-one: Bea rode 40 km on Saturday";
    const LEGACY_SEVERAL: &str = "legacy-several: Alex and Bea both rode long this week";

    /// Whether the coach's turn that answered `conversation` wrote a
    /// session: offered `prescribe_workout`, called it, and was not refused
    /// for a member read.
    fn wrote_a_session(squad: &Squad, conversation: &str) -> bool {
        squad.model.declared_first().contains("prescribe_workout")
            && results_of(conversation, "prescribe_workout") == 1
            && !conversation.contains("member_read_withheld")
            && !conversation.contains("is not available in this topic")
    }

    /// carnet#1511 A8: a legacy reply that drew on several members names
    /// none of them (main wrote `about_user_id` only for one), so replaying
    /// it recorded nothing and the turn read as drawing on nobody. In the
    /// coach's own conversation, which also holds a legacy reply about Bea,
    /// that unnamed reply drew on members the thread cannot name: the turn
    /// draws on an unknown member, and no free-text session is written onto
    /// any athlete, Bea herself included, until a new topic.
    ///
    /// In the topic holding both replies the session is not offered (the
    /// topic also drew on Bea, so no named free-text write could pass). In a
    /// later topic whose only legacy reply names no one, the session is
    /// offered and refused at dispatch, the refusal saying to start a new
    /// topic. After one more divider the session is taken.
    #[tokio::test]
    async fn a_legacy_reply_that_names_no_one_refuses_free_text_onto_an_athlete() {
        const LEGACY_LATER: &str = "legacy-later: the squad's long rides went well";
        let squad = squad().await;
        let own = own_conversation(&squad, "Legacy squad talk").await;
        coach_legacy_reply(&squad, &own, Some(squad.bea.id), LEGACY_ABOUT_BEA).await;
        coach_legacy_reply(&squad, &own, None, LEGACY_SEVERAL).await;
        let both = coach_turn_calling(&squad, &own, prescribe_for("Bea", "2026-12-02"), None).await;
        assert!(
            both.contains(LEGACY_SEVERAL) && both.contains(LEGACY_ABOUT_BEA),
            "premise: the turn replays both legacy replies: {both}"
        );
        assert!(
            !wrote_a_session(&squad, &both),
            "bug: a session for Bea was written from a topic replaying a legacy reply that drew \
             on members no row names: {both}"
        );

        coach_divider(&squad, &own).await;
        coach_legacy_reply(&squad, &own, None, LEGACY_LATER).await;
        let later =
            coach_turn_calling(&squad, &own, prescribe_for("Bea", "2026-12-03"), None).await;
        assert!(
            later.contains(LEGACY_LATER) && !later.contains(LEGACY_SEVERAL),
            "premise: the later topic replays only its own legacy reply: {later}"
        );
        assert!(
            !wrote_a_session(&squad, &later),
            "bug: a session for Bea was written from a topic replaying a legacy reply in a \
             conversation whose legacy rows named another member: {later}"
        );
        assert!(
            later.contains("is not available in this topic") && later.contains("Start a new topic"),
            "bug: the session was not refused at dispatch with the remedy (a new topic): {later}"
        );

        coach_divider(&squad, &own).await;
        let fresh =
            coach_turn_calling(&squad, &own, prescribe_for("Bea", "2026-12-04"), None).await;
        assert!(
            !fresh.contains("legacy-"),
            "premise: the new topic replays nothing from before the divider: {fresh}"
        );
        assert!(
            wrote_a_session(&squad, &fresh),
            "bug: the unknown member outlived the divider: {fresh}"
        );
    }

    /// carnet#1511 A8, still works: a legacy reply that names its one
    /// member is that member's (main's `sole`), and a legacy reply in a
    /// conversation where no legacy row names anyone else is the caller's
    /// own. Neither marks an unknown member: after a reply about Bea alone,
    /// a session for Bea is taken; in a thread whose only legacy reply names
    /// no one, so is a session for Bea.
    #[tokio::test]
    async fn a_legacy_reply_about_one_member_or_no_one_keeps_writes_onto_athletes() {
        let squad = squad().await;
        let named = own_conversation(&squad, "Legacy Bea").await;
        coach_legacy_reply(&squad, &named, Some(squad.bea.id), LEGACY_ABOUT_BEA).await;
        let after_named =
            coach_turn_calling(&squad, &named, prescribe_for("Bea", "2026-12-02"), None).await;
        assert!(
            after_named.contains(LEGACY_ABOUT_BEA)
                && results_of(&after_named, "prescribe_workout") == 1,
            "premise: the turn replays the reply about Bea and prescribes for her: {after_named}"
        );
        assert!(
            wrote_a_session(&squad, &after_named),
            "bug: a legacy reply naming Bea alone refused a session for Bea: {after_named}"
        );

        let unnamed = own_conversation(&squad, "Legacy own").await;
        coach_legacy_reply(
            &squad,
            &unnamed,
            None,
            "legacy-own: your week looks balanced",
        )
        .await;
        let after_unnamed =
            coach_turn_calling(&squad, &unnamed, prescribe_for("Bea", "2026-12-02"), None).await;
        assert!(
            after_unnamed.contains("legacy-own")
                && results_of(&after_unnamed, "prescribe_workout") == 1,
            "premise: the turn replays the unnamed reply and prescribes for Bea: {after_unnamed}"
        );
        assert!(
            wrote_a_session(&squad, &after_unnamed),
            "bug: a legacy reply in a thread that never named another member refused a session \
             for Bea: {after_unnamed}"
        );
    }
}
