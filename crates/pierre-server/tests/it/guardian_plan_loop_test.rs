// ABOUTME: The armed plan-then-verify loop end to end: real registry and catalogue, scripted provider
// ABOUTME: Pins what a staged run reads: usage rows, kept failure text, activity list, $ref binding, stops

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `plan_mode=enforce` is off by default and guarded against arming, and the
//! planned loop was written and left: it dropped failure text, kept no usage,
//! and told the planner tool names and nothing about their parameters
//! (carnet#1401). A staged run against that would have looked like a broken
//! model. Each test here runs the loop with the real registry and a real offer,
//! against a provider that answers from a script and panics if asked for more,
//! so a call the loop must not make fails the test.

use crate::common;

use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use common::{
    create_test_server_resources_with_config, create_test_user, test_server_config_with_guardian,
};
use futures_util::stream;
use pierre_config::environment::GuardianSettings;
use pierre_core::errors::AppError;
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_llm::call_record::{LlmCallRecord, LlmCallRecorder};
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
    McpServerConfig, McpTransport, StreamChunk, TokenUsage, Tool,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::chat_stream::{
    TurnEvent, TOOL_STATUS_COMPLETED, TOOL_STATUS_IN_PROGRESS, TOOL_STATUS_NOT_RUN,
};
use pierre_tool_runtime::chat_catalogue::{build_mcp_tools, declarations};
use pierre_tool_runtime::guardian::{TurnKey, Workflow, WorkflowExecutor, PLAN_BINDING_FAILED};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution::run_planned_tool_loop;
use pierre_tool_runtime::tool_loop_io::{
    ToolLoopParams, ToolLoopResult, ToolMessageRecorder, ToolRoundRecord,
};
use pierre_tool_runtime::SecurityLabels;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Tokens each scripted completion reports.
const CALL_USAGE: (u32, u32, u32) = (100, 10, 110);
const SYNTHESIS: &str = "Here is what I found.";
/// A plan's read: no provider needed, and its output is known.
const READ: &str = "get_connection_status";
/// A plan's irreversible write: answers to the taint policy.
const SINK: &str = "disconnect_provider";

/// A provider that answers from a script and records every request whole.
struct Scripted {
    responses: Mutex<VecDeque<String>>,
    requests: Mutex<Vec<ChatRequest>>,
    models: Vec<String>,
}

