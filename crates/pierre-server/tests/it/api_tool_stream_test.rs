// ABOUTME: An Anthropic-head turn streams its reply word by word, and takes back every word that is not the reply
// ABOUTME: Pins the preamble, the broken stream re-run on the tail, the refusal and a replaced reply, loop and SSE alike
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1193 lane H: on an Anthropic Messages API head the API tool loop
//! asks the turn's model through embacle's `complete_stream_with_tools`, and
//! the coach reads the reply as it is written. A scripted tier stands in for
//! the API; the client's view is rebuilt from the events the way the web
//! client builds its bubble — a delta appends, a retraction empties it — so
//! each test asserts what the athlete is left looking at, not just the reply.
//!
//! - the deltas reach the sink in order and spell the reply;
//! - words before a tool call are taken back before the tools run;
//! - a stream that breaks re-runs the turn on the chain's tail, and the head's
//!   fragment is gone before the tail's reply arrives;
//! - a refusal takes back what it streamed;
//! - over real HTTP, the deltas ride the turn body as `delta` frames, and a
//!   reply a later check replaces (an identity break re-asked away) is taken
//!   back with a `retract` frame before `done`;
//! - over real HTTP, each tool call the loop runs rides the body as a step —
//!   a `progress` frame when it starts and one when it completes — before the
//!   reply's words, so the trail names what the turn is doing on this loop
//!   too, not only on the Copilot turn.

use crate::common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{
    create_test_server_resources, create_test_server_resources_with_real_chat_provider,
    create_test_tenant_with_provider, create_test_user, spawn_chat_server,
};
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError, ToolCallRequest, ToolStream, ToolStreamEvent,
};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::config::LlmProviderType;
use pierre_llm::{
    ChatMessage, ChatProvider, EmbacleProvider, FunctionDeclaration, QuotaCooldowns, Tool,
};
use pierre_services::chat_stream::{TurnEvent, TurnEventSink};
use pierre_test_support::sse::{read_frames, Frame};
use pierre_tool_runtime::protocol::UniversalToolExecutor;
use pierre_tool_runtime::tool_execution::run_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

/// A real registry tool that needs no provider connection and no fixture data.
const ROUND_TRIP_TOOL: &str = "get_connection_status";

/// The coach's reply, verbatim.
const REPLY: &str = "Ta charge grimpe depuis trois semaines. On coupe jeudi.";

/// A reply over HTTP that asks nothing of the athlete's data, so no recovery
/// stage has a claim to adjudicate and the reply the model wrote is the one
/// delivered.
const GREETING: &str = "Salut ! Content de te lire aujourd'hui, belle journée pour courir.";

/// The 2026-07-22 Telegram break, verbatim: the model answering as itself.
const LEAKED_REPLY: &str = "I'm GitHub Copilot CLI, a terminal-based coding assistant.";

/// How one scripted response closes, after its text has streamed.
#[derive(Clone, Copy)]
enum Close {
    /// The text is the answer.
    Answers,
    /// A structured call of `ROUND_TRIP_TOOL` follows the text.
    CallsTool,
    /// Two structured calls of `ROUND_TRIP_TOOL`, each its own call id.
    CallsTwoTools,
    /// The model declines: the API discards the partial output and closes
    /// empty with `finish_reason: "refusal"`.
    Refuses,
    /// The connection drops mid-stream: an `Err` item, no closing response.
    Breaks,
}

/// One scripted response: its text, streamed a word at a time, and its close.
#[derive(Clone, Copy)]
struct Reply {
    text: &'static str,
    close: Close,
}

const fn reply(text: &'static str, close: Close) -> Reply {
    Reply { text, close }
}

/// An Anthropic-API-shaped tier streaming its script in order, whatever the
/// model, and answering every plain completion with `fallback` — the identity
/// re-ask and the pipeline's other single-shot calls ask it that way, so they
/// never consume the script the tool loop streams.
struct ScriptedStream {
    name: &'static str,
    script: Mutex<VecDeque<Reply>>,
    fallback: &'static str,
    models: Vec<String>,
}

impl ScriptedStream {
    fn new(name: &'static str, script: &[Reply], fallback: &'static str) -> Arc<Self> {
        Arc::new(Self {
            name,
            script: Mutex::new(script.iter().copied().collect()),
            fallback,
            models: vec!["claude-opus-5-5".to_owned()],
        })
    }

