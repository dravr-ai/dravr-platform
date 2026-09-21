// ABOUTME: A call that falls back must be attributable to the tier that answered, not the chain's head
// ABOUTME: Drives real chains through observe_served_tier and asserts the reported provider and position
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A chain's `name()` is its head. The tool loop used to record that name on
//! every completion, so after a fallback the log line and the `llm_usage` row
//! named a provider that had not answered — on 2026-09-21 Cohere served a turn
//! that was logged and costed as Claude. These tests pin the seam that carries
//! the truth back: embacle tells the chain observer which tier served, and the
//! observer hands it to whoever wrapped the call in `observe_served_tier`.
//!
//! One fallthrough per binary keeps the process-global chain guard far from
//! its breaker threshold, so no test here reroutes another.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError,
};
use pierre_llm::served_tier::{observe_served_tier, ServedTier};
use pierre_llm::{ChatMessage, ChatProvider, EmbacleProvider};

/// Answers with a fixed line under its own model name. An empty line is an
/// empty completion, which the chain's strict policy passes over.
struct Scripted {
    label: &'static str,
    answer: &'static str,
    models: Vec<String>,
}

impl Scripted {
    fn new(label: &'static str, answer: &'static str) -> Self {
        Self {
            label,
            answer,
            models: vec![format!("{label}-model")],
        }
    }
}

#[async_trait::async_trait]
impl EmbacleLlmProvider for Scripted {
    fn name(&self) -> &'static str {
        self.label
    }

    fn display_name(&self) -> &str {
        self.label
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        Ok(ChatResponse {
            content: self.answer.to_owned(),
            model: self.models[0].clone(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("not streamed in this test"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

fn chain(primary_answer: &'static str) -> ChatProvider {
    ChatProvider::Embacle(
        EmbacleProvider::chain(vec![
            EmbacleProvider::from_runner(
                Box::new(Scripted::new("primary", primary_answer)),
                "primary",
            ),
            EmbacleProvider::from_runner(
                Box::new(Scripted::new("secondary", "Tu as couru 42 km ce mois-ci.")),
                "secondary",
            ),
        ])
        .expect("two tiers"),
    )
}

fn request() -> ChatRequest {
    ChatRequest::new(vec![
        ChatMessage::system("agent persona"),
        ChatMessage::user("how did my week go?"),
    ])
}

#[tokio::test]
async fn a_fallback_is_attributed_to_the_tier_that_answered() {
    let chain = chain("");
    let request = request();

    let (response, served) = observe_served_tier(chain.complete_with_tools(&request, None)).await;
    let response = response.expect("the secondary answers when the primary returns nothing");

    // The premise of the defect: the chain still calls itself by its head.
    assert_eq!(chain.name(), "primary");

    assert_eq!(
        served,
        Some(ServedTier {
            provider: "secondary",
            position: 1,
        }),
        "the tier that answered is the one a caller must record"
    );
    assert_eq!(response.model, "secondary-model");
    assert_eq!(
        response.content.as_deref(),
        Some("Tu as couru 42 km ce mois-ci.")
    );
}

#[tokio::test]
async fn a_primary_that_answers_is_reported_at_position_zero() {
    let chain = chain("Belle semaine.");
    let request = request();

    let (response, served) = observe_served_tier(chain.complete_with_tools(&request, None)).await;
    let response = response.expect("the primary answers");

    assert_eq!(
        served,
        Some(ServedTier {
            provider: "primary",
            position: 0,
        })
    );
    assert_eq!(response.model, "primary-model");
}

#[tokio::test]
async fn a_provider_outside_a_chain_reports_no_tier() {
    let single = ChatProvider::Embacle(EmbacleProvider::from_runner(
        Box::new(Scripted::new("solo", "Bonjour.")),
        "solo",
    ));
    let request = request();

    let (response, served) = observe_served_tier(single.complete_with_tools(&request, None)).await;
    response.expect("the provider answers");

    assert_eq!(
        served, None,
        "no chain, no observer: the caller attributes the call to the provider it invoked"
    );
}

#[tokio::test]
async fn a_tier_reported_outside_an_observed_call_does_not_panic() {
    // A probe or a background call reads `response.model` directly and wraps
    // nothing; the observer's report must be a no-op there, not a panic on an
    // unset task-local.
    let chain = chain("Belle semaine.");
    let response = chain
        .complete_with_tools(&request(), None)
        .await
        .expect("the primary answers");
    assert_eq!(response.model, "primary-model");
}
