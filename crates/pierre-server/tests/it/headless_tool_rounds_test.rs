// ABOUTME: A Copilot headless turn writes the tool rounds its runtime ran, so the next turn replays their evidence
// ABOUTME: Drives finalize_headless_turn into the chat repository's recorder and reads the tool_result rows back (ADR-016)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! ADR-016 persists each tool round so a follow-up turn replays the evidence
//! behind a grounded answer. The API and text loops record their rounds as
//! they run them; the Copilot runtime runs its calls itself, so the headless
//! path records what the runtime reported once the turn ends (carnet#1343).
//! Without it every turn a Copilot tier served left a grounded reply with
//! nothing behind it, and the next turn refused what it had just answered.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError,
};
use pierre_chat_pipeline::recorders::ChatRepoToolMessageRecorder;
use pierre_chat_pipeline::TurnOrigin;
use pierre_core::models::{MessageRecord, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::{
    ChatProvider, EmbacleProvider, HeadlessEventStream, HeadlessToolResponse, HeadlessTurnProvider,
    ObservedToolCall, Tool,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::ai_scope::{self, Provenance};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution::finalize_headless_turn;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolMessageRecorder};
use serde_json::json;
use uuid::Uuid;

use crate::common::create_test_server_resources;
use crate::db_fixtures::seed_user;

const MODEL: &str = "claude-sonnet-5.5";
const ANSWER: &str = "Trois sorties cette semaine, 92 km au total : la charge monte doucement.";
const ACTIVITIES: &str = "{\"activity_list\":\"Mon 31 km, Wed 28 km, Sat 33 km\",\"count\":3}";
const STATUS: &str = "{\"strava\":\"connected\"}";

/// A Copilot runtime whose only converse is the degenerate-turn retry: it
/// answers with `retry`, which the test scripts.
struct ScriptedRuntime {
    retry: Mutex<Option<HeadlessToolResponse>>,
    models: Vec<String>,
}

impl ScriptedRuntime {
    fn new(retry: Option<HeadlessToolResponse>) -> Self {
        Self {
            retry: Mutex::new(retry),
            models: vec![MODEL.to_owned()],
        }
    }
}

#[async_trait]
impl EmbacleLlmProvider for ScriptedRuntime {
    fn name(&self) -> &'static str {
        "copilot_sdk"
    }

    fn display_name(&self) -> &'static str {
        "Copilot SDK (scripted)"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES | LlmCapabilities::SDK_TOOL_CALLING
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        Err(RunnerError::internal("a headless turn never completes"))
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("a headless turn never completes"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

#[async_trait]
impl HeadlessTurnProvider for ScriptedRuntime {
    async fn converse(&self, _request: &ChatRequest) -> Result<HeadlessToolResponse, RunnerError> {
        self.retry
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| RunnerError::internal("the turn under test was not degenerate"))
    }

    async fn converse_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<HeadlessEventStream, RunnerError> {
        Err(RunnerError::internal("the turn under test does not stream"))
    }
}

/// A call the SDK transport reports: named, with the text it handed back.
fn sdk_call(id: &str, name: &str, result: &str) -> ObservedToolCall {
    ObservedToolCall {
        id: id.to_owned(),
        title: name.to_owned(),
        status: "Completed".to_owned(),
        name: Some(format!("dravr-{name}")),
        arguments: Some(json!({})),
        result: Some(result.to_owned()),
    }
}

fn reply(content: &str, tool_calls: Vec<ObservedToolCall>) -> HeadlessToolResponse {
    HeadlessToolResponse {
        content: content.to_owned(),
        model: MODEL.to_owned(),
        tool_calls,
        usage: None,
        finish_reason: Some("stop".to_owned()),
    }
}

/// One athlete's conversation, and the rows a headless turn left in it.
struct Thread {
    resources: Arc<ServerContext>,
    conversation_id: String,
    user_id: String,
    tenant_id: TenantId,
}

impl Thread {
    async fn new() -> Self {
        let resources = create_test_server_resources().await.unwrap();
        let (user, tenant_id) = seed_user(&resources.agent.database).await;
        let user_id = user.to_string();
        let conversation = resources
            .common
            .repos
            .chat
            .create_conversation(&user_id, tenant_id, "Headless", MODEL, None, None)
            .await
            .unwrap();
        Self {
            resources,
            conversation_id: conversation.id,
            user_id,
            tenant_id,
        }
    }

    /// Finish one headless turn with `first` through the production assembly,
    /// recording into this thread, and return the rows it wrote.
    ///
    /// The recorder is a platform-composed turn's, which holds its rounds
    /// until `persist_held`: the rows are then on disk when this returns,
    /// rather than on a spawned write the test would have to wait out.
    async fn finish(&self, first: HeadlessToolResponse, runtime: ScriptedRuntime) -> String {
        let recorder = Arc::new(ChatRepoToolMessageRecorder::new(
            Arc::clone(&self.resources.common.repos.chat),
            self.conversation_id.clone(),
            self.user_id.clone(),
            self.tenant_id,
            TurnOrigin::Platform,
        ));
        let tool_runtime: Arc<dyn ToolRuntime> = self.resources.clone();
        let executor = Arc::new(
            UniversalToolExecutor::new(tool_runtime).with_scopes(OAuthScope::self_grant()),
        );
        let runtime = Arc::new(runtime);
        let provider = ChatProvider::Embacle(EmbacleProvider::from_runner(
            Box::new(Arc::clone(&runtime)),
            "Copilot SDK (scripted)",
        ));
        let tools = Tool {
            function_declarations: Vec::new(),
        };
        let request = ChatRequest::new(Vec::new());
        let recording: Arc<dyn ToolMessageRecorder> =
            Arc::<ChatRepoToolMessageRecorder>::clone(&recorder);
        let params = ToolLoopParams {
            provider: &provider,
            executor,
            tools: &tools,
            model: MODEL,
            user_id: &self.user_id,
            tenant_id: self.tenant_id,
            max_iterations: 1,
            call_recorder: None,
            tool_message_recorder: Some(recording),
            temperature: None,
            stream_sink: None,
            mcp_servers: Vec::new(),
        };
        let result = finalize_headless_turn(
            first,
            None,
            runtime.as_ref(),
            &request,
            &params,
            "prompt",
            &mut Vec::new(),
        )
        .await
        .expect("the turn finishes");
        recorder.persist_held().await;
        result.content
    }

    /// A group member whose data a turn in this thread may read.
    async fn member(&self) -> Uuid {
        seed_user(&self.resources.agent.database).await.0
    }

    async fn rows(&self) -> Vec<MessageRecord> {
        self.resources
            .common
            .repos
            .chat
            .get_messages(&self.conversation_id, &self.user_id, self.tenant_id)
            .await
            .unwrap()
    }

    async fn roles_and_contents(&self) -> Vec<(String, String)> {
        self.rows()
            .await
            .into_iter()
            .map(|row| (row.role, row.content))
            .collect()
    }
}

/// The regression itself: the calls the runtime ran reach the thread as the
/// round's `tool_result` row, in the `[Tool Result for <tool>]` shape the API
/// loop writes, under the platform's bare tool names.
#[tokio::test]
async fn a_headless_turn_writes_the_round_its_runtime_ran() {
    let thread = Thread::new().await;

    let content = thread
        .finish(
            reply(
                ANSWER,
                vec![
                    sdk_call("toolu_01", "get_connection_status", STATUS),
                    sdk_call("toolu_02", "get_activities", ACTIVITIES),
                ],
            ),
            ScriptedRuntime::new(None),
        )
        .await;

    assert_eq!(content, ANSWER, "the runtime's answer is the turn's reply");
    assert_eq!(
        thread.roles_and_contents().await,
        vec![(
            "tool_result".to_owned(),
            format!(
                "[Tool Result for get_connection_status]: {STATUS}\n\n\
                 [Tool Result for get_activities]: {ACTIVITIES}"
            ),
        )],
        "the round the runtime ran is the evidence the next turn replays"
    );
}

/// A degenerate first pass and its retry each ran calls of their own: both
/// rounds are evidence, written in the order they ran.
#[tokio::test]
async fn a_retried_turn_writes_both_rounds_in_order() {
    let thread = Thread::new().await;

    let content = thread
        .finish(
            reply(
                "",
                vec![sdk_call("toolu_01", "get_connection_status", STATUS)],
            ),
            ScriptedRuntime::new(Some(reply(
                ANSWER,
                vec![sdk_call("toolu_02", "get_activities", ACTIVITIES)],
            ))),
        )
        .await;

    assert_eq!(content, ANSWER, "the retry's answer is the turn's");
    assert_eq!(
        thread.roles_and_contents().await,
        vec![
            (
                "tool_result".to_owned(),
                format!("[Tool Result for get_connection_status]: {STATUS}"),
            ),
            (
                "tool_result".to_owned(),
                format!("[Tool Result for get_activities]: {ACTIVITIES}"),
            ),
        ],
        "the first pass's round and the retry's are both evidence, in the order they ran"
    );
}

/// The ACP adapter reports a title and a status, never the result: there is
/// no evidence to keep, and nothing is invented in its place. A turn that ran
/// no tool writes no round either.
#[tokio::test]
async fn calls_reported_without_a_result_write_no_round() {
    let thread = Thread::new().await;
    let acp_call = ObservedToolCall {
        id: "acp-tool-1".to_owned(),
        title: "get_activities".to_owned(),
        status: "Completed".to_owned(),
        ..ObservedToolCall::default()
    };

    thread
        .finish(reply(ANSWER, vec![acp_call]), ScriptedRuntime::new(None))
        .await;
    thread
        .finish(reply(ANSWER, Vec::new()), ScriptedRuntime::new(None))
        .await;

    assert!(
        thread.rows().await.is_empty(),
        "no result reported means no round to persist"
    );
}

/// The loopback calls mark the turn's own provenance, so the round is about
/// the one group member the turn read, and a turn that read two keeps no
/// round, as on every other path (carnet#1174).
#[tokio::test]
async fn the_round_names_the_member_the_turn_read() {
    let one = Thread::new().await;
    let two = Thread::new().await;
    // A row names its member by foreign key, so each member is a real account.
    let alice = one.member().await;
    let (alice_two, bruno) = (two.member().await, two.member().await);
    let round = || {
        reply(
            ANSWER,
            vec![sdk_call("toolu_01", "get_activities", ACTIVITIES)],
        )
    };

    ai_scope::tracking(Provenance::new(), async {
        ai_scope::note_member_read(alice);
        one.finish(round(), ScriptedRuntime::new(None)).await;
    })
    .await;
    ai_scope::tracking(Provenance::new(), async {
        ai_scope::note_member_read(alice_two);
        ai_scope::note_member_read(bruno);
        two.finish(round(), ScriptedRuntime::new(None)).await;
    })
    .await;

    let about: BTreeSet<Option<Uuid>> = one
        .rows()
        .await
        .into_iter()
        .map(|row| row.about_user_id)
        .collect();
    assert_eq!(
        about,
        BTreeSet::from([Some(alice)]),
        "the round's row names the one member the turn read"
    );
    assert!(
        two.rows().await.is_empty(),
        "a round about two members cannot name both, so it is not kept"
    );
}
