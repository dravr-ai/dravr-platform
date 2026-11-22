// ABOUTME: A post the platform makes on its own runs its whole turn, tool rounds included, as background work
// ABOUTME: So the session read its get_activities makes spends the athlete's background caps, never the asked ones (carnet#1661)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1661.
//!
//! Which session-read caps a read spends is decided where it is admitted, by
//! the `in_background` task-local of the task that makes it. A platform post
//! (the sync-landed post, a check-in, the first-sync post, a backfill's
//! re-ask) is a turn nobody waits on, so `PipelinePlatformPost` runs it
//! inside `in_background`. That only reaches the tools if the turn runs its
//! tool calls in the task that awaits it: a task-local does not cross a
//! spawn. This pins it where it is decided, at the session read: the model
//! lists the athlete's Strava activities, which reads their signed-in Strava
//! session through the scraper, and that read is counted in the athlete's
//! background windows and never in the asked ones.
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_core::models::ConnectionType;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::platform_post::{
    PipelinePlatformPost, PlatformPost, PlatformPostRequest, PostSurface,
};
use pierre_providers::request_budget::{
    budget_period, is_background, session_read_counter_key, ONE_DAY, PLATFORM_SCOPE,
};
use uuid::Uuid;

use crate::helpers::sciotte_mock::{seed_sciotte_session, spawn_mock_scraper};

/// A model that lists the athlete's Strava activities through a tool on its
/// first ask and answers on the next, recording for each ask whether it ran in
/// background. It declares no function calling, so the turn takes the text
/// tool loop.
#[derive(Default)]
struct RecordingModel {
    /// Whether each ask ran in background, in call order.
    asks: Mutex<Vec<bool>>,
}

impl RecordingModel {
    fn asks(&self) -> Vec<bool> {
        self.asks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl LlmProvider for RecordingModel {
    fn name(&self) -> &'static str {
        "recording_background_mock"
    }
    fn display_name(&self) -> &'static str {
        "Recording Mock LLM (background scope)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let first = {
            let mut asks = self.asks.lock().unwrap_or_else(PoisonError::into_inner);
            asks.push(is_background());
            asks.len() == 1
        };
        let content = if first {
            r#"Let me look. <tool_call>{"name":"get_activities","arguments":{"provider":"strava","limit":5}}</tool_call>"#
                .to_owned()
        } else {
            "Your morning ride is in, so the week stays as planned.".to_owned()
        };
        Ok(ChatResponse {
            replay_blocks: None,
            content,
            model: "mock-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let response = self.complete(request).await?;
        Ok(Box::pin(stream::iter(vec![Ok(StreamChunk {
            delta: response.content,
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        })])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// The athlete's session reads at the scraper's Strava, counted as `counter`
/// today.
async fn strava_session_reads(resources: &Arc<ServerContext>, user_id: Uuid, counter: &str) -> i64 {
    resources
        .common
        .repos
        .usage_counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &session_read_counter_key("sciotte", counter, user_id),
            &budget_period(Timestamp::now(), ONE_DAY),
        )
        .await
        .unwrap()
        .value
}

#[tokio::test]
async fn a_platform_posts_turn_runs_its_tool_round_in_background() {
    common::init_server_config();
    common::init_test_http_clients();
    let model = Arc::new(RecordingModel::default());
    let scraper = spawn_mock_scraper().await;
    let resources = common::create_test_server_resources_with_chat_provider_and_config(
        model.clone(),
        common::test_server_config_with_sciotte(&scraper),
    )
    .await
    .unwrap();
    let email = format!("background-post-{}@example.com", Uuid::new_v4());
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let conversation_id = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant_id,
            "Accueil",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    // Signed in to Strava through the scraper: every read of the athlete's
    // Strava is a read of their session.
    seed_sciotte_session(&resources, user_id, tenant_id).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, "sciotte", &ConnectionType::Manual, None)
        .await
        .unwrap();

    assert!(
        !is_background(),
        "the test itself is a task someone waits on"
    );
    // Whether the post is kept (it is grounded in nothing here) does not
    // matter: what its turn ran as does.
    PipelinePlatformPost::new(Arc::downgrade(&resources))
        .post(PlatformPostRequest {
            user_id,
            tenant_id,
            conversation_id: &conversation_id,
            surface: PostSurface::InApp,
            locale: "en",
            prompt: "Tell the athlete what their recovery says about this week.",
        })
        .await;

    // The first two asks are the turn's: the tool call, then the answer the
    // model gives once the tool round is in. Later asks (the memory
    // extractor's) are the platform's own detached work.
    let asks = model.asks();
    assert!(
        asks.len() >= 2,
        "bug: the turn did not ask the model again after its tool round: {asks:?}"
    );
    assert_eq!(
        &asks[..2],
        &[true, true],
        "bug: a platform post's turn left background before or after its tool round, so its \
         tools' session reads would spend the athlete's asked caps: {asks:?}"
    );
    assert!(
        strava_session_reads(&resources, user_id, "background").await > 0,
        "the turn's get_activities never read the athlete's Strava session"
    );
    assert_eq!(
        strava_session_reads(&resources, user_id, "asked").await,
        0,
        "bug: a tool a platform post's turn called spent the athlete's asked session reads: \
         the tool ran outside the turn's background scope"
    );
}