impl Scripted {
    fn new(responses: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.iter().map(|s| (*s).to_owned()).collect()),
            requests: Mutex::new(Vec::new()),
            models: vec!["scripted-model".to_owned()],
        })
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for Scripted {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn display_name(&self) -> &'static str {
        "Scripted"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &'static str {
        "scripted-model"
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.requests.lock().unwrap().push(request.clone());
        let content = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("the loop asked the provider for a call the test did not script");
        let (prompt, completion, total) = CALL_USAGE;
        Ok(ChatResponse {
            content,
            model: "scripted-model".to_owned(),
            usage: Some(TokenUsage::new(prompt, completion, total)),
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

#[derive(Default)]
struct Rows(Mutex<Vec<LlmCallRecord>>);

impl LlmCallRecorder for Rows {
    fn record(&self, record: LlmCallRecord) {
        self.0.lock().unwrap().push(record);
    }
}

#[derive(Default)]
struct Rounds(Mutex<Vec<ToolRoundRecord>>);

impl ToolMessageRecorder for Rounds {
    fn record(&self, record: ToolRoundRecord) {
        self.0.lock().unwrap().push(record);
    }
}

/// A real athlete under a real registry, with the turn's recorders in reach.
struct Turn {
    resources: Arc<ServerContext>,
    executor: Arc<UniversalToolExecutor>,
    provider: ChatProvider,
    scripted: Arc<Scripted>,
    user_id: Uuid,
    tenant: TenantId,
    rows: Arc<Rows>,
    rounds: Arc<Rounds>,
    token: String,
}

async fn turn(settings: GuardianSettings, script: &[&str]) -> Turn {
    let resources =
        create_test_server_resources_with_config(test_server_config_with_guardian(settings))
            .await
            .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenant lookup")
        .first()
        .expect("the athlete has a tenant")
        .id;
    let token = Uuid::new_v4().to_string();
    let scripted = Scripted::new(script);
    Turn {
        executor: Arc::new(
            UniversalToolExecutor::new(resources.clone())
                .with_scopes(OAuthScope::self_grant())
                .with_transport(Transport::WebApp)
                .with_turn_token(token.clone()),
        ),
        provider: ChatProvider::Custom(scripted.clone()),
        scripted,
        resources,
        user_id,
        tenant,
        rows: Arc::new(Rows::default()),
        rounds: Arc::new(Rounds::default()),
        token,
    }
}

impl Turn {
    /// The offer a chat turn makes: every chat-callable tool in the registry.
    fn whole_offer(&self) -> Tool {
        build_mcp_tools(self.resources.tool_registry())
    }

    /// An offer of the named tools alone.
    fn offering(&self, names: &[&str]) -> Tool {
        declarations(
            self.resources
                .tool_registry()
                .chat_callable_schemas()
                .into_iter()
                .filter(|schema| names.contains(&schema.name.as_str()))
                .collect(),
        )
    }

    fn params<'a>(&'a self, tools: &'a Tool, user_id: &'a str) -> ToolLoopParams<'a> {
        ToolLoopParams {
            provider: &self.provider,
            executor: self.executor.clone(),
            tools,
            model: "scripted-model",
            user_id,
            tenant_id: self.tenant,
            max_iterations: 3,
            call_recorder: Some(self.rows.clone()),
            tool_message_recorder: Some(self.rounds.clone()),
            temperature: None,
            stream_sink: None,
            mcp_servers: Vec::new(),
        }
    }

    /// Run the loop over the whole offer.
    async fn run(&self) -> ToolLoopResult {
        let tools = self.whole_offer();
        let user_id = self.user_id.to_string();
        let params = self.params(&tools, &user_id);
        run_planned_tool_loop(&params, &mut prompt())
            .await
            .expect("the planned loop returns")
    }

    /// The per-call `llm_usage` rows, as `(sequence, total tokens)`.
    fn usage_rows(&self) -> Vec<(Option<i64>, i64)> {
        self.rows
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|row| (row.call_sequence, row.prompt_tokens + row.completion_tokens))
            .collect()
    }

    /// The tool text the turn persisted, one entry per round.
    fn recorded_rounds(&self) -> Vec<String> {
        self.rounds
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|round| round.tool_result_text.clone())
            .collect()
    }

    /// Everything the synthesis call was sent, as one string.
    fn synthesis_prompt(&self) -> String {
        let requests = self.scripted.requests();
        assert_eq!(requests.len(), 2, "one plan call and one synthesis");
        requests[1]
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Taint this turn out of band: the static verifier cannot see it, so a
    /// sink later in a verified plan is the runtime Guardian's to stop.
    fn taint(&self) {
        self.resources.guardian_turns().record_taint(
            &TurnKey::new(Some(self.tenant.as_uuid()), self.token.clone()),
            "seeded_source",
            SecurityLabels::UNTRUSTED_OUTPUT,
        );
    }
}

fn prompt() -> Vec<ChatMessage> {
    vec![
        ChatMessage::system("You are a test coach."),
        ChatMessage::user("What is connected?"),
    ]
}

fn usage_of(result: &ToolLoopResult) -> u32 {
    result
        .usage
        .as_ref()
        .expect("the turn spent tokens")
        .total_tokens
}

#[tokio::test]
async fn a_two_step_ref_plan_binds_the_first_output_into_the_second() {
    let plan = format!(
        r#"{{"steps":[
            {{"id":0,"tool":"{READ}","args":{{"provider":"strava"}}}},
            {{"id":1,"tool":"{READ}","args":{{"provider":{{"$ref":{{"step":0,"path":"/status"}}}}}}}}
        ]}}"#
    );
    let t = turn(GuardianSettings::default(), &[&plan, SYNTHESIS]).await;

    let result = t.run().await;

    assert_eq!(result.content, SYNTHESIS);
    assert_eq!(result.tools_called, [READ, READ], "both steps ran");
    assert_eq!(result.tool_calls_count, 2);
    assert!(result.guardian_denied.is_none());
    assert_eq!(
        usage_of(&result),
        CALL_USAGE.2 * 2,
        "the plan call and the synthesis both count"
    );
    assert_eq!(
        t.usage_rows(),
        [(Some(1), 110), (Some(2), 110)],
        "one llm_usage row per call, in sequence"
    );
    let synthesis = t.synthesis_prompt();
    assert_eq!(
        synthesis
            .matches(&format!("[Tool Result for {READ}]:"))
            .count(),
        2,
        "each step's result reaches the synthesis: {synthesis}"
    );
    assert_eq!(
        t.recorded_rounds().len(),
        1,
        "the run persists one round, so a follow-up turn replays its evidence"
    );
    assert!(t.recorded_rounds()[0].contains(READ));
}

