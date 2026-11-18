// ABOUTME: A turn the athlete stops after a write that went through still carries that write's envelope notice
// ABOUTME: Drives the chat routes with a model that saves an edited family day, stops the turn, reads the stopped row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Founder decision D3: every edit goes through and earns its warning. A turn
//! the athlete stops ends on the stop request's notice, and its tool loop's
//! result is dropped with it, yet a `save_training_plan` that ran before the
//! stop still earns its envelope notice (carnet#1519). The notice rides on
//! the stopped row, as it rides on every other reply the platform writes in
//! the model's place (a Guardian block, a reconnect prompt, a withheld reply:
//! `withheld_reply_envelope_notice_test`).
//!
//! The findings sit in one of two places when the stop lands, and each test
//! stops the turn while they sit in one of them:
//!
//! - mid-loop, the model parked on its call after the save: the store the
//!   dispatch chokepoint fills under the turn's key still holds them;
//! - after the loop settled, the model's reply in hand: the store was drained
//!   into the loop's result, which the stop drops, so only what the turn kept
//!   on its stop handle reaches the notice.
//!
//! The assertions are on the envelope the held stream ends on, the stopped
//! row the database holds, and the transcript a reload reads. Two more pin
//! what reaches that row around the stop: the stop request answers only once
//! the turn has settled it, so a client with no stream re-reading right after
//! reads the notice whole; and a finding a headless tool call reports after
//! the stop still reaches it, rather than the store the stopped turn left.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::Router;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_core::models::{TenantId, ENVELOPE_NOTICE_BLOCK_TYPE, STOPPED_TURN_FINISH_REASON};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_tool_runtime::envelope_notices::{NoticeDay, NoticeSource};
use pierre_tool_runtime::guardian::TurnKey;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::sync::{watch, Notify};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::drained_turn::wait_for_a_tracked_turn;
use crate::training_plan_tools_test::tabata_style_plan;

const QUESTION: &str = "Put a Tabata on Tuesday instead of the tempo.";

/// What the model answers once the save went through, when it answers.
const REPLY: &str = "Done: Tuesday is now a Tabata.";

/// Long enough for the turn's one-second stop poll to fire a few times over.
const STOP_BUDGET: Duration = Duration::from_secs(15);

/// When the model's calls after the save answer.
#[derive(Clone, Copy)]
enum AfterTheSave {
    /// Never: the turn is stopped mid-loop.
    Park,
    /// Once the test has stopped the turn: the loop settles with the stop on
    /// file, and the turn finds it at its last look before storing a reply.
    ReplyOnceStopped,
}

/// A model whose first call saves the edited plan through the text tool loop.
struct SaveThenWaitProvider {
    calls: AtomicUsize,
    after: AfterTheSave,
    /// Signalled when a call after the save starts: the tool round finished.
    after_the_save: Notify,
    /// Set once the test has stopped the turn.
    stopped: watch::Sender<bool>,
}

impl SaveThenWaitProvider {
    fn new(after: AfterTheSave) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            after,
            after_the_save: Notify::new(),
            stopped: watch::Sender::new(false),
        }
    }

    /// The text of this call: the save on the first, then whatever
    /// [`AfterTheSave`] says.
    async fn answer(&self) -> Result<String, AppError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let call = json!({ "name": "save_training_plan", "arguments": tabata_style_plan() });
            return Ok(format!("<tool_call>{call}</tool_call>"));
        }
        self.after_the_save.notify_one();
        match self.after {
            AfterTheSave::Park => {
                sleep(Duration::from_mins(10)).await;
                Err(AppError::internal("a parked call must never answer"))
            }
            AfterTheSave::ReplyOnceStopped => {
                let mut stopped = self.stopped.subscribe();
                stopped
                    .wait_for(|stopped| *stopped)
                    .await
                    .map_err(|e| AppError::internal(format!("the test went away: {e}")))?;
                Ok(REPLY.to_owned())
            }
        }
    }
}

