// ABOUTME: A chain passing over a head inside its quota cooldown must not page the fallthrough alert
// ABOUTME: The refusal that started the window was already reported; the skip only logs an info line
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The tool loops and the chain share one `QuotaCooldowns` book, so a single
//! quota refusal on a tool turn puts the head in a window every later
//! `complete()` / `complete_stream()` through the chain reads. embacle reports
//! each of those skips to the observer as `Skipped(QUOTA_COOLDOWN_SKIP)`. The
//! dravr-llm-chain-fallthrough alert counts the "Runtime LLM fallback engaged"
//! line, so emitting the `embacle.fallback_triggered` notify event on that
//! skip would page on every chain call for the whole window.
//!
//! `CHAIN_GUARD` is process-global; the test holds `fresh_chain_guard`, which
//! puts it back to its fail-open state first, so the observer's own veto never
//! fires here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError,
};
use pierre_llm::{ChatMessage, ChatProvider, EmbacleProvider, QuotaCooldowns};
use pierre_test_support::log_capture;

use crate::chain_guard_lock::fresh_chain_guard;

/// The notify event the fallthrough alert pages on.
const FALLBACK_TRIGGERED: &str = "embacle.fallback_triggered";

/// A tier that answers, or fails with a provider fault once told to.
struct Switchable {
    label: &'static str,
    fails: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    models: Vec<String>,
}

impl Switchable {
    fn new(label: &'static str) -> (Self, Arc<AtomicBool>, Arc<AtomicUsize>) {
        let fails = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let tier = Self {
            label,
            fails: Arc::clone(&fails),
            calls: Arc::clone(&calls),
            models: vec!["test-model".to_owned()],
        };
        (tier, fails, calls)
    }
}

#[async_trait]
impl EmbacleLlmProvider for Switchable {
    fn name(&self) -> &'static str {
        self.label
    }

    fn display_name(&self) -> &str {
        self.label
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fails.load(Ordering::SeqCst) {
            return Err(RunnerError::external_service(self.label, "503 upstream"));
        }
        Ok(ChatResponse {
            replay_blocks: None,
            content: format!("from {}", self.label),
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

/// Ask `chain` once under a fresh capture; return the reply and every event
/// the call emitted.
async fn ask(chain: &ChatProvider) -> (String, Vec<(String, BTreeMap<String, String>)>) {
    let request = ChatRequest::new(vec![ChatMessage::user("how did my week go?")]);
    let (captured, capturing) = log_capture::capture_events();
    let response = chain
        .complete(&request)
        .await
        .expect("the secondary answers");
    drop(capturing);
    let recorded = captured
        .drain()
        .into_iter()
        .map(|event| (event.target, event.fields.into_iter().collect()))
        .collect();
    (response.content, recorded)
}

/// Whether `events` holds the paging fallthrough notify event.
fn pages(events: &[(String, BTreeMap<String, String>)]) -> bool {
    events.iter().any(|(target, fields)| {
        target == "notify" && fields.get("event").map(String::as_str) == Some(FALLBACK_TRIGGERED)
    })
}

#[tokio::test]
async fn a_head_inside_its_quota_cooldown_is_skipped_without_paging() {
    let _chain_guard = fresh_chain_guard().await;
    let (primary, primary_fails, primary_calls) = Switchable::new("primary");
    let (secondary, _, secondary_calls) = Switchable::new("secondary");
    let book = QuotaCooldowns::default();
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(
            vec![
                EmbacleProvider::from_runner(Box::new(primary), "primary"),
                EmbacleProvider::from_runner(Box::new(secondary), "secondary"),
            ],
            book.clone(),
        )
        .expect("two tiers"),
    );

    // A tool turn's quota refusal on the head, recorded in the shared book.
    book.start("primary");
    let (content, events) = ask(&chain).await;
    assert_eq!(content, "from secondary");
    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        0,
        "a head inside its cooldown is not asked"
    );
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 1);
    assert!(
        events.iter().any(|(_, fields)| fields
            .get("message")
            .is_some_and(|m| m.contains("quota cooldown"))),
        "the skip is still logged: {events:?}"
    );
    assert!(
        !pages(&events),
        "a cooldown skip must not emit {FALLBACK_TRIGGERED}: {events:?}"
    );

    // Control: a provider fault on the head still pages, so the recorder sees
    // the event when it is emitted.
    book.end("primary");
    primary_fails.store(true, Ordering::SeqCst);
    let (content, events) = ask(&chain).await;
    assert_eq!(content, "from secondary");
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert!(
        pages(&events),
        "a provider fault on the head pages: {events:?}"
    );
}
