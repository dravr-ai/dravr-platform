// ABOUTME: A coach's athlete thread and group-room seat read none of their rides and file nothing on them, save a room walk on themselves
// ABOUTME: Real turns against a scripted model; the coach's own conversation and the athlete in the room are the controls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1070. A human coach's thread bound to one athlete is the coach's
//! private workspace about that athlete. Every turn there is about the
//! athlete, in the coach's words, so nothing it says may be filed on the
//! coach's own account — where "her FTP is 250" would become the coach's own
//! FTP and reach every conversation that reads the coach's dossier — nor on
//! the athlete's, who never said it.
//!
//! Each test drives real turns through `pierre_chat_pipeline::execute`
//! against a model that records what every call carried, and reads the
//! repositories the writes would land in. The coach's own conversation with
//! the same agent is the control in each: there the same words are
//! extracted, the same advice captured, the same tool calls written and the
//! same rides prefetched, so their absence in the thread is the thread's
//! doing and not the fixture's.
//!
//! A guided walk interviews the person typing about their own training, so
//! every walk command is refused in the thread before it writes anything —
//! it would otherwise supersede the coach's earlier answers there and file
//! the new ones nowhere. The same commands still run in the coach's own
//! conversation.
//!
//! carnet#1084. In a group room the coach holds the coach's seat, and the same
//! holds there: what the coach says of an athlete is filed on nobody's
//! account, and neither the prefetches nor capability recovery read the
//! coach's own rides as the room's. The athlete speaking in the same room is
//! the control: her own message is still extracted, as hers. The one thing
//! filed from the coach's seat is a guided walk the coach runs on themselves
//! there: its state names the coach, and its answers are the coach's own.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{Datelike, Days, Duration, SecondsFormat, Utc};
use common::{
    create_test_server_resources_with_chat_provider_and_config, create_test_user_with_plan,
    generate_test_token, test_server_config, test_server_config_with_sciotte,
};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use helpers::axum_test::AxumTestRequest;
use helpers::sciotte_mock::{seed_sciotte_session, spawn_mock_scraper_serving};
use pierre_chat_pipeline::stages::capability_recovery::REPAIR_EVIDENCE_FRAME;
use pierre_chat_pipeline::stages::prefetch::{REFRESH_GROUNDING_LEAD, STARTUP_GROUNDING_LEAD};
use pierre_chat_pipeline::{
    ChatPipelineContext, CommandPersistence, InputSource, PipelineHooks, ServedTurn, SurfaceId,
    SurfaceProfile, SurfaceRequest, TurnEnvelope, TurnOrigin, TurnRequest,
};
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::messaging_strings::KEY_WALK_PLAN_THREAD;
use pierre_core::errors::AppError;
use pierre_core::models::agents::{ActivityDataRequirements, DataRequirements};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    AgentCategory, ConnectionType, ConversationTurnId, CreateAgentRequest, GuidedFlow,
    OnboardingState, Pillar, TenantId,
};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_database::RepositoryRegistry;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::time::{sleep, Instant};
use uuid::Uuid;

const MODEL: &str = "subject-writes-model";

/// What the coach says of the athlete in her thread.
const THREAD_FACT: &str = "Mira a maintenant un FTP de 250 W.";
/// What the coach says of themselves in their own conversation.
const OWN_FACT: &str = "Mon propre FTP est de 280 W.";
/// The fact the extractor reads out of each.
const THREAD_OBJECT: &str = "FTP 250 W";
const OWN_OBJECT: &str = "FTP 280 W";

/// A reply the advice gate takes for a recommendation.
const ADVICE_REPLY: &str =
    "Cette semaine, je te recommande deux sorties faciles de 45 minutes avant d'ajouter du dénivelé.";

/// The message on which the model writes through every memory tool at once.
const WRITE_ASK: &str = "Garde tout ça en mémoire pour la suite.";
const PLAIN_ASK: &str = "Comment s'est passée la semaine ?";
/// The coach asking their room about one athlete, by her roster name.
const ROOM_ASK_ABOUT_ATHLETE: &str = "Comment va Mira Subject cette semaine ?";
const REMEMBERED: &str = "seuil à 250 W";
const NOTE: &str = "genou sensible après les côtes";
const FOLLOWUP: &str = "demander comment va le genou";
const COMMITMENT: &str = "trois sorties faciles cette semaine";

/// The writes that land on the caller's own account, in the order the model
/// calls them.
const OWN_ACCOUNT_WRITES: [&str; 4] = [
    "remember_fact",
    "agent_note_add",
    "agent_followup_schedule",
    "commitment_create",
];

/// The ride the stand-in scraper serves as the coach's own.
const COACH_RIDE: &str = "Sortie vélo matinale";

/// The athlete's roster display name.
const ATHLETE_NAME: &str = "Mira Subject";
/// An answer the coach gave an earlier walk of their own.
const WALK_ANSWER: &str = "le marathon de Montréal en septembre";
/// Every command that opens a guided walk, aliases included, as the coach
/// types it, with the flow it opens.
const WALK_COMMANDS: [(&str, GuidedFlow); 6] = [
    ("/pillars full", GuidedFlow::Pillars),
    ("/onboarding", GuidedFlow::Pillars),
    ("/calibrate", GuidedFlow::Calibration),
    ("/season", GuidedFlow::Season),
    ("/fortnight", GuidedFlow::Fortnight),
    ("/next", GuidedFlow::Fortnight),
];