#[async_trait]
impl LlmProvider for SaveThenWaitProvider {
    fn name(&self) -> &'static str {
        "save_then_wait_mock"
    }
    fn display_name(&self) -> &'static str {
        "Save Then Wait Mock LLM (stopped-turn envelope notice)"
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
        Ok(ChatResponse {
            content: self.answer().await?,
            model: "mock-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: self.answer().await?,
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
    provider: Arc<SaveThenWaitProvider>,
    resources: Arc<ServerContext>,
    router: Router,
    auth: String,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: String,
}

async fn fixture(after: AfterTheSave, email: &str) -> Fixture {
    let provider = Arc::new(SaveThenWaitProvider::new(after));
    let resources = create_test_server_resources_with_chat_provider(provider.clone())
        .await
        .unwrap();
    let (user_id, user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .unwrap();
    let auth = format!("Bearer {token}");
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let created = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &auth)
        .json(&json!({"title": "Plan", "model": "gemini-1.5-flash"}))
        .send(router.clone())
        .await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "the fixture's conversation is created"
    );
    let conversation: Value = created.json();
    let conversation_id = conversation["id"].as_str().expect("id").to_owned();

    Fixture {
        provider,
        resources,
        router,
        auth,
        user_id,
        tenant_id,
        conversation_id,
    }
}

