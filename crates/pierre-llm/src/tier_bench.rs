// ABOUTME: The tiers of a fallback chain that cannot answer the probe request, benched until they can
// ABOUTME: One book per chain, shared by the full chain, every tail and the tool loops
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The tier bench
//!
//! A fallback chain moves past a tier on the *kind* of its error: a provider
//! fault falls through, a rejected request does not, because a second tier
//! would reject the same request the same way. That rule trusts the vendor's
//! own classification. On 2026-11-20 Anthropic reported a spent prepaid credit
//! as `400 invalid_request_error`, the head read as "the request is wrong" on
//! every call, and the healthy tiers behind it served nothing for 14 hours.
//!
//! The bench does not read error kinds. A tier that cannot answer
//! [`probe_request`] — one user message, "ping" — is dead whatever its error
//! says, because no request is simpler. Such a tier is benched: passed over by
//! the chain and by the tool loops until it answers the probe again.
//!
//! A benched tier sits in the chain's quota cooldown book for
//! [`BENCH_WINDOW`], which is the mechanism every route already honours. The
//! recovery tick re-asks the benched tiers every [`BENCH_REPROBE_INTERVAL`],
//! well inside that window, and renews it while they stay dead. Nothing here
//! asks a tier that is not benched, so a healthy chain costs no probe.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::{ChatMessage, ChatRequest};

/// How long one bench keeps a tier passed over. Three recovery ticks: a tick
/// that runs late still finds the tier inside its window.
pub(crate) const BENCH_WINDOW: Duration = Duration::from_mins(15);

/// How often the benched tiers are asked again.
pub const BENCH_REPROBE_INTERVAL: Duration = Duration::from_mins(5);

const _: () = assert!(
    BENCH_WINDOW.as_secs() >= BENCH_REPROBE_INTERVAL.as_secs() * 2,
    "code bug: a bench window shorter than two recovery ticks lets a dead tier back into the chain when one tick runs late"
);

/// The probe's request: one user message, "ping", temperature 0, no output
/// cap.
///
/// The cap used to be one token, "the cheapest possible check". Claude Code
/// treats an output cap the answer exceeds as an error — the CLI reports
/// `Claude's response exceeded the 1 output token maximum`, sets `is_error`
/// and exits 1 — so with it the probe failed on Claude at every instance
/// start, the chain's later tiers answered "ping" for it, and the verdict
/// still read healthy. The answer to "ping" is one sentence; the prompt is
/// the cost, and it is the same with or without a cap.
#[must_use]
pub fn probe_request() -> ChatRequest {
    ChatRequest::new(vec![ChatMessage::user("ping".to_owned())]).with_temperature(0.0)
}

/// The benched tiers of one chain, each with the reason it gave.
///
/// Cloning shares the book: the full chain, each tail and each tier taken
/// alone hold the same one.
#[derive(Debug, Clone, Default)]
pub(crate) struct TierBench {
    benched: Arc<Mutex<HashMap<&'static str, String>>>,
}

impl TierBench {
    /// Bench `tier` for `reason`. Returns whether it was not benched already.
    pub(crate) fn bench(&self, tier: &'static str, reason: impl Into<String>) -> bool {
        self.book().insert(tier, reason.into()).is_none()
    }

    /// Take `tier` off the bench; a no-op when it is not on it.
    pub(crate) fn reinstate(&self, tier: &str) {
        self.book().remove(tier);
    }

    /// Whether `tier` is benched.
    pub(crate) fn is_benched(&self, tier: &str) -> bool {
        self.book().contains_key(tier)
    }

    /// Whether no tier is benched.
    pub(crate) fn is_empty(&self) -> bool {
        self.book().is_empty()
    }

    /// A poisoned lock still holds a usable map: a panic elsewhere must not
    /// make every later turn panic on the bench.
    fn book(&self) -> MutexGuard<'_, HashMap<&'static str, String>> {
        self.benched.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::{probe_request, TierBench};

    #[test]
    fn a_benched_tier_stays_benched_until_reinstated() {
        let bench = TierBench::default();
        assert!(bench.is_empty());

        assert!(bench.bench("anthropic", "credit balance is too low"));
        assert!(
            !bench.bench("anthropic", "still too low"),
            "a second bench of the same tier is not a new one"
        );
        assert!(bench.is_benched("anthropic"));
        assert!(!bench.is_benched("anthropic#2"));

        bench.reinstate("anthropic");
        bench.reinstate("anthropic");
        assert!(bench.is_empty());
    }

    #[test]
    fn a_clone_shares_the_book() {
        let bench = TierBench::default();
        let tail_view = bench.clone();

        bench.bench("copilot_sdk", "not signed in");

        assert!(tail_view.is_benched("copilot_sdk"));
    }

    #[test]
    fn the_probe_request_carries_no_output_cap() {
        // Claude Code treats an output cap the answer exceeds as an error
        // ("Claude's response exceeded the 1 output token maximum", is_error,
        // exit 1), so a one-token cap made the probe fail on Claude at every
        // instance start while the chain's later tiers answered "ping" for it.
        let request = probe_request();
        assert_eq!(request.max_tokens, None, "no cap the answer could exceed");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].content, "ping");
    }
}
