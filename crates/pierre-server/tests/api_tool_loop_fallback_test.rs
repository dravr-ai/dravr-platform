// ABOUTME: A FUNCTION_CALLING chain head that faults hands the turn to the tail by the TAIL's capability
// ABOUTME: Pins that a Copilot tail gets its MCP servers through the headless loop, and that a head cooling on quota is skipped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Sent through the whole embacle chain, the API tool loop's tool-carrying
//! request would reach `copilot_sdk.complete()` when an Anthropic head
//! answered 429 or 402 — a session with an empty tool allowlist when the
//! request carries no MCP servers, so the athlete would get a fluent reply
//! with no data behind it.
//!
//! The loop therefore converses with the chain's head alone and, on a
//! provider fault or a lost turn, re-runs the whole turn on the chain's tail,
//! routed by the tail head's own capabilities. These tests drive scripted
//! tiers through `run_tool_loop`:
//!
//! - (a) the head's quota refusal sends the turn to a Copilot-shaped tail
//!   through the headless loop, which hands it the turn's MCP servers;
//! - (b) a head that faults on its second call leaves its tool results with the
//!   tail, and the turn still records the tool the head ran;
//! - (c) a second Anthropic account behind the first keeps the API loop;
//! - (d) an empty answer from the head is a lost turn the tail serves;
//! - (e) a head that spends its iteration budget is not rerouted;
//! - (f) a head error that is not a provider fault is not rerouted;
//! - (g) driven through the chat pipeline, an Anthropic head opens the turn's
//!   MCP tool session for the Copilot tier behind it;
//! - (h) an API head refused on quota is not asked again while its cooldown in
//!   the chain's shared book runs, and is asked again once it ends;
//! - (i) the same for a Copilot head on the headless loop;
//! - (j) an API head that faults for any reason but quota starts no cooldown:
//!   the tail serves that turn, and the head is asked again on the next one;
//! - (k) the same for a Copilot head on the headless loop.

mod common;

use std::collections::VecDeque;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{
    create_test_server_resources, create_test_server_resources_with_real_chat_provider,
    create_test_user, create_test_user_with_plan,
};
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError, ToolCallRequest,
};
use pierre_chat_pipeline::{
    CommandPersistence, InputSource, PipelineHooks, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnOrigin, TurnRequest,
};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_llm::{
    ChatMessage, ChatProvider, EmbacleProvider, FunctionDeclaration, HeadlessEventStream,
    HeadlessToolResponse, HeadlessTurnProvider, McpServerConfig, McpTransport, ObservedToolCall,
    QuotaCooldowns, Tool,
};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult, PLATFORM_MCP_SERVER_NAME};
use serde_json::json;
use serial_test::serial;
use tokio::time::sleep;
use uuid::Uuid;

/// A real registry tool that needs no provider connection and no fixture data.
const ROUND_TRIP_TOOL: &str = "get_connection_status";

/// What the Copilot-shaped tail answers.
const TAIL_ANSWER: &str = "Tu es connecté à Strava.";

/// The MCP server name the turn hands the Copilot runtime.
const TURN_MCP_SERVER: &str = PLATFORM_MCP_SERVER_NAME;

