// ABOUTME: Pins the `llm.request` span an EmbacleProvider opens around every outbound call
// ABOUTME: The span carries the head provider, the resolved model, the operation, its outcome and tokens
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! embacle's HTTP providers take a plain `reqwest::Client`, so no client
//! middleware traces the outbound LLM call. `EmbacleProvider` opens the span
//! itself; this is the only place the trace tree learns which provider and
//! model answered a turn — and what it cost in tokens — so its name and
//! fields are pinned here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use async_trait::async_trait;
use embacle::types::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities,
    LlmProvider as EmbacleLlmProvider, RunnerError, TokenUsage,
};
use pierre_llm::{EmbacleProvider, LlmProvider, QuotaCooldowns};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::instrument::WithSubscriber;
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

use crate::chain_guard_lock::fresh_chain_guard;

/// A scripted runner whose `complete()` answers without any I/O.
struct Scripted {
    models: Vec<String>,
}

impl Scripted {
    fn new() -> Self {
        Self {
            models: vec!["scripted-model".to_owned()],
        }
    }
}

#[async_trait]
impl EmbacleLlmProvider for Scripted {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn display_name(&self) -> &'static str {
        "Scripted"
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
        Ok(ChatResponse {
            content: "answered".to_owned(),
            model: request
                .model
                .clone()
                .unwrap_or_else(|| self.models[0].clone()),
            usage: Some(TokenUsage::new(12, 34, 46)),
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

/// A runner whose every call fails, standing for a dead chain head.
struct Failing {
    models: Vec<String>,
}

#[async_trait]
impl EmbacleLlmProvider for Failing {
    fn name(&self) -> &'static str {
        "failing"
    }

    fn display_name(&self) -> &'static str {
        "Failing"
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
        Err(RunnerError::internal("head is down"))
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("head is down"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(false)
    }
}

/// Every span created while the layer is installed: its id, name and fields,
/// including the ones recorded after it opened.
type Recorded = Arc<Mutex<Vec<(Id, String, BTreeMap<String, String>)>>>;

struct SpanRecorder {
    spans: Recorded,
}

struct FieldCollector<'a>(&'a mut BTreeMap<String, String>);

impl Visit for FieldCollector<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

impl<S: Subscriber> Layer<S> for SpanRecorder {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        let mut fields = BTreeMap::new();
        attrs.record(&mut FieldCollector(&mut fields));
        self.spans
            .lock()
            .unwrap()
            .push((id.clone(), attrs.metadata().name().to_owned(), fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut spans = self.spans.lock().unwrap();
        if let Some((_, _, fields)) = spans.iter_mut().rev().find(|(span_id, _, _)| span_id == id) {
            values.record(&mut FieldCollector(fields));
        }
    }
}

/// Runs `call` under a fresh recording subscriber and returns the
/// `llm.request` spans it opened.
async fn llm_request_spans<F, Fut>(call: F) -> Vec<BTreeMap<String, String>>
where
    F: FnOnce(EmbacleProvider) -> Fut,
    Fut: Future<Output = ()>,
{
    let provider = EmbacleProvider::from_runner(Box::new(Scripted::new()), "Scripted");
    llm_request_spans_of(provider, call).await
}

/// Runs `call` on `provider` under a fresh recording subscriber and returns
/// the `llm.request` spans it opened.
async fn llm_request_spans_of<F, Fut>(
    provider: EmbacleProvider,
    call: F,
) -> Vec<BTreeMap<String, String>>
where
    F: FnOnce(EmbacleProvider) -> Fut,
    Fut: Future<Output = ()>,
{
    let spans: Recorded = Arc::default();
    let subscriber = Registry::default().with(SpanRecorder {
        spans: Arc::clone(&spans),
    });
    call(provider).with_subscriber(subscriber).await;
    let recorded = spans.lock().unwrap();
    recorded
        .iter()
        .filter(|(_, name, _)| name == "llm.request")
        .map(|(_, _, fields)| fields.clone())
        .collect()
}

#[tokio::test]
async fn complete_opens_one_llm_request_span_naming_provider_and_default_model() {
    let spans = llm_request_spans(|provider| async move {
        let request = ChatRequest::new(vec![ChatMessage::user("hi")]);
        let response = provider.complete(&request).await.expect("scripted answer");
        assert_eq!(response.content, "answered");
    })
    .await;

    assert_eq!(spans.len(), 1, "one outbound call, one llm.request span");
    assert_eq!(spans[0]["provider"], "scripted");
    assert_eq!(spans[0]["model"], "scripted-model");
    assert_eq!(spans[0]["op"], "complete");
}

#[tokio::test]
async fn span_reports_the_model_the_request_carries() {
    let spans = llm_request_spans(|provider| async move {
        let mut request = ChatRequest::new(vec![ChatMessage::user("hi")]);
        request.model = Some("override-model".to_owned());
        provider.complete(&request).await.expect("scripted answer");
    })
    .await;

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0]["model"], "override-model");
}

