// ABOUTME: Content derived from Garmin or WHOOP data names their AI consent: a withdrawal keeps it out of every later prompt
// ABOUTME: And Garmin-derived content never becomes a memory note or a summary's figures (carnet#1157 gap 1, carnet#1158)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin's agreement v9 §15.11 deems prompts and logs derived from Garmin
//! data to be Garmin data, and §15.11(c) bars building an AI knowledge base
//! from it. So a reply, a memory note or a compaction summary drawn from a
//! provider's data names that provider's AI consent, and:
//!
//! - once the athlete withdraws the consent, no model reads a row naming it,
//!   an earlier reply in the thread included (carnet#1157 gap 1); the athlete
//!   still reads every row in the app;
//! - Garmin-derived content stays out of the agent's memory: a turn that
//!   served Garmin data files no fact, a fact the agent writes in one is
//!   never read back by a model, and a compaction summary carries none of a
//!   Garmin row's figures (carnet#1158, until Garmin authorizes it in
//!   writing).
//!
//! Turns run through `pierre_chat_pipeline::execute`, the entry every surface
//! calls, against a model that records what every call carried.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use axum::http::StatusCode;
use dravr_tronc::dates::{Elapsed, Timestamp};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages;
use pierre_chat_pipeline::{
    ChatPipelineContext, CommandPersistence, InputSource, PipelineHooks, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_core::config::CompactionConfig;
use pierre_core::constants::oauth_providers::{NoticeKind, GARMIN, WHOOP};
use pierre_core::errors::AppError;
use pierre_core::models::{
    ActivityBuilder, AddMessageParams, ConnectionType, ConversationTurnId, MessageRecord,
    SportType, TenantId, User,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_llm::prompts::CONVERSATION_SUMMARY_PROMPT;
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
    StreamChunk,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::conversation_compaction::{
    CompactionContext, CompactionOutcome, ConversationCompactor,
};
use pierre_services::provider_notice::{grant_consent, withdraw_consent};
use serde_json::{json, Value};
use tokio::time::{sleep, Instant};
use uuid::Uuid;

use crate::common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::garmin_consent::give_garmin_ai_consent;

const MODEL: &str = "derived-consent-model";

/// The figure the Garmin ride carries, and every reply quotes.
const GARMIN_FIGURE: &str = "187 km";
/// What the agent answers: it quotes the ride it read.
const REPLY: &str = "Ta sortie de 187 km domine ta semaine ; on garde le tempo pour jeudi.";
/// The athlete's question, which reads their week.
const READ_ASK: &str = "Comment s'est passée ma semaine ?";
/// The athlete's question, which reads their week and asks the agent to
/// remember it.
const REMEMBER_ASK: &str = "Lis ma semaine et garde-la en mémoire.";
/// A question that reads nothing.
const PLAIN_ASK: &str = "Que fais-je demain ?";
/// What the agent remembers in a Garmin turn.
const REMEMBERED: &str = "sortie longue de 187 km";
/// An earlier reply drawn from no consent-gated data: always replayed.
const OPEN_FIGURE: &str = "42 km";

/// How long a detached learning pass is given to reach the model.
const PATIENCE: StdDuration = StdDuration::from_secs(10);
/// How long to keep watching for a pass that must NOT run.
const SETTLE: StdDuration = StdDuration::from_millis(700);

/// How the tool loop hands a tool's answer back to the model.
const TOOL_RESULT_OPENING: &str = "[Tool Result for ";

// ════════════════════════════════════════════════════════════════════════
// The model
// ════════════════════════════════════════════════════════════════════════

/// What one model call carried.
#[derive(Debug, Clone)]
struct Call {
    /// Every system message, joined.
    system: String,
    /// Every user and assistant message that is not a tool's answer, joined.
    conversation: String,
    /// How many tool answers the call carried.
    tool_results: usize,
    /// The last user message that is not a tool's answer.
    asked: String,
}

impl Call {
    fn carries(&self, text: &str) -> bool {
        self.system.contains(text) || self.conversation.contains(text)
    }
}

/// Records every call. A coaching call reads the athlete's activities when
/// asked about the week, then remembers them when asked to, then answers
/// [`REPLY`]. The fact extractor files the figure the turn quoted; the advice
/// capture finds nothing; the summarizer answers with what it was handed.
struct ScriptedModel {
    calls: Mutex<Vec<Call>>,
    extraction_prompt: Mutex<String>,
    advice_prompt: Mutex<String>,
    models: Vec<String>,
}

impl ScriptedModel {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            extraction_prompt: Mutex::new(String::new()),
            advice_prompt: Mutex::new(String::new()),
            models: vec![MODEL.to_owned()],
        }
    }

    fn know(&self, ctx: &ChatPipelineContext) {
        self.extraction_prompt
            .lock()
            .unwrap()
            .clone_from(&ctx.memory_extraction_prompt);
        ctx.prompt_registry
            .advice_extraction_prompt()
            .trim()
            .clone_into(&mut self.advice_prompt.lock().unwrap());
    }

    fn calls_so_far(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn calls_since(&self, from: usize) -> Vec<Call> {
        self.calls.lock().unwrap()[from..].to_vec()
    }

    fn is_extraction(&self, call: &Call) -> bool {
        let prompt = self.extraction_prompt.lock().unwrap();
        !prompt.is_empty() && call.system.starts_with(prompt.as_str())
    }

    fn is_advice_capture(&self, call: &Call) -> bool {
        let prompt = self.advice_prompt.lock().unwrap();
        !prompt.is_empty() && call.system == *prompt
    }

    /// The coaching calls since `from`: every call that is not a background
    /// learning pass.
    fn coaching_since(&self, from: usize) -> Vec<Call> {
        self.calls_since(from)
            .into_iter()
            .filter(|call| !self.is_extraction(call) && !self.is_advice_capture(call))
            .collect()
    }

    /// The first extraction call after `from`, waiting up to `patience`.
    async fn extraction_since(&self, from: usize, patience: StdDuration) -> Option<Call> {
        let deadline = Instant::now() + patience;
        loop {
            if let Some(found) = self
                .calls_since(from)
                .into_iter()
                .find(|call| self.is_extraction(call))
            {
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
            tool_results: request
                .messages
                .iter()
                .filter(|m| is_tool_result(&m.role, &m.content))
                .count(),
            asked: request
                .messages
                .iter()
                .rev()
                .find(|m| {
                    matches!(m.role, MessageRole::User) && !is_tool_result(&m.role, &m.content)
                })
                .map_or_else(String::new, |m| m.content.clone()),
        }
    }

    /// The facts the extractor files: the quoted figure, when the turn
    /// quoted one.
    fn extracted(call: &Call) -> String {
        if call.conversation.contains(GARMIN_FIGURE) {
            json!([{
                "kind": "physiology",
                "predicate_code": "have_baseline",
                "object": REMEMBERED,
                "confidence": 0.9,
                "stated_by": "user",
            }])
            .to_string()
        } else {
            "[]".to_owned()
        }
    }

    fn coaching(call: &Call) -> (String, Option<Vec<ToolCallRequest>>) {
        let reads = call.asked.contains(READ_ASK) || call.asked.contains(REMEMBER_ASK);
        let call_tool = |name: &str, arguments: Value| {
            Some(vec![ToolCallRequest {
                id: format!("call-{name}"),
                function_name: name.to_owned(),
                arguments,
            }])
        };
        match call.tool_results {
            0 if reads => (
                String::new(),
                call_tool("get_activities", json!({ "limit": 10, "mode": "summary" })),
            ),
            1 if call.asked.contains(REMEMBER_ASK) => (
                String::new(),
                call_tool(
                    "remember_fact",
                    json!({
                        "kind": "physiology",
                        "predicate_code": "have_baseline",
                        "object": REMEMBERED,
                        "confidence": 0.9,
                    }),
                ),
            ),
            _ => (REPLY.to_owned(), None),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedModel {
    fn name(&self) -> &'static str {
        "derived_consent_mock"
    }
    fn display_name(&self) -> &'static str {
        "Derived-content consent mock"
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
            Self::coaching(&call)
        };
        Ok(ChatResponse {
            replay_blocks: None,
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

struct Fixture {
    resources: Arc<ServerContext>,
    ctx: ChatPipelineContext,
    model: Arc<ScriptedModel>,
    user: User,
    user_id: Uuid,
    tenant: TenantId,
    conversation: String,
}

impl Fixture {
    /// An athlete who consents to Garmin's AI use, with one long ride in
    /// their Strava cache that a Garmin device recorded (`source: garmin`)
    /// when `garmin_ride`, or that names no source otherwise.
    async fn new(email: &str, garmin_ride: bool) -> Self {
        let model = Arc::new(ScriptedModel::new());
        let resources = create_test_server_resources_with_chat_provider(model.clone())
            .await
            .unwrap();
        let (user_id, user, tenant) =
            create_test_user_with_plan(&resources.agent.database, email, "professional")
                .await
                .unwrap();
        let ctx = resources.chat_pipeline_context();
        model.know(&ctx);
        let repos = &resources.common.repos;
        give_garmin_ai_consent(repos, user_id, tenant).await;
        repos
            .provider_connections
            .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        let ride = ActivityBuilder::new(
            "strava-ride-1".to_owned(),
            "Sortie longue".to_owned(),
            SportType::Ride,
            Timestamp::now() - Elapsed::days(2),
            21_600,
            "strava".to_owned(),
        )
        .distance_meters(187_000.0)
        .source_opt(garmin_ride.then(|| GARMIN.to_owned()))
        .build();
        repos
            .activity_cache
            .upsert_activities(user_id, &tenant, "strava", &[ride])
            .await
            .unwrap();
        // A second connection with no session, elected last: the live read
        // fails on it and the Strava cache serves the window.
        repos
            .provider_connections
            .register_connection(user_id, tenant, "garmin", &ConnectionType::OAuth, None)
            .await
            .unwrap();
        let conversation = repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                tenant,
                "derived consent",
                MODEL,
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        Self {
            resources,
            ctx,
            model,
            user,
            user_id,
            tenant,
            conversation,
        }
    }

    /// Run one turn; return the assistant row it persisted.
    async fn turn(&self, content: &str) -> MessageRecord {
        pierre_chat_pipeline::execute(
            &self.ctx,
            TurnRequest {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                conversation_id: self.conversation.clone(),
                user_id: self.user_id,
                conversation_tenant_id: self.tenant,
                tool_tenant_id: self.tenant,
                content: content.to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                quote: None,
                channel_type: "web",
                transport: Transport::WebApp,
                is_direct_message: true,
                ambient_group_fallback: false,
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                caller: None,
                hooks: PipelineHooks::none(),
                grant: OAuthScope::self_grant(),
            },
            &SurfaceProfile::resolve(&SurfaceRequest {
                surface: SurfaceId::Web,
                locale: "fr".to_owned(),
                transport: None,
                prose_contract: None,
            }),
        )
        .await
        .expect("the turn is served");
        self.rows()
            .await
            .into_iter()
            .rfind(|row| row.role == "assistant")
            .expect("the turn persisted a reply")
    }

    /// Every row of the conversation, as the athlete's own read returns it.
    async fn rows(&self) -> Vec<MessageRecord> {
        self.resources
            .common
            .repos
            .chat
            .get_messages(&self.conversation, &self.user_id.to_string(), self.tenant)
            .await
            .unwrap()
    }

    /// Write an earlier exchange: the athlete's question and a reply naming
    /// `sources`.
    async fn earlier_exchange(&self, question: &str, reply: &str, sources: &[String]) {
        let user_id = self.user_id.to_string();
        for (role, content, sources) in [("user", question, &[][..]), ("assistant", reply, sources)]
        {
            self.resources
                .common
                .repos
                .chat
                .add_message(&AddMessageParams {
                    tenant_id: self.tenant,
                    conversation_id: &self.conversation,
                    user_id: &user_id,
                    role,
                    content,
                    token_count: None,
                    finish_reason: None,
                    prompt_tokens: None,
                    model: None,
                    content_blocks: None,
                    transport_policy: TransportPolicy::AnyTransport,
                    channel_type: None,
                    about_user_id: None,
                    turn_attempted_write: None,
                    ai_consent_sources: sources,
                })
                .await
                .unwrap();
        }
    }

    /// File a North Star memory note naming `sources`: the kind every
    /// coaching prompt carries.
    async fn memory_note(&self, object: &str, sources: &[String]) {
        self.resources
            .common
            .repos
            .memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id: self.tenant,
                user_id: &self.user_id.to_string(),
                agent_id: None,
                scope: MemoryScope::User,
                kind: FactKind::NorthStar,
                pillar: None,
                predicate_code: PredicateCode::Have,
                object,
                confidence: 0.9,
                source: FactSource::Conversation,
                valid_until: None,
                source_msg_id: None,
                transport_policy: TransportPolicy::AnyTransport,
                ai_consent_sources: sources,
            })
            .await
            .unwrap();
    }

    /// The athlete's facts, kept and drafts alike, as their own memory page
    /// reads them.
    async fn facts(&self) -> Vec<pierre_memory::UserFact> {
        let memory = &self.resources.common.repos.memory;
        let user = self.user_id.to_string();
        let mut facts = memory
            .list_user_facts(
                self.tenant,
                &user,
                None,
                None,
                100,
                TransportPolicy::FirstPartyOnly,
            )
            .await
            .unwrap();
        facts.extend(
            memory
                .list_draft_user_facts(self.tenant, &user, 100, TransportPolicy::FirstPartyOnly)
                .await
                .unwrap(),
        );
        facts
    }

    async fn withdraw(&self, key: &str) {
        let withdrawn = withdraw_consent(
            &self.resources.common.repos,
            self.user_id,
            key,
            NoticeKind::AiConsent,
            key,
        )
        .await
        .unwrap();
        assert!(withdrawn, "premise: the {key} AI consent was standing");
    }
}

fn keys(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

// ════════════════════════════════════════════════════════════════════════
// carnet#1157 gap 1: a withdrawal reaches what was derived before it
// ════════════════════════════════════════════════════════════════════════

/// A reply drawn from the athlete's Garmin ride names Garmin's consent,
/// whichever service relayed the ride; one drawn from Strava data names none.
#[tokio::test]
async fn a_reply_names_the_consent_of_the_data_it_was_drawn_from() {
    let garmin = Fixture::new("derived-consent-names@test.com", true).await;
    let reply = garmin.turn(READ_ASK).await;
    assert!(reply.content.contains(GARMIN_FIGURE), "{}", reply.content);
    assert_eq!(reply.ai_consent_sources, keys(&[GARMIN]));
    let tool_rows: Vec<MessageRecord> = garmin
        .rows()
        .await
        .into_iter()
        .filter(|row| row.role == "tool_result")
        .collect();
    assert!(!tool_rows.is_empty(), "premise: the read round was kept");
    assert!(
        tool_rows
            .iter()
            .all(|row| row.ai_consent_sources == keys(&[GARMIN])),
        "the round that served the ride names Garmin too"
    );

    let strava = Fixture::new("derived-consent-names-strava@test.com", false).await;
    let reply = strava.turn(READ_ASK).await;
    assert!(
        reply.ai_consent_sources.is_empty(),
        "Strava data owes no AI consent: {:?}",
        reply.ai_consent_sources
    );
}

/// After the athlete withdraws Garmin's AI consent, a turn that would quote a
/// Garmin figure from an earlier reply or a memory note sends none of it to
/// the model; the rest of the thread still reaches it.
#[tokio::test]
async fn after_withdrawal_no_garmin_figure_from_history_or_memory_reaches_the_model() {
    let fx = Fixture::new("derived-consent-withdraw@test.com", true).await;
    fx.earlier_exchange(
        "Et ma sortie longue ?",
        "Ta sortie Garmin de 187 km était solide.",
        &keys(&[GARMIN]),
    )
    .await;
    fx.earlier_exchange("Et ma course ?", "Ta course de 42 km aussi.", &[])
        .await;
    fx.memory_note("rouler 187 km d'une traite", &keys(&[GARMIN]))
        .await;

    // While the consent stands, the earlier Garmin reply is replayed.
    let from = fx.model.calls_so_far();
    fx.turn(PLAIN_ASK).await;
    let consented = fx.model.coaching_since(from);
    assert!(
        consented
            .iter()
            .any(|call| call.conversation.contains(GARMIN_FIGURE)),
        "premise: a consented Garmin reply is replayed"
    );
    assert!(
        consented
            .iter()
            .all(|call| !call.system.contains("rouler 187 km")),
        "a Garmin-derived memory note never reaches a prompt, consent or not"
    );

    fx.withdraw(GARMIN).await;
    let from = fx.model.calls_so_far();
    let reply = fx.turn(PLAIN_ASK).await;
    let withdrawn = fx.model.coaching_since(from);
    assert!(
        withdrawn.iter().any(|call| call.asked.contains(PLAIN_ASK)),
        "the turn reached the model"
    );
    for call in &withdrawn {
        assert!(
            !call.carries(GARMIN_FIGURE),
            "no Garmin figure reaches the model after withdrawal"
        );
    }
    assert!(
        withdrawn
            .iter()
            .filter(|call| call.asked.contains(PLAIN_ASK))
            .all(|call| call.conversation.contains(OPEN_FIGURE)),
        "the rest of the thread is still replayed"
    );
    assert!(
        reply.ai_consent_sources.is_empty(),
        "the turn served nothing Garmin's consent covers: {:?}",
        reply.ai_consent_sources
    );
}

/// The rule is the consent's, not Garmin's: withdrawing WHOOP's AI consent
/// keeps a WHOOP-derived reply and memory note out of every later prompt.
#[tokio::test]
async fn withdrawing_whoop_keeps_whoop_derived_history_and_memory_from_the_model() {
    let fx = Fixture::new("derived-consent-whoop@test.com", false).await;
    grant_consent(
        &fx.resources.common.repos,
        fx.tenant.as_uuid(),
        fx.user_id,
        WHOOP,
        NoticeKind::AiConsent,
        "WHOOP",
    )
    .await
    .unwrap();
    fx.earlier_exchange(
        "Et ma récupération ?",
        "Ta récupération WHOOP est de 91 %.",
        &keys(&[WHOOP]),
    )
    .await;
    fx.earlier_exchange("Et ma course ?", "Ta course de 42 km aussi.", &[])
        .await;
    fx.memory_note("récupération de 91 % au réveil", &keys(&[WHOOP]))
        .await;

    let from = fx.model.calls_so_far();
    fx.turn(PLAIN_ASK).await;
    let consented = fx.model.coaching_since(from);
    assert!(
        consented
            .iter()
            .any(|call| call.conversation.contains("91 %")),
        "premise: a consented WHOOP reply is replayed"
    );
    assert!(
        consented
            .iter()
            .any(|call| call.system.contains("récupération de 91 %")),
        "premise: a consented WHOOP memory note reaches the prompt"
    );

    fx.withdraw(WHOOP).await;
    let from = fx.model.calls_so_far();
    fx.turn(PLAIN_ASK).await;
    let withdrawn = fx.model.coaching_since(from);
    assert!(
        withdrawn.iter().any(|call| call.asked.contains(PLAIN_ASK)),
        "the turn reached the model"
    );
    for call in &withdrawn {
        assert!(!call.carries("91 %"), "no WHOOP figure reaches the model");
    }
    assert!(
        withdrawn
            .iter()
            .filter(|call| call.asked.contains(PLAIN_ASK))
            .all(|call| call.conversation.contains(OPEN_FIGURE)),
        "the rest of the thread is still replayed"
    );
}

/// The athlete always reads their own thread: the message route returns a
/// Garmin-derived reply after the consent is withdrawn.
#[tokio::test]
async fn the_athletes_message_route_still_returns_withdrawn_rows() {
    let fx = Fixture::new("derived-consent-route@test.com", true).await;
    fx.earlier_exchange(
        "Et ma sortie longue ?",
        "Ta sortie Garmin de 187 km était solide.",
        &keys(&[GARMIN]),
    )
    .await;
    fx.withdraw(GARMIN).await;

    let token = generate_test_token(&fx.resources, &fx.user).await;
    let page = AxumTestRequest::get(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation
    ))
    .header("authorization", &format!("Bearer {token}"))
    .send(ChatRoutes::routes(Arc::clone(&fx.resources)))
    .await;
    assert_eq!(page.status_code(), StatusCode::OK);
    let page: Value = page.json();
    assert!(
        page["messages"]
            .as_array()
            .expect("the thread's messages")
            .iter()
            .any(|message| message["content"]
                .as_str()
                .is_some_and(|content| content.contains(GARMIN_FIGURE))),
        "the athlete still reads the Garmin reply: {page}"
    );
}

// ════════════════════════════════════════════════════════════════════════
// carnet#1158: Garmin-derived content stays out of the agent's memory
// ════════════════════════════════════════════════════════════════════════

/// A turn quoting the athlete's Garmin ride files no memory note: no
/// extraction runs, and no fact carries its figures.
#[tokio::test]
async fn a_turn_quoting_a_garmin_activity_files_no_memory_note() {
    let fx = Fixture::new("derived-consent-no-note@test.com", true).await;
    let from = fx.model.calls_so_far();
    let reply = fx.turn(READ_ASK).await;
    assert!(
        reply.content.contains(GARMIN_FIGURE),
        "premise: {}",
        reply.content
    );
    assert!(
        fx.model.extraction_since(from, SETTLE).await.is_none(),
        "a turn drawn from Garmin data is not extracted"
    );
    assert!(
        fx.facts()
            .await
            .iter()
            .all(|fact| !fact.object.contains(GARMIN_FIGURE)),
        "no memory note carries the Garmin figure"
    );
}

/// Replays count too (JF 2026-11-23, carnet#1158): a turn that serves no
/// Garmin data of its own but replays an earlier Garmin-derived reply from
/// the thread files no memory, neither a fact nor a captured advice.
#[tokio::test]
async fn a_turn_replaying_a_garmin_reply_files_no_memory() {
    let fx = Fixture::new("derived-consent-replay@test.com", false).await;
    fx.earlier_exchange(
        "Et ma sortie longue ?",
        "Ta sortie Garmin de 187 km était solide.",
        &keys(&[GARMIN]),
    )
    .await;
    let from = fx.model.calls_so_far();
    let reply = fx.turn(READ_ASK).await;
    assert!(
        fx.model
            .coaching_since(from)
            .iter()
            .any(|call| call.conversation.contains("sortie Garmin de 187 km")),
        "premise: the consented Garmin reply is replayed"
    );
    assert_eq!(
        reply.ai_consent_sources,
        keys(&[GARMIN]),
        "a reply drawn from a replayed Garmin row names Garmin"
    );
    assert!(
        fx.model.extraction_since(from, SETTLE).await.is_none(),
        "a turn replaying Garmin content is not extracted"
    );
    assert!(
        fx.model
            .calls_since(from)
            .iter()
            .all(|call| !fx.model.is_advice_capture(call)),
        "nor is its advice captured"
    );
}

/// The control: the same turn over Strava data extracts as it always did.
#[tokio::test]
async fn a_non_garmin_turn_still_extracts_normally() {
    let fx = Fixture::new("derived-consent-extracts@test.com", false).await;
    let from = fx.model.calls_so_far();
    fx.turn(READ_ASK).await;
    let extraction = fx
        .model
        .extraction_since(from, PATIENCE)
        .await
        .expect("a Strava turn is extracted");
    assert!(extraction.conversation.contains(GARMIN_FIGURE));
    assert!(
        fx.model
            .calls_since(from)
            .iter()
            .any(|call| fx.model.is_advice_capture(call)),
        "a Strava turn's advice is captured"
    );
    let deadline = Instant::now() + PATIENCE;
    while !fx
        .facts()
        .await
        .iter()
        .any(|fact| fact.object == REMEMBERED)
    {
        assert!(Instant::now() < deadline, "the extracted fact never landed");
        sleep(StdDuration::from_millis(20)).await;
    }
}

/// A note the agent asks to remember in a turn that served Garmin data is
/// refused, so nothing of it is filed or reaches a later prompt.
#[tokio::test]
async fn a_note_the_agent_asks_to_write_in_a_garmin_turn_is_never_filed() {
    let fx = Fixture::new("derived-consent-remember@test.com", true).await;
    fx.turn(REMEMBER_ASK).await;
    // A turn carrying Garmin data files no memory at all, a note the agent
    // asks for included (JF 2026-11-23, carnet#1158): the remember tool
    // refuses it.
    assert!(
        fx.facts()
            .await
            .iter()
            .all(|fact| fact.object != REMEMBERED),
        "no note is filed from a Garmin turn"
    );

    let from = fx.model.calls_so_far();
    fx.turn(PLAIN_ASK).await;
    let later = fx.model.coaching_since(from);
    assert!(!later.is_empty());
    assert!(
        later.iter().all(|call| !call.system.contains(REMEMBERED)),
        "nothing of it reaches a later prompt"
    );
}

/// A compaction summary over a Garmin turn carries none of its figures: the
/// summarizer is never handed the Garmin rows, which the block still covers,
/// and the block names the consents of the rows it did summarize.
#[tokio::test]
async fn a_compaction_summary_over_a_garmin_turn_carries_none_of_its_figures() {
    let fx = Fixture::new("derived-consent-summary@test.com", false).await;
    // Over the message cap and far under the token bands: the oldest run is
    // summarized. The second exchange quotes Garmin, the third WHOOP.
    let config = CompactionConfig::default();
    let exchanges = config.max_messages / 2 + 2;
    for i in 0..exchanges {
        let (reply, sources) = match i {
            1 => ("Ta sortie Garmin de 187 km.".to_owned(), keys(&[GARMIN])),
            2 => ("Ta récupération WHOOP de 91 %.".to_owned(), keys(&[WHOOP])),
            _ => (format!("échange {i:02} sur la semaine"), Vec::new()),
        };
        fx.earlier_exchange(&format!("question {i:02}"), &reply, &sources)
            .await;
    }
    let history = fx.rows().await;
    let (mut llm_messages, source_ids) = build_llm_messages(Some("system prompt"), &history);
    let source_consents: Vec<Vec<String>> = source_ids
        .iter()
        .map(|id| {
            history
                .iter()
                .find(|row| Some(&row.id) == id.as_ref())
                .map_or_else(Vec::new, |row| row.ai_consent_sources.clone())
        })
        .collect();
    let summarizer = Arc::new(Summarizer::default());
    let provider = ChatProvider::Custom(summarizer.clone());
    let outcome = ConversationCompactor::new(config)
        .compact_if_needed(CompactionContext {
            repo: fx.resources.common.repos.memory.as_ref(),
            provider: &provider,
            summary_prompt: CONVERSATION_SUMMARY_PROMPT,
            recorder: None,
            tenant_id: fx.tenant,
            conversation_id: &fx.conversation,
            source_ids: &source_ids,
            source_members: &vec![None; source_ids.len()],
            source_consents: &source_consents,
            llm_messages: &mut llm_messages,
            summary_for_prompt: str::to_owned,
        })
        .await
        .unwrap();

    let CompactionOutcome::Summarized { block, .. } = outcome else {
        panic!("premise: the oldest run is summarized, got {outcome:?}");
    };
    let handed = summarizer.handed.lock().unwrap().clone();
    assert!(handed.contains("question 01"), "premise: {handed}");
    assert!(
        !handed.contains(GARMIN_FIGURE),
        "the summarizer is never handed a Garmin row"
    );
    assert!(
        handed.contains("91 %"),
        "a consented WHOOP row is summarized"
    );
    assert!(!block.summary.contains(GARMIN_FIGURE));
    assert_eq!(
        block.ai_consent_sources,
        keys(&[WHOOP]),
        "the block names the consents of what it summarized, and not Garmin's"
    );
    let stored = fx
        .resources
        .common
        .repos
        .memory
        .list_compaction_blocks(&fx.conversation, fx.tenant)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].ai_consent_sources, keys(&[WHOOP]));
    assert!(
        llm_messages
            .iter()
            .all(|message| !message.content.contains(GARMIN_FIGURE)),
        "the Garmin row is covered by the block, so this turn no longer carries it"
    );
}

/// Answers every summary request with what it was handed, keeping it.
#[derive(Default)]
struct Summarizer {
    handed: Mutex<String>,
}

#[async_trait]
impl LlmProvider for Summarizer {
    fn name(&self) -> &'static str {
        "derived_consent_summarizer"
    }
    fn display_name(&self) -> &'static str {
        "Derived-content consent summarizer"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        MODEL
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let handed = request
            .messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.handed.lock().unwrap().clone_from(&handed);
        Ok(ChatResponse {
            replay_blocks: None,
            content: format!("Résumé : {handed}"),
            model: MODEL.to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let response = self.complete(request).await?;
        let chunk = StreamChunk {
            delta: response.content,
            is_final: true,
            finish_reason: response.finish_reason,
        };
        let stream: ChatStream = Pin::from(Box::new(stream::once(async move { Ok(chunk) })));
        Ok(stream)
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}
