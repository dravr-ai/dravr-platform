// ABOUTME: The platform's FallbackObserver: drives the chain guard and emits the catalogued notify events
// ABOUTME: One copy of the policy that complete(), complete_stream() and complete_with_tools() used to inline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What the platform adds to embacle's fallback chain.
//!
//! The chain itself — which errors move a request to the next tier, what an
//! empty completion is, when the forwarded request drops the primary's model —
//! is [`embacle::FallbackProvider`] under [`embacle::ResponsePolicy::strict`].
//! This observer contributes the two things embacle cannot know: the
//! process-wide [`ChainGuard`] (GitHub budget headroom written by a probe in
//! pierre-services, plus the circuit breaker on the primary), and the three
//! `notify` events operators page on. Every callback runs on the calling task,
//! so `user_id` / `tenant_id` reach the events from the enclosing span.
//!
//! Only the outermost chain is guarded: the breaker measures the primary, so
//! the tail chain the headless tool loop re-runs against must not count a
//! Cohere failure as a primary failure.

use embacle::types::RunnerError;
use embacle::{Attempt, FallbackObserver, FallthroughReason, Tier};
use tracing::{info, warn};

use crate::chain_guard::{ChainGuard, CircuitTransition, CHAIN_GUARD};
use crate::served_tier::{record_served_tier, ServedTier};

/// Position of the primary tier, the only one the guard measures.
const PRIMARY: usize = 0;

/// The platform-side observer a chain is built with.
pub struct ChainObserver {
    /// `Some` on the outermost chain; `None` on the tail a headless re-run
    /// walks, so its hops are reported but never counted against the primary.
    guard: Option<&'static ChainGuard>,
}

impl ChainObserver {
    /// The observer for the chain the chat pipeline calls: consults and
    /// records on [`CHAIN_GUARD`].
    #[must_use]
    pub fn guarded() -> Self {
        Self {
            guard: Some(&CHAIN_GUARD),
        }
    }

    /// The observer for a tail chain: reports every hop, touches no guard.
    #[must_use]
    pub const fn unguarded() -> Self {
        Self { guard: None }
    }

    /// The guard, when this observer is the one that records on it and
    /// `tier` is the tier it measures.
    fn guard_for(&self, tier: Tier<'_>) -> Option<&'static ChainGuard> {
        self.guard.filter(|_| tier.position == PRIMARY)
    }

    /// The guard vetoed `from`; the request goes straight to `to`.
    fn note_skipped(&self, from: Tier<'_>, to: Tier<'_>, reason: &'static str) {
        warn!(
            from = from.provider.name(),
            to = to.provider.name(),
            budget_low = self.guard.is_some_and(ChainGuard::is_github_budget_low),
            circuit_open = self.guard.is_some_and(ChainGuard::is_circuit_open),
            "Chain skipping a tier preemptively; using the next tier directly"
        );
        info!(
            target: "notify",
            event = "embacle.fallback_triggered",
            from_provider = from.provider.name(),
            to_provider = to.provider.name(),
            reason = reason,
            "Runtime LLM fallback engaged preemptively (guard)"
        );
    }

    /// `from` failed with a fault the chain moves past. A primary fault counts
    /// on the breaker, and the transition that opens the circuit is announced.
    fn note_provider_fault(&self, from: Tier<'_>, to: Tier<'_>, error: &RunnerError) {
        if let Some(guard) = self.guard_for(from) {
            if matches!(guard.record_primary_failure(), CircuitTransition::Opened) {
                info!(
                    target: "notify",
                    event = "llm.circuit_opened",
                    provider = from.provider.name(),
                    reason = ?error.kind,
                    "Chain circuit opened after consecutive primary failures"
                );
            }
        }
        warn!(
            from = from.provider.name(),
            to = to.provider.name(),
            error = %error,
            "LLM tier failed with a provider fault; falling back"
        );
        info!(
            target: "notify",
            event = "embacle.fallback_triggered",
            from_provider = from.provider.name(),
            to_provider = to.provider.name(),
            reason = ?error.kind,
            "Runtime LLM fallback engaged on a provider fault"
        );
    }
}

/// `from` answered with nothing deliverable.
///
/// Records NEITHER success nor failure on the breaker, deliberately. Recording
/// success is the bug this arm exists to fix (an empty `Ok` used to count as
/// primary health). Recording failure would open the circuit after a few
/// empties and route every later turn to the paid secondary, including the
/// ones the free primary would have answered. The breaker keeps judging on
/// errors, which is what it measures well.
fn note_empty_completion(from: Tier<'_>, to: Tier<'_>) {
    warn!(
        from = from.provider.name(),
        to = to.provider.name(),
        "LLM tier returned an empty completion; falling back"
    );
    info!(
        target: "notify",
        event = "embacle.fallback_triggered",
        from_provider = from.provider.name(),
        to_provider = to.provider.name(),
        reason = "empty_completion",
        "Runtime LLM fallback engaged on an empty completion"
    );
}

impl FallbackObserver for ChainObserver {
    fn before_attempt(&self, tier: Tier<'_>) -> Attempt {
        match self.guard_for(tier) {
            Some(guard) if guard.should_skip_primary() => Attempt::Skip("preemptive_guard"),
            _ => Attempt::Try,
        }
    }

    fn on_fallthrough(&self, from: Tier<'_>, to: Tier<'_>, reason: FallthroughReason<'_>) {
        match reason {
            FallthroughReason::Skipped(reason) => self.note_skipped(from, to, reason),
            FallthroughReason::EmptyCompletion => note_empty_completion(from, to),
            FallthroughReason::Error(error) => self.note_provider_fault(from, to, error),
        }
    }

    fn on_success(&self, tier: Tier<'_>) {
        // Tell the caller who answered: the chain's own name is its head, so
        // without this a fallback is logged and costed as the primary.
        record_served_tier(ServedTier {
            provider: tier.provider.name(),
            position: tier.position,
            capabilities: tier.provider.capabilities(),
        });

        // A stream counts on OPEN — the breaker tracks transport-level
        // establishment, which is what auth and rate-limit failures hit first.
        if let Some(guard) = self.guard_for(tier) {
            if matches!(guard.record_primary_success(), CircuitTransition::Closed) {
                info!(
                    target: "notify",
                    event = "llm.circuit_closed",
                    provider = tier.provider.name(),
                    "Chain circuit closed on primary recovery"
                );
            }
        }
    }

    fn on_exhausted(&self, last: Tier<'_>, error: &RunnerError) {
        warn!(
            tier = last.provider.name(),
            position = last.position,
            error = %error,
            "Every LLM tier failed; returning the last tier's error"
        );
    }
}
