// ABOUTME: The athlete's stop ends their own running in-app turn on the server, is tied to that turn's question, and is accounted for
// ABOUTME: Drives the chat routes with hung, tool-calling and parked providers; asserts the transcript, the wire, the usage counters and who may stop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A turn is detached from the request that started it, so dropping the
//! stream never ends it (carnet#109, carnet#500). The only way to end one on
//! purpose is `POST /api/chat/conversations/{id}/stop` (carnet#705): it
//! writes the stopped notice naming the caller's own unanswered question, and
//! the turn answering that question ends on reading it — whatever tool rows
//! landed since, whoever else is in the conversation, and with the message
//! and the tokens it spent counted against the athlete's caps.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;
mod helpers;

use pierre_core::transport::TransportPolicy;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::Router;
use dravr_tronc::mcp::schema::{Content, Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use futures_util::stream;
use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages;
use pierre_chat_pipeline::turn_stop::{
    author_marker, latest_question_by, question_state, stop_marker, QuestionState,
};
use pierre_contremaitre::messaging_strings::{DEFAULT_LOCALE, KEY_TURN_STOPPED};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_core::models::{TenantId, User, STOPPED_TURN_FINISH_REASON};
use pierre_database::database::MessageRecord;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_runtime_context::default_admin_config;
use pierre_services::usage_counter::UsageCounterService;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::RuntimeTool;
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

use common::{
    create_test_server_resources_with_chat_provider,
    create_test_server_resources_with_chat_provider_and_tools, create_test_tenant,
};
use helpers::axum_test::AxumTestRequest;
use helpers::drained_turn::{
    wait_for_a_tracked_turn, wait_for_turns_to_finish, HangingProvider, ParkedProvider,
};

const QUESTION: &str = "combien de watts sur ma dernière sortie ?";

/// Long enough for the turn's one-second stop poll to fire a few times over.
const STOP_BUDGET: Duration = Duration::from_secs(15);

/// The tool the tool-round provider asks for.
const STUB_TOOL_NAME: &str = "turn_stop_stub_tool";
/// What the tool-round provider says before it asks for the tool.
const TOOL_ROUND_PREAMBLE: &str = "Je regarde ta dernière sortie.";
/// What the tool-round provider reports its one finished call cost.
const FIRST_CALL_PROMPT_TOKENS: u32 = 100;
const FIRST_CALL_COMPLETION_TOKENS: u32 = 20;

/// A no-auth tool returning canned data, so the tool loop completes a round.
struct StubTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for StubTool {
    fn definition(&self) -> Tool {
        Tool {
            name: STUB_TOOL_NAME.to_owned(),
            description: "Turn-stop stub tool — returns canned data so a tool round completes."
                .to_owned(),
            input_schema: json!({"type": "object"}),
            annotations: None,
            output_schema: None,
            execution: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let payload = json!({"watts": 212});
        ToolResponse {
            content: vec![Content::text(payload.to_string())],
            is_error: false,
            structured_content: Some(payload),
        }
    }
}

pierre_tool_runtime::declare_security!(StubTool => empty);

/// A provider whose first call asks for a tool — and reports what the call
/// cost — and whose every later call never returns: a turn stopped after its
/// tool round, which is where the tool rows sit behind the question.
#[derive(Default)]
struct ToolRoundThenHangProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl LlmProvider for ToolRoundThenHangProvider {
    fn name(&self) -> &'static str {
        "tool_round_then_hang_mock"
    }
    fn display_name(&self) -> &'static str {
        "Tool Round Then Hang Mock LLM (turn-stop e2e)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        // No function calling: the dispatcher takes the text tool loop, which
        // reads `<tool_call>` blocks out of `complete()`.
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            sleep(Duration::from_mins(10)).await;
            return Err(AppError::internal("the second call must never answer"));
        }
        Ok(ChatResponse {
            // The sentence ahead of the block is what survives the recorder's
            // scaffolding strip, so the round leaves a `tool_call` row behind
            // the question — a round of pure scaffolding stores nothing.
            content: format!(
                r#"{TOOL_ROUND_PREAMBLE} <tool_call>{{"name":"{STUB_TOOL_NAME}","arguments":{{}}}}</tool_call>"#
            ),
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(
                FIRST_CALL_PROMPT_TOKENS,
                FIRST_CALL_COMPLETION_TOKENS,
                FIRST_CALL_PROMPT_TOKENS + FIRST_CALL_COMPLETION_TOKENS,
            )),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        sleep(Duration::from_mins(10)).await;
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

struct Fixture {
    resources: Arc<ServerContext>,
    router: Router,
    user: User,
    token: String,
    conversation_id: String,
}

async fn fixture_over(resources: Arc<ServerContext>, email: &str) -> Fixture {
    let (user, token) = create_test_tenant(&resources, email)
        .await
        .expect("seed user + tenant");
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let created = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({"title": "stopped", "model": "gemini-1.5-flash"}))
        .send(router.clone())
        .await;
    assert_eq!(created.status_code(), StatusCode::CREATED);
    let conversation: Value = created.json();
    let conversation_id = conversation["id"].as_str().expect("id").to_owned();

    Fixture {
        resources,
        router,
        user,
        token,
        conversation_id,
    }
}

/// A conversation whose turns park on a provider that never answers.
async fn fixture(email: &str) -> Fixture {
    let provider: Arc<dyn LlmProvider> = Arc::new(HangingProvider);
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .expect("resources");
    fixture_over(resources, email).await
}

/// A conversation whose turns run one tool round and then park.
async fn tool_round_fixture(email: &str) -> Fixture {
    let provider: Arc<dyn LlmProvider> = Arc::new(ToolRoundThenHangProvider::default());
    let tool: Arc<dyn RuntimeTool> = Arc::new(StubTool);
    let resources = create_test_server_resources_with_chat_provider_and_tools(provider, vec![tool])
        .await
        .expect("resources");
    fixture_over(resources, email).await
}

impl Fixture {
    async fn tenant(&self) -> TenantId {
        self.resources
            .common
            .repos
            .tenants
            .list_for_user(self.user.id)
            .await
            .unwrap()[0]
            .id
    }

    async fn messages(&self) -> Vec<MessageRecord> {
        self.resources
            .common
            .repos
            .chat
            .get_messages(
                &self.conversation_id,
                &self.user.id.to_string(),
                self.tenant().await,
            )
            .await
            .unwrap()
    }

    /// Wait until the conversation holds a row with `role`.
    async fn wait_for_a_row(&self, role: &str) {
        let mut stored = false;
        for _ in 0..150 {
            stored = self.messages().await.iter().any(|m| m.role == role);
            if stored {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(stored, "the turn never stored a `{role}` row");
    }

    /// Open a turn over SSE and hang up: only the headers are read, the stream
    /// is dropped, and the turn keeps running detached.
    async fn ask_and_hang_up(&self, token: &str) {
        let response = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{}/messages",
            self.conversation_id
        ))
        .header("authorization", &format!("Bearer {token}"))
        .header("accept", "text/event-stream")
        .json(&json!({"content": QUESTION}))
        .send_sse(self.router.clone())
        .await;
        assert_eq!(response.status_code(), StatusCode::OK, "the stream opened");
        assert!(
            wait_for_a_tracked_turn(&self.resources).await,
            "the turn is running, detached from the dropped stream"
        );
        self.wait_for_a_row("user").await;
    }

    async fn stop(&self, token: &str) -> (StatusCode, Value) {
        let response = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{}/stop",
            self.conversation_id
        ))
        .header("authorization", &format!("Bearer {token}"))
        .send(self.router.clone())
        .await;
        let status = response.status_code();
        (status, response.json())
    }

    fn notice(&self) -> String {
        self.resources
            .mcp
            .messaging_strings_registry
            .get(KEY_TURN_STOPPED, DEFAULT_LOCALE)
    }

    /// The transcript a stopped turn with no tool round leaves: the question,
    /// then the notice naming it.
    async fn assert_stopped_transcript(&self) {
        let messages = self.messages().await;
        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(
            roles,
            vec!["user", "assistant"],
            "the question is on file and the stop closed it with exactly one row"
        );
        assert_eq!(messages[0].content, QUESTION);
        assert_eq!(messages[1].content, self.notice());
        assert_eq!(
            messages[1].finish_reason.as_deref(),
            Some(STOPPED_TURN_FINISH_REASON)
        );
        assert_eq!(
            messages[1].content_blocks,
            stop_marker(&messages[0].id),
            "the notice names the question it closed"
        );
    }

    async fn counter(&self, counter_type: &str) -> i64 {
        UsageCounterService::new(
            self.resources.common.repos.usage_counters.as_ref(),
            default_admin_config(),
        )
        .get_current(
            &self.tenant().await.to_string(),
            &self.user.id.to_string(),
            counter_type,
        )
        .await
        .unwrap()
    }
}

