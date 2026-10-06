// ABOUTME: carnet#521 — a coach reply derived from Garmin device-sourced data carries the platform's Garmin attribution line
// ABOUTME: Through the public chat pipeline entry: a Garmin-sourced turn gets the line, a turn with none does not

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin's API brand guidelines require an AI insight derived in part from
//! Garmin device-sourced data to say so. The platform knows whether a turn
//! handed the model such data — every activity a model reads passes the
//! AI-read filter, which notes its attribution — so the line is appended by
//! the pipeline, never left to the model.
//!
//! These tests drive `pierre_chat_pipeline::run`, the entry every surface
//! calls, with a model that reads the athlete's activities once and answers:
//! the delivered and persisted reply carries the localized line when a read
//! activity was Garmin-recorded, nothing when none was, and nothing on the
//! platform's own reconnect text. Another pins the turn's provenance
//! (carnet#769), the route a tool read on the Copilot loopback takes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use pierre_chat_pipeline::{
    PipelineHooks, QuotaState, SurfaceId, SurfaceProfile, SurfaceRequest, TurnInput, TurnOrigin,
};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_REPLY_GARMIN_ATTRIBUTION,
};
use pierre_core::errors::AppError;
use pierre_core::models::{
    ActivityBuilder, ConnectionType, ConversationTurnId, SportType, TenantId,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_mcp_server::context::ServerContext;
use pierre_providers::ai_scope::{self, Provenance};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user};

/// The agent's answer over the athlete's week.
const AGENT_ANSWER: &str =
    "Ta sortie longue de 200 km domine ta semaine. On garde le tempo pour jeudi.";

/// A model that reads the athlete's activities once, then answers.
struct ActivitiesThenAnswer {
    asked: Mutex<bool>,
    models: Vec<String>,
}

impl ActivitiesThenAnswer {
    fn new() -> Self {
        Self {
            asked: Mutex::new(false),
            models: vec!["attribution-model".to_owned()],
        }
    }
}

#[async_trait]
impl LlmProvider for ActivitiesThenAnswer {
    fn name(&self) -> &'static str {
        "activities_then_answer"
    }
    fn display_name(&self) -> &'static str {
        "Activities-then-answer mock (Garmin attribution pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "attribution-model"
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
                    id: "call-activities".to_owned(),
                    function_name: "get_activities".to_owned(),
                    arguments: json!({ "limit": 10, "mode": "summary" }),
                }])
            }
        };
        let content = if tool_calls.is_some() {
            String::new()
        } else {
            AGENT_ANSWER.to_owned()
        };
        Ok(ChatResponse {
            content,
            model: "attribution-model".to_owned(),
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

/// An athlete whose Strava connection holds a long ride in the durable cache,
/// recorded by a Garmin watch (`source: garmin`) or by nothing named.
async fn athlete_with_a_ride(
    resources: &Arc<ServerContext>,
    source: Option<&str>,
    sibling_serves: bool,
) -> (Uuid, TenantId) {
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
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
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
    .source_opt(source.map(str::to_owned))
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &[ride])
        .await
        .unwrap();
    // A second connection with no session, elected last: the turn's live read
    // fails on it and the Strava cache serves the window, as it does for an
    // athlete whose other provider lapsed. The model still answers. Without
    // it, the lapsed Strava link blanks the turn into the reconnect message.
    if !sibling_serves {
        return (user_id, tenant);
    }
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    (user_id, tenant)
}

fn web_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

/// The reply `pierre_chat_pipeline::run` delivers for one turn.
async fn delivered_reply(source: Option<&str>, sibling_serves: bool) -> String {
    let provider: Arc<dyn LlmProvider> = Arc::new(ActivitiesThenAnswer::new());
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, tenant) = athlete_with_a_ride(&resources, source, sibling_serves).await;
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "garmin attribution pin",
            "attribution-model",
            None,
            None,
        )
        .await
        .unwrap();
    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        conversation_id: conversation.id.clone(),
        user_id: user_id.to_string(),
        conversation_tenant_id: tenant,
        tool_tenant_id: tenant,
        is_direct_message: true,
        content: "Comment se passe ma semaine ?".to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
    };
    let ctx = resources.chat_pipeline_context();
    // Tracked the way `turn_service::execute`, the only caller of `run`,
    // tracks every turn (carnet#769).
    let envelope = ai_scope::tracking(
        Provenance::new(),
        pierre_chat_pipeline::run(&ctx, input, &web_profile(), &PipelineHooks::none()),
    )
    .await
    .expect("the turn is served");

    // What the athlete receives is what the conversation keeps.
    let stored = resources
        .common
        .repos
        .chat
        .get_messages(&conversation.id, &user_id.to_string(), tenant)
        .await
        .unwrap();
    let kept = stored
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .expect("the reply is persisted");
    assert_eq!(kept.content, envelope.assistant.message.content);
    envelope.assistant.message.content
}

