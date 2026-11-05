// ABOUTME: End-to-end Guardian enforcement through the real ServerContext + UniversalExecutor chokepoint.
// ABOUTME: Deterministic (no LLM): zero destructive budget denies an IRREVERSIBLE tool, over /mcp too; reads pass.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Guardian enforcement e2e.
//!
//! Drives a real `ServerContext`-backed `UniversalExecutor` (the one true
//! dispatch chokepoint every transport funnels through) in `enforce` mode and
//! asserts the runtime Guardian blocks a consequential tool while leaving reads
//! untouched, then drives the same block through `POST /mcp` to pin what an MCP
//! client receives. A chat turn's tool batch reports the blocked call to the
//! athlete's trail as never run, and its siblings after it not at all.

use crate::common;

use pierre_core::permissions::scopes::OAuthScope;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Request, StatusCode};
use common::{
    create_test_server_resources_with_config, create_test_server_resources_with_llm_and_tools,
    create_test_user, generate_delegated_test_token, test_server_config_with_guardian,
};
use dravr_tronc::mcp::schema::{Tool as McpToolDefinition, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use futures_util::stream;
use pierre_config::environment::GuardianSettings;
use pierre_core::errors::AppError;
use pierre_core::models::{TenantId, GUARDIAN_DENIED_FINISH_REASON};
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, FunctionCall,
    LlmCapabilities, LlmProvider, StreamChunk, Tool,
};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::chat_stream::{
    TurnEvent, TOOL_STATUS_COMPLETED, TOOL_STATUS_IN_PROGRESS, TOOL_STATUS_NOT_RUN,
};
use pierre_tool_runtime::function_dispatch::execute_function_calls;
use pierre_tool_runtime::guardian::{CallFate, DenyReason, HeadlessBlock, TurnKey, TurnState};
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult};
use pierre_tool_runtime::{RuntimeTool, SecurityLabels};
use serde_json::{json, Value};
use std::future::pending;
use std::sync::Arc;
use tokio::select;
use tokio::sync::{mpsc, Notify};
use tower::ServiceExt;
use uuid::Uuid;

fn request(tool: &str, args: Value, user_id: Uuid, tenant: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: args,
        user_id: user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant.to_string()),
    }
}

