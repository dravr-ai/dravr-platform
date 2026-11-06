// ABOUTME: An assistant row records whether its turn tried to change anything, read back from the database
// ABOUTME: Regenerate is a new turn on a fresh budget, so a blocked or writing turn must never read as safe to re-send

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regenerate re-sends the question as a brand-new turn on a fresh budget, so it
//! can complete a delete the Guardian just blocked. The row says whether its turn
//! attempted a write, and clients offer Regenerate only on `false`.
//!
//! Each turn runs through the chat pipeline against a scripted model, and the
//! flag is asserted on the row read back: the record `add_message` returns is
//! built from its params, so only a re-read exercises the column.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use futures_util::stream;
use pierre_chat_pipeline::{
    CommandPersistence, InputSource, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_config::environment::{GuardianSettings, ServerConfig};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
    TokenUsage,
};
use pierre_core::models::{
    AddMessageParams, ConversationTurnId, TenantId, GUARDIAN_DENIED_FINISH_REASON,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_llm::{ChatProvider, Tool};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult};
use pierre_tool_runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common::{
    create_test_server_resources_with_chat_provider,
    create_test_server_resources_with_llm_and_config, create_test_tenant,
    create_test_user_with_plan, test_server_config, test_server_config_with_guardian,
};
use crate::helpers::axum_test::AxumTestRequest;

/// Declares only `READ_ONLY`, and needs no provider connection.
const READ_TOOL: &str = "get_connection_status";

/// Declares `WRITES_DATA`. Called with no arguments it fails loudly and saves
/// nothing, which is the point: an attempted write counts whether it landed.
const WRITE_TOOL: &str = "save_training_plan";

/// Declares `IRREVERSIBLE`; a destructive budget of zero refuses it at the gate.
const DESTRUCTIVE_TOOL: &str = "forget_playbook";

/// What the scripted model answers once it has nothing left to call.
const ANSWER: &str = "Bien noté, on continue comme prévu pour cette semaine.";

/// Answers with `<tool_call>` blocks from a script, then with [`ANSWER`].
struct ScriptedModel {
    replies: Mutex<VecDeque<String>>,
}

impl ScriptedModel {
    fn calling(tool: &str) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from([format!(
                "<tool_call>{{\"name\":\"{tool}\",\"arguments\":{{}}}}</tool_call>"
            )])),
        })
    }

    fn answering() -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::new()),
        })
    }
}

