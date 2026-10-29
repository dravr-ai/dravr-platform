// ABOUTME: A per-call-billed head runs its tool-picking iterations on the iteration model, the reply on the turn's
// ABOUTME: Pins the discard-and-re-ask, the billing of every call, and the heads and turns that route nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#888: on an Anthropic head, the API tool loop asks the tool-iteration
//! model (`PIERRE_LLM_MODEL_TOOL_ITERATION`, Haiku by default) for every
//! iteration, and the turn's own model for the reply the athlete reads. These
//! tests drive a scripted Anthropic-shaped tier through `run_tool_loop` and
//! read which model each call asked for:
//!
//! - (a) tool-calling iterations run on the iteration model, and the answer it
//!   gives at the end is billed, discarded, and re-asked on the turn's model;
//! - (b) the turn's model may still call a tool after the re-ask, and having
//!   overruled the iteration model it serves every later iteration itself;
//! - (b2) a turn whose iteration model only ever calls tools never escalates;
//! - (c) a `<tool_call>` block in the iteration model's text is a tool call,
//!   not an answer;
//! - (d) the re-ask happens on the last budgeted iteration too;
//! - (e) a turn whose own model is the iteration model asks once per iteration;
//! - (f) a provider with no iteration model routes nothing;
//! - (g) a chain's tail serves a faulted turn on the turn's model, never the
//!   head's iteration model;
//! - (h) a call from the iteration model to a tool that writes is never run:
//!   it is billed, discarded, and re-asked on the turn's model, which then
//!   serves the rest of the turn — a structured call and a `<tool_call>` block
//!   alike, and a response mixing a read with the write is discarded whole.
//!   A read call still runs on the iteration model: (a), (b2) and (c). A
//!   discarded call is never shown to the athlete as a step either: only the
//!   calls that run report progress to the turn's sink.

use crate::common;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::{create_test_server_resources, create_test_user};
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError, ToolCallRequest, ToolStream, ToolStreamEvent,
};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::call_record::{LlmCallRecord, LlmCallRecorder};
use pierre_llm::config::LlmProviderType;
use pierre_llm::stage::{StageModels, ANTHROPIC_TOOL_ITERATION_MODEL, TOOL_ITERATION_ENV};
use pierre_llm::{
    ChatMessage, ChatProvider, EmbacleProvider, FunctionDeclaration, QuotaCooldowns, Tool,
};
use pierre_services::chat_stream::{
    TurnEvent, TurnEventSink, TOOL_STATUS_COMPLETED, TOOL_STATUS_IN_PROGRESS,
};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult};
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

/// A real registry tool that needs no provider connection and no fixture data.
const ROUND_TRIP_TOOL: &str = "get_connection_status";

/// A real registry tool that declares `WRITES_DATA`: the write carnet#1193
/// caught the iteration model making. Called with no arguments, it would fail
/// loudly rather than save anything, were it ever run.
const WRITE_TOOL: &str = "save_training_plan";

/// The turn's own model: the one that writes the reply.
const DRAFT_MODEL: &str = "claude-sonnet-5-5";

/// The iteration model, as the Anthropic head resolves it by default.
const ITERATION_MODEL: &str = ANTHROPIC_TOOL_ITERATION_MODEL;

