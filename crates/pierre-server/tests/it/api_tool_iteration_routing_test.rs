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
//! - (b) the turn's model may still call a tool after the re-ask, and the next
//!   iteration goes back to the iteration model;
//! - (c) a `<tool_call>` block in the iteration model's text is a tool call,
//!   not an answer;
//! - (d) the re-ask happens on the last budgeted iteration too;
//! - (e) a turn whose own model is the iteration model asks once per iteration;
//! - (f) a provider with no iteration model routes nothing;
//! - (g) a chain's tail serves a faulted turn on the turn's model, never the
//!   head's iteration model.

use crate::common;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::{create_test_server_resources, create_test_user};
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError, ToolCallRequest,
};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::call_record::{LlmCallRecord, LlmCallRecorder};
use pierre_llm::config::LlmProviderType;
use pierre_llm::stage::{StageModels, ANTHROPIC_TOOL_ITERATION_MODEL, TOOL_ITERATION_ENV};
use pierre_llm::{
    ChatMessage, ChatProvider, EmbacleProvider, FunctionDeclaration, QuotaCooldowns, Tool,
};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult};
use serde_json::json;
use uuid::Uuid;

/// A real registry tool that needs no provider connection and no fixture data.
const ROUND_TRIP_TOOL: &str = "get_connection_status";

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
    /// A final answer.
    Answers(&'static str),
    /// An HTTP 402 credit exhaustion: a provider fault the tail serves.
    CreditSpent,
}

/// An Anthropic-API-shaped tier with one script per model it is asked for,
/// recording the model of every call.
struct ScriptedApi {
    scripts: Mutex<HashMap<String, VecDeque<Step>>>,
    asked: Mutex<Vec<String>>,
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
            models: vec![DRAFT_MODEL.to_owned(), ITERATION_MODEL.to_owned()],
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
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
            Step::CallsTool => (
                String::new(),
                Some(vec![ToolCallRequest {
                    id: "toolu_1".to_owned(),
                    function_name: ROUND_TRIP_TOOL.to_owned(),
                    arguments: json!({}),
                }]),
            ),
            Step::WritesToolCall => (
                format!(
                    "<tool_call>{{\"name\":\"{ROUND_TRIP_TOOL}\",\"arguments\":{{}}}}</tool_call>"
                ),
                None,
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

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal(
            "the tool loop never streams an API tier",
        ))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
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

/// `tier` with every call on the turn's model.
fn unrouted(tier: &Arc<ScriptedApi>) -> EmbacleProvider {
    EmbacleProvider::from_runner(Box::new(Arc::clone(tier)), "Anthropic API (scripted)")
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
        function_declarations: vec![FunctionDeclaration {
            name: ROUND_TRIP_TOOL.to_owned(),
            description: "Report which fitness providers are connected".to_owned(),
            parameters: Some(json!({ "type": "object", "properties": {} })),
        }],
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
        call_recorder: Some(Arc::clone(&rows) as Arc<dyn LlmCallRecorder>),
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
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
/// next iteration starts on the iteration model again.
#[tokio::test]
async fn the_turns_model_may_still_call_a_tool_after_the_re_ask() {
    let tier = ScriptedApi::new(&[
        (
            ITERATION_MODEL,
            &[Step::Answers("cheap one"), Step::Answers("cheap two")],
        ),
        (DRAFT_MODEL, &[Step::CallsTool, Step::Answers("Connecté.")]),
    ]);
    let provider = ChatProvider::Embacle(routed(&tier));

    let turn = run_turn(&provider, DRAFT_MODEL, 4).await;

    assert_eq!(
        tier.asked(),
        models(&[ITERATION_MODEL, DRAFT_MODEL, ITERATION_MODEL, DRAFT_MODEL])
    );
    assert_eq!(turn.result.content, "Connecté.");
    assert_eq!(turn.result.tools_called, vec![ROUND_TRIP_TOOL.to_owned()]);
    assert_discarded(&turn, "cheap one");
    assert_discarded(&turn, "cheap two");
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
/// built by `ChatProvider::from_env` — runs every call on the turn's model.
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
