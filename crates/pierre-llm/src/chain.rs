// ABOUTME: The Chain arm of tool-calling completion — guard, fallback, breaker
// ABOUTME: Holds the policy that the dispatch match in provider.rs used to bury
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Runtime fallback policy for the tool-calling path.
//!
//! [`ChatProvider::complete`](crate::ChatProvider::complete) and
//! [`ChatProvider::complete_with_tools`](crate::ChatProvider::complete_with_tools)
//! make the same three decisions — the preemptive guard, the empty-completion
//! fallback, and the circuit-breaker bookkeeping. They disagreed for as long as
//! the tool-calling copy lived inside a one-line-per-provider dispatch match,
//! where a screen of policy is easy to miss. This module is where the
//! tool-calling half lives now.

use tracing::{info, warn};

use crate::chain_guard::{CircuitTransition, CHAIN_GUARD};
use crate::errors::AppError;
use crate::fallback_policy::{is_empty_tool_completion, is_retryable_for_fallback};
use crate::provider::request_for_secondary;
use crate::{ChatProvider, ChatRequest, ChatResponseWithTools, Tool};

/// The `Chain` arm of [`ChatProvider::complete_with_tools`].
///
/// Makes the same three decisions `complete()` makes, on the tool-calling
/// shape: the preemptive guard, the empty-completion fallback, and the
/// circuit-breaker bookkeeping. It sits outside the dispatch match because that
/// match is a one-line-per-provider table, and burying a screen of chain policy
/// in one of its arms is how the two paths came to disagree.
///
/// Recursive async over [`ChatProvider`] needs `Box::pin` to satisfy the
/// compiler's "recursive async fn must introduce indirection" rule.
pub async fn chain_complete_with_tools(
    primary: &ChatProvider,
    secondary: &ChatProvider,
    request: &ChatRequest,
    tools: Option<Vec<Tool>>,
) -> Result<ChatResponseWithTools, AppError> {
    if CHAIN_GUARD.should_skip_primary() {
        note_preemptive_skip(primary, secondary);
        let forwarded = request_for_secondary(request);
        return Box::pin(secondary.complete_with_tools(&forwarded, tools)).await;
    }

    match Box::pin(primary.complete_with_tools(request, tools.clone())).await {
        Ok(response) if is_empty_tool_completion(&response) => {
            note_empty_tool_completion(primary, secondary, response.finish_reason.as_deref());
            let forwarded = request_for_secondary(request);
            Box::pin(secondary.complete_with_tools(&forwarded, tools)).await
        }
        Ok(response) => {
            note_primary_recovery(primary);
            Ok(response)
        }
        Err(primary_err) if is_retryable_for_fallback(&primary_err) => {
            note_retryable_primary_failure(primary, secondary, &primary_err);
            let forwarded = request_for_secondary(request);
            Box::pin(secondary.complete_with_tools(&forwarded, tools)).await
        }
        Err(primary_err) => Err(primary_err),
    }
}

/// Record that the guard sent this turn straight to the secondary.
fn note_preemptive_skip(primary: &ChatProvider, secondary: &ChatProvider) {
    warn!(
        primary = primary.name(),
        secondary = secondary.name(),
        budget_low = CHAIN_GUARD.is_github_budget_low(),
        circuit_open = CHAIN_GUARD.is_circuit_open(),
        "Chain skipping primary preemptively; using secondary directly"
    );
    info!(
        target: "notify",
        event = "embacle.fallback_triggered",
        from_provider = primary.name(),
        to_provider = secondary.name(),
        reason = "preemptive_guard",
        "Runtime LLM fallback engaged preemptively (guard)"
    );
}

/// Record a primary completion that carried neither prose nor a tool call.
///
/// Deliberately touches neither side of the circuit breaker, for the reason
/// spelled out in [`crate::fallback_policy`]: recording success is the bug this
/// arm exists to fix, and recording failure would open the circuit after a few
/// empties and route every later turn to the paid secondary.
fn note_empty_tool_completion(
    primary: &ChatProvider,
    secondary: &ChatProvider,
    finish_reason: Option<&str>,
) {
    warn!(
        primary = primary.name(),
        secondary = secondary.name(),
        finish_reason = finish_reason.unwrap_or("none"),
        "Primary LLM returned an empty tool-calling completion; falling back"
    );
    info!(
        target: "notify",
        event = "embacle.fallback_triggered",
        from_provider = primary.name(),
        to_provider = secondary.name(),
        reason = "empty_completion",
        "Runtime LLM fallback engaged on an empty completion"
    );
}

/// Record a healthy primary answer, announcing a circuit that just closed.
fn note_primary_recovery(primary: &ChatProvider) {
    if matches!(
        CHAIN_GUARD.record_primary_success(),
        CircuitTransition::Closed
    ) {
        info!(
            target: "notify",
            event = "llm.circuit_closed",
            provider = primary.name(),
            "Chain circuit closed on primary recovery"
        );
    }
}

/// Record a retryable primary failure, announcing a circuit that just opened.
fn note_retryable_primary_failure(
    primary: &ChatProvider,
    secondary: &ChatProvider,
    primary_err: &AppError,
) {
    if matches!(
        CHAIN_GUARD.record_primary_failure(),
        CircuitTransition::Opened
    ) {
        info!(
            target: "notify",
            event = "llm.circuit_opened",
            provider = primary.name(),
            reason = ?primary_err.code,
            "Chain circuit opened after consecutive primary failures"
        );
    }
    warn!(
        primary = primary.name(),
        secondary = secondary.name(),
        error = %primary_err,
        "Primary LLM complete_with_tools() failed with retryable error; falling back"
    );
    info!(
        target: "notify",
        event = "embacle.fallback_triggered",
        from_provider = primary.name(),
        to_provider = secondary.name(),
        reason = ?primary_err.code,
        "Runtime LLM fallback engaged on complete_with_tools()"
    );
}
