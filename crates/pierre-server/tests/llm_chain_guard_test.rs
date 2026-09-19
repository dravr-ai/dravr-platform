// SPDX-License-Identifier: MIT OR Apache-2.0
// ABOUTME: E2E tests for the global LLM chain guard — preemptive fallback when
// ABOUTME: the GitHub-Models budget is low and circuit-breaking after provider faults.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError, StreamChunk, TokenUsage,
};
use futures_util::stream;
use pierre_llm::chain_guard::{CHAIN_GUARD, CIRCUIT_FAILURE_THRESHOLD, GITHUB_BUDGET_THRESHOLD};
use pierre_llm::{ChatMessage, ChatProvider, EmbacleProvider};
use serial_test::serial;

/// Mock runner with a configurable response and a per-call counter.
///
/// `mode == "ok"` returns success; any other value returns
/// `RunnerError::auth_failure`, which `ErrorKind::is_provider_fault`
/// classifies as a fault the chain moves past (`ResponsePolicy::strict`).
struct MockProvider {
    name: &'static str,
    mode: &'static str,
    calls: Arc<AtomicUsize>,
}

impl MockProvider {
    fn new_ok(name: &'static str) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                name,
                mode: "ok",
                calls: calls.clone(),
            },
            calls,
        )
    }

    fn new_failing(name: &'static str) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                name,
                mode: "fail",
                calls: calls.clone(),
            },
            calls,
        )
    }
}

#[async_trait]
impl EmbacleLlmProvider for MockProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &str {
        self.name
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING
            | LlmCapabilities::FUNCTION_CALLING
            | LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        const NAME: &str = "mock-model";
        NAME
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.mode == "ok" {
            Ok(ChatResponse {
                content: format!("hello from {}", self.name),
                usage: Some(TokenUsage::new(1, 1, 2)),
                model: "mock-model".to_owned(),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        } else {
            Err(RunnerError::auth_failure(format!(
                "{} mock failure (provider fault)",
                self.name
            )))
        }
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let name = self.name.to_owned();
        let s = stream::iter(vec![Ok(StreamChunk {
            delta: format!("hi from {name}"),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        })]);
        Ok(Pin::from(Box::new(s)))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(self.mode == "ok")
    }
}

/// Restore the chain guard to a clean baseline so the next test starts with a
/// closed circuit and an unbounded GitHub budget.
fn reset_chain_guard() {
    CHAIN_GUARD.record_github_rate_limit(u64::MAX, 0);
    // Recording success closes the circuit and resets the failure counter.
    CHAIN_GUARD.record_primary_success();
}

fn epoch_secs_in(offset_secs: i64) -> u64 {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must be after epoch")
        .as_secs();
    let now = i64::try_from(now_secs).unwrap_or(i64::MAX);
    let target = now.saturating_add(offset_secs);
    u64::try_from(target.max(0)).unwrap_or(0)
}

fn tier(runner: impl EmbacleLlmProvider + 'static) -> EmbacleProvider {
    EmbacleProvider::from_runner(Box::new(runner), "mock")
}

fn make_chain(tiers: Vec<EmbacleProvider>) -> ChatProvider {
    ChatProvider::Embacle(EmbacleProvider::chain(tiers).expect("at least one tier"))
}

/// Sentinel recorded by [`ModelCapturingProvider`] before it is invoked.
const MODEL_NOT_CAPTURED: &str = "<not-called>";

/// Sentinel recorded when the captured request carried `model: None`.
const MODEL_CLEARED: &str = "<none>";

/// Secondary mock that records the `model` field of the request it receives,
/// so a test can assert the chain cleared the primary's per-request model
/// override before delegating to the fallback tier. The captured value is
/// flattened to a `String` (using the sentinels above) to avoid a nested
/// `Option<Option<String>>` that `clippy::option_option` rejects.
struct ModelCapturingProvider {
    name: &'static str,
    seen_model: Arc<Mutex<String>>,
}

impl ModelCapturingProvider {
    fn new(name: &'static str) -> (Self, Arc<Mutex<String>>) {
        let seen_model = Arc::new(Mutex::new(MODEL_NOT_CAPTURED.to_owned()));
        (
            Self {
                name,
                seen_model: seen_model.clone(),
            },
            seen_model,
        )
    }