#[tokio::test]
async fn workflow_executor_binds_a_reference_to_the_real_output() {
    let t = turn(GuardianSettings::default(), &[]).await;
    let user_id = t.user_id.to_string();
    let workflow = Workflow::from_json(&format!(
        r#"{{"steps":[
            {{"id":0,"tool":"{READ}","args":{{"provider":"strava"}}}},
            {{"id":1,"tool":"{READ}","args":{{"provider":{{"$ref":{{"step":0,"path":"/status"}}}}}}}}
        ]}}"#
    ))
    .expect("a parseable plan");

    let run = WorkflowExecutor::new(&t.executor, &user_id, t.tenant)
        .run(&workflow)
        .await;

    assert!(run.denial.is_none());
    assert_eq!(run.outputs.len(), 2);
    assert!(run.outputs.iter().all(|output| output.success));
    assert_eq!(
        run.outputs[1].result.get("provider"),
        run.outputs[0].result.get("status"),
        "the second step asked about what the first one returned"
    );
}

#[tokio::test]
async fn a_mid_plan_denial_keeps_the_earlier_step_and_never_reaches_a_synthesis() {
    let plan = format!(
        r#"{{"steps":[
            {{"id":0,"tool":"{READ}","args":{{}}}},
            {{"id":1,"tool":"{SINK}","args":{{"provider":"strava"}}}}
        ]}}"#
    );
    // One scripted call: a synthesis would panic the provider.
    let t = turn(
        GuardianSettings {
            mode: Some("enforce".to_owned()),
            tainted_destructive: Some("deny".to_owned()),
            ..GuardianSettings::default()
        },
        &[&plan],
    )
    .await;
    t.taint();

    let result = t.run().await;

    let denial = result.guardian_denied.as_ref().expect("the sink is denied");
    assert_eq!(denial.tool_name, SINK);
    assert_eq!(denial.reason, "tainted_sink");
    assert_eq!(
        result.tools_called,
        [READ],
        "only what ran is evidence, and the denied step did not"
    );
    assert_eq!(result.tool_calls_count, 2, "two steps were dispatched");
    assert_eq!(
        usage_of(&result),
        CALL_USAGE.2,
        "the plan call's tokens count on this exit too"
    );
    assert_eq!(t.usage_rows(), [(Some(1), 110)]);
    let rounds = t.recorded_rounds();
    assert_eq!(rounds.len(), 1, "the earlier step stays on the record");
    assert!(rounds[0].contains(READ));
    assert!(
        !rounds[0].contains(SINK),
        "the denied step's refusal is not evidence: {}",
        rounds[0]
    );
}

