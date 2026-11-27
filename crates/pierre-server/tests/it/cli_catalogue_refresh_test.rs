// ABOUTME: The CLI tool loop re-writes its text catalogue once a round's member read narrows the turn's writes
// ABOUTME: A write dispatch would refuse after the read is never offered to the next model call (carnet#1511 I7)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A CLI runner reads the turn's tools as a text catalogue in its system
//! prompt. A member read a round makes withholds the caller's own free-text
//! writes from then on, so the loop re-writes the catalogue before the next
//! call: the round-13 listing-dispatch finding that no test failed when the
//! re-write was deleted.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
    TokenUsage,
};
use pierre_core::models::CarriedRead;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_llm::ChatProvider;
use pierre_providers::ai_scope::{self, Provenance};
use pierre_tool_runtime::chat_catalogue::declarations;
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::run_cli_tool_loop;
use pierre_tool_runtime::tool_loop_io::ToolLoopParams;
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};

/// The caller's own free-text write a member read withholds (call 16).
const FILING_WRITE: &str = "remember_fact";

/// A read that needs no provider connection, which the first round calls.
const READ_TOOL: &str = "get_connection_status";

/// Reads another member during its first call and asks for [`READ_TOOL`],
/// then answers; keeps the system prompt each call was sent.
struct ReadingModel {
    member: Uuid,
    systems: Mutex<Vec<String>>,
}

#[async_trait]
impl LlmProvider for ReadingModel {
    fn name(&self) -> &'static str {
        "scripted_mock"
    }
    fn display_name(&self) -> &'static str {
        "Scripted Mock LLM (CLI catalogue refresh)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let system = request
            .messages
            .first()
            .map_or_default(|message| message.content.clone());
        let first = {
            let mut systems = self.systems.lock().unwrap();
            systems.push(system);
            systems.len() == 1
        };
        let content = if first {
            // What a tool reading another member does mid-turn.
            ai_scope::note_member_read(CarriedRead::peer(self.member, None));
            format!("<tool_call>{{\"name\":\"{READ_TOOL}\",\"arguments\":{{}}}}</tool_call>")
        } else {
            "Noted.".to_owned()
        };
        Ok(ChatResponse {
            replay_blocks: None,
            content,
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(10, 10, 20)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: String::new(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// A round's member read takes the caller's own free-text write out of the
/// catalogue the next call reads: offered before the read, gone after it.
#[tokio::test]
async fn a_round_that_read_a_member_rewrites_the_catalogue_for_the_next_call() {
    let model = Arc::new(ReadingModel {
        member: Uuid::new_v4(),
        systems: Mutex::new(Vec::new()),
    });
    let provider = ChatProvider::Custom(model.clone());
    let resources = create_test_server_resources_with_chat_provider(model.clone())
        .await
        .unwrap();
    let (user_id, _, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        "cli-catalogue@test.com",
        "professional",
    )
    .await
    .unwrap();
    let registry = &resources.mcp.tool_registry;
    let tools = declarations(registry.list_schemas_by_names(&[FILING_WRITE, READ_TOOL]));
    assert_eq!(
        tools.function_declarations.len(),
        2,
        "premise: both tools are registered"
    );
    let user = user_id.to_string();

    ai_scope::tracking(Provenance::new(), async {
        let params = ToolLoopParams {
            provider: &provider,
            executor: Arc::new(
                UniversalToolExecutor::new(resources.clone())
                    .with_scopes(OAuthScope::self_grant())
                    .with_transport(Transport::WebApp),
            ),
            tools: &tools,
            model: "mock-model",
            user_id: &user,
            tenant_id,
            max_iterations: 3,
            call_recorder: None,
            tool_message_recorder: None,
            temperature: None,
            stream_sink: None,
            mcp_servers: Vec::new(),
        };
        let mut messages = vec![
            ChatMessage::system("You are a test coach."),
            ChatMessage::user("Remember that I ride on Sundays."),
        ];
        run_cli_tool_loop(&params, &mut messages, None)
            .await
            .expect("the loop answers");
    })
    .await;

    let systems = model.systems.lock().unwrap().clone();
    assert_eq!(systems.len(), 2, "two calls were made: {systems:?}");
    assert!(
        systems[0].contains(FILING_WRITE) && systems[0].contains(READ_TOOL),
        "before any read both tools are in the catalogue: {}",
        systems[0]
    );
    assert!(
        !systems[1].contains(FILING_WRITE),
        "bug: after a member read the next call is still offered {FILING_WRITE}: {}",
        systems[1]
    );
    assert!(
        systems[1].contains(READ_TOOL),
        "the read stays offered: {}",
        systems[1]
    );
}