fn row(id: &str, role: &str, finish_reason: Option<&str>, blocks: Option<String>) -> MessageRecord {
    MessageRecord {
        id: id.to_owned(),
        conversation_id: "conv".to_owned(),
        role: role.to_owned(),
        content: format!("content of {id}"),
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: finish_reason.map(ToOwned::to_owned),
        content_blocks: blocks,
        created_at: "2026-10-01T10:00:00Z".to_owned(),
        transport_policy: TransportPolicy::AnyTransport,
    }
}

fn question(id: &str, author: &str) -> MessageRecord {
    row(id, "user", None, author_marker(author))
}

fn stop_of(id: &str, question_id: &str) -> MessageRecord {
    row(
        id,
        "assistant",
        Some(STOPPED_TURN_FINISH_REASON),
        stop_marker(question_id),
    )
}

#[tokio::test]
async fn the_stopped_notice_is_a_real_localized_sentence() {
    let fx = fixture("stop-notice@test.local").await;
    let notice = fx.notice();
    assert!(
        !notice.is_empty() && notice != KEY_TURN_STOPPED,
        "the catalogue carries the stopped notice in the default locale, got {notice:?}"
    );
}

/// A stop is found by the question it names, not by where it sits: tool rows
/// before it, tool rows after it and another member's question in between all
/// leave it in place — and leave everyone else's question alone.
#[test]
fn a_stop_is_tied_to_its_question_whatever_rows_surround_it() {
    let rows = vec![
        question("q-alice", "alice"),
        row("t1", "tool_call", None, None),
        row("t2", "tool_result", None, None),
        question("q-bob", "bob"),
        stop_of("stop-alice", "q-alice"),
        row("t3", "tool_call", None, None),
        row("t4", "tool_result", None, None),
    ];

    let stopped_by = match question_state(&rows, "q-alice") {
        Some(QuestionState::Stopped(notice)) => Some(notice.id),
        _ => None,
    };
    assert_eq!(
        stopped_by.as_deref(),
        Some("stop-alice"),
        "the notice names Alice's question, with tool rows on both sides of it"
    );
    assert!(
        matches!(
            question_state(&rows, "q-bob"),
            Some(QuestionState::Unanswered)
        ),
        "a notice naming Alice's question stops nothing of Bob's"
    );
    assert!(question_state(&rows, "not-in-these-rows").is_none());

    assert_eq!(
        latest_question_by(&rows, "alice").map(|q| q.id.as_str()),
        Some("q-alice"),
        "Bob's newer question is not Alice's"
    );
    assert_eq!(
        latest_question_by(&rows, "bob").map(|q| q.id.as_str()),
        Some("q-bob")
    );
    assert!(latest_question_by(&rows, "carol").is_none());
}