    fn next(&self) -> Reply {
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| panic!("{}: no streamed response left", self.name))
    }

    fn closing(&self, text: &str, close: Close) -> Result<ChatResponse, RunnerError> {
        let (content, tool_calls, finish_reason) = match close {
            Close::Answers => (text.to_owned(), None, "stop"),
            Close::CallsTool => (
                text.to_owned(),
                Some(vec![ToolCallRequest {
                    id: "toolu_status".to_owned(),
                    function_name: ROUND_TRIP_TOOL.to_owned(),
                    arguments: json!({}),
                }]),
                "tool_use",
            ),
            Close::CallsTwoTools => (
                text.to_owned(),
                Some(
                    ["toolu_status_a", "toolu_status_b"]
                        .into_iter()
                        .map(|id| ToolCallRequest {
                            id: id.to_owned(),
                            function_name: ROUND_TRIP_TOOL.to_owned(),
                            arguments: json!({}),
                        })
                        .collect(),
                ),
                "tool_use",
            ),
            Close::Refuses => (String::new(), None, "refusal"),
            Close::Breaks => {
                return Err(RunnerError::external_service(
                    self.name,
                    "stream reset by peer",
                ))
            }
        };
        Ok(ChatResponse {
            content,
            model: self.models[0].clone(),
            usage: None,
            finish_reason: Some(finish_reason.to_owned()),
            warnings: None,
            tool_calls,
        })
    }
}

#[async_trait]
impl EmbacleLlmProvider for ScriptedStream {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &'static str {
        "Anthropic API (scripted stream)"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING
            | LlmCapabilities::SYSTEM_MESSAGES
            | LlmCapabilities::FUNCTION_CALLING
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    /// A tool-loop call that does not stream (it carries the turn's tools)
    /// is served the script whole; any other completion gets `fallback`.
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        if request.tools.is_some() {
            let Reply { text, close } = self.next();
            return self.closing(text, close);
        }
        self.closing(self.fallback, Close::Answers)
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal(
            "a tool turn streams through complete_stream_with_tools",
        ))
    }

    async fn complete_stream_with_tools(
        &self,
        _request: &ChatRequest,
    ) -> Result<ToolStream, RunnerError> {
        let Reply { text, close } = self.next();
        let mut events: Vec<Result<ToolStreamEvent, RunnerError>> = text
            .split_inclusive(' ')
            .map(|word| Ok(ToolStreamEvent::TextDelta(word.to_owned())))
            .collect();
        events.push(self.closing(text, close).map(ToolStreamEvent::Done));
        Ok(Box::pin(tokio_stream::iter(events)))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

/// `tier` as an Anthropic head.
fn anthropic(tier: &Arc<ScriptedStream>) -> EmbacleProvider {
    EmbacleProvider::from_runner(
        Box::new(Arc::clone(tier)),
        "Anthropic API (scripted stream)",
    )
    .of_kind(LlmProviderType::Anthropic)
}

/// What a client is left showing after `events`, built the way the web
/// client builds its in-flight bubble: a delta appends, a retraction empties.
fn bubble<'a>(events: impl IntoIterator<Item = &'a TurnEvent>) -> String {
    let mut shown = String::new();
    for event in events {
        match event {
            TurnEvent::ProseDelta(delta) => shown.push_str(delta),
            TurnEvent::ProseRetract => shown.clear(),
            _ => {}
        }
    }
    shown
}

/// Each prose event, as `Some(delta)` or `None` for a retraction.
fn prose(events: &[TurnEvent]) -> Vec<Option<&str>> {
    events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ProseDelta(delta) => Some(Some(delta.as_str())),
            TurnEvent::ProseRetract => Some(None),
            _ => None,
        })
        .collect()
}

/// Run one turn of the production tool loop on `provider`, streaming to a
/// sink, and return its result with every event the sink received.
async fn stream_turn(provider: &ChatProvider) -> (ToolLoopResult, Vec<TurnEvent>) {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let executor = Arc::new(
        UniversalToolExecutor::new(resources)
            .with_scopes(OAuthScope::self_grant())
            .with_turn_token("tool-stream-turn".to_owned()),
    );
    let tools = Tool {
        function_declarations: vec![FunctionDeclaration {
            name: ROUND_TRIP_TOOL.to_owned(),
            description: "Report which fitness providers are connected".to_owned(),
            parameters: Some(json!({ "type": "object", "properties": {} })),
        }],
    };
    let (sink, mut rx): (TurnEventSink, _) = mpsc::unbounded_channel();
    let user = user_id.to_string();
    let params = ToolLoopParams {
        provider,
        executor,
        tools: &tools,
        model: provider.default_model(),
        user_id: &user,
        tenant_id: TenantId::from_uuid(Uuid::new_v4()),
        max_iterations: 4,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: Some(sink),
        mcp_servers: Vec::new(),
    };
    let mut messages = vec![
        ChatMessage::system("You are a test coach."),
        ChatMessage::user("comment va ma charge ?"),
    ];
    let result = run_tool_loop(&params, &mut messages)
        .await
        .expect("the turn is served");
    drop(params);
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    (result, events)
}

