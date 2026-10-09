// ABOUTME: A side call a turn owes writes its own llm_usage row, typed by its stage and run on its stage model
// ABOUTME: Memory extraction and advice capture, driven through their public entry points against a test database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Before these rows existed only the tool loop and the language
//! classification wrote `llm_usage`, so every call a turn made after its reply
//! cost nothing on paper. Each case here drives one side call against a
//! `copilot_sdk`-shaped head carrying the default stage models, then reads the
//! row back by the turn it was billed to.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError, TokenUsage,
};
use pierre_contremaitre::registry::PromptRegistry;
use pierre_core::models::usage::LlmUsageRecord;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::MemoryExtractionJobRow;
use pierre_database::RepositoryRegistry;
use pierre_llm::call_record::LlmCallRecorder;
use pierre_llm::config::LlmProviderType;
use pierre_llm::stage::{LlmStage, StageModels, COPILOT_SDK_BACKGROUND_MODEL};
use pierre_llm::{ChatProvider, EmbacleProvider};
use pierre_memory::FactSource;
use pierre_services::advice_capture::{
    AdviceCaptureStrategy, CapturedTurn, HeuristicGatedLlmExtraction,
};
use pierre_services::llm_usage_recorder::UsageRepoCallRecorder;
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_extraction::{
    run_extraction_job, spawn_extract_for_turn, ExtractionJobPayload, SpawnedExtractionRequest,
};
use pierre_services::memory_extraction_resume::resume_extraction_jobs;
use pierre_test_support::db::create_test_db;
use tokio::time::sleep;
use uuid::Uuid;

/// The model each request a scripted runner received asked for, in order.
type ModelLog = Arc<Mutex<Vec<Option<String>>>>;

/// A `copilot_sdk`-named runner that answers every request with `answer`,
/// reporting usage, and remembers the model each request asked for.
struct Scripted {
    answer: &'static str,
    models: Vec<String>,
    requested: ModelLog,
}

