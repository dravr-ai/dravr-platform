// ABOUTME: Classifies a Cohere API error response into an AppError the fallback chain can act on
// ABOUTME: The distinction that matters — a provider that could not answer vs a request that is wrong
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Cohere error classification.
//!
//! One decision lives here, and it is load-bearing: whether a 4xx means *this
//! provider could not answer* (retry on the next one) or *this request is
//! wrong* (retrying elsewhere would fail the same way and hide the real
//! diagnostic). [`crate::is_retryable_for_fallback`] reads the code this
//! produces, so getting it wrong either strands a recoverable turn or
//! silently reroutes a genuine bad request.
//!
//! Split out of `cohere.rs` because it is a distinct concern from the
//! provider's transport, and because that file had grown past the size ceiling
//! and could not carry another test.

use serde::Deserialize;
use tracing::debug;

use crate::errors::AppError;
use pierre_core::errors::ErrorCode;

/// Cohere's error envelope.
#[derive(Debug, Deserialize)]
pub struct CohereErrorResponse {
    /// Free-form error message. Some 4xx responses use `data.error.message`
    /// instead; we fall back to the raw body when neither shape parses.
    #[serde(default)]
    pub message: Option<String>,
}

/// Phrases Cohere 400/422s with when the model produced no answer.
///
/// Both mean "the provider could not generate", not "your request is
/// malformed", so both are classified retryable and cascade to the next
/// provider. The first came from the 2026-06-17 cold-start incident. The second
/// was observed 2026-09-01 ending 2 of 16 empty-turn handoffs in a canned
/// outage reply — the same class of failure, phrased differently, and the
/// single-phrase match sent it down the non-retryable branch.
const EMPTY_COMPLETION_MARKERS: [&str; 2] =
    ["no tool calls or response", "no valid response generated"];

/// Parse an error response from the Cohere API into an [`AppError`].
pub fn parse_error_response(status: reqwest::StatusCode, body: &str) -> AppError {
    let parsed_message = serde_json::from_str::<CohereErrorResponse>(body)
        .ok()
        .and_then(|err| err.message)
        .filter(|m| !m.is_empty());

    let error_message = parsed_message.unwrap_or_else(|| {
        debug!(
            status = %status,
            body_preview = %body.chars().take(200).collect::<String>(),
            "Cohere API returned non-JSON error response"
        );
        format!("HTTP {status}")
    });

    match status.as_u16() {
        401 | 403 => {
            AppError::auth_invalid(format!("Cohere API authentication failed: {error_message}"))
        }
        429 => AppError::new(
            ErrorCode::ExternalRateLimited,
            format!("Cohere rate limit reached. {error_message}"),
        ),
        400 | 422 => {
            // An empty-completion 400/422 is a retryable "provider couldn't
            // answer": classify it as an external-service error so the runtime
            // fallback chain cascades to the next provider instead of surfacing
            // a user-facing failure. Genuine validation errors stay
            // non-retryable InvalidInput.
            let lower = error_message.to_lowercase();
            if EMPTY_COMPLETION_MARKERS.iter().any(|m| lower.contains(m)) {
                AppError::external_service("Cohere", error_message)
            } else {
                AppError::invalid_input(format!("Cohere API validation error: {error_message}"))
            }
        }
        _ => AppError::external_service("Cohere", error_message),
    }
}