/// One scripted reply.
#[derive(Clone, Copy)]
enum Step {
    /// A structured call of `ROUND_TRIP_TOOL`.
    CallsTool,
    /// The same call written as a `<tool_call>` block in the text.
    WritesToolCall,
    /// A structured call of `WRITE_TOOL`.
    CallsWrite,
    /// The same write as a `<tool_call>` block in the text.
    WritesWriteCall,
    /// One response calling `ROUND_TRIP_TOOL` and `WRITE_TOOL` together.
    CallsReadAndWrite,
    /// A final answer.
    Answers(&'static str),
    /// An HTTP 402 credit exhaustion: a provider fault the tail serves.
    CreditSpent,
}

/// An Anthropic-API-shaped tier with one script per model it is asked for,
/// recording the model of every call, and of every call asked to stream.
struct ScriptedApi {
    scripts: Mutex<HashMap<String, VecDeque<Step>>>,
    asked: Mutex<Vec<String>>,
    streamed: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl ScriptedApi {
    fn new(scripts: &[(&str, &[Step])]) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(
                scripts
                    .iter()
                    .map(|(model, steps)| ((*model).to_owned(), steps.iter().copied().collect()))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
            streamed: Mutex::new(Vec::new()),
            models: vec![DRAFT_MODEL.to_owned(), ITERATION_MODEL.to_owned()],
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    fn streamed(&self) -> Vec<String> {
        self.streamed.lock().unwrap().clone()
    }

    /// The next scripted response for the model `request` asks for.
    fn respond(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| self.models[0].clone());
        self.asked.lock().unwrap().push(model.clone());
        let step = self
            .scripts
            .lock()
            .unwrap()
            .get_mut(&model)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| panic!("no script left for {model}"));
        let (content, tool_calls) = match step {
            Step::CallsTool => (String::new(), Some(vec![structured_call(ROUND_TRIP_TOOL)])),
            Step::WritesToolCall => (text_call(ROUND_TRIP_TOOL), None),
            Step::CallsWrite => (String::new(), Some(vec![structured_call(WRITE_TOOL)])),
            Step::WritesWriteCall => (text_call(WRITE_TOOL), None),
            Step::CallsReadAndWrite => (
                String::new(),
                Some(vec![
                    structured_call(ROUND_TRIP_TOOL),
                    structured_call(WRITE_TOOL),
                ]),
            ),
            Step::Answers(text) => (text.to_owned(), None),
            Step::CreditSpent => {
                return Err(RunnerError::rate_limit(
                    "anthropic",
                    "credit balance exhausted: Your credit balance is too low",
                ))
            }
        };
        Ok(ChatResponse {
            content,
            model,
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls,
        })
    }
}

#[async_trait]
impl EmbacleLlmProvider for ScriptedApi {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn display_name(&self) -> &'static str {
        "Anthropic API (scripted)"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING
            | LlmCapabilities::SYSTEM_MESSAGES
            | LlmCapabilities::FUNCTION_CALLING
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.respond(request)
    }

