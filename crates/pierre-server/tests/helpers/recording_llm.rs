// ABOUTME: A model that records every request it is sent and answers from a script the test sets
// ABOUTME: For tests that assert on what a turn puts on the wire rather than on the source that builds it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]

//! What a turn sent, in call order.
//!
//! A turn makes more than one model call: the coaching completion, a re-ask
//! after a withheld reply, and the background passes it spawns — memory
//! extraction, advice capture. They all reach the one provider the test wired,
//! so each recorded request keeps both halves — the system text and everything
//! else — and the test picks the call it means by what that call carries.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
    TokenUsage,
};
use tokio::time::{sleep, Instant};

/// One request as the model received it.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// Every system message, joined in order.
    pub system: String,
    /// Every non-system message, joined in order.
    pub conversation: String,
}

/// Records every completion request and answers from a queue, falling back to
/// a standing reply once the queue is empty.
pub struct RecordingProvider {
    requests: Mutex<Vec<RecordedRequest>>,
    queued: Mutex<VecDeque<String>>,
    standing: Mutex<String>,
}

impl RecordingProvider {
    /// A provider that answers every call with `standing` until told otherwise.
    pub fn answering(standing: &str) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            queued: Mutex::new(VecDeque::new()),
            standing: Mutex::new(standing.to_owned()),
        }
    }

    /// Replace the reply given once the queue is empty.
    pub fn answer_with(&self, reply: &str) {
        reply.clone_into(&mut self.standing.lock().unwrap());
    }

    /// Answer the next call with `reply`, ahead of the standing one.
    pub fn answer_next_with(&self, reply: &str) {
        self.queued.lock().unwrap().push_back(reply.to_owned());
    }

    /// How many requests have been recorded.
    pub fn calls_so_far(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Every request recorded from call `from` on.
    pub fn requests_since(&self, from: usize) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .skip(from)
            .cloned()
            .collect()
    }

    /// The system prompt of the first call after `from` carrying `marker`.
    pub fn system_prompt_since(&self, from: usize, marker: &str) -> String {
        self.requests_since(from)
            .into_iter()
            .find(|request| request.system.contains(marker))
            .map_or_else(
                || panic!("no call carrying {marker:?} was made after call #{from}"),
                |request| request.system,
            )
    }

    /// The first request after `from` that `wanted` accepts, waiting up to
    /// `patience` for a detached task to make it.
    pub async fn request_matching(
        &self,
        from: usize,
        patience: Duration,
        wanted: impl Fn(&RecordedRequest) -> bool,
    ) -> Option<RecordedRequest> {
        let deadline = Instant::now() + patience;
        loop {
            if let Some(found) = self.requests_since(from).into_iter().find(&wanted) {
                return Some(found);
            }
            if Instant::now() >= deadline {
                return None;
            }
            sleep(Duration::from_millis(20)).await;
        }
    }
}

#[async_trait]
impl LlmProvider for RecordingProvider {
    fn name(&self) -> &'static str {
        "recording_mock"
    }
    fn display_name(&self) -> &'static str {
        "Recording Mock LLM"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let joined = |system: bool| {
            request
                .messages
                .iter()
                .filter(|m| (m.role == MessageRole::System) == system)
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.requests.lock().unwrap().push(RecordedRequest {
            system: joined(true),
            conversation: joined(false),
        });
        let queued = self.queued.lock().unwrap().pop_front();
        let content = queued.unwrap_or_else(|| self.standing.lock().unwrap().clone());
        Ok(ChatResponse {
            content,
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(25, 15, 40)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: String::new(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}
