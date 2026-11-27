// ABOUTME: Tests for RecordedJudge, the platform's judge model behind dravr-colleur's claim judge
// ABOUTME: Pins the shipped prompt, the stage model, one usage record per claim, and the re-ask split
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dravr_colleur::claims::ClaimStatus;
use dravr_colleur::judge::{self, ClaimJudge};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
};
use pierre_llm::call_record::{LlmCallRecord, LlmCallRecorder};
use pierre_llm::claim_judge::RecordedJudge;
use pierre_llm::prompts::CLAIM_JUDGE_PROMPT;
use pierre_llm::stage::COPILOT_SDK_BACKGROUND_MODEL;

/// A provider answering from a script, remembering what each call asked.
struct CannedJudge {
    bodies: Vec<String>,
    calls: AtomicUsize,
    last_model: Mutex<Option<String>>,
    last_system_prompt: Mutex<Option<String>>,
}

impl CannedJudge {
    /// Answers each call with the next body, repeating the last; no body
    /// makes every call fail, as a rate-limited account does.
    fn answering(bodies: &[&str]) -> Self {
        Self {
            bodies: bodies.iter().map(|b| (*b).to_owned()).collect(),
            calls: AtomicUsize::new(0),
            last_model: Mutex::new(None),
            last_system_prompt: Mutex::new(None),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_model(&self) -> Option<String> {
        self.last_model.lock().expect("model lock").clone()
    }

    fn last_system_prompt(&self) -> Option<String> {
        self.last_system_prompt
            .lock()
            .expect("system prompt lock")
            .clone()
    }
}

#[async_trait]
impl LlmProvider for CannedJudge {
    fn name(&self) -> &'static str {
        "canned-judge"
    }

    fn display_name(&self) -> &'static str {
        "Canned Judge"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &'static str {
        "canned"
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.last_model
            .lock()
            .expect("model lock")
            .clone_from(&request.model);
        *self.last_system_prompt.lock().expect("system prompt lock") = request
            .messages
            .iter()
            .find(|m| m.role == MessageRole::System)
            .map(|m| m.content.clone());
        let Some(last) = self.bodies.len().checked_sub(1) else {
            return Err(AppError::external_service("canned-judge", "rate limited"));
        };
        Ok(ChatResponse {
            replay_blocks: None,
            content: self.bodies[call.min(last)].clone(),
            model: "canned".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::internal("canned judge does not stream"))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// Every record it is handed.
#[derive(Default)]
struct Captured(Mutex<Vec<LlmCallRecord>>);

impl LlmCallRecorder for Captured {
    fn record(&self, record: LlmCallRecord) {
        self.0.lock().expect("records lock").push(record);
    }
}

const SUPPORTED: &str =
    r#"{"verdict":"supported","confidence":0.8,"rationale":"Matches the protein consensus."}"#;

fn shipped<'a>(model: &'a RecordedJudge<'a>) -> ClaimJudge<'a> {
    ClaimJudge {
        provider: model,
        system_prompt: CLAIM_JUDGE_PROMPT,
        model: None,
    }
}

/// The catalogue prompt the production judge runs under still states the
/// verdict vocabulary colleur's parser maps.
#[test]
fn shipped_judge_prompt_names_every_verdict_the_parser_maps() {
    for verdict in ["supported", "contradicted", "unverifiable"] {
        assert!(
            CLAIM_JUDGE_PROMPT.contains(verdict),
            "claim_judge prompt no longer names the '{verdict}' verdict"
        );
    }
}

/// The judge's instructions reach the provider as the system message.
#[tokio::test]
async fn the_call_carries_the_system_prompt_it_was_given() {
    let provider = CannedJudge::answering(&[SUPPORTED]);
    let model = RecordedJudge::new(&provider, None);
    judge::judge_claim(shipped(&model), "Zone 2 is below LT1.", "")
        .await
        .unwrap();

    assert_eq!(
        provider.last_system_prompt().as_deref(),
        Some(CLAIM_JUDGE_PROMPT),
        "bug: the judge's instructions must reach the provider as its system message"
    );
}

/// The judge asks for the model its stage runs on, and every claim it judges
/// is handed to the recorder as a call of its own.
#[tokio::test]
async fn the_judge_asks_for_its_stage_model_and_records_each_claim() {
    let provider = CannedJudge::answering(&[SUPPORTED]);
    let captured = Arc::new(Captured::default());
    let recorder: Arc<dyn LlmCallRecorder> = captured.clone();
    let model = RecordedJudge::new(&provider, Some(&recorder));
    let stage_judge = ClaimJudge {
        provider: &model,
        system_prompt: CLAIM_JUDGE_PROMPT,
        model: Some(COPILOT_SDK_BACKGROUND_MODEL),
    };

    let first = judge::judge_claim(
        stage_judge,
        "Protein at 1.6 g/kg/day maximizes synthesis",
        "",
    )
    .await
    .unwrap();
    let second = judge::judge_claim(stage_judge, "Max heart rate is 208 minus 0.7 times age", "")
        .await
        .unwrap();

    assert_eq!(
        first.status,
        ClaimStatus::Supported,
        "bug: the first claim must take the canned supported verdict"
    );
    assert_eq!(
        second.status,
        ClaimStatus::Supported,
        "bug: the second claim must take the canned supported verdict"
    );
    assert_eq!(
        provider.last_model().as_deref(),
        Some(COPILOT_SDK_BACKGROUND_MODEL),
        "bug: the judge must ask for the model its stage runs on"
    );
    let records = mem::take(&mut *captured.0.lock().expect("records lock"));
    assert_eq!(records.len(), 2, "one record per judged claim: {records:?}");
    assert!(
        records
            .iter()
            .all(|r| r.success && r.provider == "canned-judge"),
        "bug: each record must name the provider and a successful call: {records:?}"
    );
}

/// An answer with no JSON reaches colleur as unparseable, so it asks once
/// more and takes the second answer's verdict.
#[tokio::test]
async fn an_answer_without_json_is_asked_again_once() {
    let provider = CannedJudge::answering(&[
        "I cannot assess this claim without more context.",
        SUPPORTED,
    ]);
    let model = RecordedJudge::new(&provider, None);
    let judgement = judge::judge_claim(shipped(&model), "Zone 2 is below LT1.", "")
        .await
        .expect("the re-ask's verdict is used");

    assert_eq!(
        judgement.status,
        ClaimStatus::Supported,
        "bug: the re-ask's verdict must be the one returned"
    );
    assert_eq!(
        provider.call_count(),
        2,
        "bug: an unparseable answer is asked again exactly once"
    );
}

/// A provider error reaches colleur as a provider failure and is returned at
/// once: asking again would only hit it twice.
#[tokio::test]
async fn a_provider_error_is_not_asked_again() {
    let provider = CannedJudge::answering(&[]);
    let model = RecordedJudge::new(&provider, None);
    let result = judge::judge_claim(shipped(&model), "Zone 2 is below LT1.", "").await;

    assert!(result.is_err(), "bug: a provider error must stay an error");
    assert_eq!(
        provider.call_count(),
        1,
        "bug: a provider error must not be asked again"
    );
}