    /// Flatten a request's `model` field to the recorded sentinel/value form.
    fn capture(model: Option<&str>) -> String {
        model.map_or_else(|| MODEL_CLEARED.to_owned(), ToOwned::to_owned)
    }
}

#[async_trait]
impl EmbacleLlmProvider for ModelCapturingProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &str {
        self.name
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING | LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        const NAME: &str = "secondary-default-model";
        NAME
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        *self.seen_model.lock().unwrap() = Self::capture(request.model.as_deref());
        Ok(ChatResponse {
            content: format!("hello from {}", self.name),
            usage: None,
            model: "secondary-default-model".to_owned(),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        *self.seen_model.lock().unwrap() = Self::capture(request.model.as_deref());
        let s = stream::iter(vec![Ok(StreamChunk {
            delta: "hi".to_owned(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        })]);
        Ok(Pin::from(Box::new(s)))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

/// Regression: when the primary fails and the chain falls back, the secondary
/// must NOT receive the primary's per-request model override. The chat pipeline
/// stamps the primary's model (e.g. Copilot's `claude-opus-4.8`) onto every
/// request; forwarding it verbatim made Gemini 404 and Cohere return
/// "model 'claude-opus-4.8' not found", collapsing the whole fallback chain.
/// The secondary must see `model: None` so it uses its own configured model.
#[tokio::test]
#[serial(chain_guard)]
async fn test_fallback_clears_primary_model_override_for_secondary() {
    reset_chain_guard();

    let (primary, _primary_calls) = MockProvider::new_failing("mock-primary-fail");
    let (secondary, seen_model) = ModelCapturingProvider::new("mock-secondary-capture");
    let chain = make_chain(vec![tier(primary), tier(secondary)]);

    // Stamp the primary's model onto the request, exactly as the chat pipeline
    // does via `ChatRequest::with_model(active_model)`.
    let req =
        ChatRequest::new(vec![ChatMessage::user("trigger fallback")]).with_model("claude-opus-4.8");

    let response = chain.complete(&req).await.expect("chain should fall back");
    assert!(
        response.content.contains("mock-secondary-capture"),
        "response should come from the secondary, got: {}",
        response.content
    );

    let captured = seen_model.lock().unwrap().clone();
    assert_ne!(
        captured, MODEL_NOT_CAPTURED,
        "secondary must have been invoked"
    );
    assert_eq!(
        captured, MODEL_CLEARED,
        "secondary must receive model=None (its own default), not the primary's override"
    );

    reset_chain_guard();
}

#[tokio::test]
#[serial(chain_guard)]
async fn test_chain_preemptively_falls_back_when_guard_low() {
    reset_chain_guard();

    // Force the guard's GitHub budget BELOW the skip threshold and into the
    // future window, so the chain decides to skip the primary outright.
    let low_budget = GITHUB_BUDGET_THRESHOLD.saturating_sub(1);
    let reset_at = epoch_secs_in(3_600);
    CHAIN_GUARD.record_github_rate_limit(low_budget, reset_at);
    assert!(
        CHAIN_GUARD.should_skip_primary(),
        "guard should report skip-primary with budget {low_budget} below {GITHUB_BUDGET_THRESHOLD}"
    );

    let (primary, primary_calls) = MockProvider::new_ok("mock-primary");
    let (secondary, secondary_calls) = MockProvider::new_ok("mock-secondary");
    let chain = make_chain(vec![tier(primary), tier(secondary)]);

    let req = ChatRequest::new(vec![ChatMessage::user("hello")]);
    let response = chain.complete(&req).await.expect("chain should succeed");

    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        0,
        "primary must NOT be invoked when guard says skip"
    );
    assert_eq!(
        secondary_calls.load(Ordering::SeqCst),
        1,
        "secondary must serve the request"
    );
    assert!(
        response.content.contains("mock-secondary"),
        "response should come from secondary, got: {}",
        response.content
    );

    reset_chain_guard();
}

#[tokio::test]
#[serial(chain_guard)]
async fn test_chain_opens_circuit_after_provider_faults() {
    reset_chain_guard();
    assert!(
        !CHAIN_GUARD.is_circuit_open(),
        "circuit must start closed after reset"
    );

    let (primary, primary_calls) = MockProvider::new_failing("mock-primary-fail");
    let (secondary, secondary_calls) = MockProvider::new_ok("mock-secondary-ok");
    let chain = make_chain(vec![tier(primary), tier(secondary)]);

    let req = ChatRequest::new(vec![ChatMessage::user("trigger fallback")]);

    // Drive enough primary failures to trip the breaker. Each call should still
    // succeed end-to-end because the secondary is healthy.
    for i in 0..CIRCUIT_FAILURE_THRESHOLD {
        let response = chain
            .complete(&req)
            .await
            .unwrap_or_else(|e| panic!("call {i} should fall back to secondary: {e:?}"));
        assert!(
            response.content.contains("mock-secondary-ok"),
            "call {i} should be served by secondary, got: {}",
            response.content
        );
    }

    assert!(
        CHAIN_GUARD.is_circuit_open(),
        "circuit should be OPEN after {CIRCUIT_FAILURE_THRESHOLD} provider faults"
    );

    let primary_count_when_open = primary_calls.load(Ordering::SeqCst);
    let secondary_count_when_open = secondary_calls.load(Ordering::SeqCst);

    // With the circuit open, the next call must skip the primary entirely.
    let _ = chain
        .complete(&req)
        .await
        .expect("post-open call should be served by secondary");
    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        primary_count_when_open,
        "primary must not be invoked while the circuit is open"
    );
    assert_eq!(
        secondary_calls.load(Ordering::SeqCst),
        secondary_count_when_open + 1,
        "secondary must serve the post-open call"
    );

    reset_chain_guard();
}

/// The guard measures the primary only. In a three-tier chain a low budget
/// skips tier 0 and asks tier 1; tier 2 is not consulted while tier 1 answers.
#[tokio::test]
#[serial(chain_guard)]
async fn test_three_tier_guard_skips_only_the_primary() {
    reset_chain_guard();
    CHAIN_GUARD.record_github_rate_limit(GITHUB_BUDGET_THRESHOLD - 1, epoch_secs_in(3_600));

    let (primary, primary_calls) = MockProvider::new_ok("mock-primary");
    let (secondary, secondary_calls) = MockProvider::new_ok("mock-secondary");
    let (tertiary, tertiary_calls) = MockProvider::new_ok("mock-tertiary");
    let chain = make_chain(vec![tier(primary), tier(secondary), tier(tertiary)]);

    let req = ChatRequest::new(vec![ChatMessage::user("hello")]);
    let response = chain.complete(&req).await.expect("the secondary answers");

    assert!(response.content.contains("mock-secondary"));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 0, "tier 0 skipped");
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 1, "tier 1 asked");
    assert_eq!(
        tertiary_calls.load(Ordering::SeqCst),
        0,
        "tier 2 is not asked while tier 1 answers"
    );

