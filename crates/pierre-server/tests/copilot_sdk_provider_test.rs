// ABOUTME: The copilot_sdk provider is selectable, reaches the tool loop as a turn provider, and reports tools by name
// ABOUTME: Pins the platform side of the SDK transport: env selection, the trait-typed seam, records off the runtime's events
//
// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The headless tool loop used to downcast to `CopilotHeadlessRunner`. It now
//! asks the CLI provider for a `HeadlessTurnProvider`, which either Copilot
//! transport answers — so `PIERRE_LLM_PROVIDER=copilot_sdk` takes the same
//! native tool loop the ACP provider does, and what that loop records comes
//! off the runtime's own events: the SDK transport names the tool that ran,
//! where ACP only ever sent a display title.

mod common;

use std::env;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::config::LlmProviderType;
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, CliLlmProvider, CopilotSdkConfig,
    CopilotSdkRunner, HeadlessToolResponse, LlmCapabilities, LlmProvider, ObservedToolCall,
    StreamChunk, Tool,
};
use pierre_tool_runtime::protocol::UniversalExecutor;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution::finalize_headless_turn;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult};
use serde_json::json;
use serial_test::serial;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::common::create_test_server_resources;

const ANSWER: &str = "Belle semaine : trois sorties, la charge monte doucement.";
const SERVED_MODEL: &str = "claude-haiku-4.5";

/// Process env for one test, restored when the guard drops — on the
/// panicking path too, so a failed assertion leaks nothing into the next test.
struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, &str)]) -> Self {
        let saved = vars
            .iter()
            .map(|(k, v)| {
                let prev = env::var(k).ok();
                env::set_var(k, v);
                (*k, prev)
            })
            .collect();
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.saved.drain(..) {
            match prev {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
    }
}

#[test]
#[serial]
fn copilot_sdk_is_a_provider_type_with_its_own_name() {
    assert_eq!(
        LlmProviderType::from_str_or_default("copilot_sdk"),
        LlmProviderType::CopilotSdk
    );
    assert_eq!(
        LlmProviderType::from_str_or_default("copilot-sdk"),
        LlmProviderType::CopilotSdk
    );
    assert_eq!(LlmProviderType::CopilotSdk.to_string(), "copilot_sdk");
    assert_ne!(
        LlmProviderType::CopilotSdk,
        LlmProviderType::CopilotHeadless,
        "the two Copilot transports are distinct selections"
    );
}

#[test]
#[serial]
fn copilot_sdk_builds_from_env_and_offers_the_turn_provider() {
    // A runtime of its own: the env must be set before `from_env` reads it
    // and restored after, and both sides of that sit outside any executor.
    let env = EnvGuard::set(&[
        ("PIERRE_LLM_PROVIDER", "copilot_sdk"),
        ("PIERRE_LLM_MODEL", SERVED_MODEL),
        ("COPILOT_SDK_MCP_TOOL_CALLING", "true"),
    ]);
    let provider = Runtime::new()
        .expect("a runtime for the async builder")
        .block_on(CliLlmProvider::from_env())
        .expect("the SDK provider builds without touching the runtime; the client starts lazily");
    drop(env);

    assert_eq!(provider.name(), "copilot_sdk");
    assert_eq!(provider.display_name(), "GitHub Copilot (SDK)");
    assert_eq!(
        provider.default_model(),
        SERVED_MODEL,
        "PIERRE_LLM_MODEL is the unified override"
    );
    let capabilities = provider.capabilities();
    assert!(
        capabilities.contains(LlmCapabilities::SDK_TOOL_CALLING),
        "COPILOT_SDK_MCP_TOOL_CALLING=true routes tool turns through the native loop: {capabilities:?}"
    );
    assert!(capabilities.contains(LlmCapabilities::STREAMING));
    let turn_provider = provider
        .as_turn_provider()
        .expect("the SDK provider is reachable as a HeadlessTurnProvider, like the ACP one");
    assert_eq!(turn_provider.name(), "copilot_sdk");
}

/// A provider the loop never calls: the reply under test is complete, so no
/// degenerate-turn retry reaches it.
struct Silent {
    models: Vec<String>,
}

#[async_trait]
impl LlmProvider for Silent {
    fn name(&self) -> &'static str {
        "silent"
    }
    fn display_name(&self) -> &'static str {
        "Silent mock (never called)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        SERVED_MODEL
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }
    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Err(AppError::internal(
            "the loop must not call the provider for a complete reply",
        ))
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: String::new(),
            is_final: true,
            finish_reason: None,
        };
        Ok(Box::pin(stream::once(async move { Ok(chunk) })))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

fn reply(tool_call: ObservedToolCall) -> HeadlessToolResponse {
    HeadlessToolResponse {
        content: ANSWER.to_owned(),
        model: SERVED_MODEL.to_owned(),
        tool_calls: vec![tool_call],
        usage: None,
        finish_reason: Some("stop".to_owned()),
    }
}

/// Finish one headless turn through the production assembly with `reply`.
async fn finish(reply: HeadlessToolResponse) -> ToolLoopResult {
    let resources = create_test_server_resources().await.unwrap();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = Arc::new(UniversalExecutor::new(runtime).with_scopes(OAuthScope::self_grant()));
    let provider = ChatProvider::Custom(Arc::new(Silent {
        models: vec![SERVED_MODEL.to_owned()],
    }));
    let runner = CopilotSdkRunner::with_config(CopilotSdkConfig::default());
    let tools = Tool {
        function_declarations: Vec::new(),
    };
    let request = ChatRequest::new(Vec::new());
    let user = Uuid::new_v4().to_string();
    let params = ToolLoopParams {
        provider: &provider,
        executor,
        tools: &tools,
        model: SERVED_MODEL,
        user_id: &user,
        tenant_id: TenantId::generate(),
        max_iterations: 1,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: Vec::new(),
    };
    finalize_headless_turn(reply, &runner, &request, &params, "prompt")
        .await
        .expect("a complete reply finishes without a retry")
}

#[tokio::test]
#[serial]
async fn a_tool_the_sdk_transport_names_is_recorded_by_that_name() {
    let result = finish(reply(ObservedToolCall {
        id: "toolu_01".to_owned(),
        title: "Get activities".to_owned(),
        status: "Completed".to_owned(),
        name: Some("get_activities".to_owned()),
        arguments: Some(json!({ "limit": 3 })),
        result: Some("three rides".to_owned()),
    }))
    .await;

    assert_eq!(result.content, ANSWER);
    assert_eq!(result.tool_calls_count, 1);
    assert_eq!(
        result.tools_called,
        vec!["get_activities".to_owned()],
        "the runtime's tool name, not its display title, is what the turn records"
    );
}

#[tokio::test]
#[serial]
async fn a_tool_the_acp_adapter_only_titles_is_recorded_by_its_title() {
    let result = finish(reply(ObservedToolCall {
        id: "acp-tool-1".to_owned(),
        title: "get_activities".to_owned(),
        status: "Completed".to_owned(),
        ..ObservedToolCall::default()
    }))
    .await;

    assert_eq!(result.tool_calls_count, 1);
    assert_eq!(
        result.tools_called,
        vec!["get_activities".to_owned()],
        "with no name on the wire the title is the best identifier there is"
    );
}