#[async_trait]
impl EmbacleLlmProvider for Scripted {
    fn name(&self) -> &'static str {
        "copilot_sdk"
    }

    fn display_name(&self) -> &'static str {
        "Copilot SDK (scripted)"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING | LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.model.clone());
        Ok(ChatResponse {
            content: self.answer.to_owned(),
            model: request
                .model
                .clone()
                .unwrap_or_else(|| self.models[0].clone()),
            // Reported usage, so the row's call_type carries no `_estimated`.
            usage: Some(TokenUsage::new(900, 12, 912)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("scripted runners do not stream"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

/// The platform's head as `ChatProvider::from_env` leaves a `copilot_sdk`
/// primary with no stage variable set, answering `answer`; and the log of the
/// models it was asked for.
fn copilot_sdk_head(answer: &'static str) -> (ChatProvider, ModelLog) {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let runner = Scripted {
        answer,
        models: vec![
            "claude-sonnet-5.5".to_owned(),
            COPILOT_SDK_BACKGROUND_MODEL.to_owned(),
        ],
        requested: Arc::clone(&requested),
    };
    let head = EmbacleProvider::from_runner(Box::new(runner), "Copilot SDK (scripted)")
        .of_kind(LlmProviderType::CopilotSdk)
        .with_stage_models(StageModels::resolve(
            Some(LlmProviderType::CopilotSdk),
            |_| None,
        ));
    (ChatProvider::Embacle(head), requested)
}

fn requested(log: &ModelLog) -> Vec<Option<String>> {
    mem::take(&mut *log.lock().unwrap_or_else(PoisonError::into_inner))
}

/// The rows billed to `turn`, waiting for the recorder's spawned write.
async fn rows_for(repos: &RepositoryRegistry, turn: ConversationTurnId) -> Vec<LlmUsageRecord> {
    for _ in 0..100 {
        let rows = repos
            .llm_usage
            .find_llm_usage_by_turn_id(turn)
            .await
            .unwrap();
        if !rows.is_empty() {
            return rows;
        }
        sleep(Duration::from_millis(20)).await;
    }
    Vec::new()
}

/// The de-dup tunables every extraction here runs with.
const DEDUP: DedupConfig = DedupConfig {
    candidate_limit: 50,
};

/// What one turn's extraction is owed, billed to `turn` in `conversation_id`.
fn payload(user_id: &str, conversation_id: &str, turn: ConversationTurnId) -> ExtractionJobPayload {
    ExtractionJobPayload {
        user_id: user_id.to_owned(),
        agent_id: None,
        user_message: "Je vise un ultra en mars.".to_owned(),
        assistant_reply: "Bien reçu, on construit vers mars.".to_owned(),
        source_msg_id: Some("m-1".to_owned()),
        pillar: None,
        source: FactSource::Conversation,
        force_kind: None,
        plan_was_saved: false,
        transport_policy: TransportPolicy::AnyTransport,
        conversation_id: Some(conversation_id.to_owned()),
        turn_id: Some(turn),
    }
}

#[tokio::test]
async fn memory_extraction_bills_its_own_row_to_the_turn_that_owed_it() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (provider, log) = copilot_sdk_head("[]");
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4().to_string();
    let turn = ConversationTurnId::new();

    run_extraction_job(
        repos.memory.as_ref(),
        &repos.llm_usage,
        &provider,
        DEDUP,
        "SYSTEM",
        &SpawnedExtractionRequest {
            tenant_id,
            usage_tenant_id: tenant_id,
            payload: payload(&user_id, "conv-1", turn),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        requested(&log),
        vec![Some(COPILOT_SDK_BACKGROUND_MODEL.to_owned())],
        "the extraction asked the head for its stage model"
    );
    let rows = rows_for(repos, turn).await;
    assert_eq!(rows.len(), 1, "one call, one row: {rows:?}");
    let row = &rows[0];
    // A persisted value: the type the cost dashboards group these rows by.
    assert_eq!(row.call_type, "memory_extraction");
    assert_eq!(row.model, COPILOT_SDK_BACKGROUND_MODEL);
    assert_eq!(row.provider, "copilot_sdk");
    assert_eq!(row.tenant_id, tenant_id.to_string());
    assert_eq!(row.user_id, user_id);
    assert_eq!(row.conversation_id.as_deref(), Some("conv-1"));
    assert_eq!((row.prompt_tokens, row.completion_tokens), (900, 12));
}

/// A guided answer in a shared room stamps its facts under the athlete's own
/// tenant; its usage row is still the turn's, under the conversation's tenant,
/// beside every other row that turn wrote.
#[tokio::test]
async fn a_guided_answer_bills_its_extraction_to_the_conversation_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (provider, log) = copilot_sdk_head("[]");
    let athlete = TenantId::generate();
    let channel = TenantId::generate();
    let user_id = Uuid::new_v4().to_string();
    let turn = ConversationTurnId::new();

    let run = spawn_extract_for_turn(
        Arc::clone(&repos.memory),
        Arc::clone(&repos.memory_extraction_jobs),
        Arc::clone(&repos.llm_usage),
        Some(Arc::new(provider)),
        DEDUP,
        "SYSTEM".to_owned(),
        SpawnedExtractionRequest {
            tenant_id: athlete,
            usage_tenant_id: channel,
            payload: payload(&user_id, "room-conv", turn),
        },
    )
    .await;
    run.await.unwrap();

    assert_eq!(
        requested(&log),
        vec![Some(COPILOT_SDK_BACKGROUND_MODEL.to_owned())]
    );
    let rows = rows_for(repos, turn).await;
    assert_eq!(rows.len(), 1, "one call, one row: {rows:?}");
    assert_eq!(rows[0].call_type, "memory_extraction");
    assert_eq!(
        rows[0].tenant_id,
        channel.to_string(),
        "billed under the conversation's tenant, not the tenant the facts go to"
    );
}

/// The resume sweep bills a job the way the turn that recorded it would have:
/// under the usage tenant on its row, or — for a row recorded before that
/// column existed — under the tenant its facts are stamped under.
#[tokio::test]
async fn a_resumed_extraction_bills_the_usage_tenant_its_row_recorded() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (provider, log) = copilot_sdk_head("[]");
    let user_id = Uuid::new_v4().to_string();
    let now = Utc::now().timestamp_millis();
    let stale = |tenant_id: TenantId,
                 usage_tenant_id: Option<TenantId>,
                 turn: ConversationTurnId| MemoryExtractionJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id,
        usage_tenant_id,
        payload: serde_json::to_string(&payload(&user_id, "room-conv", turn)).unwrap(),
        // Recorded ten minutes ago by a spawn whose lease has lapsed.
        created_at_ms: now - 10 * 60_000,
        leased_until_ms: now - 60_000,
        attempts: 1,
    };
    let (athlete, channel, guided_turn) = (
        TenantId::generate(),
        TenantId::generate(),
        ConversationTurnId::new(),
    );
    let (older, older_turn) = (TenantId::generate(), ConversationTurnId::new());
    for row in [
        stale(athlete, Some(channel), guided_turn),
        stale(older, None, older_turn),
    ] {
        repos
            .memory_extraction_jobs
            .record_extraction_job(&row)
            .await
            .unwrap();
    }

    let ran = resume_extraction_jobs(repos, &provider, DEDUP, "SYSTEM")
        .await
        .unwrap();

    assert_eq!(ran, 2, "both abandoned extractions ran");
    assert_eq!(
        requested(&log),
        vec![Some(COPILOT_SDK_BACKGROUND_MODEL.to_owned()); 2]
    );
    let guided = rows_for(repos, guided_turn).await;
    assert_eq!(guided.len(), 1, "{guided:?}");
    assert_eq!(guided[0].tenant_id, channel.to_string());
    let earlier = rows_for(repos, older_turn).await;
    assert_eq!(earlier.len(), 1, "{earlier:?}");
    assert_eq!(earlier[0].tenant_id, older.to_string());
}