#[tokio::test]
async fn a_mid_plan_park_renders_the_ask_with_its_usage() {
    let plan =
        format!(r#"{{"steps":[{{"id":0,"tool":"{SINK}","args":{{"provider":"strava"}}}}]}}"#);
    let t = turn(
        GuardianSettings {
            mode: Some("enforce".to_owned()),
            tainted_destructive: Some("confirm".to_owned()),
            ..GuardianSettings::default()
        },
        &[&plan],
    )
    .await;
    t.taint();

    let result = t.run().await;

    let confirm = result
        .guardian_confirm
        .as_ref()
        .expect("the sink is parked");
    assert_eq!(confirm.tool_name, SINK);
    assert!(result.tools_called.is_empty(), "a parked step has not run");
    assert_eq!(usage_of(&result), CALL_USAGE.2);
}

#[tokio::test]
async fn a_failed_step_keeps_its_failure_text_for_the_synthesis() {
    // The tool runs and refuses a provider it has never heard of.
    let plan = format!(
        r#"{{"steps":[{{"id":0,"tool":"{SINK}","args":{{"provider":"no-such-provider"}}}}]}}"#
    );
    let t = turn(GuardianSettings::default(), &[&plan, SYNTHESIS]).await;

    let result = t.run().await;

    assert_eq!(result.content, SYNTHESIS);
    assert!(
        result.tools_called.is_empty(),
        "a step that failed is not evidence it happened: {:?}",
        result.tools_called
    );
    assert_eq!(result.tool_calls_count, 1);
    let synthesis = t.synthesis_prompt();
    let line = synthesis
        .lines()
        .find(|line| line.starts_with(&format!("[Tool Result for {SINK}]:")))
        .unwrap_or_else(|| panic!("the step's result is missing: {synthesis}"));
    assert!(
        line.contains("\"error\":") && !line.contains("null"),
        "a truthful failure, not an empty result: {line}"
    );
}

#[tokio::test]
async fn a_reference_that_does_not_resolve_stops_the_plan_without_a_panic() {
    let plan = format!(
        r#"{{"steps":[
            {{"id":0,"tool":"{READ}","args":{{}}}},
            {{"id":1,"tool":"{SINK}","args":{{"provider":{{"$ref":{{"step":0,"path":"/absent"}}}}}}}}
        ]}}"#
    );
    let t = turn(GuardianSettings::default(), &[&plan]).await;

    let result = t.run().await;

    let denial = result.guardian_denied.as_ref().expect("the stop renders");
    assert_eq!(denial.reason, PLAN_BINDING_FAILED);
    assert_eq!(denial.tool_name, SINK);
    assert_eq!(
        result.tools_called,
        [READ],
        "what ran before the stop stays"
    );
    assert_eq!(result.tool_calls_count, 1, "the unbound step never ran");
    assert_eq!(t.recorded_rounds().len(), 1);
}

#[tokio::test]
async fn a_reference_to_a_failed_step_stops_the_plan_rather_than_pass_its_error_on() {
    let plan = format!(
        r#"{{"steps":[
            {{"id":0,"tool":"{SINK}","args":{{"provider":"no-such-provider"}}}},
            {{"id":1,"tool":"{READ}","args":{{"provider":{{"$ref":{{"step":0}}}}}}}}
        ]}}"#
    );
    let t = turn(GuardianSettings::default(), &[&plan]).await;

    let result = t.run().await;

    let denial = result.guardian_denied.as_ref().expect("the stop renders");
    assert_eq!(denial.reason, PLAN_BINDING_FAILED);
    assert_eq!(denial.tool_name, READ);
    assert_eq!(result.tool_calls_count, 1, "the second step never ran");
}

#[tokio::test]
async fn an_activity_list_survives_the_plan_and_the_payload_is_projected() {
    let plan =
        r#"{"steps":[{"id":0,"tool":"get_activities","args":{"limit":10,"mode":"summary"}}]}"#;
    let t = turn(GuardianSettings::default(), &[plan, SYNTHESIS]).await;
    let repos = &t.resources.common.repos;
    repos
        .provider_connections
        .register_connection(t.user_id, t.tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        "strava-ride-1".to_owned(),
        "Sortie longue".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(2),
        7_200,
        "strava".to_owned(),
    )
    .distance_meters(200_000.0)
    .build();
    repos
        .activity_cache
        .upsert_activities(t.user_id, &t.tenant, "strava", &[ride])
        .await
        .unwrap();
    // Elected last, so it is the dead primary a healthy sibling's window is
    // served without.
    repos
        .provider_connections
        .register_connection(t.user_id, t.tenant, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let result = t.run().await;

    assert_eq!(result.tools_called, ["get_activities"]);
    let list = result
        .activity_list
        .as_deref()
        .expect("the list the athlete sees is kept");
    assert!(list.contains("Sortie longue"), "{list}");
    assert!(
        result.served_without_provider.is_some(),
        "the reconnect offer travels with the window, as on a ReAct loop"
    );
    let synthesis = t.synthesis_prompt();
    assert!(
        synthesis.contains("Sortie longue") && !synthesis.contains("sciotte_garmin"),
        "the model reads the projection a ReAct loop gives it, which keeps the prose and \
         drops the backend key the raw payload carries for the reconnect mint: {synthesis}"
    );
}

#[tokio::test]
async fn a_dead_connection_surfaces_as_the_reconnect_signal_and_stops_the_plan() {
    let plan = format!(
        r#"{{"steps":[
            {{"id":0,"tool":"get_activities","args":{{"limit":10,"mode":"summary"}}}},
            {{"id":1,"tool":"{READ}","args":{{}}}}
        ]}}"#
    );
    let t = turn(GuardianSettings::default(), &[&plan]).await;
    t.resources
        .common
        .repos
        .provider_connections
        .register_connection(t.user_id, t.tenant, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let result = t.run().await;

    assert!(
        result.pending_provider_auth_required.is_some(),
        "the athlete gets the reconnect reply, as on a ReAct loop: {result:?}"
    );
    assert!(result.guardian_denied.is_none());
    assert_eq!(
        result.tool_calls_count, 1,
        "nothing runs behind a reply that will only ask to reconnect"
    );
    assert_eq!(usage_of(&result), CALL_USAGE.2);
}

