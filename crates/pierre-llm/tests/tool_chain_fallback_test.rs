// ABOUTME: The fallback chain must make the same decisions on the tool-calling path as on complete()
// ABOUTME: A retryable primary error, and an empty tool-calling completion, both reach the secondary
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `ChatProvider::Chain::complete_with_tools` used to match `Ok(response)`
//! unconditionally and never touch `CHAIN_GUARD`. So on a native
//! function-calling primary — the whole API tool-loop class, and the only
//! caller of this method — an `Ok` carrying no content and no function calls
//! took the success path and became the lost-turn apology, exactly the failure
//! `fallback_policy` exists to prevent, while the working secondary sat unused.
//!
//! These drive the real chain through `ChatProvider::Custom` primaries so the
//! decision is observed where it is made, not restated. The negative cases are
//! load-bearing: a deterministic error must NOT be rerouted (the caller would
//! see the secondary's version of the same rejection instead of the real
//! diagnostic), and an empty answer that carries function calls is an ordinary
//! mid-loop turn — falling that back would spend a paid completion on every
//! tool-using turn in every conversation.
//!
//! The circuit breaker's preemptive skip is process-global state, so it is
//! pinned in its own binary (`tool_chain_preemptive_skip_test.rs`) rather than
//! here, where setting it would reroute every other test in the file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use embacle::ToolCallRequest;
use pierre_llm::errors::AppError;
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
};
use serde_json::json;

/// What a scripted provider does when asked to complete.
enum Scripted {
    /// Answer with this text and no tool calls.
    Says(&'static str),
    /// Answer with no content but a `get_activities` call — an ordinary
    /// mid-loop turn.
    CallsATool,
    /// Answer `Ok` with nothing at all: no content, no tool calls.
    Empty,
    /// Fail with an error the fallback policy classifies as retryable.
    RetryableError,
    /// Fail with a deterministic error the policy refuses to reroute.
    DeterministicError,
}

/// A provider whose every call is decided in advance and counted.
struct Fake {
    label: &'static str,
    script: Scripted,
    calls: Arc<AtomicUsize>,
    models: Vec<String>,
}

impl Fake {
    fn new(label: &'static str, script: Scripted) -> (Arc<Self>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(Self {
            label,
            script,
            calls: Arc::clone(&calls),
            models: vec!["test-model".to_owned()],
        });
        (provider, calls)
    }
}

#[async_trait::async_trait]
impl LlmProvider for Fake {
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
        let reply = |content: &str, tool_calls: Option<Vec<ToolCallRequest>>| ChatResponse {
            content: content.to_owned(),
            model: "test-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls,
        };
        match self.script {
            Scripted::Says(text) => Ok(reply(text, None)),
            Scripted::CallsATool => Ok(reply(
                "",
                Some(vec![ToolCallRequest {
                    id: "call_1".to_owned(),
                    function_name: "get_activities".to_owned(),
                    arguments: json!({"limit": 10}),
                }]),
            )),
            Scripted::Empty => Ok(reply("", None)),
            Scripted::RetryableError => Err(AppError::external_service(
                self.label,
                "upstream returned 503",
            )),
            Scripted::DeterministicError => {
                Err(AppError::invalid_input("the request body was malformed"))
            }
        }
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::invalid_input(
            "the scripted provider answers only complete()",
        ))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

fn chain(primary: Arc<Fake>, secondary: Arc<Fake>) -> ChatProvider {
    let primary: Arc<dyn LlmProvider> = primary;
    let secondary: Arc<dyn LlmProvider> = secondary;
    ChatProvider::Chain {
        primary: Box::new(ChatProvider::Custom(primary)),
        secondary: Box::new(ChatProvider::Custom(secondary)),
    }
}

fn a_turn() -> ChatRequest {
    ChatRequest::new(vec![
        ChatMessage::system("coach persona"),
        ChatMessage::user("how did my week go?"),
    ])
    .with_model("primary-only-model")
}

/// The headline of carnet#274: the tool-calling path had only this decision,
/// and it is the one that must keep working after the other two are added.
#[tokio::test]
async fn a_retryable_primary_error_falls_through_to_the_secondary() {
    let (primary, primary_calls) = Fake::new("primary", Scripted::RetryableError);
    let (secondary, secondary_calls) = Fake::new("secondary", Scripted::Says("42 km this week."));

    let response = chain(primary, secondary)
        .complete_with_tools(&a_turn(), None)
        .await
        .expect("a retryable primary error must be answered by the secondary");

    assert_eq!(
        response.content.as_deref(),
        Some("42 km this week."),
        "the athlete must receive the secondary's answer, not the primary's error"
    );
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        secondary_calls.load(Ordering::SeqCst),
        1,
        "the secondary is consulted exactly once"
    );
}

/// The defect itself. A 200 carrying neither content nor function calls used to
/// return as an answered turn; the API tool loop then delivers
/// `content.unwrap_or_default()` — the empty string — as the reply.
#[tokio::test]
async fn an_empty_tool_calling_completion_falls_through_to_the_secondary() {
    let (primary, primary_calls) = Fake::new("primary", Scripted::Empty);
    let (secondary, secondary_calls) =
        Fake::new("secondary", Scripted::Says("Tu as couru 42 km ce mois-ci."));

    let response = chain(primary, secondary)
        .complete_with_tools(&a_turn(), None)
        .await
        .expect("an empty primary completion must be reissued against the secondary");

    assert_eq!(
        response.content.as_deref(),
        Some("Tu as couru 42 km ce mois-ci."),
        "an Ok with nothing in it is a lost turn, not a success"
    );
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 1);
}

/// The critical negative case. Empty content WITH function calls is how a model
/// asks for a tool; rerouting it would fall back on every tool-using turn.
#[tokio::test]
async fn an_empty_answer_carrying_function_calls_stays_with_the_primary() {
    let (primary, primary_calls) = Fake::new("primary", Scripted::CallsATool);
    let (secondary, secondary_calls) = Fake::new("secondary", Scripted::Says("should not be used"));

    let response = chain(primary, secondary)
        .complete_with_tools(&a_turn(), None)
        .await
        .expect("a tool-call turn is a success");

    let calls = response
        .function_calls
        .expect("the primary's function calls must survive the chain");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_activities");
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        secondary_calls.load(Ordering::SeqCst),
        0,
        "falling back on a tool-call turn would spend a paid completion on the common case"
    );
}

/// A deterministic failure must surface as itself. Rerouting it hides the real
/// diagnostic behind the secondary's version of the same rejection.
#[tokio::test]
async fn a_deterministic_primary_error_is_not_rerouted() {
    let (primary, primary_calls) = Fake::new("primary", Scripted::DeterministicError);
    let (secondary, secondary_calls) = Fake::new("secondary", Scripted::Says("should not be used"));

    let error = chain(primary, secondary)
        .complete_with_tools(&a_turn(), None)
        .await
        .expect_err("an invalid-input error is not retryable");

    assert!(
        error.to_string().contains("malformed"),
        "the caller must see the primary's own diagnostic, got: {error}"
    );
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 0);
}

/// A healthy primary answers, and the secondary is never woken.
#[tokio::test]
async fn a_primary_that_answers_is_the_answer() {
    let (primary, primary_calls) = Fake::new("primary", Scripted::Says("Rest today."));
    let (secondary, secondary_calls) = Fake::new("secondary", Scripted::Says("should not be used"));

    let response = chain(primary, secondary)
        .complete_with_tools(&a_turn(), None)
        .await
        .expect("a healthy primary answers");

    assert_eq!(response.content.as_deref(), Some("Rest today."));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 0);
}