/// One scripted reply of an API-shaped tier.
#[derive(Clone, Copy)]
enum Step {
    /// An HTTP 402 credit exhaustion, as embacle maps it.
    CreditSpent,
    /// An HTTP 503 from the provider: a provider fault that is not a quota
    /// refusal, so the turn falls to the tail but no cooldown starts.
    Unavailable,
    /// An HTTP 400 the provider read and refused: the caller's fault, not the
    /// provider's, so no other tier would answer it differently.
    Rejects,
    /// A structured call of `ROUND_TRIP_TOOL`.
    CallsTool,
    /// A final answer.
    Answers(&'static str),
}

/// An Anthropic-API-shaped tier: native function calling, a scripted reply per
/// call, and a record of what it was asked.
struct ScriptedApi {
    name: &'static str,
    script: Mutex<VecDeque<Step>>,
    /// What every call answers once the script is spent; `None` fails the
    /// test instead.
    then: Option<Step>,
    calls: AtomicUsize,
    calls_with_tools: AtomicUsize,
    /// The tool-result feedback messages the last call carried.
    tool_results_seen: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl ScriptedApi {
    fn new(name: &'static str, script: Vec<Step>) -> Arc<Self> {
        Self::scripted(name, script, None)
    }

    /// An account whose credit is spent for good: every call is refused, the
    /// pipeline's own background calls through the chain included.
    fn spent(name: &'static str) -> Arc<Self> {
        Self::scripted(name, Vec::new(), Some(Step::CreditSpent))
    }

    fn scripted(name: &'static str, script: Vec<Step>, then: Option<Step>) -> Arc<Self> {
        Arc::new(Self {
            name,
            script: Mutex::new(script.into_iter().collect()),
            then,
            calls: AtomicUsize::new(0),
            calls_with_tools: AtomicUsize::new(0),
            tool_results_seen: Mutex::new(Vec::new()),
            models: vec!["claude-haiku-5-5".to_owned()],
        })
    }
}

#[async_trait]
impl EmbacleLlmProvider for ScriptedApi {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &str {
        self.name
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        if request.tools.as_ref().is_some_and(|t| !t.is_empty()) {
            self.calls_with_tools.fetch_add(1, Ordering::SeqCst);
        }
        *self.tool_results_seen.lock().unwrap() = request
            .messages
            .iter()
            .filter(|m| m.content.contains("[Tool Result for "))
            .map(|m| m.content.clone())
            .collect();
        let step = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .or(self.then)
            .unwrap_or_else(|| panic!("{} ran out of script", self.name));
        let (content, tool_calls) = match step {
            Step::CreditSpent => {
                return Err(RunnerError::rate_limit(
                    self.name,
                    "credit balance exhausted: Your credit balance is too low",
                ))
            }
            Step::Unavailable => {
                return Err(RunnerError::external_service(
                    self.name,
                    "503 Service Unavailable: upstream overloaded",
                ))
            }
            Step::Rejects => {
                return Err(RunnerError::invalid_request(
                    "messages: tool_result block does not match a tool_use id",
                ))
            }
            Step::CallsTool => (
                String::new(),
                Some(vec![ToolCallRequest {
                    id: "toolu_1".to_owned(),
                    function_name: ROUND_TRIP_TOOL.to_owned(),
                    arguments: json!({}),
                }]),
            ),
            Step::Answers(text) => (text.to_owned(), None),
        };
        Ok(ChatResponse {
            content,
            model: self.models[0].clone(),
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

/// A Copilot-SDK-shaped tier: SDK tool calling through `converse()`, which
/// records the request it was handed. A bare `complete()` is the bug under
/// test, so it is counted.
struct ScriptedSdk {
    name: &'static str,
    /// How many `converse()` calls still fail before the tier answers.
    refusals_left: AtomicUsize,
    /// Whether those failures are quota refusals (`RateLimit`) or a provider
    /// fault that is not one (`ExternalService`).
    refuses_on_quota: bool,
    conversed: Mutex<Vec<ChatRequest>>,
    completes: AtomicUsize,
    models: Vec<String>,
}

impl ScriptedSdk {
    fn new() -> Arc<Self> {
        Self::refusing("copilot_sdk", 0)
    }

    /// A tier whose first `refusals` turns are refused on quota, as a spent
    /// Copilot premium-request budget is.
    fn refusing(name: &'static str, refusals: usize) -> Arc<Self> {
        Self::failing(name, refusals, true)
    }

    /// A tier whose first `faults` turns fail with a provider fault that is
    /// not a quota refusal, as a Copilot runtime outage does.
    fn faulting(name: &'static str, faults: usize) -> Arc<Self> {
        Self::failing(name, faults, false)
    }

    fn failing(name: &'static str, failures: usize, on_quota: bool) -> Arc<Self> {
        Arc::new(Self {
            name,
            refusals_left: AtomicUsize::new(failures),
            refuses_on_quota: on_quota,
            conversed: Mutex::new(Vec::new()),
            completes: AtomicUsize::new(0),
            models: vec!["claude-sonnet-5.5".to_owned()],
        })
    }
}

#[async_trait]
impl EmbacleLlmProvider for ScriptedSdk {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &'static str {
        "Copilot SDK (scripted)"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING
            | LlmCapabilities::SYSTEM_MESSAGES
            | LlmCapabilities::SDK_TOOL_CALLING
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        // What embacle's chain used to do with the head's tool-carrying
        // request: a session with no tools, answering without data.
        self.completes.fetch_add(1, Ordering::SeqCst);
        Ok(ChatResponse {
            content: "ungrounded".to_owned(),
            model: self.models[0].clone(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("not asked in this test"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

#[async_trait]
impl HeadlessTurnProvider for ScriptedSdk {
    async fn converse(&self, request: &ChatRequest) -> Result<HeadlessToolResponse, RunnerError> {
        self.conversed.lock().unwrap().push(request.clone());
        let refused = self
            .refusals_left
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if refused {
            return Err(if self.refuses_on_quota {
                RunnerError::rate_limit(
                    self.name,
                    "402 Payment Required: premium request quota exhausted",
                )
            } else {
                RunnerError::external_service(self.name, "Copilot runtime exited unexpectedly")
            });
        }
        Ok(HeadlessToolResponse {
            content: TAIL_ANSWER.to_owned(),
            model: self.models[0].clone(),
            tool_calls: vec![ObservedToolCall {
                id: "call-1".to_owned(),
                title: "Checking connections".to_owned(),
                status: "Completed".to_owned(),
                name: Some(format!("{TURN_MCP_SERVER}-{ROUND_TRIP_TOOL}")),
                arguments: Some(json!({})),
                result: Some("{\"strava\":\"connected\"}".to_owned()),
            }],
            usage: None,
            finish_reason: Some("stop".to_owned()),
        })
    }

    async fn converse_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<HeadlessEventStream, RunnerError> {
        Err(RunnerError::internal("the test turn does not stream"))
    }
}

/// An API tier as a chain tier.
fn api_tier(tier: &Arc<ScriptedApi>) -> EmbacleProvider {
    EmbacleProvider::from_runner(Box::new(Arc::clone(tier)), "Anthropic API (scripted)")
}

/// The Copilot tier as a chain tier, reachable as a turn provider.
fn sdk_tier(tier: &Arc<ScriptedSdk>) -> EmbacleProvider {
    let turn_provider: Arc<dyn HeadlessTurnProvider> = Arc::clone(tier) as _;
    EmbacleProvider::from_turn_provider(turn_provider, "GitHub Copilot (SDK)")
}

/// The MCP server a turn hands the Copilot runtime.
fn turn_mcp_servers() -> Vec<McpServerConfig> {
    vec![McpServerConfig {
        name: TURN_MCP_SERVER.to_owned(),
        transport: McpTransport::Http {
            url: "http://127.0.0.1:8081/mcp".to_owned(),
            headers: Vec::new(),
        },
    }]
}

/// What every turn of a test shares: one database, one user, one executor.
struct TurnContext {
    executor: Arc<UniversalToolExecutor>,
    user: String,
    tools: Tool,
}

impl TurnContext {
    async fn new() -> Self {
        let resources = create_test_server_resources()
            .await
            .expect("server resources");
        let (user_id, _) = create_test_user(&resources.agent.database)
            .await
            .expect("test user");
        let executor = Arc::new(
            UniversalToolExecutor::new(resources)
                .with_scopes(OAuthScope::self_grant())
                .with_turn_token("fallback-turn".to_owned()),
        );
        let tools = Tool {
            function_declarations: vec![FunctionDeclaration {
                name: ROUND_TRIP_TOOL.to_owned(),
                description: "Report which fitness providers are connected".to_owned(),
                parameters: Some(json!({ "type": "object", "properties": {} })),
            }],
        };
        Self {
            executor,
            user: user_id.to_string(),
            tools,
        }
    }

    /// Run one turn of the production tool loop on `chain`.
    async fn run(&self, chain: &ChatProvider) -> (ToolLoopResult, Vec<ChatMessage>) {
        let params = ToolLoopParams {
            provider: chain,
            executor: Arc::clone(&self.executor),
            tools: &self.tools,
            model: chain.default_model(),
            user_id: &self.user,
            tenant_id: TenantId::from_uuid(Uuid::new_v4()),
            max_iterations: 4,
            call_recorder: None,
            tool_message_recorder: None,
            temperature: None,
            stream_sink: None,
            mcp_servers: turn_mcp_servers(),
        };
        let mut messages = vec![
            ChatMessage::system("You are a test coach."),
            ChatMessage::user("suis-je connecté ?"),
        ];
        let result = run_tool_loop(&params, &mut messages)
            .await
            .expect("the turn is served by a tier behind the faulting head");
        (result, messages)
    }
}

/// Run one turn of the production tool loop on `chain`.
async fn run_turn(chain: &ChatProvider) -> (ToolLoopResult, Vec<ChatMessage>) {
    TurnContext::new().await.run(chain).await
}

/// (a) A spent Anthropic credit on the first call: the Copilot tail serves the
/// turn through the headless loop, with the turn's MCP servers.
#[tokio::test]
async fn a_spent_head_hands_the_turn_to_a_copilot_tail_with_its_tools() {
    let head = ScriptedApi::new("anthropic", vec![Step::CreditSpent]);
    let tail = ScriptedSdk::new();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![api_tier(&head), sdk_tier(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    );
    assert!(
        chain.capabilities().supports_function_calling(),
        "the Anthropic head routes the turn to the API loop"
    );

    let (result, _) = run_turn(&chain).await;

    assert_eq!(
        head.calls.load(Ordering::SeqCst),
        1,
        "the head is asked once"
    );
    assert_eq!(
        head.calls_with_tools.load(Ordering::SeqCst),
        1,
        "the head got the tool declarations"
    );
    assert_eq!(
        tail.completes.load(Ordering::SeqCst),
        0,
        "the head's tool-carrying request must never reach the Copilot tier as a bare \
         complete() — that session has no tools and answers ungrounded"
    );
    let conversed = tail.conversed.lock().unwrap();
    assert_eq!(conversed.len(), 1, "the tail ran the headless loop once");
    assert_eq!(
        conversed[0]
            .mcp_servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec![TURN_MCP_SERVER],
        "the Copilot runtime was handed the turn's MCP server, so it can call the tools"
    );
    assert_eq!(
        conversed[0].model.as_deref(),
        Some("claude-sonnet-5.5"),
        "the tail runs its own model, never the head's"
    );
    assert_eq!(result.content, TAIL_ANSWER);
    assert_eq!(
        result.tools_called,
        vec![ROUND_TRIP_TOOL.to_owned()],
        "the tool the tail ran over the bridge is recorded by its registry name"
    );
}

/// (b) The head runs a tool, then faults: the tail continues with the tool
/// result the head gathered, and the turn keeps the head's tool on record.
#[tokio::test]
async fn a_head_that_faults_mid_loop_leaves_its_tool_results_with_the_tail() {
    let head = ScriptedApi::new("anthropic", vec![Step::CallsTool, Step::CreditSpent]);
    let tail = ScriptedSdk::new();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![api_tier(&head), sdk_tier(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    );

    let (result, messages) = run_turn(&chain).await;

    assert_eq!(head.calls.load(Ordering::SeqCst), 2);
    assert_eq!(tail.completes.load(Ordering::SeqCst), 0);
    let conversed = tail.conversed.lock().unwrap();
    assert_eq!(conversed.len(), 1);
    assert!(
        conversed[0].messages.iter().any(|m| m
            .content
            .contains(&format!("[Tool Result for {ROUND_TRIP_TOOL}"))),
        "the tail is handed the tool result the head's first round produced"
    );
    assert!(
        messages.iter().any(|m| m
            .content
            .contains(&format!("[Tool Result for {ROUND_TRIP_TOOL}"))),
        "the turn's messages keep the head's round"
    );
    assert_eq!(result.content, TAIL_ANSWER);
    assert_eq!(
        result.tools_called,
        vec![ROUND_TRIP_TOOL.to_owned(), ROUND_TRIP_TOOL.to_owned()],
        "the head's executed tool stays on record ahead of the tail's"
    );
    assert_eq!(
        result.tool_calls_count, 2,
        "both tiers' tool calls count for the turn"
    );
}

/// (c) A second Anthropic account behind the first keeps the API loop: it is
/// asked with the tools, runs one, answers, and the Copilot tier is never asked.
#[tokio::test]
async fn a_pooled_api_account_behind_the_head_keeps_the_api_loop() {
    let head = ScriptedApi::new("anthropic", vec![Step::CreditSpent]);
    let second = ScriptedApi::new(
        "anthropic#2",
        vec![Step::CallsTool, Step::Answers("Strava est connecté.")],
    );
    let tail = ScriptedSdk::new();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![api_tier(&head), api_tier(&second), sdk_tier(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    );

    let (result, _) = run_turn(&chain).await;

    assert_eq!(head.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        second.calls.load(Ordering::SeqCst),
        2,
        "account 2 runs the whole API loop: a tool round, then the answer"
    );
    assert_eq!(
        second.calls_with_tools.load(Ordering::SeqCst),
        2,
        "account 2 is asked with the native tool declarations every round"
    );
    assert_eq!(
        second.tool_results_seen.lock().unwrap().len(),
        1,
        "account 2 saw its own tool result on its second round"
    );
    assert_eq!(tail.completes.load(Ordering::SeqCst), 0);
    assert!(
        tail.conversed.lock().unwrap().is_empty(),
        "the Copilot tier behind the pool is never reached"
    );
    assert_eq!(result.content, "Strava est connecté.");
    assert_eq!(result.tools_called, vec![ROUND_TRIP_TOOL.to_owned()]);
}

/// A single API provider keeps today's behaviour: its error comes back as is,
/// with no tail to try.
#[tokio::test]
async fn a_solo_api_provider_returns_its_own_fault() {
    let solo = ScriptedApi::new("anthropic", vec![Step::CreditSpent]);
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![api_tier(&solo)], QuotaCooldowns::default()).unwrap(),
    );
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let executor =
        Arc::new(UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()));
    let tools = Tool {
        function_declarations: Vec::new(),
    };
    let user = Uuid::new_v4().to_string();
    let params = ToolLoopParams {
        provider: &chain,
        executor,
        tools: &tools,
        model: chain.default_model(),
        user_id: &user,
        tenant_id: TenantId::from_uuid(Uuid::new_v4()),
        max_iterations: 2,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: Vec::new(),
    };
    let mut messages = vec![ChatMessage::user("bonjour")];
    let Err(err) = run_tool_loop(&params, &mut messages).await else {
        panic!("a solo provider has nothing to fall back to");
    };
    assert!(
        err.message.contains("anthropic: credit balance exhausted"),
        "the head's own error comes back: {}",
        err.message
    );
}

/// (d) A head that answers with nothing has lost the turn: the tail serves it.
#[tokio::test]
async fn a_head_that_answers_empty_hands_the_turn_to_the_tail() {
    let head = ScriptedApi::new("anthropic", vec![Step::Answers("")]);
    let tail = ScriptedSdk::new();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![api_tier(&head), sdk_tier(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    );

    let (result, _) = run_turn(&chain).await;

    assert_eq!(head.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        tail.conversed.lock().unwrap().len(),
        1,
        "an empty answer is a lost turn, and a lost turn is re-run on the tail"
    );
    assert_eq!(result.content, TAIL_ANSWER);
}

/// (e) A head that keeps calling tools until the budget runs out has not lost
/// the turn: a second tier would spend a second budget on the same loop.
#[tokio::test]
async fn a_head_that_runs_out_of_iterations_stays_off_the_tail() {
    let head = ScriptedApi::new(
        "anthropic",
        vec![
            Step::CallsTool,
            Step::CallsTool,
            Step::CallsTool,
            Step::CallsTool,
        ],
    );
    let tail = ScriptedSdk::new();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![api_tier(&head), sdk_tier(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    );

    let (result, _) = run_turn(&chain).await;

    assert_eq!(
        head.calls.load(Ordering::SeqCst),
        4,
        "the head spends the whole budget of 4"
    );
    assert!(
        tail.conversed.lock().unwrap().is_empty(),
        "an exhausted budget is not a lost turn: the tail is never asked"
    );
    assert_eq!(tail.completes.load(Ordering::SeqCst), 0);
    assert_eq!(result.finish_reason.as_deref(), Some("max_iterations"));
    assert_eq!(result.tool_calls_count, 4);
}

/// (f) A head error that is not a provider fault comes back as it is: the
/// request was refused as malformed, which no other tier would answer.
#[tokio::test]
async fn a_head_error_that_is_not_a_provider_fault_stays_on_the_head() {
    let head = ScriptedApi::new("anthropic", vec![Step::Rejects]);
    let tail = ScriptedSdk::new();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![api_tier(&head), sdk_tier(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    );
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let executor =
        Arc::new(UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()));
    let tools = Tool {
        function_declarations: Vec::new(),
    };
    let user = Uuid::new_v4().to_string();
    let params = ToolLoopParams {
        provider: &chain,
        executor,
        tools: &tools,
        model: chain.default_model(),
        user_id: &user,
        tenant_id: TenantId::from_uuid(Uuid::new_v4()),
        max_iterations: 2,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: turn_mcp_servers(),
    };
    let mut messages = vec![ChatMessage::user("bonjour")];
    let Err(err) = run_tool_loop(&params, &mut messages).await else {
        panic!("a refused request is not served by another tier");
    };

    assert_eq!(err.code, ErrorCode::InvalidInput, "{}", err.message);
    assert!(
        tail.conversed.lock().unwrap().is_empty(),
        "a caller-side refusal never reaches the tail"
    );
    assert_eq!(tail.completes.load(Ordering::SeqCst), 0);
}

/// (g) Through the chat pipeline: an Anthropic head opens the turn's MCP tool
/// session for the Copilot tier behind it.
///
/// Tests (a)-(c) hand `run_tool_loop` its MCP servers; in production the only
/// thing that opens them is the pipeline's tool dispatch. Asked of the head
/// alone, a function-calling head would answer no, the turn would carry no
/// servers, and the Copilot tier the turn fell to would answer with no tools
/// behind it.
#[tokio::test]
#[serial]
async fn the_pipeline_opens_the_tool_session_for_a_copilot_tier_behind_an_api_head() {
    let head = ScriptedApi::spent("anthropic");
    let tail = ScriptedSdk::new();
    let chain = Arc::new(ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![api_tier(&head), sdk_tier(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    ));
    let resources = create_test_server_resources_with_real_chat_provider(Arc::clone(&chain))
        .await
        .expect("server resources");
    let (user_id, _user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        "anthropic-head-bridge@test.com",
        "professional",
    )
    .await
    .expect("test user");
    // The bridge is armed by the Copilot tool-calling toggle, read once when
    // the pipeline context is built — as a deployed Copilot tier has it.
    let ctx = {
        let _armed = EnvGuard::set("COPILOT_SDK_MCP_TOOL_CALLING", "true");
        resources.chat_pipeline_context()
    };
    let conversation_id = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant_id,
            "Connexions",
            "claude-haiku-5-5",
            None,
            None,
        )
        .await
        .expect("conversation")
        .id;

    pierre_chat_pipeline::execute(
        &ctx,
        TurnRequest {
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            conversation_id,
            user_id,
            conversation_tenant_id: tenant_id,
            tool_tenant_id: tenant_id,
            content: "suis-je connecté à Strava ?".to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            channel_type: "web",
            transport: Transport::WebApp,
            is_direct_message: true,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            hooks: PipelineHooks::none(),
        },
        &SurfaceProfile::resolve(&SurfaceRequest {
            surface: SurfaceId::Web,
            locale: "fr".to_owned(),
            transport: None,
            prose_contract: None,
        }),
    )
    .await
    .expect("the turn is served by the Copilot tier");

    assert!(
        head.calls_with_tools.load(Ordering::SeqCst) >= 1,
        "premise: the turn was routed to the Anthropic head's API loop"
    );
    let conversed = tail.conversed.lock().unwrap();
    let first = conversed
        .first()
        .expect("the spent head handed the turn to the Copilot tier's headless loop");
    assert_eq!(
        first
            .mcp_servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec![PLATFORM_MCP_SERVER_NAME],
        "the Copilot tier was handed the turn's MCP server, so it can call the tools"
    );
}

/// Sets one environment variable and restores what it was on drop.
struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = env::var(key).ok();
        env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => env::set_var(self.key, value),
            None => env::remove_var(self.key),
        }
    }
}

/// The window the cooldown tests run under: long enough that a scripted turn
/// fits inside it many times over, short enough to wait out.
const TEST_COOLDOWN: Duration = Duration::from_secs(2);

/// Wait until `tier`'s window in `book` has run out.
async fn wait_out(book: &QuotaCooldowns, tier: &str) {
    let left = book.remaining(tier).expect("the tier is cooling");
    sleep(left + Duration::from_millis(50)).await;
    assert!(!book.is_cooling(tier), "the window is over");
}

/// (h) An API head refused on quota is not asked again while its cooldown
/// runs: the next turn on the same chain goes straight to the tail, and once
/// the window is over the head is asked again.
#[tokio::test]
async fn an_api_head_refused_on_quota_is_skipped_until_its_cooldown_ends() {
    let head = ScriptedApi::new(
        "anthropic",
        vec![Step::CreditSpent, Step::Answers("Le crédit est revenu.")],
    );
    let tail = ScriptedSdk::new();
    let book = QuotaCooldowns::new(TEST_COOLDOWN);
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![api_tier(&head), sdk_tier(&tail)], book.clone()).unwrap(),
    );
    let turns = TurnContext::new().await;

    let (first, _) = turns.run(&chain).await;
    assert_eq!(head.calls.load(Ordering::SeqCst), 1, "turn 1 asks the head");
    assert_eq!(first.content, TAIL_ANSWER, "turn 1 is served by the tail");
    assert!(
        book.is_cooling("anthropic"),
        "the head's 402 started its cooldown in the chain's book"
    );

    let (second, _) = turns.run(&chain).await;
    assert_eq!(
        head.calls.load(Ordering::SeqCst),
        1,
        "turn 2 must not ask a head inside its quota cooldown"
    );
    assert_eq!(second.content, TAIL_ANSWER, "turn 2 is served by the tail");
    assert_eq!(tail.conversed.lock().unwrap().len(), 2);

    wait_out(&book, "anthropic").await;
    let (third, _) = turns.run(&chain).await;
    assert_eq!(
        head.calls.load(Ordering::SeqCst),
        2,
        "after the window the head is asked again"
    );
    assert_eq!(third.content, "Le crédit est revenu.");
    assert_eq!(
        tail.conversed.lock().unwrap().len(),
        2,
        "the recovered head served turn 3 itself"
    );
}

/// (i) The same for a Copilot head on the headless loop: a quota refusal
/// starts its cooldown, the next turn goes to the tail without conversing
/// with it, and after the window it is asked again.
#[tokio::test]
async fn a_headless_head_refused_on_quota_is_skipped_until_its_cooldown_ends() {
    let head = ScriptedSdk::refusing("copilot_sdk", 1);
    let tail = ScriptedApi::scripted(
        "anthropic",
        Vec::new(),
        Some(Step::Answers("Réponse du second palier.")),
    );
    let book = QuotaCooldowns::new(TEST_COOLDOWN);
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![sdk_tier(&head), api_tier(&tail)], book.clone()).unwrap(),
    );
    assert!(
        chain.capabilities().supports_sdk_tool_calling(),
        "the Copilot head routes the turn to the headless loop"
    );
    let turns = TurnContext::new().await;

    let (first, _) = turns.run(&chain).await;
    assert_eq!(
        head.conversed.lock().unwrap().len(),
        1,
        "turn 1 asks the head"
    );
    assert_eq!(first.content, "Réponse du second palier.");
    assert!(
        book.is_cooling("copilot_sdk"),
        "the head's quota refusal started its cooldown in the chain's book"
    );

    let (second, _) = turns.run(&chain).await;
    assert_eq!(
        head.conversed.lock().unwrap().len(),
        1,
        "turn 2 must not converse with a head inside its quota cooldown"
    );
    assert_eq!(second.content, "Réponse du second palier.");
    assert_eq!(tail.calls.load(Ordering::SeqCst), 2);

    wait_out(&book, "copilot_sdk").await;
    let (third, _) = turns.run(&chain).await;
    assert_eq!(
        head.conversed.lock().unwrap().len(),
        2,
        "after the window the head is asked again"
    );
    assert_eq!(
        third.content, TAIL_ANSWER,
        "the recovered head served turn 3"
    );
    assert_eq!(tail.calls.load(Ordering::SeqCst), 2);
}

/// (j) An API head that faults for a reason other than quota hands the turn
/// to the tail but starts no cooldown: the next turn asks it again.
#[tokio::test]
async fn an_api_head_fault_that_is_not_quota_starts_no_cooldown() {
    let head = ScriptedApi::new(
        "anthropic",
        vec![Step::Unavailable, Step::Answers("Le service est revenu.")],
    );
    let tail = ScriptedSdk::new();
    let book = QuotaCooldowns::new(TEST_COOLDOWN);
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![api_tier(&head), sdk_tier(&tail)], book.clone()).unwrap(),
    );
    let turns = TurnContext::new().await;

    let (first, _) = turns.run(&chain).await;
    assert_eq!(head.calls.load(Ordering::SeqCst), 1, "turn 1 asks the head");
    assert_eq!(first.content, TAIL_ANSWER, "turn 1 is served by the tail");
    assert!(
        !book.is_cooling("anthropic"),
        "a 503 is the provider's fault but not a quota refusal: no cooldown"
    );

    let (second, _) = turns.run(&chain).await;
    assert_eq!(
        head.calls.load(Ordering::SeqCst),
        2,
        "turn 2 asks the head again, since no window was started"
    );
    assert_eq!(second.content, "Le service est revenu.");
    assert_eq!(
        tail.conversed.lock().unwrap().len(),
        1,
        "the head served turn 2 itself"
    );
}

/// (k) The same for a Copilot head on the headless loop: a runtime fault
/// that is not a quota refusal falls to the tail and starts no cooldown.
#[tokio::test]
async fn a_headless_head_fault_that_is_not_quota_starts_no_cooldown() {
    let head = ScriptedSdk::faulting("copilot_sdk", 1);
    let tail = ScriptedApi::scripted(
        "anthropic",
        Vec::new(),
        Some(Step::Answers("Réponse du second palier.")),
    );
    let book = QuotaCooldowns::new(TEST_COOLDOWN);
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![sdk_tier(&head), api_tier(&tail)], book.clone()).unwrap(),
    );
    let turns = TurnContext::new().await;

    let (first, _) = turns.run(&chain).await;
    assert_eq!(
        head.conversed.lock().unwrap().len(),
        1,
        "turn 1 asks the head"
    );
    assert_eq!(first.content, "Réponse du second palier.");
    assert!(
        !book.is_cooling("copilot_sdk"),
        "a runtime fault is the provider's fault but not a quota refusal: no cooldown"
    );

    let (second, _) = turns.run(&chain).await;
    assert_eq!(
        head.conversed.lock().unwrap().len(),
        2,
        "turn 2 converses with the head again, since no window was started"
    );
    assert_eq!(second.content, TAIL_ANSWER, "the head served turn 2 itself");
    assert_eq!(tail.calls.load(Ordering::SeqCst), 1);
}
