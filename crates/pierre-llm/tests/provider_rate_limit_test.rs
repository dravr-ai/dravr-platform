// ABOUTME: Every runner's RateLimit is the provider's account failing to serve (ExternalRateLimited), never the athlete's budget
// ABOUTME: A chain moves on to the next tier on it, and only an exhausted chain hands the athlete that code
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! embacle's `RunnerError` carries no origin: Gemini answering 429 and the
//! Copilot runtime refusing on a spent monthly quota both arrive as
//! `ErrorKind::RateLimit`. The platform does not need to tell them apart: it
//! holds one account per provider for every athlete, so a runner's rate limit
//! is always that provider failing to serve this request — the athlete's own
//! budget is refused by the ingress quota gate before dispatch and never
//! comes back from a runner. The bridge maps every `RateLimit` to
//! `ExternalRateLimited` (an upstream fault: an apology, HTTP 503, an ERROR
//! alert), and embacle's chain under `ResponsePolicy::strict` moves the
//! request to the next tier on it.
//!
//! The chain tests here never push `CHAIN_GUARD` — a propagating error fires
//! no observer hook — so the process-global guard stays in its fail-open
//! state for every test in this binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError,
};
use futures_util::stream;
use pierre_llm::errors::ErrorCode;
use pierre_llm::{ChatMessage, EmbacleProvider, LlmProvider};

/// What a scripted runner answers every call with.
#[derive(Clone)]
enum Script {
    Answer(&'static str),
    RateLimit(&'static str),
}

/// A runner that follows its script and counts how often it was asked.
struct Scripted {
    label: &'static str,
    script: Script,
    calls: Arc<AtomicUsize>,
    models: Vec<String>,
}

impl Scripted {
    fn new(label: &'static str, script: Script) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = Self {
            label,
            script,
            calls: Arc::clone(&calls),
            models: vec![format!("{label}-model")],
        };
        (runner, calls)
    }

    fn outcome(&self) -> Result<ChatResponse, RunnerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.script {
            Script::Answer(text) => Ok(ChatResponse {
                content: (*text).to_owned(),
                model: self.models[0].clone(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            }),
            // The shape embacle's HTTP client and the CLI runners both
            // produce: `RunnerError::rate_limit(service, message)`.
            Script::RateLimit(message) => Err(RunnerError::rate_limit(self.label, *message)),
        }
    }
}

#[async_trait]
impl EmbacleLlmProvider for Scripted {
    fn name(&self) -> &'static str {
        self.label
    }

    fn display_name(&self) -> &str {
        self.label
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

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.outcome()
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        // A 429 refuses the stream before it opens, exactly as the HTTP
        // client does; an answer is a stream this test never consumes.
        self.outcome()
            .map(|_| Box::pin(stream::empty()) as ChatStream)
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        self.outcome().map(|_| true)
    }
}

const VENDOR_MESSAGE: &str = "Quota exceeded for gemini-2.5-pro, try again in 7 seconds";
const QUOTA_MESSAGE: &str = "premium requests exhausted for this billing cycle";

fn request() -> ChatRequest {
    ChatRequest::new(vec![
        ChatMessage::system("agent persona"),
        ChatMessage::user("how did my week go?"),
    ])
}