#[tokio::test]
async fn health_check_opens_a_span_without_a_model() {
    let spans = llm_request_spans(|provider| async move {
        assert!(provider.health_check().await.expect("scripted probe"));
    })
    .await;

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0]["provider"], "scripted");
    assert_eq!(spans[0]["op"], "health_check");
    assert!(!spans[0].contains_key("model"));
}

#[tokio::test]
async fn complete_records_outcome_and_token_usage_on_the_span() {
    let spans = llm_request_spans(|provider| async move {
        let request = ChatRequest::new(vec![ChatMessage::user("hi")]);
        provider.complete(&request).await.expect("scripted answer");
    })
    .await;

    assert_eq!(spans.len(), 1, "one call, one llm.request span");
    assert_eq!(spans[0]["outcome"], "ok", "bug: an answered call is ok");
    assert_eq!(
        spans[0]["prompt_tokens"], "12",
        "bug: prompt tokens not recorded"
    );
    assert_eq!(
        spans[0]["completion_tokens"], "34",
        "bug: completion tokens not recorded"
    );
    // A solo runner is no chain: no tier reports itself, so none is recorded.
    assert!(
        !spans[0].contains_key("served_provider"),
        "bug: a solo runner reports no served tier"
    );
    assert!(
        !spans[0].contains_key("fallback_position"),
        "bug: a solo runner reports no fallback position"
    );
}

#[tokio::test]
async fn a_stream_that_fails_to_open_records_an_error_outcome() {
    let spans = llm_request_spans(|provider| async move {
        let request = ChatRequest::new(vec![ChatMessage::user("hi")]);
        assert!(provider.complete_stream(&request).await.is_err());
    })
    .await;

    assert_eq!(spans.len(), 1, "one call, one llm.request span");
    assert_eq!(
        spans[0]["op"], "complete_stream",
        "bug: wrong op on the span"
    );
    assert_eq!(
        spans[0]["outcome"], "error",
        "bug: a stream that failed to open is an error"
    );
    assert!(
        !spans[0].contains_key("prompt_tokens"),
        "bug: a failed call bills no tokens"
    );
}

#[tokio::test]
async fn a_fallback_records_the_tier_that_served_the_call() {
    let _chain_guard = fresh_chain_guard().await;
    let head = EmbacleProvider::from_runner(
        Box::new(Failing {
            models: vec!["failing-model".to_owned()],
        }),
        "Failing",
    );
    let tail = EmbacleProvider::from_runner(Box::new(Scripted::new()), "Scripted");
    let chain =
        EmbacleProvider::chain(vec![head, tail], QuotaCooldowns::default()).expect("two tiers");

    let spans = llm_request_spans_of(chain, |provider| async move {
        let request = ChatRequest::new(vec![ChatMessage::user("hi")]);
        provider.complete(&request).await.expect("the tail answers");
    })
    .await;

    assert_eq!(
        spans.len(),
        1,
        "the chain is one call, one llm.request span"
    );
    assert_eq!(spans[0]["provider"], "failing", "named after the head");
    assert_eq!(
        spans[0]["served_provider"], "scripted",
        "bug: the tail served the call"
    );
    assert_eq!(
        spans[0]["fallback_position"], "1",
        "bug: the tail is position 1"
    );
    assert_eq!(
        spans[0]["outcome"], "ok",
        "bug: a call a fallback answered is ok"
    );
}
