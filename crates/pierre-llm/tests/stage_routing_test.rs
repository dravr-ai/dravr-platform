// ABOUTME: Pins where a background stage's model goes: the head provider only, never a tenant's, never a fallback tier
// ABOUTME: And that a side call made through complete_recorded writes one usage record, answered or failed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A stage model is the head provider's: it routes the request the head
//! receives, and nothing else. A request nobody routed — the reply's draft, a
//! re-ask pinned to the turn's model — reaches the head unchanged. A provider
//! not built as the platform's head (a tenant's own key) routes nothing. A
//! stage model the head does not publish is never sent to it, so the stage is
//! served by the head on its own model rather than refused down the chain. A
//! stage call the chain falls back on reaches the next tier with its model
//! cleared, because the head's model ids are not that tier's to resolve.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use embacle::types::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities,
    LlmProvider as EmbacleLlmProvider, RunnerError, TokenUsage,
};
use pierre_llm::call_record::{complete_recorded, LlmCallRecord, LlmCallRecorder};
use pierre_llm::config::LlmProviderType;
use pierre_llm::stage::{LlmStage, StageModels, COPILOT_SDK_BACKGROUND_MODEL};
use pierre_llm::{ChatProvider, EmbacleProvider};

/// The model each request a scripted runner received asked for, in order.
type ModelLog = Arc<Mutex<Vec<Option<String>>>>;

/// A scripted embacle runner that remembers the model of every request it
/// receives, answering with its own text or failing as a provider fault.
struct Recording {
    name: &'static str,
    models: Vec<String>,
    answer: Option<&'static str>,
    usage: Option<TokenUsage>,
    requested: ModelLog,
}

impl Recording {
    fn answering(name: &'static str, answer: &'static str) -> Self {
        Self {
            name,
            models: vec![format!("{name}-model")],
            answer: Some(answer),
            usage: None,
            requested: Arc::default(),
        }
    }

    fn failing(name: &'static str) -> Self {
        Self {
            answer: None,
            ..Self::answering(name, "")
        }
    }

    /// Also publish `model`, as a head whose catalogue lists it does.
    fn publishing(mut self, model: &str) -> Self {
        self.models.push(model.to_owned());
        self
    }