#[tokio::test]
async fn the_reply_streams_word_by_word_and_the_deltas_spell_it() {
    let tier = ScriptedStream::new("anthropic", &[reply(REPLY, Close::Answers)], REPLY);
    let provider = ChatProvider::Embacle(anthropic(&tier));

    let (result, events) = stream_turn(&provider).await;

    assert_eq!(
        prose(&events),
        vec![
            Some("Ta "),
            Some("charge "),
            Some("grimpe "),
            Some("depuis "),
            Some("trois "),
            Some("semaines. "),
            Some("On "),
            Some("coupe "),
            Some("jeudi."),
        ],
        "every word reaches the sink in order, and nothing is taken back"
    );
    assert_eq!(result.content, REPLY);
    assert_eq!(bubble(&events), result.content);
}

#[tokio::test]
async fn words_before_a_tool_call_are_taken_back_before_the_reply_streams() {
    let tier = ScriptedStream::new(
        "anthropic",
        &[
            reply("Je regarde tes connexions.", Close::CallsTool),
            reply(REPLY, Close::Answers),
        ],
        REPLY,
    );
    let provider = ChatProvider::Embacle(anthropic(&tier));

    let (result, events) = stream_turn(&provider).await;

    let shown = prose(&events);
    assert_eq!(
        shown[..5],
        [
            Some("Je "),
            Some("regarde "),
            Some("tes "),
            Some("connexions."),
            None
        ],
        "the preamble streams, then is taken back once the response turns out to call a tool"
    );
    assert_eq!(result.tools_called, vec![ROUND_TRIP_TOOL.to_owned()]);
    assert_eq!(result.content, REPLY);
    assert_eq!(
        bubble(&events),
        REPLY,
        "the bubble holds the reply alone, the preamble gone"
    );
}

#[tokio::test]
async fn a_tool_call_written_as_text_never_reaches_the_sink() {
    let tier = ScriptedStream::new(
        "anthropic",
        &[
            reply(
                "Je regarde. <tool_call>{\"name\":\"get_connection_status\",\"arguments\":{}}</tool_call>",
                Close::Answers,
            ),
            reply(REPLY, Close::Answers),
        ],
        REPLY,
    );
    let provider = ChatProvider::Embacle(anthropic(&tier));

    let (result, events) = stream_turn(&provider).await;

    assert!(
        prose(&events)
            .iter()
            .flatten()
            .all(|delta| !delta.contains("tool_call") && !delta.contains("get_connection")),
        "the block is the platform's to run, never prose: {events:?}"
    );
    assert_eq!(result.tools_called, vec![ROUND_TRIP_TOOL.to_owned()]);
    assert_eq!(bubble(&events), REPLY);
}

#[tokio::test]
async fn a_stream_that_breaks_is_taken_back_before_the_tail_answers() {
    let head = ScriptedStream::new(
        "anthropic",
        &[reply("Ta charge monte", Close::Breaks)],
        "unused",
    );
    let tail = ScriptedStream::new("anthropic#2", &[reply(REPLY, Close::Answers)], REPLY);
    let provider = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![anthropic(&head), anthropic(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    );

    let (result, events) = stream_turn(&provider).await;

    let shown = prose(&events);
    assert_eq!(
        shown[..4],
        [Some("Ta "), Some("charge "), Some("monte"), None],
        "the head's fragment is taken back when its stream breaks"
    );
    assert_eq!(result.content, REPLY, "the tail serves the turn");
    assert_eq!(
        bubble(&events),
        REPLY,
        "the tail's reply stands alone, never glued to the head's fragment"
    );
}

#[tokio::test]
async fn a_refusal_takes_back_what_it_streamed() {
    let head = ScriptedStream::new(
        "anthropic",
        &[reply("Je ne peux pas", Close::Refuses)],
        "unused",
    );
    let tail = ScriptedStream::new("anthropic#2", &[reply(REPLY, Close::Answers)], REPLY);
    let provider = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![anthropic(&head), anthropic(&tail)],
            QuotaCooldowns::default(),
        )
        .unwrap(),
    );

    let (result, events) = stream_turn(&provider).await;

    assert_eq!(
        prose(&events)[..5],
        [Some("Je "), Some("ne "), Some("peux "), Some("pas"), None],
        "the refused fragment is taken back the moment the refusal closes the stream"
    );
    assert_eq!(
        result.content, REPLY,
        "an empty refusal is a lost turn, which the tail serves as before"
    );
    assert_eq!(bubble(&events), REPLY);
}

