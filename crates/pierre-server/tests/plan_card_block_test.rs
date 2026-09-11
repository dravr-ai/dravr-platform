// ABOUTME: The workout_plan block is projected from the plan save_training_plan stored, never from the model's text
// ABOUTME: Drives pierre_chat_pipeline::run with a mock that saves a plan then answers in prose; the card carries phases and steps
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! D3 of the Annual Vision plan: one vocabulary for a session's structure.
//! The builder agents used to emit a JSON plan document the platform
//! extracted from the reply and rendered as a card; a card is now what the
//! platform reads back out of the rows `save_training_plan` wrote, so the
//! card, the prompt's plan block and the calendar push cannot disagree.
//!
//! The model is the one part of the turn this test does not want real: it
//! calls the tool with a fixed payload, then answers in prose. Everything
//! after that — the save, the projection, the block on the envelope — is the
//! production path.

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Datelike, Duration, Utc};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use pierre_chat_pipeline::{
    PipelineHooks, QuotaState, ReplyBlock, SurfaceId, SurfaceProfile, SurfaceRequest, TurnInput,
    TurnOrigin,
};
use pierre_core::errors::AppError;
use pierre_core::models::ConversationTurnId;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
};
use serde_json::{json, Value};

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user};

const MODEL: &str = "plan-card-model";
const SAVE_ASK: &str = "Pose-moi la première quinzaine.";
const READ_ASK: &str = "Montre-moi ma saison.";
const ANSWER: &str = "Voilà ta première quinzaine — deux séances dures, le reste facile.";

/// The plan the mock saves: a build phase covering today, a taper after it,
/// and one week whose Tuesday runs the catalogue's threshold template.
fn plan_payload(
    week_start: &str,
    tuesday: &str,
    thursday: &str,
    b_race: &str,
    race: &str,
) -> Value {
    json!({
        "outline": {
            "goal_race": { "name": "Parkrun PB", "date": race, "discipline": "run_5k", "priority": "A" },
            "races": [
                { "name": "Club 10k", "date": b_race, "discipline": "run_10k", "priority": "B" }
            ],
            "strategy": "polarised build, two hard days",
            "flavour": { "id": "polarized-classic", "selected_by": "coach", "override_reason": "house style" },
            "phases": [
                {"kind": "build", "start": week_start, "weeks": 6, "intent": "two hard days, the rest easy", "target_hours": 8.0, "hard_sessions_max": 2},
                {"kind": "taper", "start": thursday, "weeks": 2, "intent": "sharpen"}
            ]
        },
        "weeks": [{
            "week_start": week_start,
            "focus": "first build week",
            "phase_index": 0,
            "days": [
                {"date": tuesday, "sport": "run", "workout": "4 x 8 min at threshold", "duration_min": 65, "intensity": "threshold",
                 "template_slug": "threshold_4x8", "template_params": {"reps": 4, "work_seconds": 480, "rest_seconds": 120},
                 "steps": [
                    {"label": "Warm-up", "duration_seconds": 900, "target_zone": "Z1"},
                    {"label": "Threshold", "duration_seconds": 480, "target_zone": "Threshold", "repeat": 4},
                    {"label": "Recovery jog", "duration_seconds": 120, "target_zone": "Z1", "repeat": 4},
                    {"label": "Cool-down", "duration_seconds": 600, "target_zone": "Z1"}
                 ]},
                {"date": thursday, "sport": "run", "workout": "easy", "duration_min": 60, "intensity": "Z2"}
            ]
        }]
    })
}

/// Calls one tool on its first turn, answers in prose on the second.
struct ToolThenAnswer {
    asked: Mutex<bool>,
    tool: &'static str,
    payload: Value,
    models: Vec<String>,
}

