// ABOUTME: A coach's thread bound to one athlete puts the athlete's coach view in the prompt, never the coach's own context
// ABOUTME: Every gate refusal leaves one line and nothing of the athlete; the plan block and card are the athlete's draft over live
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1070 T4. A turn in a human coach's thread about one athlete runs
//! the coaching gates again, then hands the model the athlete's coach view in
//! place of the coach's own context. Each test drives a real turn through
//! `pierre_chat_pipeline::run` against a model that records the prompt it was
//! sent, and reads that prompt — never the source that builds it.
//!
//! The coach's own season, their agent note and their connected providers are
//! seeded beside the athlete's in every fixture, as are another member's season
//! and the athlete's own agent note, so a block that read the wrong person
//! fails on content rather than on absence.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use crate::helpers::fuel_fixtures;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{Datelike, Days, Duration, NaiveDate, Utc};
use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use helpers::axum_test::AxumTestRequest;
use pierre_chat_pipeline::{
    ChatPipelineContext, InputSource, PipelineHooks, QuotaState, ReplyBlock, SurfaceId,
    SurfaceProfile, SurfaceRequest, TurnEnvelope, TurnInput, TurnOrigin,
};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::errors::AppError;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    AddMessageParams, ConnectionType, ConversationTurnId, DataSource, DelegatedConnection,
    DeviceType, RosterAthlete, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
    Tenant, TenantId,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{InsertAgentNoteParams, InsertCompactionBlockParams};
use pierre_database::RepositoryRegistry;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, ReplyBlockResponse, TurnResponse};
use pierre_memory::MemoryScope;
use pierre_services::plan_card::PlanCardDraft;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

const MODEL: &str = "subject-stage-model";
const ANSWER: &str = "Voici où en est son plan.";
/// The message on which the model reads the plan, so the turn ends with a card.
const READ_ASK: &str = "Montre-moi son plan.";
const PLAIN_ASK: &str = "Comment s'est passée sa semaine ?";

const SUBJECT_NAME: &str = "Mira Subject";
const OTHER_NAME: &str = "Oscar Other";
const COACH_NAME: &str = "Casey Coach";
const SUBJECT_RACE: &str = "Subject Season Race";
const DRAFT_RACE: &str = "Coach Drafted Race";
const COACH_RACE: &str = "Coach Own Race";
const OTHER_RACE: &str = "Other Member Race";
const COACH_NOTE: &str = "coach-own-note: prefers threshold on Tuesdays";
const SUBJECT_NOTE: &str = "subject-own-note: knee niggle after hills";
const COACH_WORKOUT: &str = "coach-drafted tempo 3x10min";

/// Heads the stage's block, granted or refused.
const SUBJECT_HEADING: &str = "## The athlete this thread works on";
const GRANTED_MARKER: &str = "The person writing is Mira Subject's coach, not Mira Subject.";
const REFUSED_MARKER: &str = "The coach can no longer act for this thread's athlete: ";
/// Heads the sender's own connected-provider block, rendered for every
/// subject and never in a coach's athlete thread.
const OWN_PROVIDERS_MARKER: &str = "Connected Fitness Data Providers";
/// Heads the prompt's plan block.
const PLAN_BLOCK: &str = "## Current training plan (persisted)";
const WELLNESS_HEADING: &str = "Mira Subject's wellness, through the coach's";
/// Heads the athlete's fuel-products block.
const FUEL_BLOCK: &str = "## Fuel products the athlete carries";
const SUBJECT_GEL: &str = "SubjectCarriedGel";
const COACH_GEL: &str = "CoachOwnGel";