// ============================================================================
// Over real HTTP: the frames a client reads off the turn body
// ============================================================================

/// Every frame of one streamed turn, up to and including its terminal frame.
async fn stream_frames(tier: &Arc<ScriptedStream>, email: &str) -> Vec<Frame> {
    stream_frames_on(anthropic(tier), email).await
}

/// [`stream_frames`] on `provider`, whatever it streams.
async fn stream_frames_on(provider: EmbacleProvider, email: &str) -> Vec<Frame> {
    let provider = Arc::new(ChatProvider::Embacle(provider));
    let resources = create_test_server_resources_with_real_chat_provider(provider)
        .await
        .expect("server resources");
    let base_url = spawn_chat_server(&resources).await;
    let (_user, token) = create_test_tenant_with_provider(&resources, email)
        .await
        .expect("create user + token + provider");
    let client = Client::builder().no_gzip().build().expect("client");

    let conversation: Value = client
        .post(format!("{base_url}/api/chat/conversations"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "title": "tool-stream-test", "model": "claude-opus-5-5" }))
        .send()
        .await
        .expect("create conversation")
        .json()
        .await
        .expect("conversation json");
    let conversation_id = conversation["id"].as_str().expect("conversation id");

    let response = client
        .post(format!(
            "{base_url}/api/chat/conversations/{conversation_id}/messages"
        ))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "text/event-stream, application/json")
        .json(&json!({ "content": "Salut !" }))
        .send()
        .await
        .expect("POST send_message");
    assert!(
        response.status().is_success(),
        "a streaming turn opens a 200 body, got {}",
        response.status()
    );
    read_frames(response, Duration::from_secs(20)).await
}

/// The frames rebuilt as the client's bubble events, in order.
fn as_events(frames: &[Frame]) -> Vec<TurnEvent> {
    frames
        .iter()
        .filter_map(|frame| match frame.event.as_str() {
            "delta" => {
                let data: Value = serde_json::from_str(&frame.data).expect("delta json");
                Some(TurnEvent::ProseDelta(
                    data["delta"].as_str().expect("delta text").to_owned(),
                ))
            }
            "retract" => Some(TurnEvent::ProseRetract),
            _ => None,
        })
        .collect()
}

