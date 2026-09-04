// ABOUTME: A tool-calling chain must honour the preemptive guard the same way complete() does
// ABOUTME: Low GitHub headroom routes straight to the secondary without spending a primary call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Copilot's session-token exchange shares the 5000/hr GitHub core pool, so a
//! near-exhausted budget is a strong predictor that the *next* primary call
//! fails with `Authentication required`. `complete()` reads that signal before
//! every request; `complete_with_tools` did not, and spent the failing call
//! anyway.
//!
//! `CHAIN_GUARD` is process-global, so this file holds exactly one test: any
//! second test in the same binary would run against a budget this one has
//! already pushed below threshold, and would be rerouted for reasons it never
//! asked for. The healthy-budget case is covered in `tool_chain_fallback_test`,
//! whose binary never touches the guard's rate-limit state.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use pierre_llm::chain_guard::{RateLimitTransition, CHAIN_GUARD, GITHUB_BUDGET_THRESHOLD};
use pierre_llm::errors::AppError;
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
};

/// Answers with a fixed line and counts how many times it was asked.
struct Counted {
    label: &'static str,
    answer: &'static str,
    calls: Arc<AtomicUsize>,
    models: Vec<String>,
}

impl Counted {
    fn new(label: &'static str, answer: &'static str) -> (Arc<Self>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(Self {
            label,
            answer,
            calls: Arc::clone(&calls),
            models: vec!["test-model".to_owned()],
        });
        (provider, calls)
    }
}

#[async_trait::async_trait]
impl LlmProvider for Counted {
    fn name(&self) -> &'static str {
        self.label
    }

    fn display_name(&self) -> &'static str {
        self.label
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING
    }

    fn default_model(&self) -> &'static str {
        "test-model"
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ChatResponse {
            content: self.answer.to_owned(),
            model: "test-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::invalid_input(
            "the counted provider answers only complete()",
        ))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

#[tokio::test]
async fn a_low_github_budget_skips_the_primary_on_the_tool_calling_path() {
    assert_eq!(
        CHAIN_GUARD.record_github_rate_limit(GITHUB_BUDGET_THRESHOLD - 1, 0),
        RateLimitTransition::EnteredLow,
        "the probe starts from the fail-open unknown state"
    );
    assert!(
        CHAIN_GUARD.should_skip_primary(),
        "the guard must read a sub-threshold budget as a reason to skip"
    );

    let (primary, primary_calls) = Counted::new("primary", "should not be reached");
    let (secondary, secondary_calls) = Counted::new("secondary", "Tu as couru 42 km ce mois-ci.");
    let primary: Arc<dyn LlmProvider> = primary;
    let secondary: Arc<dyn LlmProvider> = secondary;
    let chain = ChatProvider::Chain {
        primary: Box::new(ChatProvider::Custom(primary)),
        secondary: Box::new(ChatProvider::Custom(secondary)),
    };

    let request = ChatRequest::new(vec![
        ChatMessage::system("coach persona"),
        ChatMessage::user("how did my week go?"),
    ]);
    let response = chain
        .complete_with_tools(&request, None)
        .await
        .expect("the secondary answers when the primary is skipped");

    assert_eq!(
        response.content.as_deref(),
        Some("Tu as couru 42 km ce mois-ci."),
        "the reply must come from the secondary"
    );
    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        0,
        "the point of the preemptive skip is that the doomed primary call is never spent"
    );
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 1);
}