#[async_trait]
impl LlmProvider for ToolThenAnswer {
    fn name(&self) -> &'static str {
        "save_then_answer"
    }
    fn display_name(&self) -> &'static str {
        "Save-then-answer mock (plan card pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        MODEL
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let tool_calls = {
            let mut asked = self.asked.lock().unwrap();
            if *asked {
                None
            } else {
                *asked = true;
                Some(vec![ToolCallRequest {
                    id: "call-one".to_owned(),
                    function_name: self.tool.to_owned(),
                    arguments: self.payload.clone(),
                }])
            }
        };
        let content = if tool_calls.is_some() {
            String::new()
        } else {
            ANSWER.to_owned()
        };
        Ok(ChatResponse {
            content,
            model: MODEL.to_owned(),
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

fn web_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

/// Monday of the current week and the dates the fixture needs around it.
///
/// `b_race` is deliberately weeks out rather than inside the current week. The
/// card keeps only races still ahead (`plan_card.rs`, `date >= today`) and
/// `races` is `skip_serializing_if = "Vec::is_empty"`, so a B race dated inside
/// this week drops out of the JSON entirely from the moment it passes — which
/// made `the_reply_carries_the_saved_plan_as_a_card` pass Monday to Thursday
/// and fail Friday, Saturday and Sunday. It first went red on Friday
/// 2026-09-11 against a tree whose last green run was the Thursday.
fn dates() -> (String, String, String, String, String) {
    let today = Utc::now().date_naive();
    let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let f = |d: chrono::NaiveDate| d.format("%Y-%m-%d").to_string();
    (
        f(monday),
        f(monday + Duration::days(1)),
        f(monday + Duration::days(3)),
        f(monday + Duration::weeks(4)),
        f(monday + Duration::weeks(10)),
    )
}

#[tokio::test]
async fn the_reply_carries_the_saved_plan_as_a_card() {
    let (week_start, tuesday, thursday, b_race, race) = dates();
    let provider: Arc<dyn LlmProvider> = Arc::new(ToolThenAnswer {
        asked: Mutex::new(false),
        tool: "save_training_plan",
        payload: plan_payload(&week_start, &tuesday, &thursday, &b_race, &race),
        models: vec![MODEL.to_owned()],
    });
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .expect("user has a tenant")
        .id;

    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(&user_id.to_string(), tenant, "plan card", MODEL, None, None)
        .await
        .unwrap();
    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        conversation_id: conversation.id.clone(),
        user_id: user_id.to_string(),
        conversation_tenant_id: tenant,
        tool_tenant_id: tenant,
        is_direct_message: true,
        content: "Pose-moi la première quinzaine.".to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
    };

    let ctx = resources.chat_pipeline_context();
    let envelope = pierre_chat_pipeline::run(&ctx, input, &web_profile(), &PipelineHooks::none())
        .await
        .expect("the turn completes");

    let plan_json = envelope
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::WorkoutPlan { plan } => Some(plan.clone()),
            _ => None,
        })
        .expect("the turn that saved a plan carries the card");
    let card: Value = serde_json::from_str(&plan_json).expect("the card is JSON");

    // The season: both phases, the one covering today flagged.
    assert_eq!(card["goal_race"]["name"], json!("Parkrun PB"));
    // The B race the athlete named reaches the card too: a season is a
    // calendar, not a single date.
    assert_eq!(card["races"][0]["name"], json!("Club 10k"));
    assert_eq!(card["races"][0]["priority"], json!("B"));
    assert_eq!(card["flavour"]["id"], json!("polarized-classic"));
    assert_eq!(card["flavour"]["selected_by"], json!("coach"));
    let phases = card["phases"].as_array().expect("phases");
    assert_eq!(phases.len(), 2, "{card}");
    assert_eq!(phases[0]["kind"], json!("build"));
    assert_eq!(phases[0]["current"], json!(true));
    assert_eq!(phases[0]["hard_sessions_max"], json!(2));
    assert_eq!(phases[1]["kind"], json!("taper"));
    assert_eq!(card["current_phase_index"], json!(0));

    // The fortnight: the saved week, its days, and the steps the tool stored.
    let weeks = card["weeks"].as_array().expect("weeks");
    assert_eq!(weeks.len(), 1, "{card}");
    assert_eq!(weeks[0]["week_start"], json!(week_start));
    assert_eq!(weeks[0]["current"], json!(true));
    let days = weeks[0]["days"].as_array().expect("days");
    assert_eq!(days.len(), 2);
    assert_eq!(days[0]["template_slug"], json!("threshold_4x8"));
    assert_eq!(days[0]["template_source"], json!("catalogue"));
    let steps = days[0]["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 4);
    assert_eq!(steps[1]["label"], json!("Threshold"));
    assert_eq!(steps[1]["repeat"], json!(4));
    assert_eq!(days[0]["rest"], json!(false));
    assert!(
        days[1].get("steps").is_none(),
        "an easy day carries no steps: {}",
        days[1]
    );

    // The prose is the model's answer, untouched by the card.
    assert!(
        envelope.assistant.message.content.contains("quinzaine"),
        "the reply keeps the model's prose: {}",
        envelope.assistant.message.content
    );
}

