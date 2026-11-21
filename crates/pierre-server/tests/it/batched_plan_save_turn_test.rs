// ABOUTME: A chat turn that saves one plan in four batches keeps every batch and the model's own reply
// ABOUTME: Drives the chat route with a model that saves four weeks one call at a time, then answers

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1702: a coach wrote a long day-by-day plan in four
//! `save_training_plan` calls in one turn. Each save carried a destructive
//! charge, the fourth met the per-turn cap of three, and the athlete read the
//! Guardian's refusal instead of the plan, its last weeks unsaved. In a chat
//! turn a save hands its calendar push to the turn's end and gives its
//! destructive charge back, so the four batches land and the reply is the
//! model's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{Datelike, Duration, NaiveDate, Utc, Weekday};
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_core::models::{TenantId, GUARDIAN_DENIED_FINISH_REASON};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, TurnResponse};
use serde_json::{json, Value};
use tokio::time::sleep;
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};
use crate::helpers::axum_test::AxumTestRequest;
use crate::workout_push_test::{connect_intervals_icu, serving_intervals_at, stub_live, Stub};

const SAVES: usize = 4;

/// What the athlete asks.
const QUESTION: &str = "Build me four weeks into my spring 10K.";

/// How the text tool loop hands a save's result back to the model.
const SAVE_RESULT: &str = "<tool_result name=\"save_training_plan\">";

/// What the model answers once every batch is saved.
const REPLY: &str = "Your four weeks are saved: easy and steady runs into the 10K.";

fn iso(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// The Monday a week or more ahead, so every saved day is in the future.
fn next_monday() -> NaiveDate {
    let mut day = Utc::now().date_naive() + Duration::days(7);
    while day.weekday() != Weekday::Mon {
        day += Duration::days(1);
    }
    day
}

/// Batch `n` of the plan: week `n` of two runs, the first batch laying the
/// season's outline with it.
fn batch(monday: NaiveDate, n: usize) -> Value {
    let start = monday + Duration::weeks(i64::try_from(n).unwrap());
    let week = json!({
        "week_start": iso(start),
        "focus": "",
        "days": [
            { "date": iso(start + Duration::days(1)), "sport": "run", "workout": "Easy 40", "duration_min": 40, "intensity": "Z1" },
            { "date": iso(start + Duration::days(3)), "sport": "run", "workout": "Steady 50", "duration_min": 50, "intensity": "Z2" }
        ]
    });
    if n > 0 {
        return json!({ "weeks": [week] });
    }
    json!({
        "outline": {
            "goal_race": {
                "name": "Spring 10K",
                "date": iso(monday + Duration::weeks(5)),
                "discipline": "run",
                "priority": "A"
            },
            "strategy": "four weeks of steady volume into a spring 10K",
            "phases": [
                { "kind": "build", "start": iso(monday), "weeks": 4, "intent": "steady volume" }
            ]
        },
        "weeks": [week]
    })
}

/// A model that saves the plan one week per call, through the text tool
/// loop, and answers once every week is saved.
///
/// A call is the loop's when it ends on the athlete's question or on a tool
/// result; the pipeline's other calls (the language check before the loop,
/// memory extraction after it) read the reply text. Each round of the loop
/// hands one save's result back in its own message.
struct SavesInBatches {
    monday: NaiveDate,
}

impl SavesInBatches {
    fn answer(&self, request: &ChatRequest) -> String {
        let last = request
            .messages
            .last()
            .map_or("", |message| message.content.as_str());
        if last != QUESTION && !last.contains("<tool_result") {
            return REPLY.to_owned();
        }
        let saved = request
            .messages
            .iter()
            .filter(|message| message.content.contains(SAVE_RESULT))
            .count();
        if saved >= SAVES {
            return REPLY.to_owned();
        }
        let save = json!({ "name": "save_training_plan", "arguments": batch(self.monday, saved) });
        format!("<tool_call>{save}</tool_call>")
    }
}

#[async_trait]
impl LlmProvider for SavesInBatches {
    fn name(&self) -> &'static str {
        "saves_in_batches_mock"
    }
    fn display_name(&self) -> &'static str {
        "Saves In Batches Mock LLM (carnet#1702)"
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

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            replay_blocks: None,
            content: self.answer(request),
            model: "mock-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: self.answer(request),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// An athlete whose turn the model answers by saving the plan in batches,
/// with Intervals.icu connected at a fake calendar when one is given.
struct Athlete {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    auth: String,
}

impl Athlete {
    async fn new(monday: NaiveDate, calendar: Option<&Stub>) -> Self {
        let provider = Arc::new(SavesInBatches { monday });
        let resources = create_test_server_resources_with_chat_provider(provider)
            .await
            .unwrap();
        let resources = match calendar {
            Some(stub) => serving_intervals_at(&resources, &stub.base_url),
            None => resources,
        };
        let (user_id, user, tenant_id) = create_test_user_with_plan(
            &resources.agent.database,
            &format!("batched_plan_save_{}@example.com", Uuid::new_v4()),
            "professional",
        )
        .await
        .unwrap();
        if calendar.is_some() {
            connect_intervals_icu(resources.as_ref(), user_id, tenant_id)
                .await
                .unwrap();
        }
        let token = resources
            .auth
            .auth_manager
            .generate_token_with_tenant(
                &user,
                &resources.auth.jwks_manager,
                Some(tenant_id.to_string()),
            )
            .unwrap();
        Self {
            resources,
            user_id,
            tenant_id,
            auth: format!("Bearer {token}"),
        }
    }

    /// Ask for the plan in a new conversation, over the chat route.
    async fn ask(&self) -> TurnResponse {
        let router = ChatRoutes::routes(Arc::clone(&self.resources));
        let created = AxumTestRequest::post("/api/chat/conversations")
            .header("authorization", &self.auth)
            .json(&json!({"title": "Plan", "model": "gemini-1.5-flash"}))
            .send(router.clone())
            .await;
        assert_eq!(
            created.status_code(),
            StatusCode::CREATED,
            "bug: the athlete could not open a conversation"
        );
        let conversation: Value = created.json();
        let conversation_id = conversation["id"].as_str().expect("id");

        let answered = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{conversation_id}/messages"
        ))
        .header("authorization", &self.auth)
        .json(&json!({ "content": QUESTION }))
        .send(router)
        .await;
        assert_eq!(
            answered.status_code(),
            StatusCode::OK,
            "bug: the turn that saves the plan in batches was not answered"
        );
        let turn: TurnResponse = answered.json();
        assert_ne!(
            turn.assistant.finish_reason.as_deref(),
            Some(GUARDIAN_DENIED_FINISH_REASON),
            "bug: the fourth batch of one plan was refused: {:?}",
            turn.assistant.message.content
        );
        assert_eq!(
            turn.assistant.message.content, REPLY,
            "the athlete reads the model's reply"
        );
        turn
    }

    /// The week starts the athlete's active plan holds, in order.
    async fn saved_weeks(&self) -> Vec<String> {
        let repos = self.resources.common.repos.clone();
        let (tenant, user) = (self.tenant_id.to_string(), self.user_id.to_string());
        let plan = repos
            .training_plans
            .get_active_plan(&tenant, &user)
            .await
            .unwrap()
            .expect("the first batch laid the season");
        let weeks = repos
            .training_plans
            .list_plan_weeks(&tenant, &user, &plan.id, false)
            .await
            .unwrap();
        let mut saved: Vec<String> = weeks.into_iter().map(|week| week.week_start).collect();
        saved.sort();
        saved
    }
}