#[tokio::test]
async fn advice_capture_bills_its_own_row_to_the_turn_it_read() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (provider, log) = copilot_sdk_head("[]");
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4().to_string();
    let turn = ConversationTurnId::new();
    let recorder: Arc<dyn LlmCallRecorder> = Arc::new(UsageRepoCallRecorder::new(
        Arc::clone(&repos.llm_usage),
        tenant_id.to_string(),
        user_id.clone(),
        Some("conv-2".to_owned()),
        turn,
        LlmStage::AdviceCapture.call_type(),
    ));
    let captured_turn = CapturedTurn {
        tenant_id: tenant_id.to_string(),
        user_id: user_id.clone(),
        agent_slug: None,
        user_message: "I've been skipping my Tuesday runs.".to_owned(),
        // Past the recommendation gate, so the extraction call is made.
        assistant_reply: "I'd suggest you add one tempo run this week to build your threshold."
            .to_owned(),
        source_msg_id: Some("m-2".to_owned()),
        transport_policy: TransportPolicy::AnyTransport,
    };

    let advice = HeuristicGatedLlmExtraction::new(Arc::new(PromptRegistry::new()))
        .capture(&captured_turn, &provider, Some(&recorder))
        .await;

    assert!(advice.is_empty(), "the scripted model extracted nothing");
    assert_eq!(
        requested(&log),
        vec![Some(COPILOT_SDK_BACKGROUND_MODEL.to_owned())],
        "the capture asked the head for its stage model"
    );
    let rows = rows_for(repos, turn).await;
    assert_eq!(rows.len(), 1, "one call, one row: {rows:?}");
    assert_eq!(rows[0].call_type, "advice_capture");
    assert_eq!(rows[0].model, COPILOT_SDK_BACKGROUND_MODEL);
    assert_eq!(rows[0].conversation_id.as_deref(), Some("conv-2"));
}

#[tokio::test]
async fn a_reply_the_gate_turns_away_costs_no_call_and_no_row() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let (provider, log) = copilot_sdk_head("[]");
    let turn = ConversationTurnId::new();
    let recorder: Arc<dyn LlmCallRecorder> = Arc::new(UsageRepoCallRecorder::new(
        Arc::clone(&repos.llm_usage),
        TenantId::generate().to_string(),
        Uuid::new_v4().to_string(),
        None,
        turn,
        LlmStage::AdviceCapture.call_type(),
    ));
    let greeting = CapturedTurn {
        tenant_id: TenantId::generate().to_string(),
        user_id: Uuid::new_v4().to_string(),
        agent_slug: None,
        user_message: "Thanks!".to_owned(),
        assistant_reply: "Nice work!".to_owned(),
        source_msg_id: None,
        transport_policy: TransportPolicy::AnyTransport,
    };

    HeuristicGatedLlmExtraction::new(Arc::new(PromptRegistry::new()))
        .capture(&greeting, &provider, Some(&recorder))
        .await;

    assert!(requested(&log).is_empty(), "the gate spent no call");
    assert!(
        repos
            .llm_usage
            .find_llm_usage_by_turn_id(turn)
            .await
            .unwrap()
            .is_empty(),
        "and wrote no row"
    );
}
