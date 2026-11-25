// ABOUTME: Advice playbooks and athlete commitments drawn from WHOOP data name its AI consent end to end
// ABOUTME: A withdrawal keeps both out of every prompt and tool answer; the athlete's export still holds them (carnet#1157)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A playbook the agent learned from a WHOOP-derived reply, and a commitment
//! the agent recorded in a turn that served WHOOP data, are WHOOP-derived
//! content too (carnet#1157 gap 1). While the athlete consents to WHOOP's AI
//! use, both reach the model: the prompt's playbook and commitment blocks,
//! and the `list_coaching_playbooks` answer. Once the consent is withdrawn,
//! none of it does, while the athlete's own data export still returns both.
//!
//! Turns run through `pierre_chat_pipeline::execute`, the entry every surface
//! calls, against a model that records what every call carried.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dravr_tronc::dates::{Elapsed, Timestamp};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use pierre_chat_pipeline::{
    ChatPipelineContext, CommandPersistence, InputSource, PipelineHooks, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_core::constants::oauth::providers::{sensitive_display_name, SENSITIVE_T3};
use pierre_core::constants::oauth_providers::{NoticeKind, GARMIN, WHOOP};
use pierre_core::errors::AppError;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::{RecordedOutcome, USER_EXPORT_TABLES};
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::commitments::{Commitment, CommitmentStatus};
use pierre_memory::playbooks::{
    Band, Intervention, InterventionKind, LabelSource, OutcomeLabel, OutcomeMetric, TriggerKind,
    TriggerPattern,
};
use pierre_services::provider_notice::{grant_consent, withdraw_consent};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};

const MODEL: &str = "derived-consent-playbooks-model";

/// The heading the prompt's playbook block opens with.
const PLAYBOOK_BLOCK: &str = "What works for this athlete";
/// The words of the WHOOP-derived commitment.
const COMMITMENT_STATEMENT: &str = "trois sorties faciles après une récupération WHOOP basse";
/// A question that reads nothing.
const PLAIN_ASK: &str = "Que fais-je demain ?";
/// A question the agent answers with what it learned works.
const PLAYBOOKS_ASK: &str = "Qu'as-tu appris qui marche pour moi ?";
/// What the agent answers: too short to be taken for advice.
const REPLY: &str = "D'accord.";
/// How the tool loop hands a tool's answer back to the model.
const TOOL_RESULT_OPENING: &str = "[Tool Result for ";

// ════════════════════════════════════════════════════════════════════════
// The model
// ════════════════════════════════════════════════════════════════════════

/// What one model call carried.
#[derive(Debug, Clone)]
struct Call {
    /// Every system message, joined.
    system: String,
    /// Every tool answer, joined.
    tool_results: String,
    /// The last user message that is not a tool's answer.
    asked: String,
}