/// `POST /mcp` `tools/call` of `tool` with the caller's bearer `token`, through
/// the app the server serves, and the JSON-RPC body it answers.
async fn tools_call_over_http(
    resources: &Arc<ServerContext>,
    token: &str,
    tool: &str,
    args: Value,
) -> Value {
    let request = Request::post("/mcp")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": args }
            })
            .to_string(),
        ))
        .unwrap();
    let response = ProviderToolRouter::build_http_app(resources)
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a refused tools/call is answered in-band, not as an HTTP error"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn guardian_error_code(response: &UniversalResponse) -> Option<String> {
    response
        .result
        .as_ref()
        .and_then(|r| r.get("error_code"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[tokio::test]
async fn guardian_enforce_chokepoint_denies_irreversible_allows_reads_e2e() {
    // Enforce mode with a zero destructive budget, the GUARDIAN_* overrides
    // the server's config carries.
    common::init_server_config();
    let resources = create_test_server_resources_with_config(test_server_config_with_guardian(
        GuardianSettings {
            mode: Some("enforce".to_owned()),
            max_destructive_per_turn: Some("0".to_owned()),
            ..GuardianSettings::default()
        },
    ))
    .await
    .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    // One executor = one turn; its turn token is the Guardian's per-turn key.
    let executor = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_turn_token("e2e-turn".to_owned());

    // (a) An IRREVERSIBLE tool is denied at the chokepoint by the zero budget —
    //     before the tool body ever runs. `disconnect_provider` is classified
    //     IRREVERSIBLE in the registry.
    let deny = executor
        .execute_tool(request(
            "disconnect_provider",
            json!({ "provider": "strava" }),
            user_id,
            tenant,
        ))
        .await
        .expect("dispatch returns an in-band response");
    assert!(
        !deny.success,
        "an irreversible tool must be denied under enforce + zero destructive budget"
    );
    assert_eq!(
        guardian_error_code(&deny).as_deref(),
        Some("guardian_denied"),
        "denied response must carry the guardian_denied machine code"
    );
    assert_eq!(
        deny.result
            .as_ref()
            .and_then(|r| r.get("reason"))
            .and_then(Value::as_str),
        Some("budget_exceeded"),
        "the denial reason is the per-turn budget"
    );

    // (b) A read tool is NOT guardian-denied — enforce must not break ordinary
    //     reads. (It may still fail for lack of provider data; that's fine, as
    //     long as it is not a Guardian denial.)
    let read = executor
        .execute_tool(request(
            "get_athlete",
            json!({ "provider": "strava" }),
            user_id,
            tenant,
        ))
        .await
        .expect("dispatch returns an in-band response");
    assert_ne!(
        guardian_error_code(&read).as_deref(),
        Some("guardian_denied"),
        "a read tool must never be guardian-denied"
    );

    // (c) The same block reaches an MCP client over `POST /mcp` as an `isError`
    //     result whose data rides in `content`: the message, then the Guardian's
    //     code and reason as JSON. Never as `structuredContent` —
    //     `disconnect_provider` declares an `outputSchema` this payload does not
    //     match, and a client that validates `structuredContent` on an error
    //     result (the official TypeScript SDK does) would reject the refusal
    //     instead of reading it. A `/mcp` call is its own Guardian turn, so the
    //     zero budget denies it on the first call.
    let token = generate_delegated_test_token(&resources, &user).await;
    let body = tools_call_over_http(
        &resources,
        &token,
        "disconnect_provider",
        json!({ "provider": "strava" }),
    )
    .await;
    assert!(
        body.get("error").is_none(),
        "a Guardian block is an isError result, not a JSON-RPC error: {body}"
    );
    let result = &body["result"];
    assert_eq!(result["isError"], true, "the block is an error: {body}");
    assert!(
        result.get("structuredContent").is_none(),
        "an error result carries no structuredContent: {body}"
    );
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "message, then the data as JSON: {body}");
    assert_eq!(
        content[0]["text"],
        "Error: Tool 'disconnect_provider' was blocked by security policy"
    );
    let data: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        data,
        json!({ "error_code": "guardian_denied", "reason": "budget_exceeded" }),
        "the client reads the Guardian's code and reason from content: {body}"
    );
}

#[tokio::test]
async fn a_blocked_call_in_a_chat_batch_is_reported_as_never_run() {
    common::init_server_config();
    let resources = create_test_server_resources_with_config(test_server_config_with_guardian(
        GuardianSettings {
            mode: Some("enforce".to_owned()),
            max_destructive_per_turn: Some("0".to_owned()),
            ..GuardianSettings::default()
        },
    ))
    .await
    .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let executor = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_turn_token("trail-turn".to_owned());
    let call = |name: &str, args: Value| FunctionCall {
        name: name.to_owned(),
        args,
    };
    // A read that runs, the irreversible call the zero budget denies, and a
    // read after it that the batch never reaches.
    let calls = vec![
        call("get_connection_status", json!({})),
        call("disconnect_provider", json!({ "provider": "strava" })),
        call("get_connection_status", json!({})),
    ];
    let (sink, mut events) = mpsc::unbounded_channel();

    let outcome = execute_function_calls(
        &executor,
        &calls,
        &user_id.to_string(),
        TenantId::from_uuid(Uuid::new_v4()),
        Some(&sink),
    )
    .await
    .expect("dispatch returns an outcome");

    assert!(
        outcome.guardian_denied.is_some(),
        "the batch stops on the Guardian's denial"
    );
    let mut steps: Vec<(String, String)> = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let TurnEvent::Progress(progress) = event {
            steps.push((progress.title, progress.status));
        }
    }
    assert_eq!(
        steps,
        [
            ("get_connection_status", TOOL_STATUS_IN_PROGRESS),
            ("get_connection_status", TOOL_STATUS_COMPLETED),
            ("disconnect_provider", TOOL_STATUS_IN_PROGRESS),
            ("disconnect_provider", TOOL_STATUS_NOT_RUN),
        ]
        .map(|(title, status)| (title.to_owned(), status.to_owned())),
        "the read is a step done, the denied call is withdrawn, and the read after it never starts"
    );
}