/// The week starts of the four batches.
fn four_weeks_from(monday: NaiveDate) -> Vec<String> {
    (0..SAVES)
        .map(|n| iso(monday + Duration::weeks(i64::try_from(n).unwrap())))
        .collect()
}

#[tokio::test]
async fn a_plan_saved_in_four_batches_lands_whole_under_the_models_reply() {
    let monday = next_monday();
    let athlete = Athlete::new(monday, None).await;
    athlete.ask().await;
    assert_eq!(
        athlete.saved_weeks().await,
        four_weeks_from(monday),
        "bug: a batch of the plan was lost"
    );
}

#[tokio::test]
async fn a_plan_saved_in_four_batches_reaches_the_calendar_once_after_the_reply() {
    let stub = stub_live(12_000).await;
    let monday = next_monday();
    let athlete = Athlete::new(monday, Some(&stub)).await;
    athlete.ask().await;
    assert_eq!(
        athlete.saved_weeks().await,
        four_weeks_from(monday),
        "bug: a batch of the plan was lost"
    );
    assert_eq!(
        stub.event_count().await,
        8,
        "bug: the turn's end did not put every saved day on the calendar"
    );
    assert_eq!(
        stub.window_reads().await,
        1,
        "bug: the calendar was reconciled once per batch, not once per turn"
    );
}

/// The seal a turn's end leaves outlives the turn, and must keep no server
/// alive: every test builds its own, and PostgreSQL refuses connections once
/// enough of their pools linger (carnet#1702's first branch run).
#[tokio::test]
async fn a_finished_turn_keeps_no_server_alive() {
    let athlete = Athlete::new(next_monday(), None).await;
    athlete.ask().await;
    let server = Arc::downgrade(&athlete.resources);
    drop(athlete);
    // A task the turn started may still be finishing its last write.
    for _ in 0..50 {
        if server.upgrade().is_none() {
            return;
        }
        sleep(StdDuration::from_millis(100)).await;
    }
    panic!("bug: a finished chat turn still holds its server alive");
}
