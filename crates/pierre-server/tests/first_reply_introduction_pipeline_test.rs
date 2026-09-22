// ABOUTME: A bound agent's first reply is told to introduce itself in the athlete's locale; its second never is
// ABOUTME: Drives the real chat pipeline and reads the system prompt it actually assembled for each turn
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#501.
//!
//! Live on dev, 2026-09-21 23:46Z: the first message of a web chat with the
//! Half Marathon Agent — « Montre-moi mes sorties de mars 2026 avec le
//! dénivelé. » — got a reply that opened straight into analysis, with no name
//! and no role. No introduction rule had ever existed in either prompt
//! history, so nothing asked for one.
//!
//! The rule is decided at Stage 7g.3 of prompt assembly, which only runs inside
//! a real turn. So these tests run real turns through
//! `pierre_chat_pipeline::execute` against a model that records what it was
//! sent, and assert on the system prompt of the coaching call: turn 1 with a
//! bound agent carries the introduction, turn 2 does not, and a turn with no
//! agent or in a shared room never does. The pure decisions behind it are
//! pinned in `pierre-chat-pipeline/tests/first_reply_introduction_test.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;
use uuid::Uuid;

use common::{create_test_server_resources_with_llm, create_test_user_with_plan};
use pierre_chat_pipeline::{
    CommandPersistence, PipelineHooks, SurfaceId, SurfaceProfile, SurfaceRequest, TurnOrigin,
    TurnRequest,
};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
    TokenUsage,
};
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_database::seed_models::SeedAgent;
use pierre_mcp_server::mcp::resources::ServerContext;

const MOCK_REPLY: &str = "Mars a été un mois de fond plutôt que de spécifique semi.";

/// The directive's own opening, present only when the turn carries it.
const INTRODUCTION_MARKER: &str = "This is your first reply in this conversation";

/// Heads the ordinary turn directive, which every coaching call carries. It is
/// how the coaching call is told apart from the background passes a turn
/// spawns — memory extraction, advice capture — which may land in between.
const COACHING_CALL_MARKER: &str = "# This turn";

/// Records the system prompt of every completion, in call order.
#[derive(Default)]
struct CapturingProvider {
    system_prompts: Mutex<Vec<String>>,
}

impl CapturingProvider {
    fn calls_so_far(&self) -> usize {
        self.system_prompts.lock().unwrap().len()
    }

    /// The system prompt of the first coaching call made after `from`.
    fn coaching_prompt_since(&self, from: usize) -> String {
        self.system_prompts
            .lock()
            .unwrap()
            .iter()
            .skip(from)
            .find(|prompt| prompt.contains(COACHING_CALL_MARKER))
            .cloned()
            .unwrap_or_else(|| panic!("no coaching call was made after call #{from}"))
    }
}

#[async_trait]
impl LlmProvider for CapturingProvider {
    fn name(&self) -> &'static str {
        "capturing_mock"
    }
    fn display_name(&self) -> &'static str {
        "Capturing Mock LLM (first-reply introduction)"
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
        let system = request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::System)
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.system_prompts.lock().unwrap().push(system);
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
    provider: Arc<CapturingProvider>,
}