#[tokio::test]
async fn an_http_tier_answering_429_is_an_upstream_rate_limit() {
    let (gemini, calls) = Scripted::new("gemini", Script::RateLimit(VENDOR_MESSAGE));
    let tier = EmbacleProvider::from_runner(Box::new(gemini), "Google Gemini");

    let err = tier
        .complete(&request())
        .await
        .expect_err("a 429 is an error");

    assert_eq!(err.code, ErrorCode::ExternalRateLimited, "{err}");
    assert!(err.message.contains(VENDOR_MESSAGE), "{err}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// The Copilot runtime refusing on the platform account's spent quota is the
/// same upstream fault: the account is the platform's, not the athlete's.
#[tokio::test]
async fn a_runners_spent_quota_is_an_upstream_rate_limit_too() {
    let (copilot, _) = Scripted::new("copilot-sdk", Script::RateLimit(QUOTA_MESSAGE));
    let tier = EmbacleProvider::from_runner(Box::new(copilot), "GitHub Copilot (SDK)");

    let err = tier
        .complete(&request())
        .await
        .expect_err("a spent quota is an error");

    assert_eq!(
        err.code,
        ErrorCode::ExternalRateLimited,
        "a runner's quota refusal is the provider's account, never the athlete's budget: {err}"
    );
    assert!(err.message.contains(QUOTA_MESSAGE), "{err}");
}

/// The chain's whole point: a primary out of quota hands the turn to the
/// secondary, and the athlete gets the secondary's answer.
#[tokio::test]
async fn a_spent_quota_on_the_primary_falls_through_to_the_secondary() {
    let (copilot, copilot_calls) = Scripted::new("copilot-sdk", Script::RateLimit(QUOTA_MESSAGE));
    let (gemini, gemini_calls) = Scripted::new("gemini", Script::Answer("Tu as couru 42 km."));
    let chain = EmbacleProvider::chain(vec![
        EmbacleProvider::from_runner(Box::new(copilot), "GitHub Copilot (SDK)"),
        EmbacleProvider::from_runner(Box::new(gemini), "Google Gemini"),
    ])
    .expect("two tiers");

    let response = chain
        .complete(&request())
        .await
        .expect("the secondary answers");

    assert_eq!(response.content, "Tu as couru 42 km.");
    assert_eq!(copilot_calls.load(Ordering::SeqCst), 1);
    assert_eq!(gemini_calls.load(Ordering::SeqCst), 1);
}

/// A vendor 429 on the primary moves on the same way: the tier that raised
/// it makes no difference to the chain.
#[tokio::test]
async fn a_vendor_429_on_the_primary_falls_through_to_the_secondary() {
    let (gemini, gemini_calls) = Scripted::new("gemini", Script::RateLimit(VENDOR_MESSAGE));
    let (copilot, copilot_calls) = Scripted::new("copilot-sdk", Script::Answer("served"));
    let chain = EmbacleProvider::chain(vec![
        EmbacleProvider::from_runner(Box::new(gemini), "Google Gemini"),
        EmbacleProvider::from_runner(Box::new(copilot), "GitHub Copilot (SDK)"),
    ])
    .expect("two tiers");

    let response = chain
        .complete(&request())
        .await
        .expect("the secondary answers");

    assert_eq!(response.content, "served");
    assert_eq!(gemini_calls.load(Ordering::SeqCst), 1);
    assert_eq!(copilot_calls.load(Ordering::SeqCst), 1);
}

/// Only a chain with nobody left to ask hands the athlete the code, and it is
/// the last tier's — an upstream fault, not a quota denial.
#[tokio::test]
async fn an_exhausted_chain_reports_the_last_tiers_rate_limit_as_upstream() {
    let (copilot, copilot_calls) = Scripted::new("copilot-sdk", Script::RateLimit(QUOTA_MESSAGE));
    let (gemini, gemini_calls) = Scripted::new("gemini", Script::RateLimit(VENDOR_MESSAGE));
    let chain = EmbacleProvider::chain(vec![
        EmbacleProvider::from_runner(Box::new(copilot), "GitHub Copilot (SDK)"),
        EmbacleProvider::from_runner(Box::new(gemini), "Google Gemini"),
    ])
    .expect("two tiers");

    let err = chain
        .complete(&request())
        .await
        .expect_err("nobody left to ask");

    assert_eq!(err.code, ErrorCode::ExternalRateLimited, "{err}");
    assert!(
        err.message.contains(VENDOR_MESSAGE),
        "the last tier's diagnostic: {err}"
    );
    assert_eq!(copilot_calls.load(Ordering::SeqCst), 1);
    assert_eq!(gemini_calls.load(Ordering::SeqCst), 1);
}