#[test]
fn tool_rows_do_not_answer_a_question_and_a_reply_does() {
    let working = vec![
        question("q", "alice"),
        row("t1", "tool_call", None, None),
        row("t2", "tool_result", None, None),
    ];
    assert!(matches!(
        question_state(&working, "q"),
        Some(QuestionState::Unanswered)
    ));

    let mut answered = working;
    answered.push(row("reply", "assistant", Some("stop"), None));
    assert!(matches!(
        question_state(&answered, "q"),
        Some(QuestionState::Answered)
    ));
}

/// The stopped notice is the platform's row. Replayed into the next prompt it
/// would read as the agent's own turn.
#[test]
fn a_stopped_notice_never_re_enters_a_prompt() {
    let history = vec![
        question("q1", "alice"),
        stop_of("stop", "q1"),
        question("q2", "alice"),
    ];
    let (messages, sources) = build_llm_messages(None, &history);
    let contents: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(
        contents,
        vec!["content of q1", "content of q2"],
        "both questions replay; the notice between them does not"
    );
    assert_eq!(sources, vec![Some("q1".to_owned()), Some("q2".to_owned())]);
}

/// The athlete hangs up the stream — which must not end the turn — then
/// stops it. The turn, still parked on the provider, ends on the server.
#[tokio::test]
async fn stopping_a_detached_sse_turn_ends_it_on_the_server() {
    let fx = fixture("stop-sse@test.local").await;
    fx.ask_and_hang_up(&fx.token).await;

    let (status, body) = fx.stop(&fx.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"stopped": true}));

    assert!(
        wait_for_turns_to_finish(&fx.resources, STOP_BUDGET).await,
        "the hung turn must end once it reads the stop, not run on"
    );
    fx.assert_stopped_transcript().await;

    // The turn markers on both rows are bookkeeping: a reload draws neither.
    let listed = AxumTestRequest::get(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation_id
    ))
    .header("authorization", &format!("Bearer {}", fx.token))
    .send(fx.router.clone())
    .await;
    assert_eq!(listed.status_code(), StatusCode::OK);
    let listed: Value = listed.json();
    let rows = listed["messages"].as_array().expect("messages");
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert!(
            row.get("scene_blocks").is_none() && row.get("actions").is_none(),
            "a turn marker reached the client as a block: {row}"
        );
    }
    assert_eq!(rows[1]["content"], fx.notice());
    assert_eq!(rows[1]["finish_reason"], "stopped");

    // A second stop finds the question closed and changes nothing.
    let (status, body) = fx.stop(&fx.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"stopped": false}));
    fx.assert_stopped_transcript().await;
}

