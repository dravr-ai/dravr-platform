// ABOUTME: Pins the `llm.request` span an EmbacleProvider opens around every outbound call
// ABOUTME: The span carries the head provider, the resolved model and the operation name
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! embacle's HTTP providers take a plain `reqwest::Client`, so no client
//! middleware traces the outbound LLM call. `EmbacleProvider` opens the span
//! itself; this is the only place the trace tree learns which provider and
//! model answered a turn, so its name and fields are pinned here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use async_trait::async_trait;
use embacle::types::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities,
    LlmProvider as EmbacleLlmProvider, RunnerError,
};
use pierre_llm::{EmbacleProvider, LlmProvider};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::instrument::WithSubscriber;
use tracing::span::{Attributes, Id};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

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
            usage: None,
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

/// Every span created while the layer is installed: name plus its fields.
type Recorded = Arc<Mutex<Vec<(String, BTreeMap<String, String>)>>>;

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
}

impl<S: Subscriber> Layer<S> for SpanRecorder {
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        let mut fields = BTreeMap::new();
        attrs.record(&mut FieldCollector(&mut fields));
        self.spans
            .lock()
            .unwrap()
            .push((attrs.metadata().name().to_owned(), fields));
    }
}

/// Runs `call` under a fresh recording subscriber and returns the
/// `llm.request` spans it opened.
async fn llm_request_spans<F, Fut>(call: F) -> Vec<BTreeMap<String, String>>
where
    F: FnOnce(EmbacleProvider) -> Fut,
    Fut: Future<Output = ()>,
{
    let spans: Recorded = Arc::default();
    let subscriber = Registry::default().with(SpanRecorder {
        spans: Arc::clone(&spans),
    });
    let provider = EmbacleProvider::from_runner(Box::new(Scripted::new()), "Scripted");
    call(provider).with_subscriber(subscriber).await;
    let recorded = spans.lock().unwrap();
    recorded
        .iter()
        .filter(|(name, _)| name == "llm.request")
        .map(|(_, fields)| fields.clone())
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
