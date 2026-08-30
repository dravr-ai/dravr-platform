// ABOUTME: The contract a provider must satisfy to make the vendor-specific headless loop unnecessary
// ABOUTME: A provider honouring complete_with_tools drives a full tool round trip through the generic loop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! What a provider owes the platform, in executable form.
//!
//! Five HTTP providers honour `ChatProvider::complete_with_tools`. The
//! embacle-backed `Cli` arm used to return `Err(invalid_input)` and the
//! `Custom` arm hardcoded `function_calls: None` — neither asked the provider,
//! both decided for it. That refusal is why the platform's *generic* tool
//! runtime grew a third dispatch branch that downcasts to a concrete
//! `CopilotHeadlessRunner` and parses that vendor's display strings.
//!
//! These tests pin the contract that makes that branch unnecessary: a provider
//! that advertises `FUNCTION_CALLING` and returns structured tool calls drives
//! a complete round trip — call, dispatch, result, synthesis — through
//! `run_tool_loop` with no vendor knowledge anywhere in the path. That is the
//! specification embacle must satisfy for its ACP runner; it is deliberately
//! NOT evidence that any embacle runner does yet.

mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::{create_test_server_resources, create_test_user};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, FunctionDeclaration,
    LlmCapabilities, LlmProvider, StreamChunk, Tool,
};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::ToolLoopParams;
use serde_json::json;
use uuid::Uuid;

/// A real registry tool that needs no provider connection and no fixture data,
/// so the round trip turns on the contract rather than on seeded state.
const ROUND_TRIP_TOOL: &str = "get_connection_status";

