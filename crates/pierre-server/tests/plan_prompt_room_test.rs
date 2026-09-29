// ABOUTME: The always-on plan block stays out of room turns, and a turn's own agent authors what it saves
// ABOUTME: Drives pierre_chat_pipeline::run with a capturing mock, a @mention turn that saves a week, and the headless tool surface
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Three turn-shaped facts about the athlete's one season plan:
//!
//! - The plan block rides every direct turn's system prompt and no room
//!   turn's: a room's reply is posted to every member, and the plan is the
//!   speaker's own.
//! - A `@taper` turn inside the endurance conversation writes as the taper
//!   agent — the agent the athlete addressed — not the conversation's.
//! - The headless tool surface an ACP agent calls back through carries the
//!   same turn agent, so a save made there is authored the same way.
//!
//! The model is the one part of these turns that is mocked: it records the
//! prompt it was sent, or calls one tool and then answers in prose.

mod common;
mod helpers;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Datelike, Duration, NaiveDate, Utc};
use embacle::types::ToolCallRequest;
use embacle_tool_host::ToolSurface;
use futures_util::stream;
use helpers::agent_fixtures::{install_catalogue_agent, publish_catalogue_agent};
use pierre_chat_pipeline::stages::agent_mention::resolve_agent_mention;
use pierre_chat_pipeline::{
    PipelineHooks, QuotaState, SurfaceId, SurfaceProfile, SurfaceRequest, TurnInput, TurnOrigin,
};
use pierre_core::errors::AppError;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::{ConversationTurnId, Tenant, TenantId, User, UserStatus};
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, SavePlanBundleParams};
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
};
use pierre_mcp_server::mcp::resources::tool_surface::HostedToolBridge;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::training_plans::{GoalRace, RacePriority};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common::{
    create_test_server_resources, create_test_server_resources_with_chat_provider, create_test_user,
};

const MODEL: &str = "plan-room-model";
const ANSWER: &str = "On garde le cap sur ta saison.";
/// The heading the plan block opens with in the system prompt.
const PLAN_BLOCK: &str = "## Current training plan (persisted)";