#[async_trait]
impl LlmProvider for ScriptedModel {
    fn name(&self) -> &'static str {
        "scripted_mock"
    }
    fn display_name(&self) -> &'static str {
        "Scripted Mock LLM (turn attempted write)"
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

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let content = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| ANSWER.to_owned());
        Ok(ChatResponse {
            content,
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(30, 40, 70)),
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

struct Thread {
    resources: Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
    conversation_id: String,
}

async fn thread_with(model: Arc<ScriptedModel>, config: ServerConfig, email: &str) -> Thread {
    let resources = create_test_server_resources_with_llm_and_config(model, config)
        .await
        .unwrap();
    let (user_id, _, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    let conversation_id = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant_id,
            "Semaine",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    Thread {
        resources,
        tenant_id,
        user_id,
        conversation_id,
    }
}

/// Run one athlete turn on the web surface and read the flag back from the
/// assistant row the database holds, with the finish reason it was stored under.
async fn turn_flag(thread: &Thread, content: &str) -> (Option<bool>, Option<String>) {
    let served = pierre_chat_pipeline::execute(
        &thread.resources.chat_pipeline_context(),
        TurnRequest {
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            conversation_id: thread.conversation_id.clone(),
            user_id: thread.user_id,
            conversation_tenant_id: thread.tenant_id,
            tool_tenant_id: thread.tenant_id,
            content: content.to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quote: None,
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
    .expect("the turn is served");
    let ServedTurn::Pipeline(envelope) = served else {
        panic!("an athlete's message must reach the pipeline");
    };
    let rows = thread
        .resources
        .common
        .repos
        .chat
        .get_messages(
            &thread.conversation_id,
            &thread.user_id.to_string(),
            thread.tenant_id,
        )
        .await
        .unwrap();
    let row = rows
        .into_iter()
        .find(|row| row.id == envelope.assistant.message.id)
        .expect("the assistant row is on file");
    (row.turn_attempted_write, row.finish_reason)
}

#[tokio::test]
async fn a_turn_the_guardian_blocked_reads_back_as_attempted() {
    let thread = thread_with(
        ScriptedModel::calling(DESTRUCTIVE_TOOL),
        test_server_config_with_guardian(GuardianSettings {
            mode: Some("enforce".to_owned()),
            max_destructive_per_turn: Some("0".to_owned()),
            ..GuardianSettings::default()
        }),
        "attempted-write-blocked@test.com",
    )
    .await;

    let (attempted, finish_reason) = turn_flag(&thread, "Oublie cette routine.").await;

    assert_eq!(
        finish_reason.as_deref(),
        Some(GUARDIAN_DENIED_FINISH_REASON),
        "the turn under test must actually have been blocked"
    );
    assert_eq!(attempted, Some(true));
}

#[tokio::test]
async fn a_write_that_failed_reads_back_as_attempted() {
    let thread = thread_with(
        ScriptedModel::calling(WRITE_TOOL),
        test_server_config(),
        "attempted-write-failed@test.com",
    )
    .await;

    let (attempted, _) = turn_flag(&thread, "Enregistre mon plan.").await;

    assert_eq!(attempted, Some(true));
}

#[tokio::test]
async fn a_read_only_turn_reads_back_as_not_attempted() {
    let thread = thread_with(
        ScriptedModel::calling(READ_TOOL),
        test_server_config(),
        "attempted-write-read@test.com",
    )
    .await;

    let (attempted, _) = turn_flag(&thread, "Quelles sont mes connexions ?").await;

    assert_eq!(attempted, Some(false));
}

#[tokio::test]
async fn a_turn_that_called_no_tool_reads_back_as_not_attempted() {
    let thread = thread_with(
        ScriptedModel::answering(),
        test_server_config(),
        "attempted-write-none@test.com",
    )
    .await;

    let (attempted, _) = turn_flag(&thread, "Merci pour hier.").await;

    assert_eq!(attempted, Some(false));
}

/// Run a tool loop under `turn` whose model calls nothing itself and hand back
/// what it settled: anything recorded came from another executor on that turn.
async fn settled_turn(thread: &Thread, turn: &str) -> ToolLoopResult {
    let provider = ChatProvider::Custom(ScriptedModel::answering());
    let tools = Tool {
        function_declarations: Vec::new(),
    };
    let user = thread.user_id.to_string();
    let params = ToolLoopParams {
        provider: &provider,
        executor: Arc::new(turn_executor(thread, turn)),
        tools: &tools,
        model: "mock-model",
        user_id: &user,
        tenant_id: thread.tenant_id,
        max_iterations: 2,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: Vec::new(),
    };
    run_tool_loop(&params, &mut vec![ChatMessage::user("et ma semaine ?")])
        .await
        .expect("the loop answers")
}

fn turn_executor(thread: &Thread, turn: &str) -> UniversalToolExecutor {
    UniversalToolExecutor::new(thread.resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .with_turn_token(turn.to_owned())
}

/// What an agent's tool-host session does over the turn's MCP servers: the same
/// chokepoint, under the same turn key, from an executor the loop never sees.
async fn dispatch_under(thread: &Thread, turn: &str, tool: &str) {
    let request = UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: json!({}),
        user_id: thread.user_id.to_string(),
        protocol: "mcp".to_owned(),
        tenant_id: Some(thread.tenant_id.to_string()),
    };
    // A failed call comes back as an error and a read as an answer; what is
    // under test is what the turn recorded, not what the call returned.
    drop(turn_executor(thread, turn).execute_tool(request).await);
}

/// A headless leg that fails is re-run on the chain's tail with nothing carried
/// over, so its calls never reach the loop's list of tools. Its write went
/// through the chokepoint under this turn's key all the same.
#[tokio::test]
async fn a_write_outside_the_loop_still_counts_for_the_turn() {
    let thread = thread_with(
        ScriptedModel::answering(),
        test_server_config(),
        "attempted-write-host@test.com",
    )
    .await;

    let turn = Uuid::new_v4().to_string();
    dispatch_under(&thread, &turn, WRITE_TOOL).await;
    let result = settled_turn(&thread, &turn).await;

    assert!(
        result.tools_called.is_empty(),
        "the loop itself called nothing: {:?}",
        result.tools_called
    );
    assert!(result.write_attempted);
    assert!(result.attempted_write(thread.resources.tool_registry()));
}

/// A write refused before the Guardian's gate was still asked for: the same
/// question on a later turn, once the refusal no longer applies, would run it.
#[tokio::test]
async fn a_write_refused_before_the_gate_still_counts_for_the_turn() {
    let thread = thread_with(
        ScriptedModel::answering(),
        test_server_config(),
        "attempted-write-refused@test.com",
    )
    .await;

    let turn = Uuid::new_v4().to_string();
    let request = UniversalRequest {
        tool_name: WRITE_TOOL.to_owned(),
        parameters: json!({}),
        user_id: thread.user_id.to_string(),
        protocol: "mcp".to_owned(),
        tenant_id: Some(thread.tenant_id.to_string()),
    };
    let refused = turn_executor(&thread, &turn)
        .with_scopes(vec![OAuthScope::FitnessRead])
        .execute_tool(request)
        .await;
    assert!(
        refused.is_err(),
        "a read-only grant is refused a write tool"
    );

    let result = settled_turn(&thread, &turn).await;
    assert!(result.write_attempted, "the refused write was attempted");
}

/// A read through the same seam is no reason to hide Regenerate, and neither is
/// another turn's write.
#[tokio::test]
async fn a_read_or_another_turns_write_does_not_count() {
    let thread = thread_with(
        ScriptedModel::answering(),
        test_server_config(),
        "attempted-write-other@test.com",
    )
    .await;

    let turn = Uuid::new_v4().to_string();
    let other_turn = Uuid::new_v4().to_string();
    dispatch_under(&thread, &turn, READ_TOOL).await;
    dispatch_under(&thread, &other_turn, WRITE_TOOL).await;
    let result = settled_turn(&thread, &turn).await;

    assert!(!result.write_attempted);
    assert!(!result.attempted_write(thread.resources.tool_registry()));
}

/// The rule by name: a tool the registry holds is judged by what it declares,
/// and one it does not hold counts, since nothing vouches that it only reads.
/// That is the case for a headless call the transport reported only by title.
#[tokio::test]
async fn the_registry_classifies_a_call_by_what_the_tool_declares() {
    let thread = thread_with(
        ScriptedModel::answering(),
        test_server_config(),
        "attempted-write-registry@test.com",
    )
    .await;
    let registry = thread.resources.tool_registry();

    assert!(!registry.may_change_data(READ_TOOL));
    assert!(registry.may_change_data(WRITE_TOOL));
    assert!(registry.may_change_data(DESTRUCTIVE_TOOL));
    assert!(registry.may_change_data("Viewing the athlete's calendar"));
    // The names the pipeline adds to `tools_called` itself must resolve, or
    // every grounded turn would read as a write.
    assert!(!registry.may_change_data("get_activities"));
    assert!(!registry.may_change_data("get_group_member_activities"));
}

/// What a client reads: the delivered turn's message and the thread's rows both
/// carry the field, and a row that does not know leaves it off.
#[tokio::test]
async fn the_field_reaches_the_live_turn_and_the_thread_read() {
    let resources = create_test_server_resources_with_chat_provider(ScriptedModel::answering())
        .await
        .unwrap();
    let (user, token) = create_test_tenant(&resources, "attempted-write-wire@test.com")
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let bearer = format!("Bearer {token}");
    let created = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &bearer)
        .json(&json!({"title": "Semaine", "model": "mock-model"}))
        .send(router.clone())
        .await;
    assert_eq!(created.status_code(), StatusCode::CREATED);
    let conversation: Value = created.json();
    let conversation_id = conversation["id"].as_str().unwrap().to_owned();
    let path = format!("/api/chat/conversations/{conversation_id}/messages");

    let sent = AxumTestRequest::post(&path)
        .header("authorization", &bearer)
        .json(&json!({"content": "Merci pour hier."}))
        .send(router.clone())
        .await;
    assert_eq!(sent.status_code(), StatusCode::OK);
    let turn: Value = sent.json();
    assert_eq!(
        turn["assistant"]["message"]["turn_attempted_write"],
        json!(false),
        "the live turn: {turn}"
    );

    let user_id = user.id.to_string();
    for (content, attempted) in [("écrit", Some(true)), ("inconnu", None)] {
        resources
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id,
                conversation_id: &conversation_id,
                user_id: &user_id,
                role: "assistant",
                content,
                token_count: None,
                finish_reason: Some("stop"),
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
                turn_attempted_write: attempted,
            })
            .await
            .unwrap();
    }

    let read = AxumTestRequest::get(&path)
        .header("authorization", &bearer)
        .send(router.clone())
        .await;
    assert_eq!(read.status_code(), StatusCode::OK);
    let body: Value = read.json();
    let rows = body["messages"].as_array().unwrap();
    let row = |content: &str| {
        rows.iter()
            .find(|row| row["content"] == content)
            .unwrap_or_else(|| panic!("no row {content} in {body}"))
    };
    assert_eq!(row(ANSWER)["turn_attempted_write"], json!(false));
    assert_eq!(row("écrit")["turn_attempted_write"], json!(true));
    assert!(
        row("inconnu").get("turn_attempted_write").is_none(),
        "a row that does not know leaves the field off: {body}"
    );
}
