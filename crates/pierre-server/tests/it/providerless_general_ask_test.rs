// ABOUTME: An athlete with no provider asking a general question gets the model's answer, not a reconnect
// ABOUTME: The no-provider refusal must not stop the tool loop or name a provider to reconnect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Found by the 2026-10-23 onboarding e2e on dev: a fresh athlete who chose
//! "continue without connecting" asked the fuelling agent what to eat before a
//! long ride. The model reached for a data tool, the dispatch chokepoint
//! refused it as providerless, and that refusal carried the RE-AUTH signal
//! stamped `sciotte`. The tool loop stopped, `auth_recovery` replaced the turn,
//! and the athlete read "your Strava connection expired" — for an account that
//! never connected Strava — with no answer to a question that needed no data.
//!
//! This drives the real loop with a scripted provider that calls a data tool,
//! then answers from what the refusal told it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::{create_test_server_resources, create_test_user};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, FunctionDeclaration,
    LlmCapabilities, LlmProvider, StreamChunk, Tool,
};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::ToolLoopParams;
use serde_json::json;

/// What the scripted model answers once the tool told it there is no data.
const GENERAL_ANSWER: &str = "Mange des glucides faciles à digérer 2 à 3 heures avant la sortie.";

/// A model that calls `get_activities` first, then answers in prose, keeping
/// every request it was sent so the test can read what the refusal told it.
struct DataFirstProvider {
    requests: Mutex<Vec<ChatRequest>>,
    models: Vec<String>,
}

#[async_trait]
impl LlmProvider for DataFirstProvider {
    fn name(&self) -> &'static str {
        "data-first"
    }
    fn display_name(&self) -> &'static str {
        "Data-first Provider"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "data-first-model"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let mut requests = self.requests.lock().unwrap();
        let first = requests.is_empty();
        requests.push(request.clone());
        let (content, tool_calls) = if first {
            (
                String::new(),
                Some(vec![ToolCallRequest {
                    id: "call-activities".to_owned(),
                    function_name: "get_activities".to_owned(),
                    arguments: json!({ "limit": 5, "mode": "summary" }),
                }]),
            )
        } else {
            (GENERAL_ANSWER.to_owned(), None)
        };
        Ok(ChatResponse {
            content,
            model: "data-first-model".to_owned(),
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

#[tokio::test]
async fn a_providerless_general_ask_keeps_the_answer_and_names_no_provider() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;

    let provider = Arc::new(DataFirstProvider {
        requests: Mutex::new(Vec::new()),
        models: vec!["data-first-model".to_owned()],
    });
    let chat_provider = ChatProvider::Custom(Arc::<DataFirstProvider>::clone(&provider));
    let executor = Arc::new(
        UniversalToolExecutor::new(resources)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp)
            .with_turn_token("providerless-turn".to_owned()),
    );
    let tools = Tool {
        function_declarations: vec![FunctionDeclaration::new(
            "get_activities".to_owned(),
            "Retrieve the athlete's activities".to_owned(),
            Some(json!({ "type": "object", "properties": {} })),
        )],
    };
    let user_id_str = user_id.to_string();
    let params = ToolLoopParams {
        provider: &chat_provider,
        executor,
        tools: &tools,
        model: "data-first-model",
        user_id: &user_id_str,
        tenant_id: tenant,
        max_iterations: 3,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: Vec::new(),
    };
    let mut messages = vec![
        ChatMessage::system("You are a test fuelling agent."),
        ChatMessage::user(
            "Qu'est-ce que je devrais manger avant une sortie longue de 2 heures samedi matin ?",
        ),
    ];

    let result = run_tool_loop(&params, &mut messages)
        .await
        .expect("the loop completes");

    assert_eq!(
        result.pending_provider_auth_required, None,
        "an athlete who never connected a provider has nothing to reconnect"
    );
    assert_eq!(
        result.served_without_provider, None,
        "no connection was dropped, so no reconnect offer joins the answer"
    );
    assert!(
        result.content.contains(GENERAL_ANSWER),
        "the model's general answer must reach the athlete, got: {}",
        result.content
    );

    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "the refusal must not stop the loop: the model is asked again to answer"
    );
    let told = requests[1]
        .messages
        .iter()
        .map(|m| format!("{:?}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        told.contains("No fitness provider connected"),
        "the model must be told there is no data to read, got: {told}"
    );
}