/// The persisted reply the `done` frame carries.
fn delivered(frames: &[Frame]) -> String {
    let done = frames.last().expect("the stream produced frames");
    assert_eq!(done.event, "done", "frames = {frames:?}");
    let envelope: Value = serde_json::from_str(&done.data).expect("done json");
    envelope["assistant"]["message"]["content"]
        .as_str()
        .expect("assistant content")
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_anthropic_turn_streams_delta_frames_that_spell_the_persisted_reply() {
    let tier = ScriptedStream::new("anthropic", &[reply(GREETING, Close::Answers)], GREETING);

    let frames = stream_frames(&tier, "tool-stream-sse@example.com").await;

    let events = as_events(&frames);
    // The canary gate re-chunks the words as it holds back a canary's length
    // of text, so the frames are counted, not matched word for word.
    assert!(
        events
            .iter()
            .filter(|e| matches!(e, TurnEvent::ProseDelta(_)))
            .count()
            > 1,
        "the reply rides the body as it is written, not in one piece: {frames:?}"
    );
    assert!(
        events.iter().all(|e| !matches!(e, TurnEvent::ProseRetract)),
        "a reply that stands is never taken back: {frames:?}"
    );
    let first_delta = frames.iter().position(|f| f.event == "delta");
    let dispatch_done = frames.iter().position(|f| {
        f.event == "progress" && f.data.contains("\"dispatch\"") && f.data.contains("finished")
    });
    assert!(
        first_delta < dispatch_done,
        "the deltas arrive while the model writes, before dispatch ends: {frames:?}"
    );
    let reply = delivered(&frames);
    assert_eq!(reply, GREETING);
    assert_eq!(
        bubble(&events),
        reply,
        "the persisted reply is exactly what the deltas spelled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_a_later_check_replaces_is_retracted_before_done() {
    // The streamed reply breaks character; the identity re-ask (a plain
    // completion, answered by the tier's fallback) replaces it.
    let tier = ScriptedStream::new(
        "anthropic",
        &[reply(LEAKED_REPLY, Close::Answers)],
        GREETING,
    );

    let frames = stream_frames(&tier, "tool-stream-leak@example.com").await;

    let events = as_events(&frames);
    assert!(
        matches!(events.first(), Some(TurnEvent::ProseDelta(_))),
        "the draft streamed before any check could read it whole: {frames:?}"
    );
    assert!(
        matches!(events.last(), Some(TurnEvent::ProseRetract)),
        "the replaced draft is taken back before the turn ends: {frames:?}"
    );
    let reply = delivered(&frames);
    assert_eq!(reply, GREETING, "the re-ask's clean reply is delivered");
    assert!(
        bubble(&events).is_empty(),
        "nothing of the leaked draft is left on screen when `done` paints the reply"
    );
}

/// The tool `progress` frames of a turn, as (frame index, call id, title, status).
fn tool_steps(frames: &[Frame]) -> Vec<(usize, String, String, String)> {
    frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.event == "progress")
        .filter_map(|(index, frame)| {
            let data: Value = serde_json::from_str(&frame.data).expect("progress json");
            (data["kind"] == "tool").then(|| {
                let field = |key: &str| data[key].as_str().expect("progress field").to_owned();
                (index, field("id"), field("title"), field("status"))
            })
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn each_tool_call_of_an_api_turn_rides_the_body_as_a_step_before_the_reply() {
    let tier = ScriptedStream::new(
        "anthropic",
        &[
            reply("", Close::CallsTwoTools),
            reply(GREETING, Close::Answers),
        ],
        GREETING,
    );

    let frames = stream_frames(&tier, "tool-stream-steps@example.com").await;

    let steps = tool_steps(&frames);
    // Literals on purpose: these are wire values both chat clients parse
    // (the ACP call states the shared turn-trail reducer reads).
    assert_eq!(
        steps
            .iter()
            .map(|(_, _, title, status)| (title.as_str(), status.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (ROUND_TRIP_TOOL, "InProgress"),
            (ROUND_TRIP_TOOL, "Completed"),
            (ROUND_TRIP_TOOL, "InProgress"),
            (ROUND_TRIP_TOOL, "Completed"),
        ],
        "each call starts, then completes, one after the other: {frames:?}"
    );
    assert_eq!(
        steps[0].1, steps[1].1,
        "the first call closes under the id it started with"
    );
    assert_eq!(
        steps[2].1, steps[3].1,
        "the second call closes under the id it started with"
    );
    assert_ne!(
        steps[0].1, steps[2].1,
        "two calls are two steps, never one step updated twice"
    );
    let last_step = steps.last().expect("steps were reported").0;
    let first_delta = frames
        .iter()
        .position(|f| f.event == "delta")
        .expect("the reply streamed");
    assert!(
        last_step < first_delta,
        "every step is reported before the reply's words: {frames:?}"
    );
    assert_eq!(
        delivered(&frames),
        GREETING,
        "the reply `done` carries comes after them"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_head_that_answers_whole_still_shows_each_step_and_streams_no_text() {
    // A function-calling head of no streaming kind (Gemini, Cohere, an
    // OpenAI-compatible gateway): it answers each call in one piece, so the
    // turn puts no word on the wire early. The surface still reads frames,
    // and the coach watches each tool the turn runs while the reply is built.
    let tier = ScriptedStream::new(
        "gemini",
        &[
            reply("", Close::CallsTwoTools),
            reply(GREETING, Close::Answers),
        ],
        GREETING,
    );
    let whole =
        EmbacleProvider::from_runner(Box::new(Arc::clone(&tier)), "Gemini API (scripted, whole)")
            .of_kind(LlmProviderType::Gemini);

    let frames = stream_frames_on(whole, "tool-stream-whole@example.com").await;

    let steps = tool_steps(&frames);
    assert_eq!(
        steps
            .iter()
            .map(|(_, _, title, status)| (title.as_str(), status.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (ROUND_TRIP_TOOL, "InProgress"),
            (ROUND_TRIP_TOOL, "Completed"),
            (ROUND_TRIP_TOOL, "InProgress"),
            (ROUND_TRIP_TOOL, "Completed"),
        ],
        "each call is a step on the body though the head streams no text: {frames:?}"
    );
    let dispatch_done = frames
        .iter()
        .position(|f| {
            f.event == "progress" && f.data.contains("\"dispatch\"") && f.data.contains("finished")
        })
        .expect("the dispatch stage reports its end");
    assert!(
        steps.iter().all(|(index, ..)| *index < dispatch_done),
        "the steps arrive while the turn runs, not with its reply: {frames:?}"
    );
    assert!(
        frames
            .iter()
            .all(|f| f.event != "delta" && f.event != "retract"),
        "a head that answers whole puts no text on the wire and takes none back: {frames:?}"
    );
    assert_eq!(
        delivered(&frames),
        GREETING,
        "the reply arrives whole in `done`"
    );
}
