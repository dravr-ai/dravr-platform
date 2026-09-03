// ABOUTME: A platform-composed turn answers without writing a message the athlete never sent
// ABOUTME: Pins TurnOrigin::Platform — no user row, no read-marker advance, and the model still sees the question
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#246.
//!
//! The pipeline writes a turn's prompt to `chat_messages` as a `user` row
//! before it answers, and that is right for a message an athlete typed. The
//! backfill-completion re-entry composes its prompt — it re-asks the athlete's
//! own earlier question once their history has loaded — so the row it used to
//! leave behind was attributed to them and rendered in their own thread as
//! them asking the same thing twice.
//!
//! The write was load-bearing, which is why this could not simply be deleted:
//! the model reads the current question out of the history loaded *after* that
//! write. So the interesting assertion here is not only that no row is written
//! — it is that the model still receives the question anyway.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::stream;
use uuid::Uuid;

use common::{create_test_server_resources_with_llm, create_test_user_with_plan};
use pierre_chat_pipeline::{
    CommandPersistence, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnOrigin, TurnRequest,
};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
    TokenUsage,
};
use pierre_core::models::{AddMessageParams, ConversationTurnId, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;

const MOCK_REPLY: &str = "Voici tes sorties de 2022: trois courses en janvier, deux en février.";

/// Records the user-role text the pipeline actually handed the model, so a
/// test can prove the question reached it without a persisted row behind it.
///
/// One entry per completion, in call order, because a turn makes more than
/// one: the coaching dispatch first, then the background memory extractor with
/// its own `"User turn: … Return the JSON array only."` prompt. Keeping only
/// the latest captured the extractor and hid the turn under test.
#[derive(Default)]
struct CapturingProvider {
    /// The `user` messages of each request, oldest first, one Vec per call.
    calls: Mutex<Vec<Vec<String>>>,
}

impl CapturingProvider {
    /// The user messages of the coaching dispatch — the first completion of
    /// the turn, before any background pass.
    fn coaching_call(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .first()
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl LlmProvider for CapturingProvider {
    fn name(&self) -> &'static str {
        "capturing_mock"
    }
    fn display_name(&self) -> &'static str {
        "Capturing Mock LLM (turn origin)"
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

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.calls.lock().unwrap().push(
            request
                .messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .map(|m| m.content.clone())
                .collect(),
        );
        Ok(ChatResponse {
            content: MOCK_REPLY.to_owned(),
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(25, 15, 40)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
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
    tenant_id: TenantId,
    user_id: Uuid,
    conversation_id: String,
    provider: Arc<CapturingProvider>,
}

/// An athlete with one conversation that already holds the exchange a backfill
/// re-entry follows: their question, and the placeholder that answered it.
async fn setup() -> Fixture {
    let provider = Arc::new(CapturingProvider::default());
    let resources = create_test_server_resources_with_llm(provider.clone())
        .await
        .unwrap();
    let repos = resources.coach.database.repositories();
    let (user_id, _user, _) = create_test_user_with_plan(
        &resources.coach.database,
        "proactive-turn@test.com",
        "professional",
    )
    .await
    .unwrap();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    let user = user_id.to_string();
    let conversation = repos
        .chat
        .create_conversation(&user, tenant_id, "Historique", "mock-model", None, None)
        .await
        .unwrap();

    for (role, content) in [
        ("user", "montre-moi mes sorties de 2022"),
        (
            "assistant",
            "Je récupère ton historique plus ancien, ça peut prendre une minute.",
        ),
    ] {
        repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id,
                conversation_id: &conversation.id,
                user_id: &user,
                role,
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
            })
            .await
            .unwrap();
    }

    Fixture {
        resources,
        tenant_id,
        user_id,
        conversation_id: conversation.id,
        provider,
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

fn request(fx: &Fixture, origin: TurnOrigin, content: &str) -> TurnRequest<'static> {
    TurnRequest {
        origin,
        conversation_id: fx.conversation_id.clone(),
        user_id: fx.user_id,
        conversation_tenant_id: fx.tenant_id,
        tool_tenant_id: fx.tenant_id,
        content: content.to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        channel_type: "web",
        is_direct_message: true,
        ambient_group_fallback: false,
        command_persistence: CommandPersistence::Always,
        sender_id: None,
        hooks: PipelineHooks::none(),
    }
}

/// The whole point: a platform turn answers into the thread and leaves behind
/// exactly one new row — the reply — never a question the athlete did not ask.
#[tokio::test]
async fn a_platform_turn_writes_no_message_the_athlete_never_sent() {
    let fx = setup().await;
    let ctx = fx.resources.chat_pipeline_context();
    let user = fx.user_id.to_string();

    let served = pierre_chat_pipeline::execute(
        &ctx,
        request(&fx, TurnOrigin::Platform, "montre-moi mes sorties de 2022"),
        &web_profile(),
    )
    .await
    .expect("the proactive turn is served");
    let ServedTurn::Pipeline(envelope) = served else {
        panic!("a plain-prose turn runs the pipeline");
    };
    // Not asserted against `MOCK_REPLY`: this athlete has no provider
    // connection, so `auth_recovery` rewrites the reply into a reconnect
    // offer. Which text wins is that stage's business; this test's business
    // is which ROWS the turn leaves behind.
    assert!(
        !envelope.assistant.message.content.trim().is_empty(),
        "the turn still produces a reply"
    );

    let rows = fx
        .resources
        .common
        .repos
        .chat
        .get_messages(&fx.conversation_id, &user, fx.tenant_id)
        .await
        .unwrap();

    let user_rows: Vec<_> = rows.iter().filter(|m| m.role == "user").collect();
    assert_eq!(
        user_rows.len(),
        1,
        "the athlete asked once; a proactive turn must not write a second ask: {rows:?}"
    );
    assert_eq!(user_rows[0].content, "montre-moi mes sorties de 2022");

    let assistant_rows: Vec<_> = rows.iter().filter(|m| m.role == "assistant").collect();
    assert_eq!(
        assistant_rows.len(),
        2,
        "the placeholder and the answer, and nothing else: {rows:?}"
    );
    assert_eq!(
        assistant_rows[1].content, envelope.assistant.message.content,
        "the reply IS persisted — only the prompt is not"
    );
}

/// The write that was removed was load-bearing: the model reads the current
/// question out of the history loaded after it. Removing the row without
/// replacing the path would have answered a question the model never saw, so
/// this asserts the question still arrives.
#[tokio::test]
async fn the_model_still_receives_a_platform_prompt_it_has_no_row_for() {
    let fx = setup().await;
    let ctx = fx.resources.chat_pipeline_context();

    pierre_chat_pipeline::execute(
        &ctx,
        request(
            &fx,
            TurnOrigin::Platform,
            "montre-moi mes sorties de 2022, l'historique est prêt",
        ),
        &web_profile(),
    )
    .await
    .expect("the proactive turn is served");

    let seen = fx.provider.coaching_call();
    assert_eq!(
        seen.last().map(String::as_str),
        Some("montre-moi mes sorties de 2022, l'historique est prêt"),
        "the platform prompt must reach the model as the LAST user message, with \
         no row behind it — otherwise the model answers the older question: {seen:?}"
    );
}

/// An athlete turn is unchanged: their message is still written, still theirs,
/// and still the thing the model answers. The fix must not cost the ordinary
/// path anything.
#[tokio::test]
async fn an_athlete_turn_still_writes_the_message_they_sent() {
    let fx = setup().await;
    let ctx = fx.resources.chat_pipeline_context();
    let user = fx.user_id.to_string();

    pierre_chat_pipeline::execute(
        &ctx,
        request(&fx, TurnOrigin::Athlete, "et en 2023?"),
        &web_profile(),
    )
    .await
    .expect("the athlete turn is served");

    let rows = fx
        .resources
        .common
        .repos
        .chat
        .get_messages(&fx.conversation_id, &user, fx.tenant_id)
        .await
        .unwrap();
    let user_rows: Vec<_> = rows.iter().filter(|m| m.role == "user").collect();
    assert_eq!(
        user_rows.len(),
        2,
        "the athlete's own message is still persisted: {rows:?}"
    );
    assert_eq!(user_rows[1].content, "et en 2023?");

    let seen = fx.provider.coaching_call();
    assert_eq!(
        seen.last().map(String::as_str),
        Some("et en 2023?"),
        "and it is still the message the model answers: {seen:?}"
    );
}