#[tokio::test]
async fn a_tool_the_turn_did_not_offer_is_rejected_before_anything_runs() {
    let plan = format!(
        r#"{{"steps":[
            {{"id":0,"tool":"{READ}","args":{{}}}},
            {{"id":1,"tool":"{SINK}","args":{{"provider":"strava"}}}}
        ]}}"#
    );
    let t = turn(GuardianSettings::default(), &[&plan]).await;
    let tools = t.offering(&[READ]);
    let user_id = t.user_id.to_string();
    let params = t.params(&tools, &user_id);

    let result = run_planned_tool_loop(&params, &mut prompt())
        .await
        .expect("the planned loop returns");

    assert_eq!(
        result.guardian_denied.as_ref().map(|d| d.reason.as_str()),
        Some("plan_tool_not_offered"),
        "the registry knows it; this turn did not offer it"
    );
    assert_eq!(result.tool_calls_count, 0, "rejected before the first step");
    assert_eq!(
        result.finish_reason.as_deref(),
        Some("guardian_plan_rejected")
    );
    assert_eq!(
        usage_of(&result),
        CALL_USAGE.2,
        "the plan call's tokens count"
    );
    assert_eq!(t.usage_rows(), [(Some(1), 110)]);

    // The control: the same plan over an offer that holds both runs them.
    let t = turn(GuardianSettings::default(), &[&plan, SYNTHESIS]).await;
    let tools = t.offering(&[READ, SINK]);
    let user_id = t.user_id.to_string();
    let params = t.params(&tools, &user_id);
    let result = run_planned_tool_loop(&params, &mut prompt())
        .await
        .expect("the planned loop returns");
    assert!(result.guardian_denied.is_none(), "{result:?}");
    assert_eq!(result.tool_calls_count, 2);
}