/// The MCP annotation a super admin reads and the label the Guardian enforces say
/// the same thing about every registered tool, in both directions. A tool
/// annotated destructive but unlabelled escapes the per-turn cap; one labelled
/// irreversible but annotated otherwise tells a client it is safe to run without
/// asking.
#[tokio::test]
async fn every_destructive_annotation_and_irreversible_label_agree() {
    common::init_server_config();
    let resources = create_test_server_resources_with_config(test_server_config_with_guardian(
        GuardianSettings::default(),
    ))
    .await
    .expect("server resources");
    let registry = resources.mcp.tool_registry.clone();

    let disagreements: Vec<String> = registry
        .tool_names()
        .into_iter()
        .filter_map(|name| {
            let tool = registry.get(name).expect("a listed tool resolves");
            let destructive = tool
                .definition()
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.destructive_hint)
                .unwrap_or(false);
            let irreversible = tool.security_class().is_irreversible();
            (destructive != irreversible).then(|| {
                format!("{name}: destructive_hint={destructive} IRREVERSIBLE={irreversible}")
            })
        })
        .collect();

    assert!(
        registry.get("admin_unassign_agent").is_some(),
        "the sweep must see the admin tools it pins"
    );
    assert!(disagreements.is_empty(), "{disagreements:#?}");
}

/// A server whose destructive budget is spent before the first call, so a
/// destructive call is the turn's block.
async fn enforcing_with_no_destructive_budget() -> Arc<ServerContext> {
    common::init_server_config();
    create_test_server_resources_with_config(test_server_config_with_guardian(GuardianSettings {
        mode: Some("enforce".to_owned()),
        max_destructive_per_turn: Some("0".to_owned()),
        ..GuardianSettings::default()
    }))
    .await
    .expect("server resources")
}

fn turn_executor(resources: &Arc<ServerContext>, turn: &str) -> UniversalToolExecutor {
    UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_turn_token(turn.to_owned())
}

fn guardian_reason(response: &UniversalResponse) -> Option<&str> {
    response.result.as_ref()?.get("reason")?.as_str()
}

/// carnet#1397 through the chokepoint: the budget's denial stands as the turn's
/// first block, later writes are refused because of it, and reads still run.
#[tokio::test]
async fn a_denied_call_stops_later_writes_through_the_executor() {
    let resources = enforcing_with_no_destructive_budget().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let executor = turn_executor(&resources, "latched-executor-turn");
    let run = |tool: &'static str, args: Value| {
        let call = executor.execute_tool(request(tool, args, user_id, tenant));
        async move { call.await.expect("an in-band response") }
    };

    let first = run("disconnect_provider", json!({ "provider": "strava" })).await;
    assert_eq!(guardian_reason(&first), Some("budget_exceeded"));

    let write = run("set_goal", json!({ "goal_type": "distance" })).await;
    assert_eq!(guardian_reason(&write), Some("turn_blocked"), "{write:?}");
    let second_destructive = run("disconnect_provider", json!({ "provider": "garmin" })).await;
    assert_eq!(guardian_reason(&second_destructive), Some("turn_blocked"));

    let read = run("get_connection_status", json!({})).await;
    assert!(read.success, "a read still runs: {read:?}");

    let key = TurnKey::new(Some(tenant.as_uuid()), "latched-executor-turn");
    assert_eq!(
        resources.guardian_turns().take_block(&key),
        Some(HeadlessBlock {
            tool_name: "disconnect_provider".to_owned(),
            reason: DenyReason::BudgetExceeded,
        }),
        "the first ask stands, not the refusals it caused"
    );
}

/// A chain that answers every ask in plain text, on the text loop: what is under
/// test is what the platform does with the turn's block, not what a model says.
pub(crate) struct AnswersPlainly {
    pub(crate) models: Vec<String>,
}

#[async_trait]
impl LlmProvider for AnswersPlainly {
    fn name(&self) -> &'static str {
        "answers_plainly"
    }
    fn display_name(&self) -> &'static str {
        "Plain-answer mock (Guardian turn-signal pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "plain-model"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }
    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: "the agent's answer".to_owned(),
            model: "plain-model".to_owned(),
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

