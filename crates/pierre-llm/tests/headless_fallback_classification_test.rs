// ABOUTME: Pins that an ACP-timeout-class error triggers the runtime fallback chain and a Chain exposes its secondary
// ABOUTME: Guards the tool-runtime headless loop's fallback so a stalled Copilot session degrades to the secondary provider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The headless (Copilot ACP) tool loop reaches past the [`ChatProvider::Chain`]
//! wrapper to the primary runner, so the chain's own fallback never fires for
//! SDK-tool-calling turns. `pierre-tool-runtime` re-creates that fallback by
//! classifying the primary error with [`is_retryable_for_fallback`] and, when
//! retryable, re-running the turn against [`ChatProvider::fallback_secondary`].
//!
//! These tests pin the two pieces that gate that behavior:
//! 1. An ACP prompt timeout maps to `ErrorCode::ResourceUnavailable` (see
//!    `From<RunnerError> for AppError`) and MUST be classified retryable — else
//!    a stalled Copilot session errors the turn instead of falling back.
//! 2. A `Chain` exposes its secondary; a non-chain provider does not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_llm::config::LlmModelConfig;
use pierre_llm::errors::{AppError, ErrorCode};
use pierre_llm::{
    is_retryable_for_fallback, ChatProvider, GeminiProvider, GroqProvider, LlmProvider,
};

#[test]
fn acp_timeout_error_is_retryable_for_fallback() {
    // `ErrorKind::Timeout` from embacle maps to ResourceUnavailable — the exact
    // error a stalled Copilot ACP session produces ("CLI runner timed out").
    let timeout = AppError::new(
        ErrorCode::ResourceUnavailable,
        "CLI runner timed out: copilot-acp: prompt timed out after 150s",
    );
    assert!(
        is_retryable_for_fallback(&timeout),
        "an ACP prompt timeout (ResourceUnavailable) must fall back to the secondary"
    );
}

#[test]
fn transient_errors_are_retryable_deterministic_errors_are_not() {
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
            is_retryable_for_fallback(&AppError::new(code, "transient")),
            "{code:?} should be retryable on the secondary"
        );
    }
    // Deterministic failures must NOT burn a second provider call.
    assert!(
        !is_retryable_for_fallback(&AppError::invalid_input("malformed request")),
        "InvalidInput is deterministic and must not fall back"
    );
}

#[test]
fn chain_exposes_secondary_solo_provider_does_not() {
    let model_config = LlmModelConfig {
        default_model: "claude-opus-4.8".to_owned(),
        fallback_model: "claude-sonnet-4.6".to_owned(),
    };
    let primary = ChatProvider::Gemini(GeminiProvider::with_config("k", &model_config));
    let secondary = ChatProvider::Groq(GroqProvider::new("k".to_owned()));
    let secondary_name = secondary.name();
    let chain = ChatProvider::Chain {
        primary: Box::new(primary),
        secondary: Box::new(secondary),
    };
    assert_eq!(
        chain.fallback_secondary().map(LlmProvider::name),
        Some(secondary_name),
        "a Chain must expose its secondary so the headless loop can re-run against it"
    );

    let solo = ChatProvider::Groq(GroqProvider::new("k".to_owned()));
    assert!(
        solo.fallback_secondary().is_none(),
        "a non-chain provider has no secondary; the original error is returned unchanged"
    );
}
