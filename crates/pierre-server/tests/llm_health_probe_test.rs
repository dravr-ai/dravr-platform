// SPDX-License-Identifier: MIT OR Apache-2.0
// ABOUTME: E2E tests for ChatProvider::Chain::health_check — verifies the
// ABOUTME: either-side-healthy semantics when a provider becomes unreachable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_mcp_server::llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};

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
impl LlmProvider for ProbeMock {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &'static str {
        self.name
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        "probe-mock"
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Err(AppError::internal("probe mock does not implement complete"))
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let s = stream::iter(vec![Ok(StreamChunk {
            delta: String::new(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        })]);
        Ok(Pin::from(Box::new(s)))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        match self.health {
            Ok(v) => Ok(v),
            Err(msg) => Err(AppError::external_service("probe-mock", msg)),
        }
    }
}

fn chain(primary: ProbeMock, secondary: ProbeMock) -> ChatProvider {
    ChatProvider::Chain {
        primary: Box::new(ChatProvider::Custom(Arc::new(primary))),
        secondary: Box::new(ChatProvider::Custom(Arc::new(secondary))),
    }
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
    // Primary returns false (not Err); secondary errors. The chain should
    // surface the secondary's error rather than silently report healthy.
    let cp = chain(
        ProbeMock::unhealthy("primary"),
        ProbeMock::errors("secondary", "secondary unreachable"),
    );
    let outcome = cp.health_check().await;
    assert!(
        outcome.is_err(),
        "chain must propagate secondary error when primary is unhealthy"
    );
}