#[tokio::test]
async fn each_rejection_is_told_apart_in_its_reason() {
    let dangling = format!(
        r#"{{"steps":[{{"id":0,"tool":"{READ}","args":{{"provider":{{"$ref":{{"step":9}}}}}}}}]}}"#
    );
    let too_large = format!(
        r#"{{"steps":[{}]}}"#,
        (0..33)
            .map(|id| format!(r#"{{"id":{id},"tool":"{READ}","args":{{}}}}"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    let unknown = r#"{"steps":[{"id":0,"tool":"nonexistent_tool","args":{}}]}"#.to_owned();

    for (plan, reason) in [
        (dangling, "plan_dangling_reference"),
        (too_large, "plan_too_large"),
        (unknown, "plan_unknown_tool"),
    ] {
        let t = turn(GuardianSettings::default(), &[&plan]).await;
        let result = t.run().await;
        assert_eq!(
            result.guardian_denied.as_ref().map(|d| d.reason.as_str()),
            Some(reason),
            "{plan}"
        );
        assert_eq!(usage_of(&result), CALL_USAGE.2, "{reason}");
    }
}

#[tokio::test]
async fn an_unparseable_plan_degrades_with_the_plan_calls_usage_kept() {
    let t = turn(GuardianSettings::default(), &["not a plan", SYNTHESIS]).await;

    let result = t.run().await;

    assert_eq!(result.content, SYNTHESIS);
    assert_eq!(
        usage_of(&result),
        CALL_USAGE.2 + CALL_USAGE.2,
        "the plan call was paid for, whatever came after"
    );
    assert_eq!(
        t.usage_rows(),
        [(None, 110), (Some(1), 110)],
        "the ReAct loop numbers from 1, so the plan call claims no sequence"
    );
}

#[tokio::test]
async fn the_planner_is_told_the_parameters_and_no_other_call_format() {
    let t = turn(GuardianSettings::default(), &[r#"{"steps":[]}"#, SYNTHESIS]).await;

    t.run().await;

    let requests = t.scripted.requests();
    let planner_message = &requests[0].messages[0].content;
    let grammar = t.resources.guardian_planner_prompt();
    assert!(
        planner_message.starts_with(grammar.trim()),
        "the plan grammar leads"
    );
    assert!(
        planner_message.contains("You are a test coach."),
        "the persona stays in the one system message"
    );
    assert!(
        planner_message.contains(&format!("### {SINK}")),
        "the offered tool is named"
    );
    let section = planner_message
        .split(&format!("### {SINK}"))
        .nth(1)
        .expect("the tool's section")
        .split("### ")
        .next()
        .expect("up to the next tool");
    assert!(
        section.contains("provider"),
        "and its parameter is described, so the plan can fill it: {section}"
    );
    assert!(
        !planner_message.contains("<tool_call>"),
        "a second call format contradicts the plan grammar and degrades the plan silently"
    );
    assert!(
        !requests[1].messages[0]
            .content
            .contains(&format!("### {SINK}")),
        "the synthesis is not asked to plan"
    );
}

#[tokio::test]
async fn the_turns_sampling_applies_and_no_tool_surface_rides_the_plan_calls() {
    let t = turn(GuardianSettings::default(), &[r#"{"steps":[]}"#, SYNTHESIS]).await;
    let tools = t.offering(&[READ]);
    let user_id = t.user_id.to_string();
    let mut params = t.params(&tools, &user_id);
    params.temperature = Some(0.25);
    params.mcp_servers = vec![McpServerConfig {
        name: "dravr".to_owned(),
        transport: McpTransport::Http {
            url: "http://127.0.0.1:1/mcp".to_owned(),
            headers: Vec::new(),
        },
    }];

    run_planned_tool_loop(&params, &mut prompt())
        .await
        .expect("the planned loop returns");

    for request in t.scripted.requests() {
        assert_eq!(request.temperature, Some(0.25));
        assert!(
            request.mcp_servers.is_empty(),
            "a runner given these servers could call tools around the verifier"
        );
    }
}

#[tokio::test]
async fn each_step_is_reported_to_the_turns_sink() {
    let plan = format!(
        r#"{{"steps":[
            {{"id":0,"tool":"{READ}","args":{{}}}},
            {{"id":1,"tool":"{SINK}","args":{{"provider":"strava"}}}}
        ]}}"#
    );
    let t = turn(
        GuardianSettings {
            mode: Some("enforce".to_owned()),
            tainted_destructive: Some("deny".to_owned()),
            ..GuardianSettings::default()
        },
        &[&plan],
    )
    .await;
    t.taint();
    let tools = t.offering(&[READ, SINK]);
    let user_id = t.user_id.to_string();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut params = t.params(&tools, &user_id);
    params.stream_sink = Some(tx);

    run_planned_tool_loop(&params, &mut prompt())
        .await
        .expect("the planned loop returns");
    drop(params);

    let mut frames = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let TurnEvent::Progress(progress) = event {
            frames.push((progress.title, progress.status));
        }
    }
    assert_eq!(
        frames,
        [
            (READ.to_owned(), TOOL_STATUS_IN_PROGRESS.to_owned()),
            (READ.to_owned(), TOOL_STATUS_COMPLETED.to_owned()),
            (SINK.to_owned(), TOOL_STATUS_IN_PROGRESS.to_owned()),
            (SINK.to_owned(), TOOL_STATUS_NOT_RUN.to_owned()),
        ],
        "the same trail a ReAct loop draws, with the blocked step shown as never run"
    );
}