/// The turn has run a tool round, so a `tool_call` row sits behind the
/// question and the question is no longer the newest row. The stop
/// still finds the turn, and the turn still finds the stop.
#[tokio::test]
async fn a_turn_that_ran_a_tool_round_is_still_stopped() {
    let fx = tool_round_fixture("stop-tool-round@test.local").await;
    fx.ask_and_hang_up(&fx.token).await;
    fx.wait_for_a_row("tool_call").await;
    let before: Vec<String> = fx.messages().await.into_iter().map(|m| m.role).collect();
    assert_eq!(
        before.first().map(String::as_str),
        Some("user"),
        "the question is on file"
    );
    assert_eq!(
        before.last().map(String::as_str),
        Some("tool_call"),
        "the newest row is a tool row, not the question: {before:?}"
    );

    let (status, body) = fx.stop(&fx.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"stopped": true}),
        "the tool rows behind the question do not hide it from the stop"
    );
    assert!(
        wait_for_turns_to_finish(&fx.resources, STOP_BUDGET).await,
        "the turn parked on its second model call ends on the stop"
    );

    let messages = fx.messages().await;
    let question = &messages[0];
    let notice = messages.last().expect("rows");
    assert_eq!(notice.role, "assistant");
    assert_eq!(notice.content, fx.notice());
    assert_eq!(
        notice.finish_reason.as_deref(),
        Some(STOPPED_TURN_FINISH_REASON)
    );
    assert_eq!(notice.content_blocks, stop_marker(&question.id));
    assert_eq!(
        messages.iter().filter(|m| m.role == "assistant").count(),
        1,
        "the notice is the turn's only assistant row"
    );
}

/// A stopped turn spent a message and whatever its finished model calls
/// cost. Dropping the turn's future used to skip the accounting altogether.
#[tokio::test]
async fn a_stopped_turn_is_counted_against_the_caps() {
    let fx = tool_round_fixture("stop-usage@test.local").await;
    assert_eq!(fx.counter("daily_messages").await, 0);
    assert_eq!(fx.counter("daily_tokens").await, 0);

    fx.ask_and_hang_up(&fx.token).await;
    fx.wait_for_a_row("tool_call").await;
    // The first call's usage row is written by a detached task as the call
    // returns; the tool row is stored after it, so give the insert a moment.
    sleep(Duration::from_millis(500)).await;

    let (_, body) = fx.stop(&fx.token).await;
    assert_eq!(body, json!({"stopped": true}));
    assert!(wait_for_turns_to_finish(&fx.resources, STOP_BUDGET).await);

    assert_eq!(
        fx.counter("daily_messages").await,
        1,
        "the stopped turn is one message against the daily cap"
    );
    assert_eq!(fx.counter("weekly_messages").await, 1);
    let spent = i64::from(FIRST_CALL_PROMPT_TOKENS + FIRST_CALL_COMPLETION_TOKENS);
    assert_eq!(
        fx.counter("daily_tokens").await,
        spent,
        "the one model call that finished before the stop is charged in full"
    );
    assert_eq!(fx.counter("weekly_tokens").await, spent);
}