/// Run one turn's tool loop on `turn`, whose model calls no tool itself: anything
/// recorded under the turn was raised by another executor, as an agent's
/// tool-host session is.
async fn finish_turn(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    turn: &str,
) -> ToolLoopResult {
    let provider = ChatProvider::Custom(Arc::new(AnswersPlainly {
        models: vec!["plain-model".to_owned()],
    }));
    let tools = Tool {
        function_declarations: Vec::new(),
    };
    let user = user_id.to_string();
    let params = ToolLoopParams {
        provider: &provider,
        executor: Arc::new(turn_executor(resources, turn)),
        tools: &tools,
        model: "plain-model",
        user_id: &user,
        tenant_id: tenant,
        max_iterations: 2,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: Vec::new(),
    };
    run_tool_loop(&params, &mut vec![ChatMessage::user("how was my week?")])
        .await
        .expect("the loop answers")
}

/// carnet#1404: a block belongs to the turn it fired in. An overlapping turn of
/// the same athlete neither takes nor erases it, and a one-shot MCP denial on its
/// per-call nonce lands in no turn at all.
#[tokio::test]
async fn a_block_stays_with_its_turn_whatever_else_the_athlete_runs() {
    let resources = enforcing_with_no_destructive_budget().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let destructive = || {
        request(
            "disconnect_provider",
            json!({ "provider": "strava" }),
            user_id,
            tenant,
        )
    };

    let blocked_turn = turn_executor(&resources, "blocked-turn")
        .execute_tool(destructive())
        .await
        .expect("an in-band response");
    assert_eq!(guardian_reason(&blocked_turn), Some("budget_exceeded"));

    // A one-shot call: denied in-band, with no turn to report to.
    let one_shot = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .execute_tool(destructive())
        .await
        .expect("an in-band response");
    assert_eq!(guardian_reason(&one_shot), Some("budget_exceeded"));

    let quiet = finish_turn(&resources, user_id, tenant, "overlapping-turn").await;
    assert_eq!(
        quiet.guardian_denied.as_ref().map(|d| d.reason.as_str()),
        None,
        "an overlapping turn takes nothing from the blocked one, nor from the one-shot call"
    );
    assert_eq!(quiet.content, "the agent's answer");

    let blocked = finish_turn(&resources, user_id, tenant, "blocked-turn").await;
    let denial = blocked
        .guardian_denied
        .expect("the blocked turn renders its own block");
    assert_eq!(denial.reason, "budget_exceeded");
    assert_eq!(denial.tool_name, "disconnect_provider");

    let again = finish_turn(&resources, user_id, tenant, "blocked-turn").await;
    assert!(
        again.guardian_denied.is_none(),
        "the loop took the entry when it returned"
    );
}

/// carnet#1398 on the path production runs. A block raised on the tool host
/// reaches the loop only through the store, on a turn whose model finished with
/// `stop`. The stamp is what `push_history_row` replays the row by, so without it
/// the block reply reads back as the agent's own words.
#[tokio::test]
async fn a_block_from_the_tool_host_stamps_the_row_it_replays_as() {
    let resources = enforcing_with_no_destructive_budget().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    let denied = turn_executor(&resources, "stamped-denial-turn")
        .execute_tool(request(
            "disconnect_provider",
            json!({ "provider": "strava" }),
            user_id,
            tenant,
        ))
        .await
        .expect("an in-band response");
    assert_eq!(guardian_reason(&denied), Some("budget_exceeded"));
    let finished = finish_turn(&resources, user_id, tenant, "stamped-denial-turn").await;
    assert!(finished.guardian_denied.is_some());
    assert_eq!(
        finished.finish_reason.as_deref(),
        Some(GUARDIAN_DENIED_FINISH_REASON),
        "the model's own stop must not stand on a row holding the block reply"
    );

    let quiet = finish_turn(&resources, user_id, tenant, "stamped-quiet-turn").await;
    assert_eq!(
        quiet.finish_reason.as_deref(),
        Some("stop"),
        "a turn with no block keeps the model's own stamp"
    );
}