/// How long a detached learning pass is given to reach the model.
const PATIENCE: StdDuration = StdDuration::from_secs(10);
/// How long to keep watching for a pass that must NOT run, once the pass
/// spawned after it has already been seen.
const SETTLE: StdDuration = StdDuration::from_millis(500);

// ════════════════════════════════════════════════════════════════════════
// The model
// ════════════════════════════════════════════════════════════════════════

/// How the tool loop hands a tool's answer back to the model: a user message
/// opening with this.
const TOOL_RESULT_OPENING: &str = "[Tool Result for ";

/// What one model call carried.
#[derive(Debug, Clone)]
struct Call {
    /// Every system message, joined.
    system: String,
    /// Every user and assistant message that is not a tool's answer, joined.
    conversation: String,
    /// What the tools run so far answered, joined.
    tool_results: String,
    /// The tools declared to the call.
    declared: Vec<String>,
}

impl Call {
    fn carries(&self, text: &str) -> bool {
        self.system.contains(text)
            || self.conversation.contains(text)
            || self.tool_results.contains(text)
    }

    /// Where `text` sits in what the call carried, for a failure message.
    fn around(&self, text: &str) -> String {
        [&self.system, &self.conversation, &self.tool_results]
            .into_iter()
            .find_map(|carried| {
                let at = carried.find(text)?;
                let start = carried.floor_char_boundary(at.saturating_sub(600));
                let end = carried.ceil_char_boundary((at + 200).min(carried.len()));
                Some(carried[start..end].to_owned())
            })
            .unwrap_or_default()
    }

    fn declares(&self, tool: &str) -> bool {
        self.declared.iter().any(|name| name == tool)
    }
}

/// How the model tells its calls apart, and the agent its writes name.
#[derive(Debug, Clone, Default)]
struct Script {
    extraction_prompt: String,
    advice_prompt: String,
    agent_id: String,
}

/// Records every call. The fact extractor reads [`THREAD_OBJECT`] or
/// [`OWN_OBJECT`] out of the message that states it, the advice capture finds
/// nothing, and a coaching call answers [`ADVICE_REPLY`] — after calling every
/// memory write once, on [`WRITE_ASK`].
struct ScriptedModel {
    calls: Mutex<Vec<Call>>,
    script: Mutex<Script>,
    models: Vec<String>,
}

impl ScriptedModel {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            script: Mutex::new(Script::default()),
            models: vec![MODEL.to_owned()],
        }
    }

    fn know(&self, script: Script) {
        *self.script.lock().unwrap() = script;
    }

    fn calls_so_far(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn calls_since(&self, from: usize) -> Vec<Call> {
        self.calls.lock().unwrap()[from..].to_vec()
    }

    fn is_extraction(&self, call: &Call) -> bool {
        let script = self.script.lock().unwrap();
        !script.extraction_prompt.is_empty() && call.system.starts_with(&script.extraction_prompt)
    }

    fn is_advice_capture(&self, call: &Call) -> bool {
        let script = self.script.lock().unwrap();
        !script.advice_prompt.is_empty() && call.system == script.advice_prompt
    }

    /// The coaching calls made since `from`: every call that is not a
    /// background learning pass.
    fn coaching_since(&self, from: usize) -> Vec<Call> {
        self.calls_since(from)
            .into_iter()
            .filter(|call| !self.is_extraction(call) && !self.is_advice_capture(call))
            .collect()
    }

    /// The first call after `from` that `wanted` accepts, waiting up to
    /// [`PATIENCE`] for a detached task to make it.
    async fn call_matching(&self, from: usize, wanted: impl Fn(&Call) -> bool) -> Option<Call> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(found) = self.calls_since(from).into_iter().find(&wanted) {
                return Some(found);
            }
            if Instant::now() >= deadline {
                return None;
            }
            sleep(StdDuration::from_millis(20)).await;
        }
    }

    fn record(request: &ChatRequest) -> Call {
        let is_tool_result = |role: &MessageRole, content: &str| {
            matches!(role, MessageRole::Tool)
                || (matches!(role, MessageRole::User) && content.starts_with(TOOL_RESULT_OPENING))
        };
        let joined = |wanted: &dyn Fn(&MessageRole, &str) -> bool| {
            request
                .messages
                .iter()
                .filter(|m| wanted(&m.role, &m.content))
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        Call {
            system: joined(&|role, _| matches!(role, MessageRole::System)),
            conversation: joined(&|role, content| {
                matches!(role, MessageRole::User | MessageRole::Assistant)
                    && !is_tool_result(role, content)
            }),
            tool_results: joined(&is_tool_result),
            declared: request
                .tools
                .iter()
                .flatten()
                .map(|tool| tool.name.clone())
                .collect(),
        }
    }

    fn extracted(call: &Call) -> String {
        [(THREAD_FACT, THREAD_OBJECT), (OWN_FACT, OWN_OBJECT)]
            .into_iter()
            .find(|(said, _)| call.conversation.contains(said))
            .map_or_else(
                || "[]".to_owned(),
                |(_, object)| {
                    json!([{
                        "kind": "physiology",
                        "predicate_code": "have_baseline",
                        "object": object,
                        "confidence": 0.9,
                        "stated_by": "user",
                    }])
                    .to_string()
                },
            )
    }

    fn writes(agent_id: &str) -> Vec<ToolCallRequest> {
        let due = (Utc::now().date_naive() + Days::new(5))
            .format("%Y-%m-%d")
            .to_string();
        let arguments = [
            json!({
                "kind": "physiology",
                "predicate_code": "have_baseline",
                "object": REMEMBERED,
                "confidence": 0.9,
            }),
            json!({ "content": NOTE, "agent_id": agent_id }),
            json!({ "content": FOLLOWUP, "agent_id": agent_id }),
            json!({ "statement": COMMITMENT, "sessions": 3, "due_date": due, "agent_id": agent_id }),
        ];
        OWN_ACCOUNT_WRITES
            .into_iter()
            .zip(arguments)
            .map(|(name, arguments)| ToolCallRequest {
                id: format!("call-{name}"),
                function_name: name.to_owned(),
                arguments,
            })
            .collect()
    }
}