    /// The scripted response, its text streamed a word at a time as the
    /// Anthropic Messages API streams it, then closed by the whole response.
    async fn complete_stream_with_tools(
        &self,
        request: &ChatRequest,
    ) -> Result<ToolStream, RunnerError> {
        let response = self.respond(request)?;
        self.streamed.lock().unwrap().push(response.model.clone());
        let mut events: Vec<Result<ToolStreamEvent, RunnerError>> = response
            .content
            .split_inclusive(' ')
            .map(|word| Ok(ToolStreamEvent::TextDelta(word.to_owned())))
            .collect();
        events.push(Ok(ToolStreamEvent::Done(response)));
        Ok(Box::pin(tokio_stream::iter(events)))
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal(
            "the tool loop never streams an API tier",
        ))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

/// A structured call of `tool` with no arguments.
fn structured_call(tool: &str) -> ToolCallRequest {
    ToolCallRequest {
        id: format!("toolu_{tool}"),
        function_name: tool.to_owned(),
        arguments: json!({}),
    }
}

/// A `<tool_call>` block calling `tool` with no arguments.
fn text_call(tool: &str) -> String {
    format!("<tool_call>{{\"name\":\"{tool}\",\"arguments\":{{}}}}</tool_call>")
}

/// Every `llm_usage` row the turn writes.
#[derive(Default)]
struct Rows(Mutex<Vec<LlmCallRecord>>);

impl LlmCallRecorder for Rows {
    fn record(&self, record: LlmCallRecord) {
        self.0.lock().unwrap().push(record);
    }
}

/// `tier` as an Anthropic head whose stages resolve with no variable set.
fn routed(tier: &Arc<ScriptedApi>) -> EmbacleProvider {
    unrouted(tier).with_stage_models(StageModels::resolve(
        Some(LlmProviderType::Anthropic),
        |_| None,
    ))
}

/// `tier` as an Anthropic head with every call on the turn's model.
fn unrouted(tier: &Arc<ScriptedApi>) -> EmbacleProvider {
    EmbacleProvider::from_runner(Box::new(Arc::clone(tier)), "Anthropic API (scripted)")
        .of_kind(LlmProviderType::Anthropic)
}

/// `tier` as a function-calling head whose tool loop answers whole (Gemini):
/// it holds a sink on a surface that reads frames, but streams no text into it.
fn answers_whole(tier: &Arc<ScriptedApi>) -> EmbacleProvider {
    EmbacleProvider::from_runner(Box::new(Arc::clone(tier)), "Gemini API (scripted)")
        .of_kind(LlmProviderType::Gemini)
}

/// What one turn produced.
struct Turn {
    result: ToolLoopResult,
    messages: Vec<ChatMessage>,
    rows: Vec<LlmCallRecord>,
}

/// Run one turn of the production tool loop on `provider`, on `model`, with
/// `max_iterations` iterations.
async fn run_turn(provider: &ChatProvider, model: &str, max_iterations: usize) -> Turn {
    run_turn_into(provider, model, max_iterations, None).await
}

/// [`run_turn`], reporting to `stream_sink` when one is given, and streaming
/// reply text through it when the loop's provider streams.
async fn run_turn_into(
    provider: &ChatProvider,
    model: &str,
    max_iterations: usize,
    stream_sink: Option<TurnEventSink>,
) -> Turn {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let executor = Arc::new(
        UniversalToolExecutor::new(resources)
            .with_scopes(OAuthScope::self_grant())
            .with_turn_token("iteration-routing-turn".to_owned()),
    );
    let tools = Tool {
        function_declarations: vec![FunctionDeclaration::new(
            ROUND_TRIP_TOOL.to_owned(),
            "Report which fitness providers are connected".to_owned(),
            Some(json!({ "type": "object", "properties": {} })),
        )],
    };
    let rows = Arc::new(Rows::default());
    let user = user_id.to_string();
    let params = ToolLoopParams {
        provider,
        executor,
        tools: &tools,
        model,
        user_id: &user,
        tenant_id: TenantId::from_uuid(Uuid::new_v4()),
        max_iterations,
        call_recorder: Some(Arc::<Rows>::clone(&rows)),
        tool_message_recorder: None,
        temperature: None,
        stream_sink,
        mcp_servers: Vec::new(),
    };
    let mut messages = vec![
        ChatMessage::system("You are a test coach."),
        ChatMessage::user("suis-je connecté ?"),
    ];
    let result = run_tool_loop(&params, &mut messages)
        .await
        .expect("the turn is served");
    let rows = rows.0.lock().unwrap().drain(..).collect();
    Turn {
        result,
        messages,
        rows,
    }
}

fn models(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

/// The discarded answer reached neither the reply nor the turn's messages.
fn assert_discarded(turn: &Turn, discarded: &str) {
    assert!(
        !turn.result.content.contains(discarded),
        "the iteration model's answer is never the reply"
    );
    assert!(
        turn.messages.iter().all(|m| !m.content.contains(discarded)),
        "the iteration model's answer never enters the turn's messages"
    );
}

/// (a) The iteration model picks the tool, then answers; that answer is billed
/// and discarded, and the turn's model writes the reply.
#[tokio::test]
async fn tool_iterations_run_on_the_iteration_model_and_the_reply_on_the_turns() {
    let tier = ScriptedApi::new(&[
        (
            ITERATION_MODEL,
            &[Step::CallsTool, Step::Answers("cheap answer")],
        ),
        (DRAFT_MODEL, &[Step::Answers("Tu es connecté à Strava.")]),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));
    assert_eq!(provider.tool_iteration_model(), Some(ITERATION_MODEL));

    let turn = run_turn(&provider, DRAFT_MODEL, 4).await;

    assert_eq!(
        tier.asked(),
        models(&[ITERATION_MODEL, ITERATION_MODEL, DRAFT_MODEL])
    );
    assert_eq!(turn.result.content, "Tu es connecté à Strava.");
    assert_eq!(turn.result.tools_called, vec![ROUND_TRIP_TOOL.to_owned()]);
    assert_discarded(&turn, "cheap answer");
    assert_eq!(
        turn.rows
            .iter()
            .map(|r| (r.model.as_str(), r.call_sequence))
            .collect::<Vec<_>>(),
        vec![
            (ITERATION_MODEL, Some(1)),
            (ITERATION_MODEL, Some(2)),
            (DRAFT_MODEL, Some(3)),
        ],
        "every call bills its own row on the model it ran, the discarded one included"
    );
}

/// (b) The turn's model calls a tool after the re-ask: the tool runs, and the
/// turn's model, having overruled the iteration model's "done", serves every
/// later iteration of the turn without the iteration model being asked again.
#[tokio::test]
async fn the_turns_model_may_still_call_a_tool_after_the_re_ask() {
    let tier = ScriptedApi::new(&[
        (ITERATION_MODEL, &[Step::Answers("cheap one")]),
        (
            DRAFT_MODEL,
            &[Step::CallsTool, Step::CallsTool, Step::Answers("Connecté.")],
        ),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, DRAFT_MODEL, 4).await;

    assert_eq!(
        tier.asked(),
        models(&[ITERATION_MODEL, DRAFT_MODEL, DRAFT_MODEL, DRAFT_MODEL]),
        "once overruled, the iteration model is never asked again in this turn"
    );
    assert_eq!(turn.result.content, "Connecté.");
    assert_eq!(
        turn.result.tools_called,
        vec![ROUND_TRIP_TOOL.to_owned(), ROUND_TRIP_TOOL.to_owned()]
    );
    assert_discarded(&turn, "cheap one");
    assert_eq!(
        turn.rows
            .iter()
            .map(|r| (r.model.as_str(), r.call_sequence))
            .collect::<Vec<_>>(),
        vec![
            (ITERATION_MODEL, Some(1)),
            (DRAFT_MODEL, Some(2)),
            (DRAFT_MODEL, Some(3)),
            (DRAFT_MODEL, Some(4)),
        ]
    );
}

/// (b2) A turn whose iteration model only ever calls tools stays on it: there
/// was no overrule, so there is no escalation, and only the final answer is
/// re-asked on the turn's model.
#[tokio::test]
async fn a_turn_whose_iteration_model_only_calls_tools_never_escalates() {
    let tier = ScriptedApi::new(&[
        (
            ITERATION_MODEL,
            &[
                Step::CallsTool,
                Step::CallsTool,
                Step::CallsTool,
                Step::Answers("cheap answer"),
            ],
        ),
        (DRAFT_MODEL, &[Step::Answers("Connecté.")]),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, DRAFT_MODEL, 5).await;

    assert_eq!(
        tier.asked(),
        models(&[
            ITERATION_MODEL,
            ITERATION_MODEL,
            ITERATION_MODEL,
            ITERATION_MODEL,
            DRAFT_MODEL,
        ])
    );
    assert_eq!(turn.result.content, "Connecté.");
    assert_eq!(turn.result.tools_called.len(), 3);
    assert_discarded(&turn, "cheap answer");
}

/// (c) A `<tool_call>` block in the iteration model's text is run as a tool
/// call, not thrown away as an answer.
#[tokio::test]
async fn a_text_tool_call_from_the_iteration_model_is_a_tool_call() {
    let tier = ScriptedApi::new(&[
        (
            ITERATION_MODEL,
            &[Step::WritesToolCall, Step::Answers("cheap answer")],
        ),
        (DRAFT_MODEL, &[Step::Answers("Connecté.")]),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, DRAFT_MODEL, 4).await;

    assert_eq!(
        tier.asked(),
        models(&[ITERATION_MODEL, ITERATION_MODEL, DRAFT_MODEL])
    );
    assert_eq!(turn.result.tools_called, vec![ROUND_TRIP_TOOL.to_owned()]);
    assert_eq!(turn.result.content, "Connecté.");
}

/// (d) A turn with one iteration whose iteration model answers still gets its
/// reply from the turn's model, not an exhausted budget.
#[tokio::test]
async fn the_re_ask_happens_on_the_last_budgeted_iteration() {
    let tier = ScriptedApi::new(&[
        (ITERATION_MODEL, &[Step::Answers("cheap answer")]),
        (DRAFT_MODEL, &[Step::Answers("Bonjour.")]),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, DRAFT_MODEL, 1).await;

    assert_eq!(tier.asked(), models(&[ITERATION_MODEL, DRAFT_MODEL]));
    assert_eq!(
        turn.result.content, "Bonjour.",
        "the re-ask is part of the iteration, not one past the budget"
    );
}

/// (e) A turn already on the iteration model asks once per iteration.
#[tokio::test]
async fn a_turn_on_the_iteration_model_asks_once_per_iteration() {
    let tier = ScriptedApi::new(&[(ITERATION_MODEL, &[Step::CallsTool, Step::Answers("Oui.")])]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, ITERATION_MODEL, 4).await;

    assert_eq!(tier.asked(), models(&[ITERATION_MODEL, ITERATION_MODEL]));
    assert_eq!(turn.result.content, "Oui.");
}

/// (f) A provider with no iteration model — one set to `inherit`, or one not
/// built by `ChatProvider::from_chain` — runs every call on the turn's model.
#[tokio::test]
async fn a_provider_with_no_iteration_model_routes_nothing() {
    let inherit = StageModels::resolve(Some(LlmProviderType::Anthropic), |key| {
        (key == TOOL_ITERATION_ENV).then(|| "inherit".to_owned())
    });
    let inherited = ScriptedApi::new(&[(DRAFT_MODEL, &[Step::CallsTool, Step::Answers("Oui.")])]);
    let bare = ScriptedApi::new(&[(DRAFT_MODEL, &[Step::CallsTool, Step::Answers("Oui.")])]);
    for (label, tier, provider) in [
        (
            "inherit",
            &inherited,
            unrouted(&inherited).with_stage_models(inherit),
        ),
        ("unrouted", &bare, unrouted(&bare)),
    ] {
        let provider = ChatProvider::Embacle(provider);
        assert_eq!(provider.tool_iteration_model(), None, "{label}");

        let turn = run_turn(&provider, DRAFT_MODEL, 4).await;

        assert_eq!(tier.asked(), models(&[DRAFT_MODEL, DRAFT_MODEL]), "{label}");
        assert_eq!(turn.result.content, "Oui.", "{label}");
    }
}

/// (i) A turn that streams: the iteration model's calls never stream — its
/// answer is discarded, so no word of it may reach the athlete — and the turn
/// model's reply reaches the sink a word at a time, spelling the reply.
#[tokio::test]
async fn only_the_turns_model_streams_and_its_deltas_spell_the_reply() {
    let tier = ScriptedApi::new(&[
        (
            ITERATION_MODEL,
            &[Step::CallsTool, Step::Answers("cheap answer")],
        ),
        (DRAFT_MODEL, &[Step::Answers("Tu es connecté à Strava.")]),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));
    let (sink, mut events) = mpsc::unbounded_channel();

    let turn = run_turn_into(&provider, DRAFT_MODEL, 4, Some(sink)).await;

    assert_eq!(
        tier.asked(),
        models(&[ITERATION_MODEL, ITERATION_MODEL, DRAFT_MODEL])
    );
    assert_eq!(
        tier.streamed(),
        models(&[DRAFT_MODEL]),
        "only the call on the turn's model is asked to stream"
    );
    let mut deltas = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            TurnEvent::ProseDelta(delta) => deltas.push(delta),
            // The tool round's step; what a turn's steps report is pinned by
            // `a_discarded_iteration_response_reports_no_step`.
            TurnEvent::Progress(_) => {}
            other => panic!("this turn's text needs nothing taken back, got {other:?}"),
        }
    }
    assert_eq!(
        deltas,
        vec!["Tu ", "es ", "connecté ", "à ", "Strava."],
        "the reply arrives word by word, in order"
    );
    assert_eq!(turn.result.content, deltas.concat());
    assert_discarded(&turn, "cheap answer");
}

/// (i2) A sink on a head whose loop does not stream text: the surface reads
/// frames, so each tool the turn runs is still a step on it, but no call is
/// asked to stream and no word of the reply reaches it before the turn returns.
#[tokio::test]
async fn a_sink_without_text_streaming_carries_the_steps_alone() {
    let tier = ScriptedApi::new(&[(
        DRAFT_MODEL,
        &[Step::CallsTool, Step::Answers("Tu es connecté à Strava.")],
    )]);
    let provider = ChatProvider::Embacle(answers_whole(&tier));
    let (sink, mut events) = mpsc::unbounded_channel();

    let turn = run_turn_into(&provider, DRAFT_MODEL, 4, Some(sink)).await;

    assert_eq!(tier.asked(), models(&[DRAFT_MODEL, DRAFT_MODEL]));
    assert!(
        tier.streamed().is_empty(),
        "a loop that does not stream text opens no stream: {:?}",
        tier.streamed()
    );
    let mut steps = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            TurnEvent::Progress(progress) => steps.push((progress.title, progress.status)),
            other => panic!("only steps cross a sink that carries no text, got {other:?}"),
        }
    }
    assert_eq!(
        steps,
        vec![
            (
                ROUND_TRIP_TOOL.to_owned(),
                TOOL_STATUS_IN_PROGRESS.to_owned()
            ),
            (ROUND_TRIP_TOOL.to_owned(), TOOL_STATUS_COMPLETED.to_owned()),
        ],
        "the tool the turn ran is reported as it starts and as it completes"
    );
    assert_eq!(turn.result.content, "Tu es connecté à Strava.");
}

/// (g) The head faults on its iteration model: the tail serves the turn on the
/// turn's model, since the iteration model is the head's alone.
#[tokio::test]
async fn a_chains_tail_serves_on_the_turns_model() {
    let head = ScriptedApi::new(&[(ITERATION_MODEL, &[Step::CreditSpent])]);
    let tail = ScriptedApi::new(&[(DRAFT_MODEL, &[Step::CallsTool, Step::Answers("Oui.")])]);
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![unrouted(&head), unrouted(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap()
        .with_stage_models(StageModels::resolve(
            Some(LlmProviderType::Anthropic),
            |_| None,
        )),
    );

    let turn = run_turn(&chain, DRAFT_MODEL, 4).await;

    assert_eq!(head.asked(), models(&[ITERATION_MODEL]));
    assert_eq!(tail.asked(), models(&[DRAFT_MODEL, DRAFT_MODEL]));
    assert_eq!(turn.result.content, "Oui.");
}

/// The rows the turn billed, as (model, sequence, tools listed).
fn billed(turn: &Turn) -> Vec<(&str, Option<i64>, Vec<String>)> {
    turn.rows
        .iter()
        .map(|r| (r.model.as_str(), r.call_sequence, r.tools_called.clone()))
        .collect()
}

/// No call to `WRITE_TOOL` ran, and none reached the turn's messages.
fn assert_write_never_ran(turn: &Turn) {
    assert!(
        !turn
            .result
            .tools_called
            .iter()
            .any(|tool| tool == WRITE_TOOL),
        "the iteration model's write is never executed"
    );
    assert!(
        turn.messages
            .iter()
            .all(|m| !m.content.contains(WRITE_TOOL)),
        "the iteration model's write never enters the turn's messages"
    );
}

/// (h) The iteration model calls a tool that writes: the call is billed and
/// discarded unrun, the turn's model is re-asked, and it serves every later
/// iteration of the turn — the iteration model is never asked again.
#[tokio::test]
async fn a_write_call_from_the_iteration_model_is_re_asked_on_the_turns_model() {
    let tier = ScriptedApi::new(&[
        (ITERATION_MODEL, &[Step::CallsWrite]),
        (
            DRAFT_MODEL,
            &[Step::CallsTool, Step::Answers("Ton plan est prêt.")],
        ),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, DRAFT_MODEL, 4).await;

    assert_eq!(
        tier.asked(),
        models(&[ITERATION_MODEL, DRAFT_MODEL, DRAFT_MODEL]),
        "after a write attempt the iteration model is never asked again in this turn"
    );
    assert_eq!(turn.result.content, "Ton plan est prêt.");
    assert_eq!(turn.result.tools_called, vec![ROUND_TRIP_TOOL.to_owned()]);
    assert_write_never_ran(&turn);
    assert_eq!(
        billed(&turn),
        vec![
            (ITERATION_MODEL, Some(1), Vec::new()),
            (DRAFT_MODEL, Some(2), vec![ROUND_TRIP_TOOL.to_owned()]),
            (DRAFT_MODEL, Some(3), Vec::new()),
        ],
        "the discarded write is billed, and its row lists no tool since none ran"
    );
}

/// (h) The same write as a `<tool_call>` block in the iteration model's text —
/// the shape carnet#1193 caught — is discarded the same way, block and all.
#[tokio::test]
async fn a_text_write_call_from_the_iteration_model_is_re_asked_on_the_turns_model() {
    let tier = ScriptedApi::new(&[
        (ITERATION_MODEL, &[Step::WritesWriteCall]),
        (DRAFT_MODEL, &[Step::Answers("Ton plan est prêt.")]),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, DRAFT_MODEL, 4).await;

    assert_eq!(tier.asked(), models(&[ITERATION_MODEL, DRAFT_MODEL]));
    assert_eq!(turn.result.content, "Ton plan est prêt.");
    assert!(turn.result.tools_called.is_empty(), "no tool ran");
    assert_write_never_ran(&turn);
    assert_discarded(&turn, "<tool_call>");
}

/// (h) A response that reads and writes at once is discarded whole: its read
/// does not run either, since the response is not the iteration model's.
#[tokio::test]
async fn a_read_and_write_response_from_the_iteration_model_is_discarded_whole() {
    let tier = ScriptedApi::new(&[
        (ITERATION_MODEL, &[Step::CallsReadAndWrite]),
        (DRAFT_MODEL, &[Step::Answers("Ton plan est prêt.")]),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, DRAFT_MODEL, 4).await;

    assert_eq!(tier.asked(), models(&[ITERATION_MODEL, DRAFT_MODEL]));
    assert_eq!(turn.result.content, "Ton plan est prêt.");
    assert!(
        turn.result.tools_called.is_empty(),
        "neither the write nor the read beside it runs"
    );
    assert_write_never_ran(&turn);
    assert_eq!(
        billed(&turn),
        vec![
            (ITERATION_MODEL, Some(1), Vec::new()),
            (DRAFT_MODEL, Some(2), Vec::new()),
        ]
    );
}

/// (h) A discarded response shows the athlete no step: neither its write nor
/// the read beside it reports progress, and the read the turn's model then
/// calls is the trail's one step, started and completed.
#[tokio::test]
async fn a_discarded_iteration_response_reports_no_step() {
    let tier = ScriptedApi::new(&[
        (ITERATION_MODEL, &[Step::CallsReadAndWrite]),
        (
            DRAFT_MODEL,
            &[Step::CallsTool, Step::Answers("Ton plan est prêt.")],
        ),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));
    let (sink, mut events) = mpsc::unbounded_channel();

    let turn = run_turn_into(&provider, DRAFT_MODEL, 4, Some(sink)).await;

    let mut steps: Vec<(String, String, String)> = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let TurnEvent::Progress(progress) = event {
            steps.push((progress.id, progress.title, progress.status));
        }
    }
    assert_eq!(turn.result.tools_called, vec![ROUND_TRIP_TOOL.to_owned()]);
    assert_eq!(
        steps
            .iter()
            .map(|(_, title, status)| (title.as_str(), status.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (ROUND_TRIP_TOOL, TOOL_STATUS_IN_PROGRESS),
            (ROUND_TRIP_TOOL, TOOL_STATUS_COMPLETED),
        ],
        "only the call that ran is a step; the discarded read and write show nothing"
    );
    assert_eq!(
        steps[0].0, steps[1].0,
        "the closing frame names the call its started frame opened"
    );
}