/// Records every coaching call. Asked [`PLAYBOOKS_ASK`], it lists the
/// athlete's playbooks once, then answers [`REPLY`]. A background learning
/// pass (the fact extractor, the advice capture) finds nothing.
struct RecordingModel {
    calls: Mutex<Vec<Call>>,
    learning_prompts: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl RecordingModel {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            learning_prompts: Mutex::new(Vec::new()),
            models: vec![MODEL.to_owned()],
        }
    }

    fn know(&self, ctx: &ChatPipelineContext) {
        *self.learning_prompts.lock().unwrap() = vec![
            ctx.memory_extraction_prompt.clone(),
            ctx.prompt_registry
                .advice_extraction_prompt()
                .trim()
                .to_owned(),
        ];
    }

    fn is_learning_pass(&self, call: &Call) -> bool {
        self.learning_prompts
            .lock()
            .unwrap()
            .iter()
            .any(|prompt| !prompt.is_empty() && call.system.starts_with(prompt.as_str()))
    }

    fn calls_so_far(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// The calls since `from` that answered the athlete's `ask`.
    fn answering(&self, from: usize, ask: &str) -> Vec<Call> {
        self.calls.lock().unwrap()[from..]
            .iter()
            .filter(|call| call.asked.contains(ask))
            .cloned()
            .collect()
    }

    fn record(request: &ChatRequest) -> Call {
        let is_tool_result = |role: &MessageRole, content: &str| {
            matches!(role, MessageRole::Tool)
                || (matches!(role, MessageRole::User) && content.starts_with(TOOL_RESULT_OPENING))
        };
        let joined = |wanted: &dyn Fn(&MessageRole, &str) -> bool| {
            request
                .messages
                .iter()
                .filter(|m| wanted(&m.role, &m.content))
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        Call {
            system: joined(&|role, _| matches!(role, MessageRole::System)),
            tool_results: joined(&is_tool_result),
            asked: request
                .messages
                .iter()
                .rev()
                .find(|m| {
                    matches!(m.role, MessageRole::User) && !is_tool_result(&m.role, &m.content)
                })
                .map_or_else(String::new, |m| m.content.clone()),
        }
    }
}

#[async_trait]
impl LlmProvider for RecordingModel {
    fn name(&self) -> &'static str {
        "derived_consent_playbooks_mock"
    }
    fn display_name(&self) -> &'static str {
        "Derived-consent playbooks mock"
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
        let call = Self::record(request);
        if self.is_learning_pass(&call) {
            return Ok(ChatResponse {
                replay_blocks: None,
                content: "[]".to_owned(),
                model: MODEL.to_owned(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            });
        }
        self.calls.lock().unwrap().push(call.clone());
        let tool_calls =
            (call.asked.contains(PLAYBOOKS_ASK) && call.tool_results.is_empty()).then(|| {
                vec![ToolCallRequest {
                    id: "call-playbooks".to_owned(),
                    function_name: "list_coaching_playbooks".to_owned(),
                    arguments: json!({ "limit": 10 }),
                }]
            });
        Ok(ChatResponse {
            replay_blocks: None,
            content: if tool_calls.is_some() {
                String::new()
            } else {
                REPLY.to_owned()
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

// ════════════════════════════════════════════════════════════════════════
// The fixture
// ════════════════════════════════════════════════════════════════════════

struct Fixture {
    resources: Arc<ServerContext>,
    ctx: ChatPipelineContext,
    model: Arc<RecordingModel>,
    user_id: Uuid,
    tenant: TenantId,
    conversation: String,
}

impl Fixture {
    /// An athlete who consents to WHOOP's AI use, with a well-evidenced
    /// playbook and an open commitment, both drawn from WHOOP data.
    async fn new(email: &str) -> Self {
        let model = Arc::new(RecordingModel::new());
        let resources = create_test_server_resources_with_chat_provider(model.clone())
            .await
            .unwrap();
        let (user_id, _user, tenant) =
            create_test_user_with_plan(&resources.agent.database, email, "professional")
                .await
                .unwrap();
        let ctx = resources.chat_pipeline_context();
        model.know(&ctx);
        let repos = &resources.common.repos;
        grant_consent(
            repos,
            tenant.as_uuid(),
            user_id,
            WHOOP,
            NoticeKind::AiConsent,
            "WHOOP",
        )
        .await
        .unwrap();
        // A playbook is the agent's memory (T3): the model reads it only
        // under the athlete's sensitive_t3 (carnet#1544).
        grant_consent(
            repos,
            tenant.as_uuid(),
            user_id,
            SENSITIVE_T3,
            NoticeKind::AiConsent,
            sensitive_display_name(SENSITIVE_T3).unwrap(),
        )
        .await
        .unwrap();
        let conversation = repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                tenant,
                "derived consent playbooks",
                MODEL,
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let fx = Self {
            resources,
            ctx,
            model,
            user_id,
            tenant,
            conversation,
        };
        fx.whoop_playbook().await;
        fx.whoop_commitment().await;
        fx
    }

    /// Ten successful outcomes of advice drawn from a WHOOP recovery reading:
    /// enough evidence for the prompt block to render the playbook.
    async fn whoop_playbook(&self) {
        self.playbook(TriggerKind::HrvDrop, &[WHOOP.to_owned()])
            .await;
    }

    /// Ten successful outcomes of `kind` advice drawn from data naming
    /// `sources`.
    async fn playbook(&self, kind: TriggerKind, sources: &[String]) {
        let trigger = TriggerPattern {
            kind,
            sport: None,
            magnitude: Band::High,
        };
        let intervention = Intervention {
            kind: InterventionKind::EasyBlock,
            magnitude: Some(2),
        };
        let metric = OutcomeMetric::HrvDelta { window_days: 7 };
        let tenant = self.tenant.to_string();
        let user = self.user_id.to_string();
        for _ in 0..10 {
            self.resources
                .common
                .repos
                .playbooks
                .record_outcome_and_label(
                    &RecordedOutcome {
                        tenant_id: &tenant,
                        user_id: &user,
                        agent_slug: None,
                        trigger: &trigger,
                        intervention: &intervention,
                        outcome_metric: &metric,
                        label: OutcomeLabel::Success,
                        at: Timestamp::now(),
                        transport_policy: TransportPolicy::AnyTransport,
                        ai_consent_sources: sources,
                    },
                    "advice-without-row",
                    LabelSource::DataHeuristic,
                )
                .await
                .unwrap();
        }
    }

    /// An open commitment recorded in a turn that served WHOOP data.
    async fn whoop_commitment(&self) {
        let now = Timestamp::now();
        let recorded = self
            .resources
            .common
            .repos
            .commitments
            .insert_commitment(&Commitment {
                id: Uuid::new_v4().to_string(),
                tenant_id: self.tenant.to_string(),
                user_id: self.user_id.to_string(),
                agent_id: None,
                conversation_id: Some(self.conversation.clone()),
                statement: COMMITMENT_STATEMENT.to_owned(),
                sport: Some("run".to_owned()),
                target_sessions: 3,
                window_start: now,
                window_end: now + Elapsed::days(5),
                status: CommitmentStatus::Open,
                outcome: None,
                completed_sessions: None,
                swept_at: None,
                reported_at: None,
                created_at: now,
                updated_at: now,
                transport_policy: TransportPolicy::AnyTransport,
                ai_consent_sources: vec![WHOOP.to_owned()],
            })
            .await
            .unwrap();
        assert!(recorded, "premise: the commitment was recorded");
    }

    /// Run one turn asking `content`; return the model calls that answered it.
    async fn turn(&self, content: &str) -> Vec<Call> {
        let from = self.model.calls_so_far();
        pierre_chat_pipeline::execute(
            &self.ctx,
            TurnRequest {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                conversation_id: self.conversation.clone(),
                user_id: self.user_id,
                conversation_tenant_id: self.tenant,
                tool_tenant_id: self.tenant,
                content: content.to_owned(),
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
                locale: "fr".to_owned(),
                transport: None,
                prose_contract: None,
            }),
        )
        .await
        .expect("the turn is served");
        let calls = self.model.answering(from, content);
        assert!(!calls.is_empty(), "the turn reached the model");
        calls
    }

    /// The athlete's rows of `table`, as their data export returns them.
    async fn exported(&self, table: &str) -> Vec<Value> {
        let table = USER_EXPORT_TABLES
            .iter()
            .find(|candidate| candidate.name == table)
            .expect("the table is exported");
        self.resources
            .common
            .repos
            .user_export
            .export_page(table, self.user_id, "")
            .await
            .unwrap()
            .rows
    }

    async fn withdraw(&self, key: &str) {
        let withdrawn = withdraw_consent(
            &self.resources.common.repos,
            self.user_id,
            key,
            NoticeKind::AiConsent,
            key,
        )
        .await
        .unwrap();
        assert!(withdrawn, "premise: the {key} AI consent was standing");
    }
}

/// The `list_coaching_playbooks` answer the model was handed.
fn listed_playbooks(calls: &[Call]) -> String {
    calls
        .iter()
        .map(|call| call.tool_results.as_str())
        .find(|results| !results.is_empty())
        .expect("the model was handed the playbook list")
        .to_owned()
}

// ════════════════════════════════════════════════════════════════════════
// carnet#1157 gap 1: a withdrawal reaches playbooks and commitments
// ════════════════════════════════════════════════════════════════════════

/// While the athlete consents to WHOOP's AI use, the WHOOP-derived playbook
/// and commitment reach the prompt; once they withdraw it, neither does.
#[tokio::test]
async fn withdrawing_whoop_keeps_its_playbook_and_commitment_out_of_the_prompt() {
    let fx = Fixture::new("derived-consent-playbook-prompt@test.com").await;

    let consented = fx.turn(PLAIN_ASK).await;
    assert!(
        consented
            .iter()
            .all(|call| call.system.contains(PLAYBOOK_BLOCK)),
        "premise: a consented WHOOP playbook reaches the prompt"
    );
    assert!(
        consented
            .iter()
            .all(|call| call.system.contains(COMMITMENT_STATEMENT)),
        "premise: a consented WHOOP commitment reaches the prompt"
    );

    fx.withdraw(WHOOP).await;
    let withdrawn = fx.turn(PLAIN_ASK).await;
    for call in &withdrawn {
        assert!(
            !call.system.contains(PLAYBOOK_BLOCK),
            "no WHOOP playbook reaches the prompt after withdrawal"
        );
        assert!(
            !call.system.contains(COMMITMENT_STATEMENT),
            "no WHOOP commitment reaches the prompt after withdrawal"
        );
    }
}

/// The playbook list a tool hands the model obeys the same consent.
#[tokio::test]
async fn withdrawing_whoop_keeps_its_playbook_out_of_the_tool_answer() {
    let fx = Fixture::new("derived-consent-playbook-tool@test.com").await;

    let consented = listed_playbooks(&fx.turn(PLAYBOOKS_ASK).await);
    assert!(
        consented.contains("hrv_drop"),
        "premise: the model is handed the consented WHOOP playbook: {consented}"
    );

    fx.withdraw(WHOOP).await;
    let withdrawn = listed_playbooks(&fx.turn(PLAYBOOKS_ASK).await);
    assert!(
        !withdrawn.contains("hrv_drop"),
        "no WHOOP playbook reaches the model after withdrawal: {withdrawn}"
    );
}

/// JF 2026-11-25 ("Gate on tier consent", carnet#1544): a playbook is the
/// agent's memory, labelled T3. Once the athlete withholds `sensitive_t3`, no
/// playbook reaches the prompt or the tool answer, WHOOP consent or not; the
/// commitment, which is not memory, still does.
#[tokio::test]
async fn withholding_sensitive_t3_keeps_every_playbook_from_the_model() {
    let fx = Fixture::new("derived-consent-playbook-t3@test.com").await;
    withdraw_consent(
        &fx.resources.common.repos,
        fx.user_id,
        SENSITIVE_T3,
        NoticeKind::AiConsent,
        sensitive_display_name(SENSITIVE_T3).unwrap(),
    )
    .await
    .unwrap();

    let prompt = fx.turn(PLAIN_ASK).await;
    for call in &prompt {
        assert!(
            !call.system.contains(PLAYBOOK_BLOCK),
            "bug: a playbook reached the prompt without sensitive_t3"
        );
    }
    assert!(
        prompt
            .iter()
            .all(|call| call.system.contains(COMMITMENT_STATEMENT)),
        "the commitment is no memory and still reaches the prompt"
    );

    let listed = listed_playbooks(&fx.turn(PLAYBOOKS_ASK).await);
    assert!(
        !listed.contains("hrv_drop"),
        "bug: list_coaching_playbooks handed a playbook without sensitive_t3: {listed}"
    );
    assert!(
        listed.contains("sensitive_tier"),
        "the answer says why it holds none: {listed}"
    );
}

/// A playbook learned over Garmin data is the agent's memory, which Garmin
/// data never feeds (carnet#1158): even under the athlete's Garmin AI
/// consent, the model is never handed it, while the WHOOP one beside it is.
#[tokio::test]
async fn a_playbook_naming_garmin_never_reaches_the_model_even_under_its_consent() {
    let fx = Fixture::new("derived-consent-playbook-garmin@test.com").await;
    grant_consent(
        &fx.resources.common.repos,
        fx.tenant.as_uuid(),
        fx.user_id,
        GARMIN,
        NoticeKind::AiConsent,
        "Garmin",
    )
    .await
    .unwrap();
    fx.playbook(TriggerKind::LoadRamp, &[GARMIN.to_owned()])
        .await;

    let listed = listed_playbooks(&fx.turn(PLAYBOOKS_ASK).await);
    assert!(
        listed.contains("hrv_drop"),
        "premise: the model is handed the consented WHOOP playbook: {listed}"
    );
    assert!(
        !listed.contains("load_ramp"),
        "bug: a Garmin-derived playbook reached the model: {listed}"
    );
}

/// The athlete always reads their own data: their export returns the
/// WHOOP-derived playbook and commitment after the consent is withdrawn.
#[tokio::test]
async fn the_athletes_export_still_returns_withdrawn_playbooks_and_commitments() {
    let fx = Fixture::new("derived-consent-playbook-export@test.com").await;
    fx.withdraw(WHOOP).await;

    let playbooks = fx.exported("coaching_playbooks").await;
    assert_eq!(playbooks.len(), 1, "the athlete still reads the playbook");
    assert_eq!(playbooks[0]["ai_consent_sources"], WHOOP);
    let commitments = fx.exported("athlete_commitments").await;
    assert!(
        commitments
            .iter()
            .any(|row| row["statement"] == COMMITMENT_STATEMENT),
        "the athlete still reads the commitment: {commitments:?}"
    );
}