impl Fixture {
    /// Ask over a stream the athlete holds, so the envelope the stopped turn
    /// ends on is read where the client reads it; return once the save's
    /// tool round finished and the loop went back to the model.
    async fn ask_until_saved(&self) -> JoinHandle<AxumTestResponse> {
        let ask = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{}/messages",
            self.conversation_id
        ))
        .header("authorization", &self.auth)
        .header("accept", "text/event-stream")
        .json(&json!({"content": QUESTION}));
        let router = self.router.clone();
        let held = tokio::spawn(async move { ask.send(router).await });
        assert!(
            wait_for_a_tracked_turn(&self.resources).await,
            "the turn is running"
        );
        timeout(STOP_BUDGET, self.provider.after_the_save.notified())
            .await
            .expect("the save's tool round finished and the loop went back to the model");
        held
    }

    /// Ask over a stream that hangs up at once — the client that reopened the
    /// app — and return once the save's tool round finished.
    async fn ask_detached_until_saved(&self) {
        let response = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{}/messages",
            self.conversation_id
        ))
        .header("authorization", &self.auth)
        .header("accept", "text/event-stream")
        .json(&json!({"content": QUESTION}))
        .send_sse(self.router.clone())
        .await;
        assert_eq!(response.status_code(), StatusCode::OK, "the stream opened");
        assert!(
            wait_for_a_tracked_turn(&self.resources).await,
            "the turn is running, detached from the dropped stream"
        );
        timeout(STOP_BUDGET, self.provider.after_the_save.notified())
            .await
            .expect("the save's tool round finished and the loop went back to the model");
    }

    /// The stopped row as a reload lists it.
    async fn listed_stopped_row(&self) -> Value {
        let listed = AxumTestRequest::get(&format!(
            "/api/chat/conversations/{}/messages",
            self.conversation_id
        ))
        .header("authorization", &self.auth)
        .send(self.router.clone())
        .await;
        assert_eq!(
            listed.status_code(),
            StatusCode::OK,
            "the athlete reads their own transcript"
        );
        let listed: Value = listed.json();
        listed["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .find(|row| row["finish_reason"] == "stopped")
            .cloned()
            .expect("the stopped row is listed")
    }

    /// The lines of the envelope notice the stopped row holds.
    async fn stored_notice_lines(&self) -> Vec<String> {
        let messages = self
            .resources
            .common
            .repos
            .chat
            .get_messages(
                &self.conversation_id,
                &self.user_id.to_string(),
                self.tenant_id,
            )
            .await
            .unwrap();
        let notice = messages
            .iter()
            .find(|row| row.finish_reason.as_deref() == Some(STOPPED_TURN_FINISH_REASON))
            .expect("the stopped row");
        let stored: Vec<Value> =
            serde_json::from_str(notice.content_blocks.as_deref().unwrap_or("[]")).unwrap();
        stored
            .iter()
            .filter(|block| block["type"] == ENVELOPE_NOTICE_BLOCK_TYPE)
            .flat_map(|block| block["lines"].as_array().cloned().unwrap_or_default())
            .filter_map(|line| line.as_str().map(str::to_owned))
            .collect()
    }

    async fn stop(&self) {
        let stopped = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{}/stop",
            self.conversation_id
        ))
        .header("authorization", &self.auth)
        .send(self.router.clone())
        .await;
        assert_eq!(
            stopped.status_code(),
            StatusCode::OK,
            "the athlete stops their own turn"
        );
        assert_eq!(
            stopped.json::<Value>(),
            json!({"stopped": true}),
            "the turn was running, so the stop closes it"
        );
    }

    /// The stopped turn's live envelope notice, its stored one, and what a
    /// reload lists: one notice, the same everywhere, on the stopped row.
    /// Returns the envelope.
    async fn assert_the_stopped_row_carries_the_notice(
        &self,
        held: JoinHandle<AxumTestResponse>,
    ) -> Value {
        let response = timeout(STOP_BUDGET, held)
            .await
            .expect("the held stream ends once the turn is stopped")
            .unwrap();
        let frames = response.text();
        let done = frames
            .split("\n\n")
            .find(|frame| frame.contains("event: done"))
            .unwrap_or_else(|| panic!("the stream ends on a done frame: {frames}"));
        let data = done
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .expect("the done frame carries the envelope");
        let envelope: Value = serde_json::from_str(data.trim()).unwrap();
        assert_eq!(
            envelope["assistant"]["finish_reason"], "stopped",
            "the turn ended on the stop, not on the model's reply: {envelope:#}"
        );
        let live: Vec<&Value> = envelope["assistant"]["blocks"]
            .as_array()
            .expect("blocks")
            .iter()
            .filter(|block| block["type"] == ENVELOPE_NOTICE_BLOCK_TYPE)
            .collect();
        assert_eq!(
            live.len(),
            1,
            "bug: the stopped turn's envelope carries the save's notice: {envelope:#}"
        );

        let messages = self
            .resources
            .common
            .repos
            .chat
            .get_messages(
                &self.conversation_id,
                &self.user_id.to_string(),
                self.tenant_id,
            )
            .await
            .unwrap();
        let question = messages
            .iter()
            .find(|row| row.role == "user")
            .expect("the question is on file");
        let assistants: Vec<_> = messages
            .iter()
            .filter(|row| row.role == "assistant")
            .collect();
        assert_eq!(
            assistants.len(),
            1,
            "bug: the notice rides on the stopped row; no other assistant row is written"
        );
        let notice = assistants[0];
        assert_eq!(
            notice.finish_reason.as_deref(),
            Some(STOPPED_TURN_FINISH_REASON),
            "bug: the one assistant row is the stopped notice, not a reply"
        );
        let stored: Vec<Value> = serde_json::from_str(
            notice
                .content_blocks
                .as_deref()
                .expect("the stopped row keeps its marker"),
        )
        .unwrap();
        // The settled marker as the row persists it, written out.
        let marker = json!({ "type": "turn_stop", "question_id": question.id, "state": "settled" });
        assert_eq!(
            stored.first(),
            Some(&marker),
            "bug: the stopped row still names the question it closed: {stored:#?}"
        );
        let notices: Vec<&Value> = stored
            .iter()
            .filter(|block| block["type"] == ENVELOPE_NOTICE_BLOCK_TYPE)
            .collect();
        assert_eq!(
            notices.len(),
            1,
            "bug: the save earns one notice on the stopped row: {stored:#?}"
        );
        assert_eq!(
            notices[0], live[0],
            "bug: a reload shows the notice the live turn showed"
        );
        let lines: Vec<&str> = notices[0]["lines"]
            .as_array()
            .map_or_default(|lines| lines.iter().filter_map(Value::as_str).collect());
        assert_eq!(
            lines.len(),
            1,
            "bug: only the edited day warns, so the notice holds one line: {stored:#?}"
        );
        assert!(
            lines[0].contains("2026-07-14"),
            "bug: the notice names the edited day: {}",
            lines[0]
        );

        // A reload draws the notice on the stopped row, and the marker nowhere.
        let listed = AxumTestRequest::get(&format!(
            "/api/chat/conversations/{}/messages",
            self.conversation_id
        ))
        .header("authorization", &self.auth)
        .send(self.router.clone())
        .await;
        assert_eq!(
            listed.status_code(),
            StatusCode::OK,
            "the athlete reloads their own transcript"
        );
        let listed: Value = listed.json();
        let row = listed["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .find(|row| row["finish_reason"] == "stopped")
            .expect("the stopped row is listed");
        assert_eq!(
            row["envelope_notice"]["lines"], notices[0]["lines"],
            "bug: a reload reads the stopped row's notice: {row:#}"
        );
        assert!(
            row.get("scene_blocks").is_none(),
            "the marker reaches no client as a chart: {row:#}"
        );
        envelope
    }
}

