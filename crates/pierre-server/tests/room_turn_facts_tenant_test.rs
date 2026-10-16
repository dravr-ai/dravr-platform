// ABOUTME: A room turn reads the athlete's facts from their own workspace, not the channel owner's tenant
// ABOUTME: Drives pierre_chat_pipeline::run with a prompt-recording mock and two distinct tenants (carnet#926)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! In a group room the conversation row sits under the channel's tenant while
//! the athlete's tools — and their facts — run under their own workspace. The
//! guided walk stamps its answers there, so stage 7e's OKF bundle has to read
//! there too, or a room turn coaches without the North Star the athlete just
//! gave. The model is the one part of the turn that is mocked: it records the
//! system prompt it was sent.

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;
use pierre_chat_pipeline::{
    InputSource, PipelineHooks, QuotaState, SurfaceId, SurfaceProfile, SurfaceRequest, TurnInput,
    TurnOrigin,
};
use pierre_core::errors::AppError;
use pierre_core::models::{ConversationTurnId, Tenant, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user};

const MODEL: &str = "room-facts-model";
const ANSWER: &str = "On garde le cap.";
const NORTH_STAR: &str = "finish the mountain ultra with my brother";

/// Records every system prompt it is sent and answers in prose.
///
/// Mock is test-only: the LLM boundary is exactly what this test reads.
struct RecordingModel {
    prompts: Arc<Mutex<Vec<String>>>,
    models: Vec<String>,
}

#[async_trait]
impl LlmProvider for RecordingModel {
    fn name(&self) -> &'static str {
        "room_facts_recorder"
    }
    fn display_name(&self) -> &'static str {
        "Recording mock (room facts tenant pin)"
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
        let system: String = request
            .messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::System))
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.prompts.lock().unwrap().push(system);
        Ok(ChatResponse {
            content: ANSWER.to_owned(),
            model: MODEL.to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
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

async fn athlete_tenant(resources: &ServerContext, user: Uuid) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user)
        .await
        .unwrap()
        .first()
        .expect("the athlete has a tenant")
        .id
}

/// The channel owner's tenant, which the room's conversation row sits under.
async fn channel_tenant(resources: &ServerContext, owner: Uuid) -> TenantId {
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Channel owner".to_owned(),
            slug: format!("channel-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    tenant_id
}

async fn seed_north_star(resources: &ServerContext, tenant: TenantId, user: Uuid) {
    resources
        .common
        .repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: &user.to_string(),
            agent_id: None,
            scope: MemoryScope::User,
            kind: FactKind::NorthStar,
            pillar: None,
            predicate_code: PredicateCode::Have,
            object: NORTH_STAR,
            confidence: 0.9,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_room_turn_carries_the_north_star_stamped_in_the_athletes_own_workspace() {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let model = RecordingModel {
        prompts: Arc::clone(&prompts),
        models: vec![MODEL.to_owned()],
    };
    let resources = create_test_server_resources_with_chat_provider(Arc::new(model))
        .await
        .unwrap();
    let (user, _) = create_test_user(&resources.agent.database).await.unwrap();
    let home = athlete_tenant(&resources, user).await;
    let channel = channel_tenant(&resources, user).await;
    assert_ne!(home, channel);

    // Where the guided walk stamps the athlete's answer: their own workspace.
    seed_north_star(&resources, home, user).await;

    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(&user.to_string(), channel, "room", MODEL, None, None)
        .await
        .unwrap();
    let ctx = resources.chat_pipeline_context();
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    });

    pierre_chat_pipeline::run(
        &ctx,
        TurnInput {
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            conversation_id: conversation.id.clone(),
            user_id: user.to_string(),
            conversation_tenant_id: channel,
            tool_tenant_id: home,
            is_direct_message: false,
            content: "What should I focus on this week?".to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quota: QuotaState::Ok,
            mentioned_agent: None,
            steps_after: None,
        },
        &profile,
        &PipelineHooks::none(),
    )
    .await
    .expect("the room turn completes");

    let prompt = prompts
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the room turn reached the model");
    assert!(
        prompt.contains(NORTH_STAR),
        "a room turn's prompt carries the North Star the athlete gave in their own workspace"
    );
}
