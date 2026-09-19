// ABOUTME: A vendor 429 from an HTTP tier is an upstream throttle (ExternalRateLimited); a CLI runner's RateLimit is the account's quota (RateLimitExceeded)
// ABOUTME: Neither moves a request to the next tier of a chain, so the chain's decisions are the same on both
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! embacle's `RunnerError` carries no origin: Gemini answering 429 and the
//! Copilot CLI refusing on an exhausted subscription both arrive as
//! `ErrorKind::RateLimit`. The platform tells them apart by how the tier was
//! built — `from_http_runner` marks a vendor throttle, `from_runner` leaves a
//! quota alone — and the codes matter: the messaging ingress reads
//! `RateLimitExceeded` as the athlete's own budget refusing the turn (a quota
//! denial, WARN, HTTP 429) and `ExternalRateLimited` as an upstream fault (an
//! apology, HTTP 503).
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
async fn an_http_tier_answering_429_is_an_upstream_throttle() {
    let (gemini, _) = Scripted::new("gemini", Script::RateLimit(VENDOR_MESSAGE));
    let tier = EmbacleProvider::from_http_runner(Box::new(gemini), "Google Gemini");

    let err = tier
        .complete(&request())
        .await
        .expect_err("a 429 is an error");

    assert_eq!(
        err.code,
        ErrorCode::ExternalRateLimited,
        "a vendor throttling the platform's key is not the athlete's quota: {err}"
    );
    assert!(
        err.message.contains(VENDOR_MESSAGE),
        "the vendor's message and the wait it asked for must survive: {err}"
    );
    assert!(
        err.message.contains("gemini:"),
        "the vendor that throttled must be named: {err}"
    );
}

#[tokio::test]
async fn an_http_tier_refusing_a_stream_is_an_upstream_throttle_too() {
    let (groq, _) = Scripted::new("groq", Script::RateLimit(VENDOR_MESSAGE));
    let tier = EmbacleProvider::from_http_runner(Box::new(groq), "Groq");

    let err = tier
        .complete_stream(&request())
        .await
        .err()
        .expect("a 429 refuses the stream before it opens");

    assert_eq!(err.code, ErrorCode::ExternalRateLimited, "{err}");
    assert!(err.message.contains(VENDOR_MESSAGE), "{err}");
}

#[tokio::test]
async fn an_http_tier_passes_every_other_outcome_through() {
    let (cohere, calls) = Scripted::new("cohere", Script::Answer("Tu as couru 42 km."));
    let tier = EmbacleProvider::from_http_runner(Box::new(cohere), "Cohere");

    assert_eq!(tier.name(), "cohere");
    assert_eq!(tier.default_model(), "cohere-model");
    let response = tier.complete(&request()).await.expect("the answer");
    assert_eq!(response.content, "Tu as couru 42 km.");
    assert!(tier.health_check().await.expect("healthy"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_cli_runner_rate_limit_is_the_accounts_quota() {
    let (copilot, _) = Scripted::new("copilot-acp", Script::RateLimit(QUOTA_MESSAGE));
    let tier = EmbacleProvider::from_runner(Box::new(copilot), "GitHub Copilot (Headless)");

    let err = tier
        .complete(&request())
        .await
        .expect_err("an exhausted subscription is an error");

    assert_eq!(
        err.code,
        ErrorCode::RateLimitExceeded,
        "a CLI runner's quota refusal keeps meaning the account's quota: {err}"
    );
    assert!(err.message.contains(QUOTA_MESSAGE), "{err}");
}

/// A vendor 429 on the primary propagates as an upstream throttle; the
/// secondary is never asked.
#[tokio::test]
async fn a_vendor_429_does_not_fall_through_the_chain() {
    let (gemini, gemini_calls) = Scripted::new("gemini", Script::RateLimit(VENDOR_MESSAGE));
    let (copilot, copilot_calls) = Scripted::new("copilot-acp", Script::Answer("unreached"));
    let chain = EmbacleProvider::chain(vec![
        EmbacleProvider::from_http_runner(Box::new(gemini), "Google Gemini"),
        EmbacleProvider::from_runner(Box::new(copilot), "GitHub Copilot (Headless)"),
    ])
    .expect("two tiers");

    let err = chain
        .complete(&request())
        .await
        .expect_err("the throttle propagates");

    assert_eq!(err.code, ErrorCode::ExternalRateLimited, "{err}");
    assert!(err.message.contains(VENDOR_MESSAGE), "{err}");
    assert_eq!(gemini_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        copilot_calls.load(Ordering::SeqCst),
        0,
        "a rate limit is not a provider fault: the chain must not spend the secondary on it"
    );
}

/// A quota refusal on the primary propagates as the account's quota; the
/// secondary is never asked. Same chain decision as the vendor case.
#[tokio::test]
async fn a_quota_refusal_does_not_fall_through_the_chain() {
    let (copilot, copilot_calls) = Scripted::new("copilot-acp", Script::RateLimit(QUOTA_MESSAGE));
    let (gemini, gemini_calls) = Scripted::new("gemini", Script::Answer("unreached"));
    let chain = EmbacleProvider::chain(vec![
        EmbacleProvider::from_runner(Box::new(copilot), "GitHub Copilot (Headless)"),
        EmbacleProvider::from_http_runner(Box::new(gemini), "Google Gemini"),
    ])
    .expect("two tiers");

    let err = chain
        .complete(&request())
        .await
        .expect_err("the refusal propagates");

    assert_eq!(err.code, ErrorCode::RateLimitExceeded, "{err}");
    assert!(err.message.contains(QUOTA_MESSAGE), "{err}");
    assert_eq!(copilot_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        gemini_calls.load(Ordering::SeqCst),
        0,
        "a quota refusal is not a provider fault: the chain must not spend the secondary on it"
    );
}

/// The origin survives the chain: a throttle on the *secondary* — reached
/// after the primary faulted — still classifies by the tier that raised it.
#[tokio::test]
async fn a_vendor_429_on_the_secondary_keeps_its_origin() {
    let (copilot, _) = Scripted::new("copilot-acp", Script::Answer(""));
    let (gemini, _) = Scripted::new("gemini", Script::RateLimit(VENDOR_MESSAGE));
    let chain = EmbacleProvider::chain(vec![
        EmbacleProvider::from_runner(Box::new(copilot), "GitHub Copilot (Headless)"),
        EmbacleProvider::from_http_runner(Box::new(gemini), "Google Gemini"),
    ])
    .expect("two tiers");

    // An empty completion falls through under the strict policy, so the
    // chain reaches Gemini, whose 429 is the chain's answer.
    let err = chain
        .complete(&request())
        .await
        .expect_err("the secondary's throttle propagates");

    assert_eq!(err.code, ErrorCode::ExternalRateLimited, "{err}");
    assert!(err.message.contains(VENDOR_MESSAGE), "{err}");
}