/// Stopped mid-loop: the findings are still in the store under the turn's key.
#[tokio::test]
async fn a_turn_stopped_mid_loop_carries_the_notice_of_the_write_it_made() {
    let fx = fixture(AfterTheSave::Park, "stopped-mid-loop-notice@example.com").await;
    let held = fx.ask_until_saved().await;
    fx.stop().await;
    fx.assert_the_stopped_row_carries_the_notice(held).await;
}

/// Stopped once the loop settled: the model answers only after the stop is
/// on file, so the loop drains the store into its result and the turn finds
/// the stop at its last look before storing the reply, which it never stores.
#[tokio::test]
async fn a_turn_stopped_after_its_loop_settled_carries_the_notice_of_the_write_it_made() {
    let fx = fixture(
        AfterTheSave::ReplyOnceStopped,
        "stopped-settled-notice@example.com",
    )
    .await;
    let held = fx.ask_until_saved().await;
    fx.stop().await;
    fx.provider.stopped.send_replace(true);
    fx.assert_the_stopped_row_carries_the_notice(held).await;
}

/// The client that reopened the app holds no stream: it stops the turn and
/// re-reads at once. The stop answers only once the turn has settled the
/// notice — the turn reads its stop on a one-second poll — so that re-read
/// holds the envelope notice, never the notice as the stop request wrote it.
#[tokio::test]
async fn a_reread_right_after_the_stop_reads_the_notice_whole() {
    let fx = fixture(AfterTheSave::Park, "stopped-reread-notice@example.com").await;
    fx.ask_detached_until_saved().await;
    fx.stop().await;

    let row = fx.listed_stopped_row().await;
    let lines = row["envelope_notice"]["lines"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        lines.len(),
        1,
        "bug: the re-read right after the stop must hold the save's notice: {row:#}"
    );
    assert!(
        lines[0]
            .as_str()
            .is_some_and(|line| line.contains("2026-07-14")),
        "bug: the notice names the edited day: {row:#}"
    );
}

/// A headless turn's tools run on the tool host's own task, which outlives a
/// stop: a save that finishes after the turn took its findings reports into a
/// store the stopped turn will never read again. The stop sealed the turn's
/// key, so that late finding settles the stopped row instead, and the store
/// keeps nothing for the turn.
#[tokio::test]
async fn a_finding_reported_after_the_stop_reaches_the_stopped_row() {
    let fx = fixture(AfterTheSave::Park, "stopped-late-notice@example.com").await;
    let held = fx.ask_until_saved().await;
    fx.stop().await;
    let envelope = fx.assert_the_stopped_row_carries_the_notice(held).await;
    let turn_id = envelope["turn_id"]
        .as_str()
        .expect("the envelope names its turn");
    let key = TurnKey::new(Some(fx.tenant_id.as_uuid()), turn_id);

    let late = NoticeDay {
        date: "2026-07-16".to_owned(),
        findings: vec![serde_json::from_value(json!({
            "code": "cycle_mean_floor", "action": "warn", "message": "English, for the model.",
            "measured": 71.6, "bound": 80.0
        }))
        .unwrap()],
        relabelled_purpose: None,
        anchor: None,
        source: NoticeSource::PlanDay,
    };
    let notices = ToolRuntime::envelope_notices(fx.resources.as_ref());
    notices.record(&key, vec![late]);

    let mut lines = Vec::new();
    for _ in 0..50 {
        lines = fx.stored_notice_lines().await;
        if lines.len() == 2 {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        lines.len(),
        2,
        "bug: a finding reported after the stop reaches the stopped row: {lines:#?}"
    );
    assert!(
        lines[0].contains("2026-07-14") && lines[1].contains("2026-07-16"),
        "bug: the stopped row keeps the turn's finding and adds the late one: {lines:#?}"
    );
    assert!(
        notices.take(&key).is_empty(),
        "bug: the late finding must not wait in the store for a turn that ended"
    );
    let row = fx.listed_stopped_row().await;
    assert_eq!(
        row["envelope_notice"]["lines"].as_array().map(Vec::len),
        Some(2),
        "bug: a reload reads the late finding on the stopped row: {row:#}"
    );
}
