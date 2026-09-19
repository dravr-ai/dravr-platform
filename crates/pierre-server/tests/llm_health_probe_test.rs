// SPDX-License-Identifier: MIT OR Apache-2.0
// ABOUTME: E2E tests for a chained EmbacleProvider's health_check — any tier healthy
// ABOUTME: reads healthy, otherwise the last tier's outcome (and its error) propagates.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::pin::Pin;

use async_trait::async_trait;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError, StreamChunk,
};
use futures_util::stream;
use pierre_llm::{ChatProvider, EmbacleProvider};

/// Mock whose `health_check()` returns a configured value (or error) and whose
/// `complete()` is unused but trait-required.
struct ProbeMock {
    name: &'static str,
    health: Result<bool, &'static str>,
}

impl ProbeMock {
    const fn healthy(name: &'static str) -> Self {
        Self {
            name,
            health: Ok(true),
        }
    }

    const fn unhealthy(name: &'static str) -> Self {
        Self {
            name,
            health: Ok(false),
        }
    }

    const fn errors(name: &'static str, msg: &'static str) -> Self {
        Self {
            name,
            health: Err(msg),
        }
    }
}

#[async_trait]
impl EmbacleLlmProvider for ProbeMock {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &str {
        self.name
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        self.name
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        Err(RunnerError::internal(
            "probe mock does not implement complete",
        ))
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        let s = stream::iter(vec![Ok(StreamChunk {
            delta: String::new(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        })]);
        Ok(Pin::from(Box::new(s)))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        match self.health {
            Ok(v) => Ok(v),
            Err(msg) => Err(RunnerError::external_service("probe-mock", msg)),
        }
    }
}

fn chain(primary: ProbeMock, secondary: ProbeMock) -> ChatProvider {
    let tiers = vec![
        EmbacleProvider::from_runner(Box::new(primary), "probe primary"),
        EmbacleProvider::from_runner(Box::new(secondary), "probe secondary"),
    ];
    ChatProvider::Embacle(EmbacleProvider::chain(tiers).expect("two tiers"))
}

#[tokio::test]
async fn test_chain_health_returns_true_when_primary_healthy() {
    let cp = chain(
        ProbeMock::healthy("primary"),
        ProbeMock::unhealthy("secondary"),
    );
    let result = cp
        .health_check()
        .await
        .expect("health_check should not error");
    assert!(result, "chain must report healthy when primary is healthy");
}

#[tokio::test]
async fn test_chain_falls_through_to_secondary_when_primary_errors() {
    // Primary returns an outright error (e.g. network timeout) — chain should
    // not propagate that failure as long as secondary is reachable.
    let cp = chain(
        ProbeMock::errors("primary", "simulated unreachable host"),
        ProbeMock::healthy("secondary"),
    );
    let result = cp
        .health_check()
        .await
        .expect("chain should mask primary error when secondary is healthy");
    assert!(result, "chain should report healthy via secondary");
}

#[tokio::test]
async fn test_chain_reports_unhealthy_when_both_sides_down() {
    let cp = chain(
        ProbeMock::unhealthy("primary"),
        ProbeMock::unhealthy("secondary"),
    );
    let result = cp
        .health_check()
        .await
        .expect("health_check should resolve, not panic");
    assert!(
        !result,
        "chain must report unhealthy when both providers are down"
    );
}

#[tokio::test]
async fn test_chain_propagates_secondary_error_when_both_fail() {
    // Primary returns false (not Err); secondary errors. The chain must
    // surface the last tier's error verbatim — it is the diagnostic the
    // `llm.provider_unhealthy` event carries — rather than a bare `false`.
    let cp = chain(
        ProbeMock::unhealthy("primary"),
        ProbeMock::errors("secondary", "secondary unreachable"),
    );
    let err = cp
        .health_check()
        .await
        .expect_err("chain must propagate the secondary's error when the primary is unhealthy");
    assert!(
        err.message.contains("secondary unreachable"),
        "the last tier's diagnostic must survive: {}",
        err.message
    );
}