fn garmin_line(locale: &str) -> String {
    MessagingStringsRegistry::new().get(KEY_REPLY_GARMIN_ATTRIBUTION, locale)
}

#[tokio::test]
async fn a_reply_derived_from_garmin_data_carries_the_attribution_line() {
    let reply = delivered_reply(Some("garmin"), true).await;
    let line = garmin_line("fr");
    assert!(!line.is_empty(), "the string exists");
    assert!(
        reply.contains("200 km"),
        "the agent's answer stays: {reply}"
    );
    assert!(
        reply.trim_end().ends_with(&line),
        "the reply ends with the attribution, in its language: {reply}"
    );
    assert_eq!(reply.matches(&line).count(), 1, "{reply}");
}

#[tokio::test]
async fn a_reply_with_no_garmin_data_carries_no_attribution_line() {
    let reply = delivered_reply(None, true).await;
    assert!(reply.contains("200 km"), "{reply}");
    for locale in ["fr", "en"] {
        assert!(
            !reply.contains(&garmin_line(locale)),
            "no Garmin data, no Garmin line: {reply}"
        );
    }
}

/// The platform's own reconnect message answers nothing from the athlete's
/// data, so it owes no attribution even though the turn read a Garmin ride.
#[tokio::test]
async fn the_platforms_reconnect_message_carries_no_attribution_line() {
    let reply = delivered_reply(Some("garmin"), false).await;
    assert!(!reply.contains("200 km"), "the turn blanked: {reply}");
    for locale in ["fr", "en"] {
        assert!(!reply.contains(&garmin_line(locale)), "{reply}");
    }
}

/// A tool read on the Copilot loopback runs on a task the turn never sees: an
/// executor built inside the turn carries the turn's provenance there, so what
/// it handed the model reaches the turn's reply (carnet#769's accumulator).
#[tokio::test]
async fn a_loopback_tool_read_attributes_the_turn_that_built_its_executor() {
    let provider: Arc<dyn LlmProvider> = Arc::new(ActivitiesThenAnswer::new());
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, tenant) = athlete_with_a_ride(&resources, Some("garmin"), true).await;
    let turn = Provenance::new();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = ai_scope::tracking(turn.clone(), async {
        UniversalExecutor::new(Arc::clone(&runtime)).with_scopes(OAuthScope::self_grant())
    })
    .await;
    let response = tokio::spawn(async move {
        executor
            .execute_tool(UniversalRequest {
                tool_name: "get_activities".to_owned(),
                parameters: json!({ "limit": 10, "mode": "summary" }),
                user_id: user_id.to_string(),
                protocol: "mcp".to_owned(),
                tenant_id: Some(tenant.to_string()),
            })
            .await
    })
    .await
    .unwrap()
    .expect("the window is served");
    assert!(response.success, "{:?}", response.error);
    assert_eq!(turn.attributions(), BTreeSet::from(["Garmin"]));
}
