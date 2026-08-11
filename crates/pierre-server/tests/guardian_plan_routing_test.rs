// ABOUTME: Plan-then-verify routing e2e — armed plan mode routes EVERY provider class through the planned loop
// ABOUTME: Scripted Custom providers: SDK-tool-calling gets the planner (no ACP); unparseable plans degrade by capability

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Plan-mode routing through the real `run_tool_loop` entry.
//!
//! The regression these pin: `GUARDIAN_PLAN_MODE=enforce` used to carve out
//! SDK-tool-calling providers (`!supports_sdk_tool_calling()`), which made the
//! knob a silent no-op on the production messaging provider (Copilot ACP).
//! Now Enforce routes every provider class through the planned loop — the
//! planner and synthesis calls are plain completions — and an unparseable
//! plan degrades to the capability-routed `ReAct` loop, never blindly to the
//! API loop.
//!
//! Both tests need `GUARDIAN_PLAN_MODE=enforce` in the environment before the
//! server resources are created (the guardian registry captures env at
//! construction), and they set the same value, so parallel execution is safe.

mod common;

use std::collections::VecDeque;
use std::env;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::{create_test_server_resources, create_test_user};
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
    StreamChunk, Tool,
};
use pierre_tool_runtime::guardian::planner_system_prompt;
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::{run_tool_loop, ToolLoopParams};
use uuid::Uuid;

/// Scripted provider: pops one canned response per `complete()` call and
/// records the first (system) message of every request it receives, so tests
/// can assert exactly which loop talked to it.
struct ScriptedProvider {
    caps: LlmCapabilities,
    responses: Mutex<VecDeque<String>>,
    first_messages: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl ScriptedProvider {
    fn new(caps: LlmCapabilities, responses: &[&str]) -> Self {
        Self {
            caps,
            responses: Mutex::new(responses.iter().map(|s| (*s).to_owned()).collect()),
            first_messages: Mutex::new(Vec::new()),
            models: vec!["scripted-model".to_owned()],
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn display_name(&self) -> &'static str {
        "Scripted Provider"
    }

    fn capabilities(&self) -> LlmCapabilities {
        self.caps
    }

    fn default_model(&self) -> &'static str {
        "scripted-model"
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.first_messages.lock().unwrap().push(
            request
                .messages
                .first()
                .map(|m| m.content.clone())
                .unwrap_or_default(),
        );
        let content = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted provider ran out of responses");
        Ok(ChatResponse {
            content,
            model: "scripted-model".to_owned(),
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
        Ok(Box::pin(stream::once(async move { Ok(chunk) })))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

struct Harness {
    provider: ChatProvider,
    scripted: Arc<ScriptedProvider>,
    executor: Arc<UniversalToolExecutor>,
    user_id: String,
    tenant: TenantId,
}

async fn harness(caps: LlmCapabilities, responses: &[&str], turn: &str) -> Harness {
    // Must be set before the resources (and their guardian registry) exist.
    env::set_var("GUARDIAN_PLAN_MODE", "enforce");

    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let scripted = Arc::new(ScriptedProvider::new(caps, responses));
    Harness {
        provider: ChatProvider::Custom(scripted.clone()),
        scripted,
        executor: Arc::new(UniversalToolExecutor::new(resources).with_turn_token(turn.to_owned())),
        user_id: user_id.to_string(),
        tenant: TenantId::from_uuid(Uuid::new_v4()),
    }
}

fn loop_params<'a>(h: &'a Harness, tools: &'a Tool) -> ToolLoopParams<'a> {
    ToolLoopParams {
        provider: &h.provider,
        executor: h.executor.clone(),
        tools,
        model: "scripted-model",
        user_id: &h.user_id,
        tenant_id: h.tenant,
        max_iterations: 3,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: Vec::new(),
    }
}

#[tokio::test]
async fn sdk_tool_calling_provider_routes_through_the_planned_loop() {
    let h = harness(
        LlmCapabilities::SDK_TOOL_CALLING | LlmCapabilities::SYSTEM_MESSAGES,
        // Planner call → empty (trivially verifiable) plan; synthesis call → answer.
        &[r#"{"steps": []}"#, "PLANNED_OK"],
        "plan-route-sdk-turn",
    )
    .await;
    let tools = Tool {
        function_declarations: Vec::new(),
    };
    let mut messages = vec![
        ChatMessage::system("You are a test coach."),
        ChatMessage::user("hello"),
    ];

    let result = run_tool_loop(&loop_params(&h, &tools), &mut messages)
        .await
        .expect("planned loop completes");

    // The answer came from the synthesis call — the ACP subprocess loop was
    // never entered (it would have failed: no copilot binary in tests).
    assert_eq!(result.content, "PLANNED_OK");
    assert_eq!(result.tool_calls_count, 0);
    assert!(result.tools_called.is_empty());
    assert!(result.guardian_denied.is_none());

    // Exactly two completions: plan + synthesis; and the FIRST carried the
    // planner prompt folded into the system message.
    let first_messages = h.scripted.first_messages.lock().unwrap();
    assert_eq!(first_messages.len(), 2);
    assert!(
        first_messages[0].starts_with(&planner_system_prompt()),
        "planner call must lead with the planner prompt"
    );
    assert!(
        first_messages[0].contains("You are a test coach."),
        "planner prompt must be FOLDED into the persona system message, not replace it"
    );
    assert!(
        !first_messages[1].starts_with(&planner_system_prompt()),
        "synthesis call must not re-send the planner prompt"
    );
}

#[tokio::test]
async fn unparseable_plan_degrades_to_the_capability_routed_react_loop() {
    let h = harness(
        // A text-CLI-class provider: no function calling, no SDK tool calling.
        LlmCapabilities::SYSTEM_MESSAGES,
        // Planner emits garbage → degrade; the ReAct CLI loop then completes
        // with a plain final answer (no tool_call block).
        &["this is not a plan", "REACT_OK"],
        "plan-route-degrade-turn",
    )
    .await;
    let tools = Tool {
        function_declarations: Vec::new(),
    };
    let mut messages = vec![
        ChatMessage::system("You are a test coach."),
        ChatMessage::user("hello"),
    ];

    let result = run_tool_loop(&loop_params(&h, &tools), &mut messages)
        .await
        .expect("degraded ReAct loop completes");

    assert_eq!(result.content, "REACT_OK");
    assert!(result.guardian_denied.is_none());

    let first_messages = h.scripted.first_messages.lock().unwrap();
    assert_eq!(
        first_messages.len(),
        2,
        "planner call + one ReAct completion"
    );
    assert!(
        first_messages[0].starts_with(&planner_system_prompt()),
        "first call was the planner"
    );
    assert!(
        !first_messages[1].starts_with(&planner_system_prompt()),
        "degraded ReAct call must use the original messages, without the planner prompt"
    );
}