    fn with_usage(mut self, usage: TokenUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// The models requested so far, shared with the runner once it is boxed.
    fn log(&self) -> ModelLog {
        Arc::clone(&self.requested)
    }
}

#[async_trait]
impl EmbacleLlmProvider for Recording {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &str {
        self.name
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING
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
        let Some(answer) = self.answer else {
            return Err(RunnerError::external_service(self.name, "scripted outage"));
        };
        Ok(ChatResponse {
            content: answer.to_owned(),
            model: request
                .model
                .clone()
                .unwrap_or_else(|| self.models[0].clone()),
            usage: self.usage.clone(),
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

/// Every record it is handed.
#[derive(Default)]
struct Captured(Mutex<Vec<LlmCallRecord>>);

impl LlmCallRecorder for Captured {
    fn record(&self, record: LlmCallRecord) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

impl Captured {
    fn taken(&self) -> Vec<LlmCallRecord> {
        mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

fn requested(log: &ModelLog) -> Vec<Option<String>> {
    mem::take(&mut *log.lock().unwrap_or_else(PoisonError::into_inner))
}

/// The stage models a `copilot_sdk` head resolves with no stage variable set.
fn sdk_defaults() -> StageModels {
    StageModels::resolve(Some(LlmProviderType::CopilotSdk), |_| None)
}

fn ask() -> ChatRequest {
    ChatRequest::new(vec![ChatMessage::user("which language is this?")])
}

#[tokio::test]
async fn every_background_stage_reaches_a_copilot_sdk_head_on_haiku() {
    let runner = Recording::answering("copilot_sdk", "en").publishing(COPILOT_SDK_BACKGROUND_MODEL);
    let log = runner.log();
    let head = ChatProvider::Embacle(
        EmbacleProvider::from_runner(Box::new(runner), "Copilot SDK (scripted)")
            .with_stage_models(sdk_defaults()),
    );

    for stage in LlmStage::ALL {
        head.complete(&head.routed(stage, ask())).await.unwrap();
    }

    assert_eq!(
        requested(&log),
        vec![Some(COPILOT_SDK_BACKGROUND_MODEL.to_owned()); LlmStage::ALL.len()],
        "each of the six stages asked the head for the background model"
    );
    // A wire value: Copilot's id for Haiku 5.5, which the SDK runtime refuses
    // to substitute for. The routing decision is that every background stage
    // runs on it.
    assert_eq!(COPILOT_SDK_BACKGROUND_MODEL, "claude-haiku-5.5");
}

#[tokio::test]
async fn a_request_nobody_routed_keeps_the_turn_model() {
    let runner =
        Recording::answering("copilot_sdk", "the reply").publishing(COPILOT_SDK_BACKGROUND_MODEL);
    let log = runner.log();
    let head = ChatProvider::Embacle(
        EmbacleProvider::from_runner(Box::new(runner), "Copilot SDK (scripted)")
            .with_stage_models(sdk_defaults()),
    );

    // The draft and every re-ask pin the turn's model; nothing routes them.
    head.complete(&ask().with_model("claude-sonnet-5.5"))
        .await
        .unwrap();
    head.complete(&ask()).await.unwrap();

    assert_eq!(
        requested(&log),
        vec![Some("claude-sonnet-5.5".to_owned()), None],
        "stage models never touch a request that was not routed to a stage"
    );
}

#[tokio::test]
async fn a_provider_not_built_as_the_head_routes_nothing() {
    // A tenant's own key is built from its credential, never with stage models.
    let runner = Recording::answering("copilot_sdk", "en");
    let log = runner.log();
    let tenant = ChatProvider::Embacle(
        EmbacleProvider::from_runner(Box::new(runner), "Tenant key (scripted)")
            .of_kind(LlmProviderType::CopilotSdk),
    );
    for stage in LlmStage::ALL {
        assert_eq!(tenant.stage_model(stage), None, "{stage:?}");
        tenant.complete(&tenant.routed(stage, ask())).await.unwrap();
    }
    assert_eq!(requested(&log), vec![None; LlmStage::ALL.len()]);

    let double = ChatProvider::Custom(Arc::new(ChatProvider::Embacle(
        EmbacleProvider::from_runner(
            Box::new(Recording::answering("double", "en").publishing(COPILOT_SDK_BACKGROUND_MODEL)),
            "Test double (scripted)",
        )
        .with_stage_models(sdk_defaults()),
    )));
    assert_eq!(double.stage_model(LlmStage::ClaimJudge), None);
    assert_eq!(double.routed(LlmStage::ClaimJudge, ask()).model, None);
}

#[tokio::test]
async fn a_stage_model_the_head_does_not_publish_is_served_by_the_head_on_its_own_model() {
    // The head publishes its own model only — the catalogue of a runtime that
    // predates the stage model's id.
    let head = Recording::answering("copilot_sdk", "en");
    let head_log = head.log();
    let tail = Recording::answering("claude-code", "en");
    let tail_log = tail.log();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![
            EmbacleProvider::from_runner(Box::new(head), "Copilot SDK (scripted)"),
            EmbacleProvider::from_runner(Box::new(tail), "Claude Code (scripted)"),
        ])
        .unwrap()
        .with_stage_models(sdk_defaults()),
    );
    let captured = Arc::new(Captured::default());
    let recorder: Arc<dyn LlmCallRecorder> = captured.clone();

    for stage in LlmStage::ALL {
        assert_eq!(
            chain.stage_model(stage),
            None,
            "{stage:?} inherits the head's model"
        );
        let request = chain.routed(stage, ask());
        complete_recorded(&chain, &request, Some(&recorder))
            .await
            .unwrap();
    }

    assert_eq!(
        requested(&head_log),
        vec![None; LlmStage::ALL.len()],
        "the head serves every stage on its own model"
    );
    assert!(
        requested(&tail_log).is_empty(),
        "no stage call was refused onto the next tier"
    );
    let records = captured.taken();
    assert_eq!(records.len(), LlmStage::ALL.len(), "{records:?}");
    for record in &records {
        assert_eq!(
            (record.provider.as_str(), record.model.as_str()),
            ("copilot_sdk", "copilot_sdk-model")
        );
    }
}

/// The one test in this binary whose chain head fails: the circuit breaker is
/// process-wide and opens after three consecutive primary failures, so a second
/// would make the head's attempt depend on test order.
#[tokio::test]
async fn a_stage_call_that_falls_back_runs_on_the_next_tier_own_model() {
    // The head publishes Haiku and refuses it anyway — an account the vendor
    // stopped entitling between boot and the call.
    let head = Recording::failing("copilot_sdk").publishing(COPILOT_SDK_BACKGROUND_MODEL);
    let head_log = head.log();
    let tail = Recording::answering("claude-code", "en");
    let tail_log = tail.log();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![
            EmbacleProvider::from_runner(Box::new(head), "Copilot SDK (scripted)")
                .with_stage_models(sdk_defaults()),
            EmbacleProvider::from_runner(Box::new(tail), "Claude Code (scripted)"),
        ])
        .unwrap(),
    );
    let captured = Arc::new(Captured::default());
    let recorder: Arc<dyn LlmCallRecorder> = captured.clone();

    assert_eq!(
        chain.stage_model(LlmStage::MemoryExtraction),
        Some(COPILOT_SDK_BACKGROUND_MODEL),
        "a chain routes through its head's stage models"
    );
    let request = chain.routed(LlmStage::MemoryExtraction, ask());
    let reply = complete_recorded(&chain, &request, Some(&recorder))
        .await
        .unwrap();

    assert_eq!(reply.content, "en");
    assert_eq!(
        requested(&head_log),
        vec![Some(COPILOT_SDK_BACKGROUND_MODEL.to_owned())],
        "the head was asked for Haiku"
    );
    assert_eq!(
        requested(&tail_log),
        vec![None],
        "the tier behind it resolves its own model, not the head's id"
    );
    let records = captured.taken();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        records[0].provider, "claude-code",
        "the row is priced against the tier that served, not the head"
    );
    assert_eq!(records[0].model, "claude-code-model");
}

#[tokio::test]
async fn a_recorded_side_call_writes_one_record_answered_or_failed() {
    let captured = Arc::new(Captured::default());
    let recorder: Arc<dyn LlmCallRecorder> = captured.clone();

    let answers = ChatProvider::Embacle(
        EmbacleProvider::from_runner(
            Box::new(
                Recording::answering("copilot_sdk", "en")
                    .publishing(COPILOT_SDK_BACKGROUND_MODEL)
                    .with_usage(TokenUsage::new(120, 3, 123).with_cache(Some(100), Some(20))),
            ),
            "Copilot SDK (scripted)",
        )
        .with_stage_models(sdk_defaults()),
    );
    let request = answers.routed(LlmStage::ClaimJudge, ask());
    complete_recorded(&answers, &request, Some(&recorder))
        .await
        .unwrap();
    let records = captured.taken();
    assert_eq!(records.len(), 1, "{records:?}");
    let call = &records[0];
    assert!(call.success);
    assert_eq!(call.provider, "copilot_sdk");
    assert_eq!(call.model, COPILOT_SDK_BACKGROUND_MODEL);
    assert_eq!((call.prompt_tokens, call.completion_tokens), (120, 3));
    assert_eq!((call.cached_tokens, call.cached_write_tokens), (100, 20));
    assert!(!call.token_counts_estimated);

    let fails = ChatProvider::Embacle(
        EmbacleProvider::from_runner(
            Box::new(Recording::failing("copilot_sdk").publishing(COPILOT_SDK_BACKGROUND_MODEL)),
            "Copilot SDK (scripted)",
        )
        .with_stage_models(sdk_defaults()),
    );
    let request = fails.routed(LlmStage::MemoryExtraction, ask());
    assert!(complete_recorded(&fails, &request, Some(&recorder))
        .await
        .is_err());
    let records = captured.taken();
    assert_eq!(
        records.len(),
        1,
        "a failed call is recorded too: {records:?}"
    );
    let call = &records[0];
    assert!(!call.success);
    assert_eq!(
        call.model, COPILOT_SDK_BACKGROUND_MODEL,
        "a failure names the model it asked for"
    );
    assert!(
        call.token_counts_estimated && call.prompt_tokens > 0 && call.completion_tokens == 0,
        "the prompt it sent is billed, estimated: {call:?}"
    );

    complete_recorded(&answers, &ask(), None).await.unwrap();
    assert!(captured.taken().is_empty(), "no recorder, no record");
}
