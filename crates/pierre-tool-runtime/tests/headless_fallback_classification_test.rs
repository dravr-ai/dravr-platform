// ABOUTME: Pins which AppErrors reroute a headless (Copilot) turn to the chain's tail, and that a chain exposes that tail
// ABOUTME: The headless loop bypasses the chain, so this is the one place the fall-through classification exists on AppError
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The headless (Copilot) tool loop converses with the chain's head directly,
//! so the chain's own fallback never fires for SDK-tool-calling turns.
//! `pierre-tool-runtime` re-creates that fallback by classifying the head's
//! error with `reroutes_headless_turn` and, when it reroutes, re-running the
//! turn against `ChatProvider::fallback_tail`.
//!
//! These tests pin the two pieces that gate that behavior:
//! 1. `reroutes_headless_turn` is the `AppError` image of
//!    `ErrorKind::is_provider_fault`: an ACP prompt timeout (`Timeout` →
//!    `ResourceUnavailable`) reroutes, a rejected request (`InvalidRequest` →
//!    `InvalidInput`) and a quota refusal (`RateLimit`) do not.
//! 2. A chain exposes its tail; a solo provider does not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use async_trait::async_trait;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, ErrorKind, LlmCapabilities,
    LlmProvider as EmbacleLlmProvider, RunnerError,
};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_llm::{ChatProvider, EmbacleProvider, LlmProvider};
use pierre_tool_runtime::tool_loop_io::reroutes_headless_turn;

#[test]
fn an_acp_timeout_reroutes() {
    // `ErrorKind::Timeout` from embacle maps to ResourceUnavailable — the exact
    // error a stalled Copilot ACP session produces.
    let timeout: AppError = RunnerError::timeout("copilot-acp: prompt timed out after 150s").into();
    assert_eq!(timeout.code, ErrorCode::ResourceUnavailable);
    assert!(
        reroutes_headless_turn(&timeout),
        "an ACP prompt timeout must re-run the turn against the tail"
    );
}

#[test]
fn every_provider_fault_kind_reroutes_and_no_other_does() {
    for kind in [
        ErrorKind::Timeout,
        ErrorKind::ExternalService,
        ErrorKind::AuthFailure,
        ErrorKind::Internal,
        ErrorKind::BinaryNotFound,
        ErrorKind::ModelUnavailable,
        ErrorKind::Config,
        ErrorKind::Guardrail,
        ErrorKind::ContextLength,
        ErrorKind::RateLimit,
        ErrorKind::InvalidRequest,
    ] {
        let runner_error = RunnerError {
            kind,
            message: "scripted".to_owned(),
        };
        let app_error: AppError = runner_error.into();
        assert_eq!(
            reroutes_headless_turn(&app_error),
            kind.is_provider_fault(),
            "{kind:?} -> {:?} must reroute exactly when embacle calls it a provider fault",
            app_error.code
        );
    }
}

#[test]
fn the_platform_auth_codes_reroute_and_invalid_input_does_not() {
    for code in [
        ErrorCode::ResourceUnavailable,
        ErrorCode::ExternalServiceUnavailable,
        ErrorCode::ExternalServiceError,
        ErrorCode::ExternalAuthFailed,
        ErrorCode::AuthInvalid,
        ErrorCode::AuthExpired,
        ErrorCode::InternalError,
    ] {
        assert!(
            reroutes_headless_turn(&AppError::new(code, "transient")),
            "{code:?} should reroute to the tail"
        );
    }
    // Deterministic failures must NOT burn a second provider call.
    assert!(
        !reroutes_headless_turn(&AppError::invalid_input("malformed request")),
        "InvalidInput is deterministic and must not reroute"
    );
    assert!(
        !reroutes_headless_turn(&AppError::new(ErrorCode::RateLimitExceeded, "quota")),
        "a quota refusal is the operator's problem, not a reason to spend another tier"
    );
    assert!(
        !reroutes_headless_turn(&AppError::new(
            ErrorCode::ExternalRateLimited,
            "gemini: try again in 7 seconds"
        )),
        "a vendor throttle on an HTTP tier keeps the chain decision its RunnerError had"
    );
}

/// A scripted embacle runner that never answers; only its identity matters here.
struct Named(&'static str);

#[async_trait]
impl EmbacleLlmProvider for Named {
    fn name(&self) -> &'static str {
        self.0
    }

    fn display_name(&self) -> &str {
        self.0
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &str {
        const MODEL: &str = "scripted-model";
        MODEL
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        Err(RunnerError::internal("not asked in this test"))
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("not asked in this test"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

#[test]
fn a_chain_exposes_its_tail_and_a_solo_provider_does_not() {
    let chain = ChatProvider::Embacle(
        EmbacleProvider::chain(vec![
            EmbacleProvider::from_runner(Box::new(Named("copilot_headless")), "primary"),
            EmbacleProvider::from_runner(Box::new(Named("cohere")), "secondary"),
        ])
        .unwrap(),
    );
    assert_eq!(
        chain.fallback_tail().map(LlmProvider::name),
        Some("cohere"),
        "a chain must expose its tail so the headless loop can re-run against it"
    );

    let solo = ChatProvider::Embacle(EmbacleProvider::from_runner(
        Box::new(Named("cohere")),
        "solo",
    ));
    assert!(
        solo.fallback_tail().is_none(),
        "a solo provider has no tail; the original error is returned unchanged"
    );
}
