// ABOUTME: An auth trip exits the tool loop without discarding the sibling calls that already ran
// ABOUTME: The reconnect turn still carries the activity list another connection served

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Only a Guardian block stops a batch mid-flight, so when one tool trips
//! `ProviderAuthRequired` its siblings in the same batch have already run and
//! produced real data. The loop exits on the trip — continuing would let the
//! model rephrase a refusal or re-call the broken tool — but the window a
//! healthy connection served has to leave with it, or the athlete is handed a
//! reconnect sentence and nothing else.
//!
//! This drives the real loop with a scripted provider that asks for two
//! `get_activities` calls in one batch: a dead connection and a healthy one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use common::{create_test_server_resources, create_test_user};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use helpers::sciotte_mock::{seed_sciotte_session, spawn_mock_scraper};
use pierre_core::errors::AppError;
use pierre_core::models::ConnectionType;
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, FunctionDeclaration,
    LlmCapabilities, LlmProvider, StreamChunk, Tool,
};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::ToolLoopParams;
use serde_json::json;
use serial_test::serial;

/// The ride the healthy connection's provider serves; its name must reach the
/// reply even though the batch tripped on the other connection.
const HEALTHY_RIDE_NAME: &str = "Sortie vélo matinale";

/// A provider that asks for both connections in ONE batch, then would answer.
struct BatchingProvider {
    asked: Mutex<bool>,
    models: Vec<String>,
}

impl BatchingProvider {
    fn new() -> Self {
        Self {
            asked: Mutex::new(false),
            models: vec!["batch-model".to_owned()],
        }
    }
}

#[async_trait]
impl LlmProvider for BatchingProvider {
    fn name(&self) -> &'static str {
        "batching"
    }
    fn display_name(&self) -> &'static str {
        "Batching Provider"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "batch-model"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let mut asked = self.asked.lock().unwrap();
        let tool_calls = if *asked {
            None
        } else {
            *asked = true;
            Some(vec![
                // The dead connection first: its trip must not cancel what follows.
                ToolCallRequest {
                    id: "call-dead".to_owned(),
                    function_name: "get_activities".to_owned(),
                    arguments: json!({ "provider": "whoop", "limit": 5, "mode": "summary" }),
                },
                ToolCallRequest {
                    id: "call-healthy".to_owned(),
                    function_name: "get_activities".to_owned(),
                    arguments: json!({ "provider": "strava", "limit": 5, "mode": "summary" }),
                },
            ])
        };

        Ok(ChatResponse {
            content: String::new(),
            model: "batch-model".to_owned(),
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

fn advertised_tools() -> Tool {
    Tool {
        function_declarations: vec![FunctionDeclaration {
            name: "get_activities".to_owned(),
            description: "Retrieve the athlete's activities".to_owned(),
            parameters: Some(json!({ "type": "object", "properties": {} })),
        }],
    }
}

#[tokio::test]
#[serial]
async fn an_auth_trip_keeps_the_window_a_sibling_call_already_served() {
    let scraper_url = spawn_mock_scraper().await;
    env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
    // The remote client is both-or-neither: a URL with no audience disables it.
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");

    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    // A healthy connection whose live fetch really answers (the `strava` ask
    // resolves to the seeded sciotte session), and a dead one with no token.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::Manual, None)
        .await
        .unwrap();
    seed_sciotte_session(&resources, user_id, tenant).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let provider = Arc::new(BatchingProvider::new());
    let chat_provider = ChatProvider::Custom(provider);
    let executor = Arc::new(
        UniversalToolExecutor::new(resources).with_turn_token("auth-trip-turn".to_owned()),
    );
    let tools = advertised_tools();
    let user_id_str = user_id.to_string();

    let params = ToolLoopParams {
        provider: &chat_provider,
        executor,
        tools: &tools,
        model: "batch-model",
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
        ChatMessage::system("You are a test coach."),
        ChatMessage::user("mes sorties récentes ?"),
    ];

    let result = run_tool_loop(&params, &mut messages)
        .await
        .expect("the loop completes the batch");

    assert_eq!(
        result.pending_provider_auth_required.as_deref(),
        Some("whoop"),
        "the dead connection must still hand auth_recovery its slug"
    );
    assert_eq!(
        result.tool_calls_count, 2,
        "both calls in the batch dispatch — only a Guardian block stops siblings"
    );
    assert_eq!(
        result.tools_called,
        vec!["get_activities".to_owned()],
        "exactly one of the two calls succeeded, and it is the one that ran"
    );

    let activity_list = result
        .activity_list
        .expect("the sibling's window must survive the auth trip");
    assert!(
        activity_list.contains(HEALTHY_RIDE_NAME),
        "the reconnect turn carries the ride the healthy connection served, got: \
         {activity_list}"
    );
}