/// One scripted turn: either a structured tool call with its arguments, or a
/// final answer.
enum Scripted {
    CallsTool(&'static str, serde_json::Value),
    Answers(&'static str),
}

/// A provider that honours the tool contract — the thing embacle must become.
struct ContractProvider {
    script: Mutex<VecDeque<Scripted>>,
    /// How many requests arrived carrying tool declarations. Proves the
    /// platform's tools reached the provider, not merely that it replied.
    saw_tools: AtomicUsize,
    /// Every `[Tool Result for ...]` feedback message this provider was
    /// shown, in arrival order — the tool results the loop fed back for
    /// synthesis (the API loop injects them as user messages, not tool-role
    /// messages). Lets a test assert on WHAT the tool answered, not merely
    /// that it was dispatched.
    tool_results: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl ContractProvider {
    fn new(script: Vec<Scripted>) -> Self {
        Self {
            script: Mutex::new(script.into_iter().collect()),
            saw_tools: AtomicUsize::new(0),
            tool_results: Mutex::new(Vec::new()),
            models: vec!["contract-model".to_owned()],
        }
    }
}

#[async_trait]
impl LlmProvider for ContractProvider {
    fn name(&self) -> &'static str {
        "contract"
    }
    fn display_name(&self) -> &'static str {
        "Contract Provider"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "contract-model"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        if request.tools.as_ref().is_some_and(|t| !t.is_empty()) {
            self.saw_tools.fetch_add(1, Ordering::SeqCst);
        }
        {
            let mut seen = self.tool_results.lock().unwrap();
            seen.clear();
            seen.extend(
                request
                    .messages
                    .iter()
                    .filter(|m| m.content.contains("[Tool Result for "))
                    .map(|m| m.content.clone()),
            );
        }
        let next = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .expect("contract provider ran out of script");

        let (content, tool_calls) = match next {
            Scripted::CallsTool(name, arguments) => (
                String::new(),
                Some(vec![ToolCallRequest {
                    id: "call-1".to_owned(),
                    function_name: name.to_owned(),
                    arguments,
                }]),
            ),
            Scripted::Answers(text) => (text.to_owned(), None),
        };

        Ok(ChatResponse {
            content,
            model: "contract-model".to_owned(),
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

/// The tool surface the platform advertises for the round trip.
fn advertised_tools() -> Tool {
    Tool {
        function_declarations: vec![FunctionDeclaration {
            name: ROUND_TRIP_TOOL.to_owned(),
            description: "Report which fitness providers are connected".to_owned(),
            parameters: Some(json!({ "type": "object", "properties": {} })),
        }],
    }
}

/// A provider that asks for one tool, receives its result, and answers, drives
/// the whole turn through the generic loop.
///
/// This is the claim step 1 exists to establish. Every assertion below is a
/// thing the vendor-specific headless path cannot deliver: it never sees the
/// individual tool responses, so it cannot report which tool ran, and it
/// reports display titles rather than registry names.
#[tokio::test]
async fn a_contract_provider_completes_a_tool_round_trip_through_the_generic_loop() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");

    let provider = Arc::new(ContractProvider::new(vec![
        Scripted::CallsTool(ROUND_TRIP_TOOL, json!({})),
        Scripted::Answers("Tu es connecté."),
    ]));
    let chat_provider = ChatProvider::Custom(provider.clone());
    let executor =
        Arc::new(UniversalToolExecutor::new(resources).with_turn_token("contract-turn".to_owned()));
    let tools = advertised_tools();
    let user = user_id.to_string();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let params = ToolLoopParams {
        provider: &chat_provider,
        executor,
        tools: &tools,
        model: "contract-model",
        user_id: &user,
        tenant_id: tenant,
        max_iterations: 3,
        call_recorder: None,
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
        .expect("the generic loop completes a round trip");

    // 1. The turn reached synthesis — so the tool result was fed back and a
    //    SECOND completion happened. A one-shot would return empty content.
    assert_eq!(
        result.content, "Tu es connecté.",
        "the loop must feed the tool result back and let the provider answer"
    );

    // 2. The tool was dispatched, exactly once.
    assert_eq!(
        result.tool_calls_count, 1,
        "one structured tool call must dispatch exactly one tool"
    );

    // 3. And it RAN — `tools_called` is the executed set, so a refused or
    //    unknown tool would leave it empty. This is the registry name, not a
    //    vendor display title.
    assert_eq!(
        result.tools_called,
        vec![ROUND_TRIP_TOOL.to_owned()],
        "the executed set must name the registry tool that actually ran"
    );

    // 4. The platform's declarations reached the provider. Without the request
    //    side of the contract a provider is asked to call tools it was never
    //    shown.
    assert!(
        provider.saw_tools.load(Ordering::SeqCst) >= 1,
        "the advertised tools must reach the provider on the wire"
    );

    // 5. Nothing vendor-specific was consulted: no Guardian block, no parked
    //    confirmation, no provider-auth handoff.
    assert!(result.guardian_denied.is_none());
    assert!(result.guardian_confirm.is_none());
    assert!(result.pending_provider_auth_required.is_none());
}

/// A provider that answers without calling anything still terminates cleanly.
///
/// Guards the other side of the branch: `function_calls: None` must fall
/// through to the plain-text exit rather than looping to `max_iterations`.
#[tokio::test]
async fn a_contract_provider_that_calls_nothing_answers_directly() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");

    let provider = Arc::new(ContractProvider::new(vec![Scripted::Answers("Bonjour.")]));
    let chat_provider = ChatProvider::Custom(provider.clone());
    let executor =
        Arc::new(UniversalToolExecutor::new(resources).with_turn_token("contract-none".to_owned()));
    let tools = advertised_tools();
    let user = user_id.to_string();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let params = ToolLoopParams {
        provider: &chat_provider,
        executor,
        tools: &tools,
        model: "contract-model",
        user_id: &user,
        tenant_id: tenant,
        max_iterations: 3,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: Vec::new(),
    };

    let mut messages = vec![
        ChatMessage::system("You are a test coach."),
        ChatMessage::user("bonjour"),
    ];

    let result = run_tool_loop(&params, &mut messages)
        .await
        .expect("the loop completes without tools");

    assert_eq!(result.content, "Bonjour.");
    assert_eq!(result.tool_calls_count, 0);
    assert!(result.tools_called.is_empty());
}

/// A `{"parameters": {...}}` argument envelope is lifted before dispatch, so
/// the tool body sees the flat arguments the model meant to send.
///
/// Cohere's v1 tool-call format keyed arguments under `parameters` and command
/// models still emit that shape on the v2 surface. On 2026-08-22 the
/// headless-fallback path dispatched four such calls to
/// `get_group_member_activities`; the tool saw no `member` argument and every
/// retry died on the same "Missing required 'member'" error. This pins the
/// repair at the loop level: the tool's answer must show the `member` argument
/// ARRIVED (a roster miss for the queried name), never that it was absent.
#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn a_parameters_argument_envelope_is_lifted_before_dispatch() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");

    let provider = Arc::new(ContractProvider::new(vec![
        Scripted::CallsTool(
            "get_group_member_activities",
            json!({"parameters": {"member": "Phil"}}),
        ),
        Scripted::Answers("Phil n'est pas dans ton groupe."),
    ]));
    let chat_provider = ChatProvider::Custom(provider.clone());
    let executor =
        Arc::new(UniversalToolExecutor::new(resources).with_turn_token("contract-env".to_owned()));
    let tools = advertised_tools();
    let user = user_id.to_string();
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let params = ToolLoopParams {
        provider: &chat_provider,
        executor,
        tools: &tools,
        model: "contract-model",
        user_id: &user,
        tenant_id: tenant,
        max_iterations: 3,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: Vec::new(),
    };

    let mut messages = vec![
        ChatMessage::system("You are a test coach."),
        ChatMessage::user("compare-moi avec Phil"),
    ];

    let result = run_tool_loop(&params, &mut messages)
        .await
        .expect("the loop completes the enveloped round trip");

    assert_eq!(
        result.tool_calls_count, 1,
        "the enveloped call must dispatch"
    );
    assert_eq!(
        result.content, "Phil n'est pas dans ton groupe.",
        "the turn must reach synthesis with the tool's real answer in context"
    );

    let tool_results = provider.tool_results.lock().unwrap().clone();
    let fed_back = tool_results.join("\n");
    assert!(
        fed_back.contains("No group member matching"),
        "the tool must have received member='Phil' and answered with a roster \
         miss for that name; instead the model was shown: {fed_back}"
    );
    assert!(
        !fed_back.contains("Missing required 'member'"),
        "the envelope must be lifted before dispatch — the tool must never \
         see the arguments hidden under a 'parameters' key: {fed_back}"
    );
}

/// The envelope lift is surgical: sole-key object envelopes only, and never
/// for a tool whose schema really declares a top-level `parameters` argument.
#[test]
fn parameters_envelope_lift_is_guarded() {
    use pierre_tool_runtime::function_dispatch::unwrap_parameters_envelope;

    // The observed Cohere shape is lifted.
    assert_eq!(
        unwrap_parameters_envelope(&json!({"parameters": {"member": "Phil"}}), false),
        Some(json!({"member": "Phil"}))
    );

    // A tool that really takes `parameters` (e.g. validate_configuration)
    // receives its call untouched.
    assert_eq!(
        unwrap_parameters_envelope(&json!({"parameters": {"ftp": 285}}), true),
        None
    );

    // A sibling key means the model meant what it wrote — no lift.
    assert_eq!(
        unwrap_parameters_envelope(
            &json!({"parameters": {"member": "Phil"}, "limit": 5}),
            false
        ),
        None
    );

    // A non-object `parameters` value is not an envelope.
    assert_eq!(
        unwrap_parameters_envelope(&json!({"parameters": "Phil"}), false),
        None
    );

    // Flat arguments pass through.
    assert_eq!(
        unwrap_parameters_envelope(&json!({"member": "Phil"}), false),
        None
    );
}
