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

use crate::common;
use crate::helpers;

use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use std::mem::replace;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use common::{
    create_test_server_resources_with_no_destructive_budget,
    create_test_server_resources_with_sciotte, create_test_user,
};
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
        function_declarations: vec![FunctionDeclaration::new(
            "get_activities".to_owned(),
            "Retrieve the athlete's activities".to_owned(),
            Some(json!({ "type": "object", "properties": {} })),
        )],
    }
}

#[tokio::test]
async fn an_auth_trip_keeps_the_window_a_sibling_call_already_served() {
    let scraper_url = spawn_mock_scraper().await;
    let resources = create_test_server_resources_with_sciotte(&scraper_url)
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
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
        UniversalToolExecutor::new(resources)
            .with_scopes(OAuthScope::self_grant())
            // A chat turn in the web app: the healthy connection is Strava,
            // whose data no external transport serves (carnet#765).
            .with_transport(Transport::WebApp)
            .with_turn_token("auth-trip-turn".to_owned()),
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

/// How a mock turn asks for its one batch, one shape per branch of the tool loops.
#[derive(Clone, Copy)]
enum BatchShape {
    /// Native function calling, as structured tool calls.
    ApiStructured,
    /// A native-tool model writing `<tool_call>` blocks as text.
    ApiTextBlocks,
    /// A text-only provider, whose calls are always `<tool_call>` blocks.
    CliTextBlocks,
}

/// Asks for one batch: a read that trips auth, then a destructive call the
/// Guardian denies. Then it would answer.
struct AuthTripThenDenied {
    shape: BatchShape,
    asked: Mutex<bool>,
    models: Vec<String>,
}

#[async_trait]
impl LlmProvider for AuthTripThenDenied {
    fn name(&self) -> &'static str {
        "auth_trip_then_denied"
    }
    fn display_name(&self) -> &'static str {
        "Auth trip then Guardian denial mock"
    }
    fn capabilities(&self) -> LlmCapabilities {
        match self.shape {
            BatchShape::ApiStructured | BatchShape::ApiTextBlocks => {
                LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
            }
            BatchShape::CliTextBlocks => LlmCapabilities::SYSTEM_MESSAGES,
        }
    }
    fn default_model(&self) -> &'static str {
        "batch-model"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let first_ask = {
            let mut asked = self.asked.lock().unwrap();
            !replace(&mut *asked, true)
        };
        let auth_trip = json!({ "provider": "whoop", "limit": 5, "mode": "summary" });
        let destructive = json!({ "provider": "whoop" });
        let (content, tool_calls) = match (first_ask, self.shape) {
            (false, _) => ("the agent's answer".to_owned(), None),
            (true, BatchShape::ApiStructured) => (
                String::new(),
                Some(vec![
                    ToolCallRequest {
                        id: "call-dead".to_owned(),
                        function_name: "get_activities".to_owned(),
                        arguments: auth_trip,
                    },
                    ToolCallRequest {
                        id: "call-denied".to_owned(),
                        function_name: "disconnect_provider".to_owned(),
                        arguments: destructive,
                    },
                ]),
            ),
            (true, BatchShape::ApiTextBlocks | BatchShape::CliTextBlocks) => (
                format!(
                    "<tool_call>{}</tool_call><tool_call>{}</tool_call>",
                    json!({ "name": "get_activities", "arguments": auth_trip }),
                    json!({ "name": "disconnect_provider", "arguments": destructive }),
                ),
                None,
            ),
        };
        Ok(ChatResponse {
            content,
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

/// carnet#1411: a batch that trips auth and is then denied answers with the
/// Guardian's block, in every branch of the tool loops, because the chat pipeline
/// renders a block ahead of a reconnect and an auth exit would have hidden it.
#[tokio::test]
async fn a_batch_that_trips_auth_and_is_denied_returns_the_denial() {
    common::init_server_config();
    let resources = create_test_server_resources_with_no_destructive_budget()
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
    // A connection with no token: the read reaches the provider and trips auth.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let user_id_str = user_id.to_string();

    for (shape, label) in [
        (BatchShape::ApiStructured, "api structured"),
        (BatchShape::ApiTextBlocks, "api text blocks"),
        (BatchShape::CliTextBlocks, "cli text blocks"),
    ] {
        let chat_provider = ChatProvider::Custom(Arc::new(AuthTripThenDenied {
            shape,
            asked: Mutex::new(false),
            models: vec!["batch-model".to_owned()],
        }));
        let executor = Arc::new(
            UniversalToolExecutor::new(resources.clone())
                .with_scopes(OAuthScope::self_grant())
                .with_transport(Transport::WebApp)
                .with_turn_token(format!("auth-then-denied-{label}")),
        );
        let tools = advertised_tools();
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
            ChatMessage::user("déconnecte whoop"),
        ];

        let result = run_tool_loop(&params, &mut messages)
            .await
            .expect("the loop completes the batch");

        let denial = result
            .guardian_denied
            .unwrap_or_else(|| panic!("{label}: the denial must win over the auth trip"));
        assert_eq!(denial.reason, "budget_exceeded", "{label}");
        assert_eq!(denial.tool_name, "disconnect_provider", "{label}");
        assert_eq!(
            result.pending_provider_auth_required, None,
            "{label}: the loop exits on the block, not the auth trip"
        );
    }
}