/// The stream the athlete is still holding ends on `done`, carrying the
/// stopped notice as the turn's reply.
#[tokio::test]
async fn a_held_sse_stream_ends_on_done_with_the_stopped_notice() {
    let fx = fixture("stop-sse-held@test.local").await;

    let request = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation_id
    ))
    .header("authorization", &format!("Bearer {}", fx.token))
    .header("accept", "text/event-stream")
    .json(&json!({"content": QUESTION}));
    let router = fx.router.clone();
    // `send` reads the body to its end, which a stopped turn must reach.
    let stream = tokio::spawn(async move { request.send(router).await });

    fx.wait_for_a_row("user").await;
    let (status, body) = fx.stop(&fx.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"stopped": true}));

    let response = timeout(STOP_BUDGET, stream)
        .await
        .expect("the stream ends once the turn is stopped")
        .unwrap();
    assert_eq!(response.status_code(), StatusCode::OK);
    let frames = response.text();
    assert!(
        !frames.contains("event: failed"),
        "a stop is not a failure: {frames}"
    );
    let done = frames
        .split("\n\n")
        .find(|frame| frame.contains("event: done"))
        .expect("the stream ends on a done frame");
    let data = done
        .lines()
        .find_map(|line| line.strip_prefix("data:"))
        .expect("the done frame carries the envelope");
    let envelope: Value = serde_json::from_str(data.trim()).unwrap();
    assert_eq!(envelope["user_message"]["content"], QUESTION);
    assert_eq!(envelope["assistant"]["message"]["content"], fx.notice());
    assert_eq!(envelope["assistant"]["finish_reason"], "stopped");
    assert_eq!(
        envelope["assistant"]["blocks"],
        json!([{"type": "prose", "text": fx.notice()}]),
        "the notice is prose; its turn marker reaches no client"
    );

    fx.assert_stopped_transcript().await;
}

/// The blocking JSON shape of the same turn stops the same way.
#[tokio::test]
async fn stopping_a_blocking_turn_answers_it_with_the_stopped_notice() {
    let fx = fixture("stop-json@test.local").await;

    let request = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation_id
    ))
    .header("authorization", &format!("Bearer {}", fx.token))
    .json(&json!({"content": QUESTION}));
    let router = fx.router.clone();
    let turn = tokio::spawn(async move { request.send(router).await });

    fx.wait_for_a_row("user").await;
    let (status, body) = fx.stop(&fx.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"stopped": true}));

    let response = timeout(STOP_BUDGET, turn)
        .await
        .expect("the blocking turn returns once it is stopped")
        .unwrap();
    assert_eq!(response.status_code(), StatusCode::OK);
    let envelope: Value = response.json();
    assert_eq!(envelope["assistant"]["message"]["content"], fx.notice());
    assert_eq!(envelope["assistant"]["message"]["finish_reason"], "stopped");
    assert_eq!(envelope["assistant"]["finish_reason"], "stopped");
    assert_eq!(envelope["telemetry"]["tool_calls_count"], 0);

    fx.assert_stopped_transcript().await;
    assert_eq!(
        fx.counter("daily_messages").await,
        1,
        "a turn stopped before any model call finished still spent a message"
    );
}