/// Records every system prompt it is sent; calls `tool` once when given one.
///
/// Mock is test-only: the LLM boundary is exactly what these tests read.
struct RecordingModel {
    prompts: Arc<Mutex<Vec<String>>>,
    tool_call: Mutex<Option<(&'static str, Value)>>,
    models: Vec<String>,
}

impl RecordingModel {
    fn new(tool_call: Option<(&'static str, Value)>) -> Self {
        Self {
            prompts: Arc::new(Mutex::new(Vec::new())),
            tool_call: Mutex::new(tool_call),
            models: vec![MODEL.to_owned()],
        }
    }
}

#[async_trait]
impl LlmProvider for RecordingModel {
    fn name(&self) -> &'static str {
        "plan_room_recorder"
    }
    fn display_name(&self) -> &'static str {
        "Recording mock (plan room pin)"
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
        let tool_calls = self
            .tool_call
            .lock()
            .unwrap()
            .take()
            .map(|(tool, arguments)| {
                vec![ToolCallRequest {
                    id: "call-one".to_owned(),
                    function_name: tool.to_owned(),
                    arguments,
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

fn web_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

fn monday() -> NaiveDate {
    let today = Utc::now().date_naive();
    today - Duration::days(i64::from(today.weekday().num_days_from_monday()))
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// The season `author` lays for the athlete, straight through the repository.
async fn lay_season(
    resources: &ServerContext,
    user: Uuid,
    tenant: TenantId,
    author: PlanAuthor<'_>,
) -> String {
    let race = GoalRace {
        name: "Ultra Trail".to_owned(),
        date: ymd(monday() + Duration::weeks(12)),
        discipline: "trail".to_owned(),
        priority: RacePriority::A,
    };
    resources
        .common
        .repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant.to_string(),
            user_id: &user.to_string(),
            author,
            goal_fact_id: None,
            replace_season: false,
            outline: Some(PlanOutlineInput {
                goal_race: &race,
                races: Some(&[]),
                strategy: "long runs grow, then taper",
                flavour: None,
                season_start: None,
                season_end: None,
                phases: &[],
                source_conversation_id: None,
            }),
            weeks: &[],
        })
        .await
        .unwrap()
        .plan
        .id
}

/// A weeks-only save of one taper week, as a model would send it.
fn taper_week_payload() -> Value {
    let start = monday() + Duration::weeks(10);
    json!({
        "weeks": [{
            "week_start": ymd(start),
            "focus": "taper opener",
            "days": [
                { "date": ymd(start), "sport": "run", "workout": "taper strides", "duration_min": 40, "intensity": "Z2" }
            ]
        }]
    })
}

/// Who wrote each active week of the season.
async fn week_authors(
    resources: &ServerContext,
    user: Uuid,
    tenant: TenantId,
    plan: &str,
) -> Vec<Option<String>> {
    resources
        .common
        .repos
        .training_plans
        .list_plan_weeks(&tenant.to_string(), &user.to_string(), plan, false)
        .await
        .unwrap()
        .into_iter()
        .map(|week| week.author_agent_id)
        .collect()
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

/// Another user, owning a tenant, who publishes an agent to the store.
async fn store_author(resources: &ServerContext) -> (Uuid, TenantId) {
    let mut user = User::new(
        format!("store-author-{}@example.com", Uuid::new_v4()),
        "not-a-login".to_owned(),
        Some("Store Author".to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Store author".to_owned(),
            slug: format!("store-author-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    (user_id, tenant_id)
}

fn turn(conversation_id: &str, user: Uuid, tenant: TenantId, is_direct_message: bool) -> TurnInput {
    TurnInput {
        origin: TurnOrigin::Athlete,
        conversation_id: conversation_id.to_owned(),
        user_id: user.to_string(),
        conversation_tenant_id: tenant,
        tool_tenant_id: tenant,
        is_direct_message,
        content: "Où en est ma saison ?".to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
    }
}

#[tokio::test]
async fn the_plan_block_rides_the_dm_prompt_and_never_a_room_turns() {
    let model = RecordingModel::new(None);
    let prompts = Arc::clone(&model.prompts);
    let resources = create_test_server_resources_with_chat_provider(Arc::new(model))
        .await
        .unwrap();
    let (user, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = athlete_tenant(&resources, user).await;
    lay_season(&resources, user, tenant, PlanAuthor::none()).await;
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(&user.to_string(), tenant, "season", MODEL, None, None)
        .await
        .unwrap();
    let ctx = resources.chat_pipeline_context();

    pierre_chat_pipeline::run(
        &ctx,
        turn(&conversation.id, user, tenant, true),
        &web_profile(),
        &PipelineHooks::none(),
    )
    .await
    .expect("the direct turn completes");
    let dm_prompt = prompts
        .lock()
        .unwrap()
        .pop()
        .expect("the direct turn reached the model");
    assert!(
        dm_prompt.contains(PLAN_BLOCK),
        "the DM carries the plan block"
    );
    assert!(
        dm_prompt.contains("Ultra Trail"),
        "with the athlete's season in it"
    );

    pierre_chat_pipeline::run(
        &ctx,
        turn(&conversation.id, user, tenant, false),
        &web_profile(),
        &PipelineHooks::none(),
    )
    .await
    .expect("the room turn completes");
    let room_prompt = prompts
        .lock()
        .unwrap()
        .pop()
        .expect("the room turn reached the model");
    assert!(
        !room_prompt.contains(PLAN_BLOCK) && !room_prompt.contains("Ultra Trail"),
        "a room turn's prompt carries no plan block: the reply goes to every member"
    );
}

#[tokio::test]
async fn a_mention_turn_in_the_endurance_dm_saves_a_week_authored_by_the_mentioned_agent() {
    let model = RecordingModel::new(Some(("save_training_plan", taper_week_payload())));
    let resources = create_test_server_resources_with_chat_provider(Arc::new(model))
        .await
        .unwrap();
    let repos = &resources.common.repos;
    let (user, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = athlete_tenant(&resources, user).await;

    let endurance = repos
        .agents
        .create_system_agent(
            user,
            tenant,
            &CreateSystemAgentRequest {
                title: "Endurance Agent".to_owned(),
                description: None,
                system_prompt: "You lay seasons.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Global,
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    let season = lay_season(&resources, user, tenant, PlanAuthor::agent(&endurance)).await;

    let (author, author_tenant) = store_author(&resources).await;
    let origin = publish_catalogue_agent(
        repos,
        author,
        author_tenant,
        "Taper Builder",
        "You build tapers.",
    )
    .await;
    let taper = install_catalogue_agent(repos, origin, user, tenant).await;
    let handle = taper
        .handle
        .clone()
        .expect("the installed agent has a handle");
    let conversation = repos
        .chat
        .create_conversation(
            &user.to_string(),
            tenant,
            "endurance",
            MODEL,
            Some(&endurance),
            None,
        )
        .await
        .unwrap();
    let text = format!("@{handle} pose ma semaine d'affûtage");
    let mentioned = resolve_agent_mention(repos.agents.as_ref(), &text, user, tenant)
        .await
        .expect("the mention resolves to the installed taper agent");
    assert_eq!(mentioned.agent_id, taper.id.to_string());

    let mut input = turn(&conversation.id, user, tenant, true);
    input.content = text;
    input.mentioned_agent = Some(Box::new(mentioned));
    pierre_chat_pipeline::run(
        &resources.chat_pipeline_context(),
        input,
        &web_profile(),
        &PipelineHooks::none(),
    )
    .await
    .expect("the mention turn completes");

    assert_eq!(
        week_authors(&resources, user, tenant, &season).await,
        vec![Some(taper.id.to_string())],
        "the week lands on the endurance season, authored by the agent the athlete addressed"
    );
}

#[tokio::test]
async fn the_headless_tool_surface_authors_a_save_as_the_turns_agent() {
    let resources = create_test_server_resources().await.unwrap();
    let (user, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = athlete_tenant(&resources, user).await;
    let season = lay_season(
        &resources,
        user,
        tenant,
        PlanAuthor::agent("endurance-agent"),
    )
    .await;

    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let bridge = HostedToolBridge::new(
        true,
        resources.mcp.tool_registry.clone(),
        resources.common.repos.clone(),
        tool_runtime,
    );
    let surface = bridge.turn_surface(
        &user.to_string(),
        tenant,
        "conv-under-test",
        ConversationTurnId(Uuid::new_v4()),
        Some("taper-builder-agent"),
        16,
    );
    let outcome = surface
        .call("save_training_plan", &taper_week_payload())
        .await;
    assert!(!outcome.is_error, "the save goes through: {}", outcome.text);

    assert_eq!(
        week_authors(&resources, user, tenant, &season).await,
        vec![Some("taper-builder-agent".to_owned())]
    );
}
