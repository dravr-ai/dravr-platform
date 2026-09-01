// ABOUTME: Decides what counts as a primary-provider failure worth falling back on
// ABOUTME: Two classifiers — a retryable error, and a completion that carries nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Fallback policy.
//!
//! [`ChatProvider::Chain`](crate::ChatProvider::Chain) tries a primary and
//! reissues against a secondary when the primary fails. "Fails" is the whole
//! question, and it has two answers that used to live in one file with the
//! routing they inform:
//!
//! - [`is_retryable_for_fallback`] — an *error* worth retrying elsewhere.
//!   Transient/availability failures move; deterministic ones (bad input,
//!   quota) stay, so the caller sees the right diagnostic instead of being
//!   silently rerouted to a different provider.
//! - [`is_empty_completion`] — a *success* that carries nothing. The chain
//!   used to have no such notion: an `Ok` response with no content took the
//!   success path, recorded primary health, and never consulted the secondary,
//!   so the athlete received the lost-turn apology while a working provider
//!   sat unused (2026-08-31, carnet#165).
//!
//! Grouped here because they answer the same question and are read together
//! when that question changes. `pierre-tool-runtime`'s headless loop bypasses
//! the `Chain` wrapper entirely and re-creates the decision from these same
//! predicates, which is why both are `pub`.

use crate::errors::AppError;
use crate::ChatResponse;

/// `true` when a completion carries nothing the caller can deliver.
///
/// Empty content **with tool calls is not empty** — that is an ordinary
/// mid-loop turn where the model asked for a tool instead of speaking, and
/// treating it as a failure would fall back on every tool-using turn in every
/// conversation, against a paid provider. Whitespace-only content *is* empty:
/// every surface trims before rendering, so a reply of three spaces reaches the
/// athlete as nothing at all.
///
/// An empty `tool_calls` vec reads the same as `None`; providers disagree on
/// which they send, and treating `Some(vec![])` as "has tool calls" would
/// silently restore the bug for whichever one serialises it that way.
#[must_use]
pub fn is_empty_completion(response: &ChatResponse) -> bool {
    response.content.trim().is_empty() && response.tool_calls.as_ref().is_none_or(Vec::is_empty)
}

/// Whether a primary-provider error should trigger the runtime-fallback chain.
///
/// Transient/availability failures (auth, upstream unavailable, internal,
/// resource-unavailable — which an ACP prompt timeout maps to) are retryable on
/// the secondary. Deterministic failures (bad input, quota) are not: falling
/// back on them would hide the real diagnostic behind a second provider's
/// version of the same rejection.
#[must_use]
pub fn is_retryable_for_fallback(error: &AppError) -> bool {
    use pierre_core::errors::ErrorCode;
    matches!(
        error.code,
        ErrorCode::ExternalAuthFailed
            | ErrorCode::ExternalServiceUnavailable
            | ErrorCode::ExternalServiceError
            | ErrorCode::ResourceUnavailable
            | ErrorCode::AuthInvalid
            | ErrorCode::AuthExpired
            | ErrorCode::InternalError
    )
}