/// A stop that lands while the model is producing its reply still wins: the
/// turn looks for it immediately before it stores, so the reply that was
/// ready a moment later is never written beside the notice.
#[tokio::test]
async fn a_reply_ready_after_the_stop_is_not_stored() {
    let provider = Arc::new(ParkedProvider::answering("212 watts de moyenne."));
    let resources = create_test_server_resources_with_chat_provider(
        Arc::clone(&provider) as Arc<dyn LlmProvider>
    )
    .await
    .expect("resources");
    let fx = fixture_over(resources, "stop-race@test.local").await;

    let request = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/messages",
        fx.conversation_id
    ))
    .header("authorization", &format!("Bearer {}", fx.token))
    .json(&json!({"content": QUESTION}));
    let router = fx.router.clone();
    let turn = tokio::spawn(async move { request.send(router).await });
    fx.wait_for_a_row("user").await;
    for _ in 0..100 {
        if provider.calls() > 0 {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(provider.calls() > 0, "the turn reached the model");

    let (_, body) = fx.stop(&fx.token).await;
    assert_eq!(body, json!({"stopped": true}));
    // The model call is abandoned by the turn's own retry path once released;
    // whichever way the turn reaches its reply, the stop is already on file.
    provider.release();

    let response = timeout(STOP_BUDGET, turn)
        .await
        .expect("the turn returns")
        .unwrap();
    let envelope: Value = response.json();
    assert_eq!(envelope["assistant"]["finish_reason"], "stopped");
    fx.assert_stopped_transcript().await;
}

/// With no question waiting there is nothing to stop, and nothing is written.
#[tokio::test]
async fn a_stop_with_no_turn_running_writes_nothing() {
    let fx = fixture("stop-idle@test.local").await;

    let (status, body) = fx.stop(&fx.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"stopped": false}));
    assert_eq!(fx.messages().await.len(), 0);
}

/// An athlete outside the conversation cannot stop — or learn of — a turn in
/// it.
#[tokio::test]
async fn a_stranger_cannot_stop_someone_elses_turn() {
    let fx = fixture("stop-owner@test.local").await;
    let (_stranger, stranger_token) = create_test_tenant(&fx.resources, "stop-stranger@test.local")
        .await
        .expect("seed the stranger");
    fx.ask_and_hang_up(&fx.token).await;

    let (status, _body) = fx.stop(&stranger_token).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let messages = fx.messages().await;
    assert_eq!(messages.len(), 1, "the stranger's stop wrote nothing");
    assert_eq!(messages[0].role, "user");
    assert!(
        !fx.resources.common.turns.is_empty(),
        "the owner's turn is still running"
    );

    // The owner ends it, so the test leaves no turn parked on the provider.
    let (status, body) = fx.stop(&fx.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"stopped": true}));
    assert!(wait_for_turns_to_finish(&fx.resources, STOP_BUDGET).await);
}

/// A second participant is inside the conversation — they can read it and
/// post in it — and still cannot stop a turn another member started: the
/// question that turn answers is not theirs.
#[tokio::test]
async fn a_second_participant_cannot_stop_the_authors_turn() {
    let fx = fixture("stop-author@test.local").await;
    let tenant = fx.tenant().await;
    let (member, _) = create_test_tenant(&fx.resources, "stop-member@test.local")
        .await
        .expect("seed the member");
    fx.resources
        .common
        .repos
        .users
        .update_tenant_id(member.id, tenant)
        .await
        .unwrap();
    let member_token = fx
        .resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &member,
            &fx.resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap();
    let added = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/participants",
        fx.conversation_id
    ))
    .header("authorization", &format!("Bearer {}", fx.token))
    .json(&json!({"user_id": member.id.to_string()}))
    .send(fx.router.clone())
    .await;
    assert_eq!(added.status_code(), StatusCode::CREATED);

    fx.ask_and_hang_up(&fx.token).await;

    let (status, body) = fx.stop(&member_token).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the member is in the conversation, so this is not a 404"
    );
    assert_eq!(
        body,
        json!({"stopped": false}),
        "the unanswered question is the author's, not the member's"
    );
    let messages = fx.messages().await;
    assert_eq!(
        messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(),
        vec!["user"],
        "the member's stop wrote no notice"
    );
    assert!(
        !fx.resources.common.turns.is_empty(),
        "the author's turn is still running"
    );

    let (_, body) = fx.stop(&fx.token).await;
    assert_eq!(body, json!({"stopped": true}), "the author stops it");
    assert!(wait_for_turns_to_finish(&fx.resources, STOP_BUDGET).await);
    fx.assert_stopped_transcript().await;
}