/// A destructive write that says it started, then never answers: what the tool
/// surface's bound abandons.
struct NeverAnswers {
    started: Arc<Notify>,
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for NeverAnswers {
    fn definition(&self) -> McpToolDefinition {
        McpToolDefinition {
            name: "never_answers".to_owned(),
            description: "Starts, then never answers".to_owned(),
            input_schema: json!({ "type": "object" }),
            annotations: None,
            output_schema: None,
            execution: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        self.started.notify_one();
        pending().await
    }
}

pierre_tool_runtime::declare_security!(NeverAnswers => IRREVERSIBLE);

/// carnet#1405: a caller that drops the dispatch once its tool is running, as the
/// tool surface's timeout does, cannot know whether the call landed. The write
/// count comes back, but the destructive charge stays spent, so a retry cannot run
/// a second push past a budget of one.
#[tokio::test]
async fn abandoning_a_running_destructive_call_leaves_its_destructive_charge_spent() {
    let started = Arc::new(Notify::new());
    let tool: Arc<dyn RuntimeTool> = Arc::new(NeverAnswers {
        started: Arc::clone(&started),
    });
    let provider = Arc::new(AnswersPlainly {
        models: vec!["plain-model".to_owned()],
    });
    let resources = create_test_server_resources_with_llm_and_tools(provider, vec![tool])
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let key = TurnKey::new(Some(tenant.as_uuid()), "abandoned-turn");
    let spent = || {
        resources
            .guardian_turns()
            .decide_and_reserve(
                &key,
                SecurityLabels::empty(),
                false,
                |state: &TurnState| (state.destructive_count(), state.write_count()),
                |_| CallFate::Unchanged,
            )
            .0
    };

    let executor = turn_executor(&resources, "abandoned-turn");
    {
        let dispatch = executor.execute_tool(request("never_answers", json!({}), user_id, tenant));
        tokio::pin!(dispatch);
        select! {
            _ = &mut dispatch => panic!("the tool never answers"),
            () = started.notified() => {}
        }
        assert_eq!(spent(), (1, 1), "the running call holds its reservation");
    }
    assert_eq!(
        spent(),
        (1, 0),
        "the outcome is unknown: only the write count comes back"
    );
}

/// A destructive write that runs and says it failed.
struct ReturnsAFailure;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ReturnsAFailure {
    fn definition(&self) -> McpToolDefinition {
        McpToolDefinition {
            name: "returns_a_failure".to_owned(),
            description: "Runs and reports a failure".to_owned(),
            input_schema: json!({ "type": "object" }),
            annotations: None,
            output_schema: None,
            execution: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        ToolResponse::error("the provider refused it".to_owned())
    }
}

pierre_tool_runtime::declare_security!(ReturnsAFailure => IRREVERSIBLE);

/// carnet#1405, the other exit: a call that came back and said it failed did nothing,
/// so it costs the turn nothing, the destructive slot included.
#[tokio::test]
async fn a_destructive_call_that_returns_a_failure_refunds_both_counts() {
    let tool: Arc<dyn RuntimeTool> = Arc::new(ReturnsAFailure);
    let provider = Arc::new(AnswersPlainly {
        models: vec!["plain-model".to_owned()],
    });
    let resources = create_test_server_resources_with_llm_and_tools(provider, vec![tool])
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let key = TurnKey::new(Some(tenant.as_uuid()), "failed-turn");

    let response = turn_executor(&resources, "failed-turn")
        .execute_tool(request("returns_a_failure", json!({}), user_id, tenant))
        .await
        .expect("a tool's own failure answers in-band");
    assert!(!response.success, "the tool reported a failure");

    let spent = resources
        .guardian_turns()
        .decide_and_reserve(
            &key,
            SecurityLabels::empty(),
            false,
            |state: &TurnState| (state.destructive_count(), state.write_count()),
            |_| CallFate::Unchanged,
        )
        .0;
    assert_eq!(spent, (0, 0), "the call said it did nothing");
}

/// A destructive write that runs and succeeds, for a turn's spend to accumulate.
struct CommitsAnAction;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CommitsAnAction {
    fn definition(&self) -> McpToolDefinition {
        McpToolDefinition {
            name: "commits_an_action".to_owned(),
            description: "Runs and reports success".to_owned(),
            input_schema: json!({ "type": "object" }),
            annotations: None,
            output_schema: None,
            execution: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        ToolResponse::text("done".to_owned())
    }
}

pierre_tool_runtime::declare_security!(CommitsAnAction => IRREVERSIBLE);

/// The default budget through the real chokepoint: one message gets three
/// irreversible calls and the fourth is denied. What the turn committed is what the
/// reply will name, so it lists the three that ran, and a call that failed is not in
/// it, nor did it spend any of the budget.
#[tokio::test]
async fn three_irreversible_calls_commit_and_the_fourth_is_denied() {
    let tools: Vec<Arc<dyn RuntimeTool>> =
        vec![Arc::new(CommitsAnAction), Arc::new(ReturnsAFailure)];
    let provider = Arc::new(AnswersPlainly {
        models: vec!["plain-model".to_owned()],
    });
    let resources = create_test_server_resources_with_llm_and_tools(provider, tools)
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let turn = "three-commits-turn";
    let key = TurnKey::new(Some(tenant.as_uuid()), turn);
    let executor = turn_executor(&resources, turn);
    let run = |tool: &'static str| {
        let call = executor.execute_tool(request(tool, json!({}), user_id, tenant));
        async move { call.await.expect("an in-band response") }
    };

    let failed = run("returns_a_failure").await;
    assert!(!failed.success, "the tool reported a failure");
    assert_eq!(guardian_reason(&failed), None, "it was not the Guardian's");
    assert_eq!(
        resources.guardian_turns().committed(&key),
        Vec::<String>::new(),
        "a failed call is not something the turn did"
    );

    for call in 1..=3 {
        let committed = run("commits_an_action").await;
        assert!(committed.success, "call {call} is in budget: {committed:?}");
    }
    assert_eq!(
        resources.guardian_turns().committed(&key),
        ["commits_an_action"; 3],
        "the failure is not something the turn did"
    );

    let fourth = run("commits_an_action").await;
    assert!(!fourth.success, "the fourth is past the budget: {fourth:?}");
    assert_eq!(
        guardian_error_code(&fourth).as_deref(),
        Some("guardian_denied")
    );
    assert_eq!(guardian_reason(&fourth), Some("budget_exceeded"));
    assert_eq!(
        resources.guardian_turns().committed(&key).len(),
        3,
        "a denied call never ran"
    );

    // What the loop hands the reply stage: the block, and the three that ran.
    let denial = finish_turn(&resources, user_id, tenant, turn)
        .await
        .guardian_denied
        .expect("the turn's block reaches the loop's result");
    assert_eq!(
        denial.tool_name, "commits_an_action",
        "the block names the denied call"
    );
    assert_eq!(
        denial.reason,
        DenyReason::BudgetExceeded.as_str(),
        "the block carries why it was denied"
    );
    assert_eq!(
        denial.committed, ["commits_an_action"; 3],
        "the reply must be able to say what went through"
    );
    assert_eq!(
        resources.guardian_turns().take_block(&key),
        None,
        "the loop took the entry when it returned"
    );

    // The next message gets a fresh budget and a clean list.
    let next_turn = "the-next-message";
    let fresh = turn_executor(&resources, next_turn)
        .execute_tool(request("commits_an_action", json!({}), user_id, tenant))
        .await
        .expect("an in-band response");
    assert!(fresh.success, "{fresh:?}");
    assert_eq!(
        resources
            .guardian_turns()
            .committed(&TurnKey::new(Some(tenant.as_uuid()), next_turn))
            .len(),
        1
    );
}

/// carnet#1403: both tools' own descriptions, on every surface that lists them, say
/// the push is a separate step the coach asks for, and neither repeats a budget a
/// super admin can change.
#[tokio::test]
async fn both_plan_tools_say_the_push_is_a_separate_step() {
    common::init_server_config();
    let resources = create_test_server_resources_with_config(test_server_config_with_guardian(
        GuardianSettings::default(),
    ))
    .await
    .expect("server resources");
    for tool in ["publish_training_plan", "push_training_plan"] {
        let description = resources
            .mcp
            .tool_registry
            .get(tool)
            .expect("registered")
            .definition()
            .description;
        assert!(
            description.contains("separate step") || description.contains("does not push"),
            "{tool}: {description}"
        );
        assert!(!description.contains("per turn"), "{tool}: {description}");
    }
}