    reset_chain_guard();
}

/// Tier 1's outcomes are not the primary's: its failures do not count toward
/// the breaker and its successes do not close an open circuit.
#[tokio::test]
#[serial(chain_guard)]
async fn test_tier_one_outcomes_do_not_touch_the_breaker() {
    reset_chain_guard();

    let (primary, primary_calls) = MockProvider::new_failing("mock-primary-fail");
    let (secondary, secondary_calls) = MockProvider::new_failing("mock-secondary-fail");
    let (tertiary, tertiary_calls) = MockProvider::new_ok("mock-tertiary-ok");
    let chain = make_chain(vec![tier(primary), tier(secondary), tier(tertiary)]);

    let req = ChatRequest::new(vec![ChatMessage::user("cascade")]);

    // Two rounds: two primary faults and two secondary faults. Were the
    // secondary's counted, four faults would already have opened the circuit.
    for round in 0..CIRCUIT_FAILURE_THRESHOLD - 1 {
        let response = chain
            .complete(&req)
            .await
            .unwrap_or_else(|e| panic!("round {round} should cascade to the tertiary: {e:?}"));
        assert!(response.content.contains("mock-tertiary-ok"));
    }
    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        CIRCUIT_FAILURE_THRESHOLD - 1
    );
    assert_eq!(
        secondary_calls.load(Ordering::SeqCst),
        CIRCUIT_FAILURE_THRESHOLD - 1
    );
    assert_eq!(
        tertiary_calls.load(Ordering::SeqCst),
        CIRCUIT_FAILURE_THRESHOLD - 1
    );
    assert!(
        !CHAIN_GUARD.is_circuit_open(),
        "tier 1 faults must not count toward the primary's breaker"
    );

    // The third primary fault opens it.
    chain
        .complete(&req)
        .await
        .expect("the tertiary still answers");
    assert!(
        CHAIN_GUARD.is_circuit_open(),
        "three primary faults open the circuit"
    );

    // With the circuit open the primary is skipped; the secondary faults and
    // the tertiary answers — and that success, on tier 2, leaves the circuit
    // open. Only a primary success may close it.
    let primary_count_when_open = primary_calls.load(Ordering::SeqCst);
    chain.complete(&req).await.expect("the tertiary answers");
    assert_eq!(
        primary_calls.load(Ordering::SeqCst),
        primary_count_when_open
    );
    assert!(
        CHAIN_GUARD.is_circuit_open(),
        "a later tier's success must not close the primary's circuit"
    );

    reset_chain_guard();
}