#[async_trait]
impl LlmProvider for ScriptedModel {
    fn name(&self) -> &'static str {
        "subject_writes_mock"
    }
    fn display_name(&self) -> &'static str {
        "Subject-thread writes mock"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        MODEL
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let call = Self::record(request);
        self.calls.lock().unwrap().push(call.clone());
        let (content, tool_calls) = if self.is_extraction(&call) {
            (Self::extracted(&call), None)
        } else if self.is_advice_capture(&call) {
            ("[]".to_owned(), None)
        } else {
            let asked_to_write = request
                .messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::User))
                .is_some_and(|m| m.content.contains(WRITE_ASK));
            if asked_to_write && call.tool_results.is_empty() {
                let agent_id = self.script.lock().unwrap().agent_id.clone();
                (String::new(), Some(Self::writes(&agent_id)))
            } else {
                (ADVICE_REPLY.to_owned(), None)
            }
        };
        Ok(ChatResponse {
            content,
            model: MODEL.to_owned(),
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

// ════════════════════════════════════════════════════════════════════════
// The fixture
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@subject-writes.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    let token = generate_test_token(res, &user).await;
    Person {
        id,
        tenant,
        auth: format!("Bearer {token}"),
    }
}

fn web_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

/// A group room the coach coaches ([`Fixture::room`]).
struct Room {
    group: Uuid,
    /// The coach's conversation in it.
    coach: String,
    /// The athlete's conversation in it.
    athlete: String,
}

/// A group whose agent declares an activity window, coached by the coach;
/// the athlete a consenting member. The coach's thread about her is open, and
/// so is the coach's own conversation with the same agent.
struct Fixture {
    res: Arc<ServerContext>,
    ctx: ChatPipelineContext,
    model: Arc<ScriptedModel>,
    coach: Person,
    subject: Person,
    agent_id: String,
    thread: String,
    own: String,
}

impl Fixture {
    async fn new() -> Self {
        Self::over(test_server_config()).await
    }

    /// The fixture with the server's scraper client pointed at `scraper`
    /// ([`stand_in_scraper`]).
    async fn with_scraper(scraper: &str) -> Self {
        Self::over(test_server_config_with_sciotte(scraper)).await
    }

    async fn over(config: ServerConfig) -> Self {
        let model = Arc::new(ScriptedModel::new());
        let res = create_test_server_resources_with_chat_provider_and_config(model.clone(), config)
            .await
            .unwrap();
        let owner = person(&res, "owner", "Olive Owner").await;
        let coach = person(&res, "coach", "Casey Coach").await;
        let subject = person(&res, "subject", ATHLETE_NAME).await;
        let repos = &res.common.repos;
        let agent_id = windowed_agent(repos, &owner).await;
        let group = coached_group(repos, &owner, &coach, &agent_id).await;
        join(repos, &group, &subject).await;

        let resp = AxumTestRequest::post(&format!("/api/chat/athletes/{}/thread", subject.id))
            .header("authorization", &coach.auth)
            .json(&json!({ "group_id": group.id }))
            .send(ChatRoutes::routes(Arc::clone(&res)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let thread = resp.json::<Value>()["id"].as_str().unwrap().to_owned();
        let own = repos
            .chat
            .create_conversation(
                &coach.id.to_string(),
                coach.tenant,
                "Mes séances",
                MODEL,
                Some(&agent_id),
                None,
            )
            .await
            .unwrap()
            .id;

        let ctx = res.chat_pipeline_context();
        model.know(Script {
            extraction_prompt: ctx.memory_extraction_prompt.clone(),
            advice_prompt: ctx
                .prompt_registry
                .advice_extraction_prompt()
                .trim()
                .to_owned(),
            agent_id: agent_id.clone(),
        });
        Self {
            res,
            ctx,
            model,
            coach,
            subject,
            agent_id,
            thread,
            own,
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// What `who` typing `content` in `conversation` is served, on the in-app
    /// web surface. `direct` is false in a group's room.
    async fn serve_as(
        &self,
        who: &Person,
        conversation: &str,
        content: &str,
        direct: bool,
    ) -> ServedTurn {
        pierre_chat_pipeline::execute(
            &self.ctx,
            TurnRequest {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                conversation_id: conversation.to_owned(),
                user_id: who.id,
                conversation_tenant_id: who.tenant,
                tool_tenant_id: who.tenant,
                content: content.to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                channel_type: "web",
                transport: Transport::WebApp,
                is_direct_message: direct,
                ambient_group_fallback: false,
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                hooks: PipelineHooks::none(),
            },
            &web_profile(),
        )
        .await
        .expect("the turn is served")
    }

    /// What the coach typing `content` in a direct `conversation` is served.
    async fn serve(&self, conversation: &str, content: &str) -> ServedTurn {
        self.serve_as(&self.coach, conversation, content, true)
            .await
    }

    /// One coach turn in `conversation`: the envelope, and the index of the
    /// first model call the turn made.
    async fn turn(&self, conversation: &str, content: &str) -> (TurnEnvelope, usize) {
        let from = self.model.calls_so_far();
        let ServedTurn::Pipeline(envelope) = self.serve(conversation, content).await else {
            panic!("a coaching turn, not a command");
        };
        (*envelope, from)
    }

    /// One turn by `who` in the group room `conversation`: the envelope, and
    /// the index of the first model call the turn made.
    async fn room_turn(
        &self,
        who: &Person,
        conversation: &str,
        content: &str,
    ) -> (TurnEnvelope, usize) {
        let from = self.model.calls_so_far();
        let ServedTurn::Pipeline(envelope) = self.serve_as(who, conversation, content, false).await
        else {
            panic!("a coaching turn, not a command");
        };
        (*envelope, from)
    }

    /// A group the coach owns and coaches in their own tenant, the athlete a
    /// consenting member with no training source, and each one's conversation
    /// in its room.
    async fn room(&self) -> Room {
        let repos = self.repos();
        let now = Utc::now();
        let group = repos
            .groups
            .create_group(
                self.coach.tenant,
                &CoachingGroup {
                    id: Uuid::new_v4(),
                    tenant_id: self.coach.tenant.to_string(),
                    name: "Tempo Room".to_owned(),
                    description: None,
                    agent_id: self.agent_id.clone(),
                    owner_id: self.coach.id,
                    coach_user_id: Some(self.coach.id),
                    peer_data_sharing: false,
                    respond_mode: GroupRespondMode::default(),
                    digest_mode: GroupDigestMode::Off,
                    max_members: 20,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        for (who, role) in [
            (&self.coach, GroupRole::Owner),
            (&self.subject, GroupRole::Member),
        ] {
            repos
                .groups
                .add_member(&GroupMember {
                    id: Uuid::new_v4(),
                    group_id: group.id,
                    user_id: who.id,
                    tenant_id: self.coach.tenant.to_string(),
                    role,
                    peer_sharing_consent: false,
                    coach_sharing_consent: true,
                    coach_consents: CoachConsents::default(),
                    consent_given_at: now,
                    joined_at: now,
                    left_at: None,
                    display_name: None,
                })
                .await
                .unwrap();
        }
        Room {
            group: group.id,
            coach: self.room_conversation(&self.coach, group.id).await,
            athlete: self.room_conversation(&self.subject, group.id).await,
        }
    }

    /// A conversation of `who`'s in the room of `group`, opened the way the
    /// in-app clients open one.
    async fn room_conversation(&self, who: &Person, group: Uuid) -> String {
        let resp = AxumTestRequest::post("/api/chat/conversations")
            .header("authorization", &who.auth)
            .json(&json!({ "title": "Tempo Room", "group_id": group.to_string() }))
            .send(ChatRoutes::routes(Arc::clone(&self.res)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::CREATED);
        resp.json::<Value>()["id"].as_str().unwrap().to_owned()
    }

    /// Connect the coach's own rides, which the stand-in scraper serves
    /// ([`stand_in_scraper`]).
    async fn coach_rides(&self) {
        self.repos()
            .provider_connections
            .register_connection(
                self.coach.id,
                self.coach.tenant,
                "sciotte",
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
        seed_sciotte_session(&self.res, self.coach.id, self.coach.tenant).await;
    }

    /// The reply the coach reads to the slash command `typed`.
    async fn command(&self, conversation: &str, typed: &str) -> String {
        let ServedTurn::Command { command, .. } = self.serve(conversation, typed).await else {
            panic!("{typed} ran as a coaching turn, not a command");
        };
        command.text
    }

    /// A fresh conversation of the coach's own with the group's agent.
    async fn own_conversation(&self) -> String {
        self.repos()
            .chat
            .create_conversation(
                &self.coach.id.to_string(),
                self.coach.tenant,
                "Mon profil",
                MODEL,
                Some(&self.agent_id),
                None,
            )
            .await
            .unwrap()
            .id
    }

    /// The guided walk `conversation` carries, if any.
    async fn walk_state(&self, conversation: &str) -> Option<OnboardingState> {
        let row = self
            .repos()
            .chat
            .get_conversation(conversation, &self.coach.id.to_string(), self.coach.tenant)
            .await
            .unwrap()
            .expect("the conversation exists");
        OnboardingState::from_column(row.onboarding_state.as_deref())
    }

    /// The coach's earlier walk answer, as a pillar walk files it.
    async fn seed_walk_answer(&self) {
        self.repos()
            .memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id: self.coach.tenant,
                user_id: &self.coach.id.to_string(),
                agent_id: None,
                scope: MemoryScope::User,
                kind: FactKind::Goal,
                pillar: Some(Pillar::TrainingAndMovement),
                predicate_code: PredicateCode::TrainingFor,
                object: WALK_ANSWER,
                confidence: 0.9,
                source: FactSource::Onboarding,
                valid_until: None,
                source_msg_id: None,
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();
    }

    /// The coach's walk answers still in force, by object.
    async fn walk_answers(&self) -> Vec<String> {
        self.repos()
            .memory
            .list_user_facts_by_source(
                self.coach.tenant,
                &self.coach.id.to_string(),
                FactSource::Onboarding,
                50,
                TransportPolicy::FirstPartyOnly,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|fact| fact.object)
            .collect()
    }

    /// A season of the coach's own with a phase and no week written, so
    /// `/fortnight` has two weeks to write.
    async fn coach_season_with_a_phase(&self) {
        let today = Utc::now().date_naive();
        let monday = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
        let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&self.res);
        let ctx = ToolContext::new()
            .with_user(self.coach.id.to_string())
            .with_tenant(self.coach.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let saved = SaveTrainingPlanTool
            .execute(
                &runtime,
                &ctx,
                json!({
                    "agent_id": self.agent_id,
                    "outline": {
                        "goal_race": {
                            "name": "Coach Own Race",
                            "date": "2027-05-16",
                            "discipline": "trail",
                            "priority": "A",
                        },
                        "strategy": "build toward the race",
                        "phases": [{
                            "kind": "build",
                            "start": monday.format("%Y-%m-%d").to_string(),
                            "weeks": 8,
                            "intent": "two hard days",
                        }],
                    },
                }),
            )
            .await
            .structured_content
            .expect("the save answers");
        assert!(saved.get("error").is_none(), "{saved}");
    }

    /// The facts `who` has on file, by object.
    /// Kept or waiting on the athlete alike: the question is whose account a
    /// fact is filed on, not whether they have approved it yet.
    async fn facts(&self, who: &Person) -> Vec<String> {
        let memory = &self.repos().memory;
        let user = who.id.to_string();
        let kept = memory
            .list_user_facts(
                who.tenant,
                &user,
                None,
                None,
                50,
                TransportPolicy::FirstPartyOnly,
            )
            .await
            .unwrap();
        let drafts = memory
            .list_draft_user_facts(who.tenant, &user, 50, TransportPolicy::FirstPartyOnly)
            .await
            .unwrap();
        kept.into_iter()
            .chain(drafts)
            .map(|fact| fact.object)
            .collect()
    }

    /// Everything `who` has on file through the memory tools: their notes,
    /// pending follow-ups and open commitments with the group's agent.
    async fn tool_written(&self, who: &Person) -> Vec<String> {
        let repos = self.repos();
        let user = who.id.to_string();
        let notes = repos
            .memory
            .list_agent_notes(
                who.tenant,
                &user,
                &self.agent_id,
                50,
                TransportPolicy::FirstPartyOnly,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|note| note.content);
        let followups = repos
            .memory
            .list_pending_followups(who.tenant, &user, &self.agent_id)
            .await
            .unwrap()
            .into_iter()
            .map(|followup| followup.content);
        let commitments = repos
            .commitments
            .list_open_commitments(
                &who.tenant.to_string(),
                &user,
                50,
                TransportPolicy::FirstPartyOnly,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|commitment| commitment.statement);
        notes.chain(followups).chain(commitments).collect()
    }

    /// Wait for `object` to land among `who`'s facts.
    async fn fact_lands(&self, who: &Person, object: &str) {
        let deadline = Instant::now() + PATIENCE;
        while !self.facts(who).await.iter().any(|o| o == object) {
            assert!(
                Instant::now() < deadline,
                "{object:?} never reached their facts"
            );
            sleep(StdDuration::from_millis(20)).await;
        }
    }
}

/// A local stand-in scraper that serves one ride from yesterday to whichever
/// session asks, and the URL the server's scraper client is built to reach it
/// at.
async fn stand_in_scraper() -> String {
    let ride_start = (Utc::now() - Duration::hours(20)).to_rfc3339_opts(SecondsFormat::Secs, true);
    spawn_mock_scraper_serving(Arc::new(AtomicBool::new(true)), ride_start).await
}

/// The agent the group runs: it declares an activity window, so both
/// activity prefetches serve whoever its turns are about.
async fn windowed_agent(repos: &RepositoryRegistry, owner: &Person) -> String {
    let agent = repos
        .agents
        .create(
            owner.id,
            owner.tenant,
            &CreateAgentRequest {
                title: "Tempo Agent".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
                category: AgentCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: Some(DataRequirements {
                    activities: Some(ActivityDataRequirements {
                        count: 10,
                        time_frame: Some("4w".to_owned()),
                        mode: ActivityDataRequirements::default_mode(),
                        format: ActivityDataRequirements::default_format(),
                        analysis_type: ActivityDataRequirements::default_analysis_type(),
                    }),
                    athlete_profile: false,
                }),
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap();
    agent.id.to_string()
}

async fn coached_group(
    repos: &RepositoryRegistry,
    owner: &Person,
    coach: &Person,
    agent_id: &str,
) -> CoachingGroup {
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: "Tempo Squad".to_owned(),
                description: None,
                agent_id: agent_id.to_owned(),
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    assert!(repos
        .groups
        .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
        .await
        .unwrap());
    group
}

async fn join(repos: &RepositoryRegistry, group: &CoachingGroup, who: &Person) {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: who.id,
            tenant_id: who.tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

// ════════════════════════════════════════════════════════════════════════
// Background learning
// ════════════════════════════════════════════════════════════════════════

/// What the coach says of the athlete in her thread is extracted as nobody's
/// fact and captured as nobody's advice, while what they say of themselves in
/// their own conversation still is.
#[tokio::test]
async fn what_the_coach_says_of_the_athlete_in_her_thread_is_learned_by_nobody() {
    let fx = Fixture::new().await;
    let from = fx.model.calls_so_far();

    fx.turn(&fx.thread, THREAD_FACT).await;
    fx.turn(&fx.own, OWN_FACT).await;

    // The control: the coach's own conversation extracts and captures as
    // before, through this same model and fixture.
    fx.model
        .call_matching(from, |c| {
            fx.model.is_extraction(c) && c.conversation.contains(OWN_FACT)
        })
        .await
        .expect("the coach's own conversation is still extracted");
    fx.model
        .call_matching(from, |c| {
            fx.model.is_advice_capture(c) && c.conversation.contains(OWN_FACT)
        })
        .await
        .expect("the advice in the coach's own conversation is still captured");
    fx.fact_lands(&fx.coach, OWN_OBJECT).await;
    // The thread's passes were spawned before the control's: by now they
    // would have reached the model.
    sleep(SETTLE).await;

    let learned_from_thread: Vec<Call> = fx
        .model
        .calls_since(from)
        .into_iter()
        .filter(|c| fx.model.is_extraction(c) || fx.model.is_advice_capture(c))
        .filter(|c| c.conversation.contains(THREAD_FACT))
        .collect();
    assert!(
        learned_from_thread.is_empty(),
        "a learning pass read the athlete thread: {learned_from_thread:?}"
    );
    assert_eq!(
        fx.facts(&fx.coach).await,
        vec![OWN_OBJECT.to_owned()],
        "the coach's facts are their own, and only their own"
    );
    assert!(
        fx.facts(&fx.subject).await.is_empty(),
        "what the coach said of the athlete never enters her memory"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Memory tools
// ════════════════════════════════════════════════════════════════════════

/// In the athlete thread no memory write is declared to the model, and one it
/// calls anyway is refused: no fact, note, follow-up or commitment lands on
/// the coach or the athlete. In the coach's own conversation the same calls
/// are offered and written.
#[tokio::test]
async fn a_write_on_the_coachs_own_account_is_neither_offered_nor_run_in_her_thread() {
    let fx = Fixture::new().await;

    let (_, from) = fx.turn(&fx.thread, WRITE_ASK).await;
    let coaching = fx.model.coaching_since(from);
    let asked = coaching.first().expect("the thread's coaching call");
    for write in OWN_ACCOUNT_WRITES {
        assert!(
            !asked.declares(write),
            "{write} was offered in the athlete thread"
        );
    }
    assert!(
        asked.declares("save_training_plan"),
        "the plan tools act on the athlete and stay offered"
    );
    let answered = coaching
        .iter()
        .find(|c| !c.tool_results.is_empty())
        .expect("the model read what its calls answered");
    for write in OWN_ACCOUNT_WRITES {
        let refusal =
            format!("'{write}' is not available here: this thread is the coach's workspace");
        assert!(
            answered.tool_results.contains(&refusal),
            "{write} was not refused: {}",
            answered.tool_results
        );
    }
    for who in [&fx.coach, &fx.subject] {
        assert!(fx.facts(who).await.is_empty(), "a fact was filed");
        assert!(
            fx.tool_written(who).await.is_empty(),
            "a note, follow-up or commitment was filed"
        );
    }

    // The control: the same calls in the coach's own conversation are
    // offered, and each one writes.
    let (_, from) = fx.turn(&fx.own, WRITE_ASK).await;
    let asked = fx
        .model
        .coaching_since(from)
        .into_iter()
        .next()
        .expect("the coach's own coaching call");
    for write in OWN_ACCOUNT_WRITES {
        assert!(asked.declares(write), "{write} is the coach's own to call");
    }
    assert_eq!(fx.facts(&fx.coach).await, vec![REMEMBERED.to_owned()]);
    let mut written = fx.tool_written(&fx.coach).await;
    written.sort();
    let mut expected = vec![NOTE.to_owned(), FOLLOWUP.to_owned(), COMMITMENT.to_owned()];
    expected.sort();
    assert_eq!(written, expected);
}

// ════════════════════════════════════════════════════════════════════════
// Activity prefetch
// ════════════════════════════════════════════════════════════════════════

/// Neither activity prefetch puts the coach's own rides in front of the model
/// in the athlete thread — not on its first turn (stage 10) nor a later one
/// (stage 12b) — while the coach's own conversation with the same agent is
/// grounded on them by both.
#[tokio::test]
async fn the_athlete_thread_prefetches_none_of_the_coachs_rides() {
    let fx = Fixture::with_scraper(&stand_in_scraper().await).await;
    fx.coach_rides().await;

    // The control: the coach's own conversation with the same agent is
    // grounded on the coach's ride, on its first turn and on the next.
    for lead in [STARTUP_GROUNDING_LEAD, REFRESH_GROUNDING_LEAD] {
        let (envelope, from) = fx.turn(&fx.own, PLAIN_ASK).await;
        assert!(
            envelope.telemetry.activities_prefetched,
            "the fixture serves the coach's rides to their own conversation ({lead:?})"
        );
        assert!(
            fx.model
                .coaching_since(from)
                .iter()
                .any(|c| c.carries(lead) && c.carries(COACH_RIDE)),
            "the prefetch put the coach's ride in front of the model ({lead:?})"
        );
    }

    for turn in ["first", "later"] {
        let (envelope, from) = fx.turn(&fx.thread, PLAIN_ASK).await;
        assert!(
            !envelope.telemetry.activities_prefetched,
            "the {turn} turn of the athlete thread was prefetched"
        );
        for call in fx.model.coaching_since(from) {
            for absent in [STARTUP_GROUNDING_LEAD, REFRESH_GROUNDING_LEAD, COACH_RIDE] {
                assert!(
                    !call.carries(absent),
                    "{absent:?} reached the {turn} turn of the athlete thread: …{}…",
                    call.around(absent)
                );
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
// Guided walks
// ════════════════════════════════════════════════════════════════════════

/// Every walk command, aliases included, is refused in the athlete thread
/// with the athlete's name and the command as typed, in the coach's locale —
/// and nothing is written: no walk state, the coach's earlier answer still in
/// force, their profile untouched.
///
/// The coach has a season with a phase and an earlier walk answer, so each
/// command would write if it ran: `/pillars full` supersedes the answer,
/// `/calibrate` and `/season` stamp the profile and open their walk, and
/// `/fortnight` opens its rail.
#[tokio::test]
async fn a_guided_walk_typed_in_the_athlete_thread_is_refused_before_anything_is_written() {
    let fx = Fixture::new().await;
    fx.seed_walk_answer().await;
    fx.coach_season_with_a_phase().await;
    let profile_before = fx.repos().profiles.get_profile(fx.coach.id).await.unwrap();
    let locale = web_profile().locale;

    for (typed, _) in WALK_COMMANDS {
        let command = typed.split_whitespace().next().unwrap();
        let reply = fx.command(&fx.thread, typed).await;

        assert_eq!(
            reply,
            fx.ctx.messaging_strings_registry.render(
                KEY_WALK_PLAN_THREAD,
                &locale,
                &[ATHLETE_NAME, command]
            ),
            "{typed} was not refused"
        );
        assert!(
            reply.contains(ATHLETE_NAME) && reply.contains(&format!("`{command}`")),
            "the refusal names the athlete and the command as typed: {reply}"
        );
        assert!(
            fx.walk_state(&fx.thread).await.is_none(),
            "{typed} opened a walk in the athlete thread"
        );
    }
    assert_eq!(
        fx.walk_answers().await,
        vec![WALK_ANSWER.to_owned()],
        "the coach's earlier walk answer was superseded"
    );
    assert_eq!(
        fx.repos().profiles.get_profile(fx.coach.id).await.unwrap(),
        profile_before,
        "a refused walk stamped the coach's profile"
    );
}

/// The control: each walk command typed in a conversation of the coach's own
/// still runs — it opens its walk there, and `/pillars full` supersedes the
/// coach's earlier answer — so the thread's refusal is the thread's doing.
#[tokio::test]
async fn the_same_walk_in_the_coachs_own_conversation_still_starts() {
    let fx = Fixture::new().await;
    fx.seed_walk_answer().await;
    fx.coach_season_with_a_phase().await;
    let locale = web_profile().locale;

    for (typed, flow) in WALK_COMMANDS {
        let command = typed.split_whitespace().next().unwrap();
        let own = fx.own_conversation().await;
        let reply = fx.command(&own, typed).await;

        assert_ne!(
            reply,
            fx.ctx.messaging_strings_registry.render(
                KEY_WALK_PLAN_THREAD,
                &locale,
                &[ATHLETE_NAME, command]
            ),
            "{typed} was refused in the coach's own conversation"
        );
        let state = fx.walk_state(&own).await.unwrap_or_else(|| {
            panic!("{typed} opened no walk in the coach's own conversation: {reply}")
        });
        assert_eq!(state.flow, flow, "{typed}");
        if typed == "/pillars full" {
            assert!(
                fx.walk_answers().await.is_empty(),
                "/pillars full supersedes the coach's earlier answer"
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
// The coach's seat in a group room
// ════════════════════════════════════════════════════════════════════════

/// In their group room the coach is not the turn's subject, so what they say
/// of an athlete there is extracted as nobody's fact and captured as nobody's
/// advice (carnet#1084). The athlete's own message in the same room is the
/// control: it is still extracted, and filed as hers.
#[tokio::test]
async fn what_the_coach_says_in_their_group_room_is_learned_by_nobody() {
    let fx = Fixture::new().await;
    let room = fx.room().await;
    let from = fx.model.calls_so_far();

    fx.room_turn(&fx.coach, &room.coach, THREAD_FACT).await;
    fx.room_turn(&fx.subject, &room.athlete, OWN_FACT).await;

    fx.model
        .call_matching(from, |c| {
            fx.model.is_extraction(c) && c.conversation.contains(OWN_FACT)
        })
        .await
        .expect("the athlete's own message in the room is still extracted");
    fx.fact_lands(&fx.subject, OWN_OBJECT).await;
    // The coach's passes were spawned before the athlete's: by now they would
    // have reached the model.
    sleep(SETTLE).await;

    let learned_from_coach: Vec<Call> = fx
        .model
        .calls_since(from)
        .into_iter()
        .filter(|c| fx.model.is_extraction(c) || fx.model.is_advice_capture(c))
        .filter(|c| c.conversation.contains(THREAD_FACT))
        .collect();
    assert!(
        learned_from_coach.is_empty(),
        "a learning pass read the coach's room message: {learned_from_coach:?}"
    );
    assert!(
        fx.facts(&fx.coach).await.is_empty(),
        "nothing said from the coach's seat is filed on the coach"
    );
    assert_eq!(
        fx.facts(&fx.subject).await,
        vec![OWN_OBJECT.to_owned()],
        "the athlete holds her own fact, and nothing the coach said of her"
    );
}

/// Neither the activity prefetches nor capability recovery read the coach's
/// own rides for their group room, on its first turn or a later one. A "how
/// was the week" turn there looks ungrounded, and was re-asked with the
/// coach's rides as the room's; it now stands as written. The coach's own
/// conversation is the control that the rides are served at all.
#[tokio::test]
async fn the_coach_seat_never_reads_the_coachs_own_rides_for_the_room() {
    let fx = Fixture::with_scraper(&stand_in_scraper().await).await;
    fx.coach_rides().await;
    let coach_room = fx.room().await.coach;

    let (own, _) = fx.turn(&fx.own, PLAIN_ASK).await;
    assert!(
        own.telemetry.activities_prefetched,
        "the fixture serves the coach's rides to their own conversation"
    );

    for turn in ["first", "later"] {
        let (envelope, from) = fx.room_turn(&fx.coach, &coach_room, PLAIN_ASK).await;
        assert!(
            !envelope.telemetry.activities_prefetched,
            "the {turn} room turn was prefetched with the coach's rides"
        );
        assert!(
            !envelope
                .telemetry
                .tools_called
                .iter()
                .any(|tool| tool == "get_activities"),
            "the {turn} room turn read the coach's own activities: {:?}",
            envelope.telemetry.tools_called
        );
        for call in fx.model.calls_since(from) {
            assert!(
                !call.carries(COACH_RIDE),
                "the coach's ride reached the {turn} room turn: …{}…",
                call.around(COACH_RIDE)
            );
        }
    }
}

/// When the coach asks their room about one athlete, the recovery re-ask
/// grounds on her side alone and tells the model the coach's own activities
/// are not read in the group — never that their data source failed, and
/// never with the coach's rides as evidence.
#[tokio::test]
async fn a_coach_seat_re_ask_about_an_athlete_never_reads_the_coachs_side() {
    let fx = Fixture::with_scraper(&stand_in_scraper().await).await;
    fx.coach_rides().await;
    let coach_room = fx.room().await.coach;

    let (_, from) = fx
        .room_turn(&fx.coach, &coach_room, ROOM_ASK_ABOUT_ATHLETE)
        .await;

    let calls = fx.model.calls_since(from);
    let reask = calls
        .iter()
        .find(|c| c.carries(REPAIR_EVIDENCE_FRAME))
        .expect("the room ask about one athlete is re-asked with her side");
    assert!(
        reask.carries("coaches this group, and their own activities are never read here"),
        "the re-ask says whose data is not read: …{}…",
        reask.around(REPAIR_EVIDENCE_FRAME)
    );
    assert!(
        !reask.carries("did not respond"),
        "never an outage that did not happen: …{}…",
        reask.around("did not respond")
    );
    for call in &calls {
        assert!(
            !call.carries(COACH_RIDE),
            "the coach's ride reached the room: …{}…",
            call.around(COACH_RIDE)
        );
    }
}

/// A guided walk the coach starts on themselves in their room interviews the
/// coach about the coach, so its answers are filed on the coach exactly as
/// every walk files them: the exemption follows the walk state, which names
/// the coach as its subject. An ordinary message the coach sends in the same
/// room right after, outside that walk, still files nothing; the athlete's own
/// message, sent last, is the control that extraction still ran.
#[tokio::test]
async fn a_walk_the_coach_runs_on_themselves_in_their_room_files_its_answers_on_the_coach() {
    let fx = Fixture::new().await;
    let room = fx.room().await;

    let opener = fx
        .serve_as(&fx.coach, &room.coach, "/pillars training", false)
        .await;
    assert!(
        matches!(opener, ServedTurn::Command { .. }),
        "/pillars runs as a command"
    );
    let walk = fx
        .walk_state(&room.coach)
        .await
        .expect("the walk opened in the coach's room conversation");
    assert_eq!(walk.flow, GuidedFlow::Pillars);
    assert_eq!(
        walk.subject_user_id.as_deref(),
        Some(fx.coach.id.to_string().as_str()),
        "the walk is bound to the coach"
    );

    // The coach answers their own walk: the answer is theirs, filed on them.
    let from = fx.model.calls_so_far();
    fx.room_turn(&fx.coach, &room.coach, OWN_FACT).await;
    fx.model
        .call_matching(from, |c| {
            fx.model.is_extraction(c) && c.conversation.contains(OWN_FACT)
        })
        .await
        .expect("the coach's walk answer is extracted");
    fx.fact_lands(&fx.coach, OWN_OBJECT).await;

    // Right after, in the same room but outside the walk, an ordinary message.
    let ordinary = fx.room_conversation(&fx.coach, room.group).await;
    let after_walk = fx.model.calls_so_far();
    fx.room_turn(&fx.coach, &ordinary, THREAD_FACT).await;
    fx.room_turn(&fx.subject, &room.athlete, OWN_FACT).await;
    fx.fact_lands(&fx.subject, OWN_OBJECT).await;
    // The coach's ordinary message was spawned before the athlete's: by now
    // its passes would have reached the model.
    sleep(SETTLE).await;

    let learned_from_room: Vec<Call> = fx
        .model
        .calls_since(after_walk)
        .into_iter()
        .filter(|c| fx.model.is_extraction(c) || fx.model.is_advice_capture(c))
        .filter(|c| c.conversation.contains(THREAD_FACT))
        .collect();
    assert!(
        learned_from_room.is_empty(),
        "a learning pass read the coach's ordinary room message: {learned_from_room:?}"
    );
    assert_eq!(
        fx.facts(&fx.coach).await,
        vec![OWN_OBJECT.to_owned()],
        "the coach holds their walk answer, and nothing they said of the room"
    );
    assert_eq!(fx.facts(&fx.subject).await, vec![OWN_OBJECT.to_owned()]);
}