#[tokio::test]
async fn a_turn_that_saved_nothing_carries_no_card() {
    let (week_start, tuesday, thursday, b_race, race) = dates();
    // The mock answers in prose from the first call: no tool, no card.
    let provider: Arc<dyn LlmProvider> = Arc::new(ToolThenAnswer {
        asked: Mutex::new(true),
        tool: "save_training_plan",
        payload: plan_payload(&week_start, &tuesday, &thursday, &b_race, &race),
        models: vec![MODEL.to_owned()],
    });
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .expect("user has a tenant")
        .id;
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(&user_id.to_string(), tenant, "no card", MODEL, None, None)
        .await
        .unwrap();
    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        conversation_id: conversation.id.clone(),
        user_id: user_id.to_string(),
        conversation_tenant_id: tenant,
        tool_tenant_id: tenant,
        is_direct_message: true,
        content: "Salut".to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
    };
    let ctx = resources.chat_pipeline_context();
    let envelope = pierre_chat_pipeline::run(&ctx, input, &web_profile(), &PipelineHooks::none())
        .await
        .expect("the turn completes");
    assert!(
        !envelope
            .assistant
            .blocks
            .iter()
            .any(|b| matches!(b, ReplyBlock::WorkoutPlan { .. })),
        "no save, no card"
    );
}

/// Saves the plan on the turn that asks for it, reads it back on the turn
/// that asks to see it.
///
/// The branch is the athlete's own message rather than a call counter: the
/// tool loop calls the model as many times as it needs, and a counter would
/// pin this test to that number.
struct SaveThenRead {
    payload: Value,
    models: Vec<String>,
}

/// `true` when the transcript's last athlete message is `needle`.
fn asked_for(request: &ChatRequest, needle: &str) -> bool {
    request
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::User))
        .is_some_and(|m| m.content.contains(needle))
}

#[async_trait]
impl LlmProvider for SaveThenRead {
    fn name(&self) -> &'static str {
        "save_then_read"
    }
    fn display_name(&self) -> &'static str {
        "Save-then-read mock (plan card pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        MODEL
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        // A tool result already in the transcript means this turn's call was
        // made; answer in prose so the loop ends.
        let already_ran = request
            .messages
            .iter()
            .any(|m| matches!(m.role, MessageRole::Tool));
        let tool = if already_ran {
            None
        } else if asked_for(request, SAVE_ASK) {
            Some(("save_training_plan", self.payload.clone()))
        } else if asked_for(request, READ_ASK) {
            Some(("get_training_plan", json!({})))
        } else {
            None
        };
        let tool_calls = tool.map(|(name, args)| {
            vec![ToolCallRequest {
                id: format!("call-{name}"),
                function_name: name.to_owned(),
                arguments: args,
            }]
        });
        let content = if tool_calls.is_some() {
            String::new()
        } else {
            ANSWER.to_owned()
        };
        Ok(ChatResponse {
            content,
            model: MODEL.to_owned(),
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

#[tokio::test]
async fn asking_to_see_the_plan_carries_the_same_card() {
    // The vision the athlete asked for is the vision they agreed to: a turn
    // that reads the plan renders the same projection as the turn that saved
    // it, so "show me my season" answers with the timeline rather than with
    // the agent's paraphrase of it.
    let (week_start, tuesday, thursday, b_race, race) = dates();
    let provider: Arc<dyn LlmProvider> = Arc::new(SaveThenRead {
        payload: plan_payload(&week_start, &tuesday, &thursday, &b_race, &race),
        models: vec![MODEL.to_owned()],
    });
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .expect("user has a tenant")
        .id;
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "read the plan",
            MODEL,
            None,
            None,
        )
        .await
        .unwrap();

    let turn = |content: &str| TurnInput {
        origin: TurnOrigin::Athlete,
        conversation_id: conversation.id.clone(),
        user_id: user_id.to_string(),
        conversation_tenant_id: tenant,
        tool_tenant_id: tenant,
        is_direct_message: true,
        content: content.to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
    };

    let ctx = resources.chat_pipeline_context();
    let saved =
        pierre_chat_pipeline::run(&ctx, turn(SAVE_ASK), &web_profile(), &PipelineHooks::none())
            .await
            .expect("the save turn completes");
    assert!(
        saved
            .assistant
            .blocks
            .iter()
            .any(|b| matches!(b, ReplyBlock::WorkoutPlan { .. })),
        "the save turn carries the card"
    );

    let read =
        pierre_chat_pipeline::run(&ctx, turn(READ_ASK), &web_profile(), &PipelineHooks::none())
            .await
            .expect("the read turn completes");
    let plan_json = read
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::WorkoutPlan { plan } => Some(plan.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "a turn that read the plan carries the card too; blocks: {:?}",
                read.assistant.blocks
            )
        });
    let card: Value = serde_json::from_str(&plan_json).expect("the card is JSON");
    assert_eq!(card["goal_race"]["name"], json!("Parkrun PB"));
    assert_eq!(card["current_phase_index"], json!(0));
    assert_eq!(
        card["phases"].as_array().map(Vec::len),
        Some(2),
        "the read renders the season the save wrote: {card}"
    );
}