async fn setup(email: &str) -> Fixture {
    let provider = Arc::new(CapturingProvider::default());
    let resources = create_test_server_resources_with_llm(provider.clone())
        .await
        .unwrap();
    let (user_id, _user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    Fixture {
        resources,
        tenant_id,
        user_id,
        provider,
    }
}

/// An agent the athlete authored: its prompt is their text, its name the
/// `agents.title` column.
async fn custom_agent(fx: &Fixture, title: &str) -> String {
    fx.resources
        .common
        .repos
        .agents
        .create(
            fx.user_id,
            fx.tenant_id,
            &CreateAgentRequest {
                title: title.to_owned(),
                description: None,
                system_prompt: "Tu aides des coureurs de trail à préparer leurs courses."
                    .to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

/// The French persona file the catalogue ships for this agent, frontmatter
/// first — the shape the seeder parses and the registry serves.
const HALF_MARATHON_FR: &str = "---
name: half-marathon-agent
title: Agent Semi-Marathon
category: training
tags: [course-a-pied, semi-marathon]
visibility: tenant
---

## Purpose
Spécialiste de la préparation et du pacing sur 21,1 km.

## Instructions
Tu es spécialiste du semi-marathon et tu aides les coureurs à préparer le 21,1 km.
";

/// A catalogue agent as the seeder writes it — canonical English title,
/// `source = 'contremaitre'` — with its French persona in the live registry.
async fn catalogue_agent(fx: &Fixture) -> String {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    fx.resources
        .common
        .repos
        .seeder
        .seed_insert_agent(&SeedAgent {
            id: id.clone(),
            user_id: fx.user_id,
            tenant_id: fx.tenant_id,
            title: "Half Marathon Agent".to_owned(),
            description: "13.1 mile race preparation".to_owned(),
            system_prompt: "You are a half marathon specialist.".to_owned(),
            category: "training".to_owned(),
            tags_json: "[]".to_owned(),
            sample_prompts_json: "[]".to_owned(),
            token_count: 10,
            visibility: "tenant".to_owned(),
            slug: "half-marathon-agent".to_owned(),
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            prerequisites_json: "{}".to_owned(),
            source_file: None,
            content_hash: None,
            startup_query: None,
            data_requirements: None,
            visuals: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    fx.resources
        .chat_pipeline_context()
        .prompt_registry
        .update_agent_prompt(
            "half-marathon-agent",
            "fr",
            HALF_MARATHON_FR.to_owned(),
            "0123456789abcdef".to_owned(),
        );
    id
}

async fn conversation(fx: &Fixture, agent_id: Option<&str>) -> String {
    fx.resources
        .common
        .repos
        .chat
        .create_conversation(
            &fx.user_id.to_string(),
            fx.tenant_id,
            "Semi",
            "mock-model",
            agent_id,
            None,
        )
        .await
        .unwrap()
        .id
}

fn web_profile(locale: &str) -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: locale.to_owned(),
        transport: None,
        prose_contract: None,
    })
}

/// Run one athlete turn and return the system prompt its coaching call got.
async fn turn_prompt(
    fx: &Fixture,
    conversation_id: &str,
    content: &str,
    locale: &str,
    is_direct_message: bool,
) -> String {
    let before = fx.provider.calls_so_far();
    pierre_chat_pipeline::execute(
        &fx.resources.chat_pipeline_context(),
        TurnRequest {
            origin: TurnOrigin::Athlete,
            conversation_id: conversation_id.to_owned(),
            user_id: fx.user_id,
            conversation_tenant_id: fx.tenant_id,
            tool_tenant_id: fx.tenant_id,
            content: content.to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            channel_type: "web",
            is_direct_message,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            hooks: PipelineHooks::none(),
        },
        &web_profile(locale),
    )
    .await
    .expect("the turn is served");
    fx.provider.coaching_prompt_since(before)
}

/// The defect and its bound in one conversation: the first reply is told to
/// introduce the agent, the second is not.
#[tokio::test]
async fn a_bound_agents_first_reply_is_introduced_and_its_second_is_not() {
    let fx = setup("intro-custom@test.com").await;
    let agent = custom_agent(&fx, "Coach Trail Laurentides").await;
    let conv = conversation(&fx, Some(&agent)).await;

    let first = turn_prompt(
        &fx,
        &conv,
        "Montre-moi mes sorties de mars 2026 avec le dénivelé.",
        "fr",
        true,
    )
    .await;
    assert!(
        first.contains("introducing yourself by your title, Coach Trail Laurentides,"),
        "the first reply must be told to open by naming the agent it is — got a prompt \
         of {} chars without it",
        first.len()
    );
    let task = first.find(COACHING_CALL_MARKER).unwrap();
    let introduction = first.find(INTRODUCTION_MARKER).unwrap();
    assert!(
        task < introduction,
        "the introduction rides the turn's task, after it"
    );

    let second = turn_prompt(&fx, &conv, "Et en avril ?", "fr", true).await;
    assert!(
        !second.contains(INTRODUCTION_MARKER),
        "the second reply must not introduce the agent again"
    );
    assert!(
        second.contains(COACHING_CALL_MARKER),
        "the second reply still carries the ordinary turn directive"
    );
}

/// A catalogue agent introduces itself by the title its persona carries in
/// the athlete's locale, not by the canonical English column.
#[tokio::test]
async fn a_catalogue_agent_is_introduced_by_its_title_in_the_athletes_locale() {
    let fx = setup("intro-catalogue@test.com").await;
    let agent = catalogue_agent(&fx).await;
    let conv = conversation(&fx, Some(&agent)).await;

    let first = turn_prompt(
        &fx,
        &conv,
        "Montre-moi mes sorties de mars 2026 avec le dénivelé.",
        "fr",
        true,
    )
    .await;
    assert!(
        first.contains("introducing yourself by your title, Agent Semi-Marathon,"),
        "a French athlete meets « Agent Semi-Marathon », the name the store shows them"
    );
    assert!(
        !first.contains("by your title, Half Marathon Agent"),
        "the canonical English title must not be the one a French athlete is told"
    );
}

/// Dravr itself, with no agent bound, keeps the behaviour it always had.
#[tokio::test]
async fn a_coachless_first_reply_is_not_introduced() {
    let fx = setup("intro-coachless@test.com").await;
    let conv = conversation(&fx, None).await;

    let first = turn_prompt(&fx, &conv, "Comment s'est passée ma semaine ?", "fr", true).await;
    assert!(
        !first.contains(INTRODUCTION_MARKER),
        "a turn with no agent bound carries no introduction"
    );
}

/// In a shared room every member has their own conversation row, so a
/// newcomer's first message would re-introduce an agent the room already knows.
#[tokio::test]
async fn a_shared_room_turn_is_not_introduced() {
    let fx = setup("intro-room@test.com").await;
    let agent = custom_agent(&fx, "Coach Trail Laurentides").await;
    let conv = conversation(&fx, Some(&agent)).await;

    let first = turn_prompt(&fx, &conv, "On fait quoi samedi ?", "fr", false).await;
    assert!(
        !first.contains(INTRODUCTION_MARKER),
        "a turn whose reply the whole room reads carries no introduction"
    );
}