/// Records every system prompt it is sent, and every call's other messages;
/// on [`READ_ASK`] it reads the plan once, then answers in prose.
struct ThreadModel {
    system_prompts: Mutex<Vec<String>>,
    /// Each call's non-system messages, joined.
    conversations: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl ThreadModel {
    fn new() -> Self {
        Self {
            system_prompts: Mutex::new(Vec::new()),
            conversations: Mutex::new(Vec::new()),
            models: vec![MODEL.to_owned()],
        }
    }

    /// Every non-system message sent on the calls after `from`, joined.
    fn conversation_since(&self, from: usize) -> String {
        self.conversations
            .lock()
            .unwrap()
            .iter()
            .skip(from)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn calls(&self) -> usize {
        self.system_prompts.lock().unwrap().len()
    }

    /// The system prompt of the first call after `from` carrying `marker`.
    fn system_since(&self, from: usize, marker: &str) -> String {
        self.system_prompts
            .lock()
            .unwrap()
            .iter()
            .skip(from)
            .find(|prompt| prompt.contains(marker))
            .cloned()
            .unwrap_or_else(|| panic!("no call after #{from} carried the coaching prompt"))
    }
}

#[async_trait]
impl LlmProvider for ThreadModel {
    fn name(&self) -> &'static str {
        "subject_stage_mock"
    }
    fn display_name(&self) -> &'static str {
        "Subject-stage mock (coach thread pin)"
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
        let system = request
            .messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::System))
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.system_prompts.lock().unwrap().push(system);
        let conversation = request
            .messages
            .iter()
            .filter(|m| !matches!(m.role, MessageRole::System))
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.conversations.lock().unwrap().push(conversation);
        let already_ran = request
            .messages
            .iter()
            .any(|m| matches!(m.role, MessageRole::Tool));
        let asked_to_read = request
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User))
            .is_some_and(|m| m.content.contains(READ_ASK));
        let tool_calls = (asked_to_read && !already_ran).then(|| {
            vec![ToolCallRequest {
                id: "call-read".to_owned(),
                function_name: "get_training_plan".to_owned(),
                arguments: json!({}),
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

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    email: String,
    auth: String,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@subject-stage.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    let token = generate_test_token(res, &user).await;
    Person {
        id,
        tenant,
        email,
        auth: format!("Bearer {token}"),
    }
}

/// The Monday `weeks` weeks after this one's, in UTC.
fn monday(weeks: u64) -> NaiveDate {
    let today = Utc::now().date_naive();
    let this = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    this + Days::new(7 * weeks)
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn week(start: NaiveDate, focus: &str, workout: &str) -> Value {
    let days: Vec<Value> = (0..7)
        .map(|offset| {
            json!({
                "date": ymd(start + Days::new(offset)),
                "sport": "run",
                "workout": workout,
                "duration_min": 40,
                "intensity": "Z2",
            })
        })
        .collect();
    json!({ "week_start": ymd(start), "focus": focus, "days": days })
}

fn outline(race: &str) -> Value {
    json!({
        "goal_race": { "name": race, "date": "2027-05-16", "discipline": "trail", "priority": "A" },
        "strategy": format!("build toward {race}"),
    })
}

fn web_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

/// A group its owner owns and the coach coaches, the subject and another
/// athlete consenting members; the coach's thread about the subject open.
struct Fixture {
    res: Arc<ServerContext>,
    ctx: ChatPipelineContext,
    model: Arc<ThreadModel>,
    coach: Person,
    subject: Person,
    other: Person,
    group: CoachingGroup,
    thread: String,
}

impl Fixture {
    async fn new() -> Self {
        let model = Arc::new(ThreadModel::new());
        let res = create_test_server_resources_with_chat_provider(model.clone())
            .await
            .unwrap();
        let owner = person(&res, "owner", "Olive Owner").await;
        let coach = person(&res, "coach", COACH_NAME).await;
        let subject = person(&res, "subject", SUBJECT_NAME).await;
        let other = person(&res, "other", OTHER_NAME).await;
        let repos = &res.common.repos;
        let agent_id = create_group_agent(repos, owner.id, owner.tenant)
            .await
            .unwrap();
        let now = Utc::now();
        let group = repos
            .groups
            .create_group(
                owner.tenant,
                &CoachingGroup {
                    id: Uuid::new_v4(),
                    tenant_id: owner.tenant.to_string(),
                    name: "Tempo Squad".to_owned(),
                    description: None,
                    agent_id,
                    owner_id: owner.id,
                    coach_user_id: None,
                    peer_data_sharing: false,
                    respond_mode: GroupRespondMode::default(),
                    digest_mode: GroupDigestMode::Off,
                    max_members: 20,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        assert!(repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
            .await
            .unwrap());
        for member in [&subject, &other] {
            join(repos, &group, member).await;
        }
        let resp = AxumTestRequest::post(&format!("/api/chat/athletes/{}/thread", subject.id))
            .header("authorization", &coach.auth)
            .json(&json!({ "group_id": group.id }))
            .send(ChatRoutes::routes(Arc::clone(&res)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let thread = resp.json::<Value>()["id"].as_str().unwrap().to_owned();
        let ctx = res.chat_pipeline_context();

        let fx = Self {
            res,
            ctx,
            model,
            coach,
            subject,
            other,
            group,
            thread,
        };
        fx.seed_everyone_elses_context().await;
        fx
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// What must never reach the athlete's workspace: the coach's own season,
    /// provider and agent note, the athlete's own agent note, and another
    /// member's season.
    async fn seed_everyone_elses_context(&self) {
        let repos = self.repos();
        repos
            .provider_connections
            .register_connection(
                self.coach.id,
                self.coach.tenant,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();
        self.saves(&self.coach, None, json!({ "outline": outline(COACH_RACE) }))
            .await;
        self.saves(&self.other, None, json!({ "outline": outline(OTHER_RACE) }))
            .await;
        for (who, content) in [(&self.coach, COACH_NOTE), (&self.subject, SUBJECT_NOTE)] {
            repos
                .memory
                .insert_agent_note(&InsertAgentNoteParams {
                    tenant_id: who.tenant,
                    user_id: &who.id.to_string(),
                    agent_id: &self.group.agent_id,
                    conversation_id: None,
                    scope: MemoryScope::User,
                    content,
                    transport_policy: TransportPolicy::AnyTransport,
                })
                .await
                .unwrap();
        }
    }

    /// `who` saves through the plan tool, inside `conversation` when there is
    /// one: the coach in the thread drafts the subject's plan.
    async fn saves(&self, who: &Person, conversation: Option<&str>, args: Value) -> Value {
        let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&self.res);
        let ctx = ToolContext::new()
            .with_user(who.id.to_string())
            .with_tenant(who.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let saved = CONVERSATION_ID
            .scope(conversation.map(ToOwned::to_owned), async {
                SaveTrainingPlanTool
                    .execute(&runtime, &ctx, args)
                    .await
                    .structured_content
                    .expect("tool result carries structured content")
            })
            .await;
        assert!(saved.get("error").is_none(), "{saved}");
        saved
    }

    async fn subject_season(&self) {
        self.saves(
            &self.subject,
            None,
            json!({
                "outline": outline(SUBJECT_RACE),
                "weeks": [week(monday(0), "subject week zero", "subject easy run")],
            }),
        )
        .await;
    }

    async fn coach_drafts(&self) {
        let saved = self
            .saves(
                &self.coach,
                Some(&self.thread),
                json!({
                    "outline": outline(DRAFT_RACE),
                    "weeks": [week(monday(0), "coach week zero", COACH_WORKOUT)],
                }),
            )
            .await;
        assert_eq!(saved["draft"], true, "the coach's save is a draft: {saved}");
    }

    /// `who`'s save through the plan tool inside `conversation`, which must be
    /// refused: what the refusal says.
    async fn save_refused(&self, who: &Person, conversation: &str, args: Value) -> String {
        let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&self.res);
        let ctx = ToolContext::new()
            .with_user(who.id.to_string())
            .with_tenant(who.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let refused = CONVERSATION_ID
            .scope(Some(conversation.to_owned()), async {
                SaveTrainingPlanTool
                    .execute(&runtime, &ctx, args)
                    .await
                    .structured_content
                    .expect("tool result carries structured content")
            })
            .await;
        refused
            .get("error")
            .unwrap_or_else(|| panic!("the save must be refused: {refused}"))
            .to_string()
    }

    /// One coach turn in `conversation`: the coaching call's system prompt,
    /// and the envelope the turn produced.
    async fn coach_turn(&self, conversation: &str, content: &str) -> (String, TurnEnvelope) {
        let before = self.model.calls();
        let envelope = pierre_chat_pipeline::run(
            &self.ctx,
            TurnInput {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                conversation_id: conversation.to_owned(),
                user_id: self.coach.id.to_string(),
                conversation_tenant_id: self.coach.tenant,
                tool_tenant_id: self.coach.tenant,
                is_direct_message: true,
                content: content.to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                quote: None,
                quota: QuotaState::Ok,
                mentioned_agent: None,
                steps_after: None,
            },
            &web_profile(),
            &PipelineHooks::none(),
        )
        .await
        .expect("the turn completes");
        let prompt = self
            .model
            .system_since(before, &self.ctx.tool_discipline_prompt);
        (prompt, envelope)
    }

    async fn thread_turn(&self, content: &str) -> (String, TurnEnvelope) {
        self.coach_turn(&self.thread, content).await
    }
}

/// Nothing of the athlete, of the coach's own context or of another member is
/// in `prompt` — only the refusal line under the heading.
fn assert_refused_with(prompt: &str, why: &str) {
    let at = prompt.find(SUBJECT_HEADING).expect("the stage's heading");
    let block = &prompt[at..];
    let line = block
        .lines()
        .find(|line| line.starts_with(REFUSED_MARKER))
        .unwrap_or_else(|| panic!("the refusal line: {block}"));
    assert!(line.contains(why), "{line}");
    for absent in [
        GRANTED_MARKER,
        PLAN_BLOCK,
        SUBJECT_RACE,
        SUBJECT_NOTE,
        COACH_RACE,
        COACH_NOTE,
        OWN_PROVIDERS_MARKER,
        OTHER_NAME,
        OTHER_RACE,
    ] {
        assert!(
            !prompt.contains(absent),
            "{absent:?} reached a refused turn"
        );
    }
}

async fn join(repos: &RepositoryRegistry, group: &CoachingGroup, who: &Person) {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: who.id,
            tenant_id: who.tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// The `workout_plan` block of the newest reply carrying one, as a client
/// reloading `conversation` reads it: `GET …/messages`, whose stored blocks
/// resolve through photograveur into `scene_blocks`.
async fn history_plan_block(fx: &Fixture, conversation: &str) -> Value {
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{conversation}/messages"))
        .header("authorization", &fx.coach.auth)
        .send(ChatRoutes::routes(Arc::clone(&fx.res)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let page: Value = resp.json();
    page["messages"]
        .as_array()
        .expect("the thread's messages")
        .iter()
        .rev()
        .filter_map(|message| message["scene_blocks"].as_str())
        .flat_map(|scenes| {
            serde_json::from_str::<Vec<Value>>(scenes).expect("scene_blocks is a JSON array")
        })
        .find(|block| block["kind"] == "workout_plan")
        .unwrap_or_else(|| panic!("a plan card in the history read: {page}"))
}

/// The `workout_plan` block of a turn: the card's JSON and its coach fields.
fn plan_card(envelope: &TurnEnvelope) -> (Value, Option<PlanCardDraft>) {
    envelope
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::WorkoutPlan { plan, coach_draft } => Some((
                serde_json::from_str(plan).expect("the card is JSON"),
                coach_draft.clone(),
            )),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a card on the reply: {:?}", envelope.assistant.blocks))
}

// ════════════════════════════════════════════════════════════════════════
// Access
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn with_access_the_athletes_view_replaces_the_coachs_own_context() {
    let fx = Fixture::new().await;
    fx.subject_season().await;

    let (prompt, _) = fx.thread_turn(PLAIN_ASK).await;

    assert!(prompt.contains(SUBJECT_HEADING), "{prompt}");
    assert!(prompt.contains(GRANTED_MARKER));
    assert!(prompt.contains("opened from the group 'Tempo Squad'"));
    assert!(prompt.contains("`publish_training_plan`"));
    assert!(prompt.contains("read the coach's own account, never Mira Subject's"));
    assert!(prompt.contains("Nothing said here is saved to the coach's own account"));
    // The athlete's plan, as their live season with no draft open.
    assert!(prompt.contains(&format!(
        "Live season: aimed at {SUBJECT_RACE} on 2027-05-16, 1 week(s) stored."
    )));
    assert!(prompt.contains("The coach holds no draft of it"));
    let plan_block = &prompt[prompt.find(PLAN_BLOCK).expect("the plan block")..];
    assert!(plan_block.contains(SUBJECT_RACE), "{plan_block}");
    assert!(
        prompt.find(SUBJECT_HEADING) < prompt.find(PLAN_BLOCK),
        "the athlete block introduces the plan block below it"
    );
    // A subject with no training source is said to have none.
    if cfg!(feature = "tools-groups") {
        assert!(prompt.contains("Mira Subject connects no training source yet"));
    }

    // The coach's own context, the athlete's own memory and another member's
    // data are nowhere in the athlete's workspace.
    for absent in [
        OWN_PROVIDERS_MARKER,
        COACH_RACE,
        COACH_NOTE,
        SUBJECT_NOTE,
        OTHER_NAME,
        OTHER_RACE,
        WELLNESS_HEADING,
    ] {
        assert!(
            !prompt.contains(absent),
            "{absent:?} reached the athlete thread"
        );
    }
}

/// The roster line is the one `get_roster_overview` lists for the athlete,
/// form included, read as a share of their own fitness.
#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn the_athletes_roster_line_is_the_roster_overviews() {
    let fx = Fixture::new().await;
    fx.repos()
        .provider_connections
        .register_connection(
            fx.subject.id,
            fx.subject.tenant,
            "synthetic",
            &ConnectionType::Synthetic,
            None,
        )
        .await
        .unwrap();

    let (prompt, _) = fx.thread_turn(PLAIN_ASK).await;

    let at = prompt
        .find("### Mira Subject's training, as the coach's roster shows it")
        .expect("the roster section");
    let json_line = prompt[at..].lines().nth(2).expect("the roster line");
    let training: Value = serde_json::from_str(json_line).expect("the roster line is JSON");
    for field in ["sessions_this_week", "seconds_this_week", "km_this_week"] {
        assert!(training.get(field).is_some(), "{field}: {training}");
    }
    assert!(prompt.contains("`form` is a share of Mira Subject's own fitness"));
    assert!(!prompt.contains(OWN_PROVIDERS_MARKER));
}

// ════════════════════════════════════════════════════════════════════════
// Every gate, every turn
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a_revoked_consent_leaves_one_line_and_nothing_of_the_athlete() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    let (granted, _) = fx.thread_turn(PLAIN_ASK).await;
    assert!(
        granted.contains(GRANTED_MARKER),
        "premise: access on the first turn"
    );

    assert!(fx
        .repos()
        .groups
        .update_coach_sharing_consent(&fx.group.id.to_string(), fx.subject.id, false)
        .await
        .unwrap());
    let (prompt, envelope) = fx.thread_turn(READ_ASK).await;

    assert_refused_with(
        &prompt,
        "Mira Subject has stopped sharing their data with the coach of 'Tempo Squad'",
    );
    assert!(
        !envelope
            .assistant
            .blocks
            .iter()
            .any(|block| matches!(block, ReplyBlock::WorkoutPlan { .. })),
        "a refused turn carries no card"
    );
}

/// carnet#1168 D27: once the athlete stops sharing, the thread stays the
/// coach's record, but none of it reaches the model again — not the tool
/// result that read her plan, not the coach's earlier questions: the turn
/// reads the coach's new message alone.
#[tokio::test]
async fn a_revoked_consent_replays_none_of_the_threads_history() {
    const LATER_ASK: &str = "Et maintenant, que faire ?";
    let fx = Fixture::new().await;
    fx.subject_season().await;
    fx.thread_turn(READ_ASK).await;
    let granted_from = fx.model.calls();
    fx.thread_turn(PLAIN_ASK).await;
    let granted = fx.model.conversation_since(granted_from);
    assert!(
        granted.contains(SUBJECT_RACE) && granted.contains(READ_ASK),
        "premise: a granted turn replays the plan the thread read: {granted}"
    );

    assert!(fx
        .repos()
        .groups
        .update_coach_sharing_consent(&fx.group.id.to_string(), fx.subject.id, false)
        .await
        .unwrap());
    let refused_from = fx.model.calls();
    let (prompt, _) = fx.thread_turn(LATER_ASK).await;

    assert!(prompt.contains(REFUSED_MARKER), "premise: the gates refuse");
    let refused = fx.model.conversation_since(refused_from);
    assert!(refused.contains(LATER_ASK), "{refused}");
    for absent in [SUBJECT_RACE, READ_ASK, PLAIN_ASK, ANSWER] {
        assert!(
            !refused.contains(absent),
            "{absent:?} from before the withdrawal reached the model: {refused}"
        );
    }
}

/// carnet#1168 with carnet#1174: a summary of the thread's earlier turns is
/// history too, so once the athlete stops sharing it reaches the model no more
/// than the rows it covers. It is seeded naming no member, as a summary
/// written before rows named the member they read.
#[tokio::test]
async fn a_revoked_consent_replays_no_summary_of_the_thread() {
    const SUMMARY: &str = "Earlier: Mira's long runs held 5:10/km.";
    const LATER_ASK: &str = "Et maintenant, que faire ?";
    const LAST_ASK: &str = "Et ensuite ?";
    let fx = Fixture::new().await;
    fx.subject_season().await;
    fx.thread_turn(PLAIN_ASK).await;
    let anchor = fx
        .repos()
        .chat
        .get_messages(&fx.thread, &fx.coach.id.to_string(), fx.coach.tenant)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.role == "user" && row.content == PLAIN_ASK)
        .expect("the coach's question is stored")
        .id;
    fx.repos()
        .memory
        .insert_compaction_block(&InsertCompactionBlockParams {
            tenant_id: fx.coach.tenant,
            conversation_id: &fx.thread,
            summary: SUMMARY,
            summary_tokens: 10,
            original_tokens: 40,
            first_message_id: &anchor,
            last_message_id: &anchor,
            transport_policy: TransportPolicy::AnyTransport,
            about_user_id: None,
        })
        .await
        .unwrap();
    let granted_from = fx.model.calls();
    fx.thread_turn(LATER_ASK).await;
    let granted = fx.model.conversation_since(granted_from);
    assert!(
        granted.contains(SUMMARY),
        "premise: a granted turn replays the thread's summary: {granted}"
    );

    assert!(
        fx.repos()
            .groups
            .update_coach_sharing_consent(&fx.group.id.to_string(), fx.subject.id, false)
            .await
            .unwrap(),
        "premise: the athlete's membership takes the revocation"
    );
    let refused_from = fx.model.calls();
    let (prompt, _) = fx.thread_turn(LAST_ASK).await;

    assert!(prompt.contains(REFUSED_MARKER), "premise: the gates refuse");
    let refused = fx.model.conversation_since(refused_from);
    assert!(
        refused.contains(LAST_ASK),
        "the coach's new message is the turn: {refused}"
    );
    assert!(
        !refused.contains(SUMMARY),
        "a summary from before the withdrawal reached the model: {refused}"
    );
}

#[tokio::test]
async fn a_member_who_left_is_refused_without_being_named() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    assert!(fx
        .repos()
        .groups
        .remove_member(&fx.group.id.to_string(), fx.subject.id)
        .await
        .unwrap());

    let (prompt, _) = fx.thread_turn(PLAIN_ASK).await;

    assert_refused_with(&prompt, "is no longer a member of the group");
    let block = &prompt[prompt.find(SUBJECT_HEADING).unwrap()..];
    let line = block
        .lines()
        .find(|l| l.starts_with(REFUSED_MARKER))
        .unwrap();
    assert!(!line.contains(SUBJECT_NAME), "{line}");
}

#[tokio::test]
async fn a_detached_coach_is_refused() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    let owner_tenant = TenantId::parse_str(&fx.group.tenant_id).unwrap();
    assert!(fx
        .repos()
        .groups
        .set_group_coach_user(&fx.group.id.to_string(), None, owner_tenant)
        .await
        .unwrap());

    let (prompt, _) = fx.thread_turn(PLAIN_ASK).await;

    assert_refused_with(&prompt, "no longer coaches the group");
}

#[tokio::test]
async fn an_athlete_in_two_tenants_is_refused() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    let now = Utc::now();
    let second = TenantId::generate();
    fx.repos()
        .tenants
        .create(&Tenant {
            id: second,
            name: "Second home".to_owned(),
            slug: format!("second-{second}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: fx.subject.id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    let (prompt, _) = fx.thread_turn(PLAIN_ASK).await;

    assert_refused_with(&prompt, "Mira Subject belongs to several tenants");
}

// ════════════════════════════════════════════════════════════════════════
// The plan: the coach's draft over the live season
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn the_plan_block_shows_the_coachs_draft_over_the_live_season() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    fx.coach_drafts().await;

    let (prompt, _) = fx.thread_turn(PLAIN_ASK).await;

    assert!(prompt.contains(&format!(
        "Live season: aimed at {SUBJECT_RACE} on 2027-05-16, 1 week(s) stored."
    )));
    assert!(prompt.contains(&format!(
        "The coach's draft (unpublished): aimed at {DRAFT_RACE} on 2027-05-16;"
    )));
    assert!(prompt.contains(&format!("weeks drafted: {}.", ymd(monday(0)))));
    assert!(prompt.contains("Publishing it now overwrites nothing of Mira Subject's."));
    let plan_block = &prompt[prompt.find(PLAN_BLOCK).expect("the plan block")..];
    assert!(plan_block.contains(DRAFT_RACE), "{plan_block}");
    assert!(
        plan_block.contains(COACH_WORKOUT),
        "the drafted week in place of the live one: {plan_block}"
    );
    assert!(!prompt.contains(COACH_RACE));
}

/// A week whose first day names `products`.
fn week_naming(start: NaiveDate, products: &[Uuid]) -> Value {
    let mut drafted = week(start, "coach week zero", COACH_WORKOUT);
    drafted["days"][0]["fuel_product_ids"] = json!(products);
    drafted
}

/// The athlete's fuel products reach the coach's thread through prompt
/// assembly — the athlete's, never the coach's own — and a draft day naming a
/// product the athlete does not carry is refused with the remedy the coach
/// has: the fuel tools would act on the coach's own list, so the refusal
/// names the athlete's list the thread shows instead.
#[tokio::test]
async fn the_athletes_fuel_products_reach_the_thread_and_bound_the_draft() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    let repos = fx.repos();
    let subject_gel = fuel_fixtures::approved_gel(repos, fx.other.id, SUBJECT_GEL).await;
    let coach_gel = fuel_fixtures::approved_gel(repos, fx.other.id, COACH_GEL).await;
    for (who, gel) in [(&fx.subject, subject_gel), (&fx.coach, coach_gel)] {
        repos
            .athlete_fuel_items
            .add_fuel_product_item(who.tenant, who.id, gel, None)
            .await
            .unwrap();
    }

    let (prompt, _) = fx.thread_turn(PLAIN_ASK).await;
    let fuel = &prompt[prompt.find(FUEL_BLOCK).expect("the athlete's fuel block")..];
    assert!(
        fuel.contains(SUBJECT_GEL) && fuel.contains(&format!("product_id {subject_gel}")),
        "the athlete's carried product: {fuel}"
    );
    assert!(
        !prompt.contains(COACH_GEL),
        "the coach's own list never reaches the athlete's thread"
    );

    let refusal = fx
        .save_refused(
            &fx.coach,
            &fx.thread,
            json!({ "weeks": [week_naming(monday(0), &[coach_gel])] }),
        )
        .await;
    assert!(
        refusal.contains("not on this athlete's fuel list")
            && refusal.contains("the athlete's fuel products shown in this thread"),
        "{refusal}"
    );
    assert!(
        !refusal.contains("add_fuel_item") && !refusal.contains("list_fuel_items"),
        "the fuel tools act on the coach's own list: {refusal}"
    );

    let saved = fx
        .saves(
            &fx.coach,
            Some(&fx.thread),
            json!({ "weeks": [week_naming(monday(0), &[subject_gel])] }),
        )
        .await;
    assert_eq!(saved["draft"], true, "{saved}");
}

#[tokio::test]
async fn a_week_the_athlete_changed_under_the_draft_is_named() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    fx.coach_drafts().await;
    // The athlete's own agent changes the week the coach drafted.
    fx.saves(
        &fx.subject,
        None,
        json!({ "weeks": [week(monday(0), "athlete again", "athlete again run")] }),
    )
    .await;

    let (prompt, _) = fx.thread_turn(PLAIN_ASK).await;

    assert!(
        prompt.contains(&format!(
            "Publishing it now would replace Mira Subject's own changes to the week(s) of {}",
            ymd(monday(0))
        )),
        "{prompt}"
    );
    assert!(!prompt.contains("overwrites nothing"));
}

/// The end-of-turn card is the athlete's draft over live, carrying the athlete
/// and the draft's standing from the one serializer, on the wire and in the
/// stored row.
#[tokio::test]
async fn the_card_carries_subject_and_draft_through_the_wire_and_the_store() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    fx.coach_drafts().await;

    let (_, envelope) = fx.thread_turn(READ_ASK).await;

    let (card, coach_draft) = plan_card(&envelope);
    assert_eq!(card["goal_race"]["name"], DRAFT_RACE, "{card}");
    assert!(card.to_string().contains(COACH_WORKOUT), "{card}");
    let coach_draft = coach_draft.expect("the card names the athlete and the draft");
    assert_eq!(coach_draft.subject.user_id, fx.subject.id);
    assert_eq!(coach_draft.subject.display_name, SUBJECT_NAME);
    let state = coach_draft.draft.expect("a draft is open");

    // The subject-plan read's standing is the card's: one serializer.
    let resp = AxumTestRequest::get(&format!(
        "/api/chat/conversations/{}/subject-plan",
        fx.thread
    ))
    .header("authorization", &fx.coach.auth)
    .send(ChatRoutes::routes(Arc::clone(&fx.res)))
    .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let read: Value = resp.json();
    assert_eq!(read["draft_state"], serde_json::to_value(&state).unwrap());

    // The wire block: `subject` and `draft` beside `plan`.
    let wire = serde_json::to_value(TurnResponse::from_envelope(envelope, 0)).unwrap();
    let block = wire["assistant"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["type"] == "workout_plan")
        .expect("the card on the wire")
        .clone();
    assert_eq!(block["plan"]["goal_race"]["name"], DRAFT_RACE);
    assert_eq!(block["subject"]["user_id"], fx.subject.id.to_string());
    assert_eq!(block["subject"]["display_name"], SUBJECT_NAME);
    assert_eq!(block["draft"], read["draft_state"], "{block}");
    // A client of the wire shape reads the same fields back.
    let decoded: TurnResponse = serde_json::from_value(wire.clone()).unwrap();
    let decoded_draft = decoded
        .assistant
        .blocks
        .into_iter()
        .find_map(|block| match block {
            ReplyBlockResponse::WorkoutPlan { coach_draft, .. } => Some(coach_draft),
            _ => None,
        })
        .expect("the decoded card");
    assert_eq!(
        decoded_draft.and_then(|fields| fields.draft),
        Some(state.clone())
    );

    // The stored row a reload reads carries the same fields beside `plan`.
    let stored = fx
        .repos()
        .chat
        .get_messages(&fx.thread, &fx.coach.id.to_string(), fx.coach.tenant)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|row| row.role == "assistant")
        .expect("the reply row");
    let entries: Vec<Value> =
        serde_json::from_str(stored.content_blocks.as_deref().expect("stored blocks")).unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry["type"] == "workout_plan")
        .expect("the stored card");
    assert_eq!(entry["subject"], block["subject"]);
    assert_eq!(entry["draft"], block["draft"]);
    assert_eq!(entry["plan"]["goal_race"]["name"], DRAFT_RACE);

    // A reload reads them back beside `plan`, through the history read's
    // photograveur resolution, as the live turn delivered them.
    let reloaded = history_plan_block(&fx, &fx.thread).await;
    assert_eq!(
        reloaded["plan"]["goal_race"]["name"], DRAFT_RACE,
        "{reloaded}"
    );
    assert_eq!(reloaded["subject"], block["subject"], "{reloaded}");
    assert_eq!(reloaded["draft"], read["draft_state"], "{reloaded}");
}

/// A card stored before cards carried `subject` and `draft` reads back
/// through the history read exactly as it was: its plan and its tool, and no
/// coach field it never had.
#[tokio::test]
async fn an_old_stored_card_reads_back_unchanged() {
    let fx = Fixture::new().await;
    let plan = json!({ "goal_race": { "name": SUBJECT_RACE, "date": "2027-05-16" }, "weeks": [] });
    let stored = json!([{
        "type": "workout_plan",
        "source_tool": "save_training_plan",
        "plan": plan,
    }])
    .to_string();
    fx.repos()
        .chat
        .add_message(&AddMessageParams {
            tenant_id: fx.coach.tenant,
            conversation_id: &fx.thread,
            user_id: &fx.coach.id.to_string(),
            role: "assistant",
            content: ANSWER,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: Some(&stored),
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
        })
        .await
        .unwrap();

    let reloaded = history_plan_block(&fx, &fx.thread).await;

    assert_eq!(
        reloaded,
        json!({ "kind": "workout_plan", "source_tool": "save_training_plan", "plan": plan })
    );
}

/// With no draft open the card is the athlete's live season, naming them with
/// `draft: null` so no Publish or Discard is offered.
#[tokio::test]
async fn with_no_draft_the_card_is_the_live_season_and_draft_is_null() {
    let fx = Fixture::new().await;
    fx.subject_season().await;

    let (_, envelope) = fx.thread_turn(READ_ASK).await;

    let (card, coach_draft) = plan_card(&envelope);
    assert_eq!(card["goal_race"]["name"], SUBJECT_RACE, "{card}");
    let coach_draft = coach_draft.expect("the card names the athlete");
    assert_eq!(coach_draft.subject.user_id, fx.subject.id);
    assert_eq!(coach_draft.draft, None);
    let wire = serde_json::to_value(TurnResponse::from_envelope(envelope, 0)).unwrap();
    let block = wire["assistant"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["type"] == "workout_plan")
        .expect("the card on the wire")
        .clone();
    assert_eq!(block["draft"], Value::Null);
    assert!(block.as_object().unwrap().contains_key("draft"), "{block}");
}

// ════════════════════════════════════════════════════════════════════════
// Wellness, only under the athlete's `wellness_read` to their coach
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn wellness_reaches_the_coach_only_under_the_links_wellness_read() {
    let fx = Fixture::new().await;
    let repos = fx.repos();
    let now = Utc::now();
    let proposed = repos
        .delegated_connections
        .propose(&DelegatedConnection::propose(
            INTERVALS_ICU.to_owned(),
            fx.group.id,
            fx.coach.id,
            fx.coach.tenant,
            fx.subject.id,
            RosterAthlete::new("i201", None, Some(fx.subject.email.clone())),
        ))
        .await
        .unwrap()
        .unwrap();
    let link = repos
        .delegated_connections
        .confirm(proposed.id, fx.subject.id, fx.subject.tenant, now)
        .await
        .unwrap()
        .unwrap();
    let night_ended = seed_link_wellness(repos, &fx.subject).await;

    let (without, _) = fx.thread_turn(PLAIN_ASK).await;
    assert!(
        !without.contains(WELLNESS_HEADING) && !without.contains("HRV"),
        "no wellness without the athlete's wellness_read"
    );

    repos
        .groups
        .set_coach_consents(
            &link.group_id.to_string(),
            fx.subject.id,
            None,
            Some(true),
            now,
        )
        .await
        .unwrap()
        .expect("the member grants their coach wellness_read");
    let (with, _) = fx.thread_turn(PLAIN_ASK).await;

    let at = with.find(WELLNESS_HEADING).expect("the wellness section");
    let section = &with[at..];
    let today = Utc::now().date_naive();
    for reading in [
        format!("HRV 68 ms ({today})"),
        format!("resting heart rate 48 bpm ({today})"),
        format!("sleep 7h00 (night ending {night_ended})"),
        format!("weight 68.2 kg ({today})"),
    ] {
        assert!(section.contains(&reading), "{reading}: {section}");
    }
    assert!(!with.contains(COACH_NOTE) && !with.contains(SUBJECT_NOTE));
}

/// The wellness a link files: under the member's tenant, as the platform's.
/// Returns the day the seeded night ended.
async fn seed_link_wellness(repos: &RepositoryRegistry, member: &Person) -> NaiveDate {
    let source = repos
        .data_sources
        .upsert_data_source(
            &member.tenant,
            &DataSource {
                id: String::new(),
                user_id: member.id.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let woke = Utc::now() - Duration::minutes(5);
    repos
        .sleep
        .upsert_sleep_session(
            &member.tenant,
            &StoredSleepSession {
                id: format!("icu-sleep-{}", Uuid::new_v4()),
                user_id: member.id.to_string(),
                data_source_id: source.clone(),
                is_nap: false,
                start_datetime: woke - Duration::hours(7),
                end_datetime: woke,
                total_sleep_seconds: Some(25_200),
                deep_sleep_seconds: None,
                light_sleep_seconds: None,
                rem_sleep_seconds: None,
                awake_seconds: None,
                sleep_efficiency: None,
                avg_heart_rate: None,
                min_heart_rate: None,
                avg_hrv: Some(68.0),
                sleep_score: None,
                stages: Vec::new(),
                source_name: INTERVALS_ICU.to_owned(),
            },
        )
        .await
        .unwrap();
    let today = Utc::now().date_naive();
    repos
        .recovery
        .upsert_recovery_metrics(
            &member.tenant,
            &StoredRecoveryMetrics {
                id: format!("icu-day-{}", Uuid::new_v4()),
                user_id: member.id.to_string(),
                data_source_id: source.clone(),
                date: today,
                recovery_score: None,
                readiness_score: None,
                hrv_ms: Some(68.0),
                hrv_rmssd: Some(68.0),
                resting_heart_rate: Some(48),
                stress_score: None,
                body_battery: None,
                spo2: None,
                respiratory_rate: None,
                skin_temp_deviation: None,
                daily_strain: None,
                athlete_note: None,
                source_name: INTERVALS_ICU.to_owned(),
                recorded_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    repos
        .health_snapshots
        .upsert_health_snapshot(
            &member.tenant,
            &StoredHealthMetrics {
                id: String::new(),
                user_id: member.id.to_string(),
                data_source_id: source,
                date: today,
                weight_kg: Some(68.2),
                body_fat_pct: None,
                muscle_mass_kg: None,
                bmi: None,
                bone_mass_kg: None,
                water_pct: None,
                systolic_bp: None,
                diastolic_bp: None,
                blood_glucose: None,
                vo2_max: None,
                source_name: INTERVALS_ICU.to_owned(),
                recorded_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    woke.date_naive()
}

// ════════════════════════════════════════════════════════════════════════
// No regression outside an athlete thread
// ════════════════════════════════════════════════════════════════════════

/// The same coach in a plain thread on the same agent is the turn's subject
/// as before: their own providers, note and season reach the prompt, nothing
/// of the athlete does, and their card carries no coach fields.
#[tokio::test]
async fn a_plain_thread_is_the_coachs_own_as_before() {
    let fx = Fixture::new().await;
    fx.subject_season().await;
    let plain = fx
        .repos()
        .chat
        .create_conversation(
            &fx.coach.id.to_string(),
            fx.coach.tenant,
            "own thread",
            MODEL,
            Some(&fx.group.agent_id),
            None,
        )
        .await
        .unwrap()
        .id;

    let (prompt, envelope) = fx.coach_turn(&plain, READ_ASK).await;

    assert!(!prompt.contains(SUBJECT_HEADING), "{prompt}");
    for present in [OWN_PROVIDERS_MARKER, COACH_NOTE, COACH_RACE] {
        assert!(
            prompt.contains(present),
            "{present:?} is the coach's own context"
        );
    }
    for absent in [SUBJECT_NAME, SUBJECT_RACE, SUBJECT_NOTE, OTHER_RACE] {
        assert!(
            !prompt.contains(absent),
            "{absent:?} reached the coach's own thread"
        );
    }
    let (card, coach_draft) = plan_card(&envelope);
    assert_eq!(card["goal_race"]["name"], COACH_RACE);
    assert_eq!(coach_draft, None);
    let wire = serde_json::to_value(TurnResponse::from_envelope(envelope, 0)).unwrap();
    let block = wire["assistant"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["type"] == "workout_plan")
        .expect("the card on the wire")
        .clone();
    let mut keys: Vec<&str> = block
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["plan", "type"], "{block}");
    let decoded: TurnResponse = serde_json::from_value(wire).unwrap();
    assert!(decoded.assistant.blocks.iter().any(|block| matches!(
        block,
        ReplyBlockResponse::WorkoutPlan {
            coach_draft: None,
            ..
        }
    )));
}
