// ABOUTME: A chat turn typed in an admin's support session mints no connect link, over the JSON and the streamed route
// ABOUTME: A scripted model calls connect_provider; the refusal is audited and the athlete's own turn gets the authorization

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The web and mobile chat route scopes the admin's support session around
//! the turn, and the tool executor carries it into each tool body
//! (carnet#1634). These tests drive the production route end to end: the
//! model is the only stand-in, and it asks for `connect_provider` on every
//! turn. Under the support session the tool answers with the refusal, written
//! to the admin audit trail, and no authorization reaches the model; on the
//! athlete's own session the same turn gets one.

use crate::common;
use crate::helpers;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::impersonation::{assert_refusal_audited, impersonation_of};
use pierre_auth::connect_order::{set_connect_order, ConnectOrder};
use pierre_core::errors::AppError;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, ConversationResponse};
use serde_json::json;
use uuid::Uuid;

const MODEL: &str = "support-turn-model";
const ASK: &str = "Connecte mon compte Strava.";
const ANSWER: &str = "C'est noté.";
/// How the tool loop hands a tool's answer back to the model.
const TOOL_RESULT_OPENING: &str = "[Tool Result for ";
/// What the tool answers with when it mints the authorization.
const MINTED: &str = "pending_authorization";

/// Asks for `connect_provider` until a tool answer is in the request, then
/// answers in prose; keeps every tool answer it was handed.
struct ConnectThenAnswer {
    tool_results: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl ConnectThenAnswer {
    fn answers_so_far(&self) -> Vec<String> {
        self.tool_results.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for ConnectThenAnswer {
    fn name(&self) -> &'static str {
        "connect_then_answer"
    }
    fn display_name(&self) -> &'static str {
        "Connect-then-answer mock (support session turn)"
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
        // This turn's ask is the last message carrying it; the tool answers
        // after it are this turn's, the ones before it an earlier turn's.
        let asked_at = request
            .messages
            .iter()
            .rposition(|m| matches!(m.role, MessageRole::User) && m.content.contains(ASK));
        let answered: Vec<String> = request
            .messages
            .iter()
            .skip(asked_at.map_or(0, |at| at + 1))
            .filter(|m| {
                matches!(m.role, MessageRole::Tool)
                    || (matches!(m.role, MessageRole::User)
                        && m.content.starts_with(TOOL_RESULT_OPENING))
            })
            .map(|m| m.content.clone())
            .collect();
        let asks_to_connect = asked_at.is_some_and(|at| at + 1 == request.messages.len());
        let tool_calls = if asks_to_connect {
            Some(vec![ToolCallRequest {
                id: format!("call-{}", Uuid::new_v4()),
                function_name: "connect_provider".to_owned(),
                arguments: json!({ "provider": "strava" }),
            }])
        } else {
            self.tool_results.lock().unwrap().extend(answered);
            None
        };
        Ok(ChatResponse {
            content: if tool_calls.is_some() {
                String::new()
            } else {
                ANSWER.to_owned()
            },
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

struct Fixture {
    resources: Arc<ServerContext>,
    model: Arc<ConnectThenAnswer>,
    router: axum::Router,
    athlete: Uuid,
    session: String,
    support: String,
    thread: String,
}

async fn fixture(label: &str) -> Fixture {
    let model = Arc::new(ConnectThenAnswer {
        tool_results: Mutex::new(Vec::new()),
        models: vec![MODEL.to_owned()],
    });
    let provider: Arc<dyn LlmProvider> = model.clone();
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    // The authorization rather than the hosted sign-in, so the athlete's own
    // turn is answered with a URL minted here (carnet#1494).
    set_connect_order(
        resources.common.repos.system_settings.as_ref(),
        "strava",
        ConnectOrder::OAuthFirst,
    )
    .await
    .unwrap();
    let (athlete, user, _) = create_test_user_with_plan(
        &resources.agent.database,
        &format!("imp-turn-{label}@example.com"),
        "professional",
    )
    .await
    .unwrap();
    let session = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let support = impersonation_of(&resources, &session).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &session)
        .json(&json!({ "title": "Support turn" }))
        .send(router.clone())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::CREATED,
        "the athlete's own session opens the thread: {}",
        resp.body_text()
    );
    let thread = resp.json::<ConversationResponse>().id;
    Fixture {
        resources,
        model,
        router,
        athlete,
        session,
        support,
        thread,
    }
}

impl Fixture {
    async fn ask(&self, bearer: &str, streamed: bool) -> AxumTestResponse {
        let mut request =
            AxumTestRequest::post(&format!("/api/chat/conversations/{}/messages", self.thread))
                .header("authorization", bearer);
        if streamed {
            request = request.header("accept", "text/event-stream, application/json");
        }
        let response = request
            .json(&json!({ "content": ASK }))
            .send(self.router.clone())
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.body_text()
        );
        assert_eq!(
            response
                .header("content-type")
                .is_some_and(|kind| kind.starts_with("text/event-stream")),
            streamed,
            "the turn took the route asked for"
        );
        response
    }

    /// The support session's turn hands the model the refusal, never an
    /// authorization, and audits it; the athlete's own turn mints one.
    async fn assert_support_turn_connects_nothing(&self, streamed: bool) {
        let refused = self.ask(&self.support, streamed).await;
        let answers = self.model.answers_so_far();
        assert!(
            !answers.is_empty(),
            "the model was handed the tool's answer: {}",
            refused.body_text()
        );
        assert!(
            answers.iter().all(|answer| !answer.contains(MINTED)),
            "the support session's turn mints no authorization: {answers:?}"
        );
        assert!(
            !refused.body_text().contains(MINTED),
            "nor does its reply carry one"
        );
        assert_refusal_audited(&self.resources, self.athlete, "start a provider connection").await;

        let before = answers.len();
        self.ask(&self.session, streamed).await;
        let own = self.model.answers_so_far();
        assert!(
            own[before..].iter().any(|answer| answer.contains(MINTED)),
            "the athlete's own turn mints the authorization: {:?}",
            &own[before..]
        );
    }
}

#[tokio::test]
async fn a_support_sessions_chat_turn_connects_no_provider() {
    fixture("json")
        .await
        .assert_support_turn_connects_nothing(false)
        .await;
}

#[tokio::test]
async fn a_support_sessions_streamed_chat_turn_connects_no_provider() {
    fixture("sse")
        .await
        .assert_support_turn_connects_nothing(true)
        .await;
}
