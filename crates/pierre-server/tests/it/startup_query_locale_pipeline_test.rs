// ABOUTME: A catalogue agent's first-turn startup query reaches the model in the athlete's locale
// ABOUTME: Drives a real first turn per locale and reads the user messages the model was sent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1700.
//!
//! Live on dev, 2026-11-20 10:45:10Z: a French athlete's first turn with
//! Agent Endurance was briefed with the English startup query, "Pull my latest
//! training snapshot…". `agents.startup_query` holds `en.md`'s query only; the
//! French one lives in the contremaitre registry's `fr.md`.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(missing_docs)]

use crate::common;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use futures_util::stream;
use uuid::Uuid;

use common::{create_test_server_resources_with_llm, create_test_user_with_plan};
use pierre_chat_pipeline::{
    CommandPersistence, InputSource, PipelineHooks, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnOrigin, TurnRequest,
};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
    TokenUsage,
};
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_database::seed_models::SeedAgent;
use pierre_mcp_server::mcp::resources::ServerContext;

const SLUG: &str = "endurance-agent";
const EN_QUERY: &str = "Pull my latest training snapshot, dossier, and last 28 days of \
                        history; recommend the next session.";
const FR_QUERY: &str = "Récupère mon dernier snapshot d'entraînement, mon dossier, et les 28 \
                        derniers jours d'historique ; propose la prochaine séance.";

/// Records the user messages of every completion and answers with a fixed reply.
struct CapturingProvider {
    user_messages: Mutex<Vec<String>>,
}

#[async_trait]
impl LlmProvider for CapturingProvider {
    fn name(&self) -> &'static str {
        "capturing_mock"
    }
    fn display_name(&self) -> &'static str {
        "Capturing Mock LLM (startup query locale)"
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
        self.user_messages.lock().unwrap().extend(
            request
                .messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .map(|m| m.content.clone()),
        );
        Ok(ChatResponse {
            replay_blocks: None,
            content: "Bonjour, je suis Agent Endurance de Dravr.".to_owned(),
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
    provider: Arc<CapturingProvider>,
}

/// A locale file in the shape contremaitre ships: frontmatter, then body.
fn locale_file(query: &str) -> String {
    format!(
        "---\nname: {SLUG}\ntitle: Endurance Agent\ncategory: training\nstartup:\n  \
         query: \"{query}\"\n---\n\n## Purpose\n\nEndurance coaching.\n"
    )
}

/// The endurance agent as the seeder writes it — the English query in
/// `agents.startup_query` — and as the registry serves it, `en.md` and `fr.md`.
async fn setup(email: &str) -> (Fixture, String) {
    let provider = Arc::new(CapturingProvider {
        user_messages: Mutex::new(Vec::new()),
    });
    let resources = create_test_server_resources_with_llm(provider.clone())
        .await
        .unwrap();
    let (user_id, _user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    let agent_id = Uuid::new_v4().to_string();
    let now = Timestamp::now();
    resources
        .common
        .repos
        .seeder
        .seed_insert_agent(&SeedAgent {
            id: agent_id.clone(),
            user_id,
            tenant_id,
            title: "Endurance Agent".to_owned(),
            description: "Endurance coaching".to_owned(),
            system_prompt: "You are an endurance specialist.".to_owned(),
            category: "training".to_owned(),
            tags_json: "[]".to_owned(),
            sample_prompts_json: "[]".to_owned(),
            token_count: 10,
            visibility: "tenant".to_owned(),
            slug: SLUG.to_owned(),
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            prerequisites_json: "{}".to_owned(),
            source_file: None,
            content_hash: None,
            startup_query: Some(EN_QUERY.to_owned()),
            data_requirements: None,
            visuals: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let registry = &resources.mcp.prompt_registry;
    registry.update_agent_prompt(SLUG, "en", locale_file(EN_QUERY), "sha-en".to_owned());
    registry.update_agent_prompt(SLUG, "fr", locale_file(FR_QUERY), "sha-fr".to_owned());
    (
        Fixture {
            resources,
            tenant_id,
            user_id,
            provider,
        },
        agent_id,
    )
}

/// Run the conversation's first turn in `locale`; return every user message
/// the model was sent.
async fn first_turn_user_messages(fx: &Fixture, agent_id: &str, locale: &str) -> Vec<String> {
    let conversation_id = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &fx.user_id.to_string(),
            fx.tenant_id,
            "Endurance",
            "mock-model",
            Some(agent_id),
            None,
        )
        .await
        .unwrap()
        .id;
    pierre_chat_pipeline::execute(
        &fx.resources.chat_pipeline_context(),
        TurnRequest {
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            conversation_id,
            user_id: fx.user_id,
            conversation_tenant_id: fx.tenant_id,
            tool_tenant_id: fx.tenant_id,
            content: "Salut !".to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quote: None,
            channel_type: "web",
            transport: Transport::WebApp,
            is_direct_message: true,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            caller: None,
            hooks: PipelineHooks::none(),
            grant: OAuthScope::self_grant(),
        },
        &SurfaceProfile::resolve(&SurfaceRequest {
            surface: SurfaceId::Web,
            locale: locale.to_owned(),
            transport: None,
            prose_contract: None,
        }),
    )
    .await
    .expect("the turn is served");
    fx.provider.user_messages.lock().unwrap().clone()
}

fn carries(messages: &[String], query: &str) -> bool {
    messages.iter().any(|m| m.contains(query))
}

#[tokio::test]
async fn a_french_athletes_first_turn_is_briefed_in_french() {
    let (fx, agent_id) = setup("startup-fr@test.com").await;

    let sent = first_turn_user_messages(&fx, &agent_id, "fr").await;

    assert!(
        carries(&sent, FR_QUERY),
        "the French startup query reaches the model: {sent:#?}"
    );
    assert!(
        !carries(&sent, EN_QUERY),
        "the English startup query stays out of a French turn: {sent:#?}"
    );
}

#[tokio::test]
async fn a_locale_without_its_own_file_is_briefed_in_english() {
    let (fx, agent_id) = setup("startup-it@test.com").await;

    let sent = first_turn_user_messages(&fx, &agent_id, "it").await;

    assert!(
        carries(&sent, EN_QUERY),
        "a locale with no file is briefed with the English startup query: {sent:#?}"
    );
    assert!(
        !carries(&sent, FR_QUERY),
        "another locale's startup query stays out of the turn: {sent:#?}"
    );
}
