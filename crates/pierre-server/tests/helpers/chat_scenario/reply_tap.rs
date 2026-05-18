// ABOUTME: Production reply tap — opt-in sampler that turns flagged real replies into eval seeds
// ABOUTME: Trait + DummyTap so the integration point exists today; durable backend lands in P5 follow-up
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Production reply tap.
//!
//! Bug 6 (English disclaimer on French bread) sat in production for
//! weeks before `ChefFamille` noticed during a manual Telegram test.
//! The reply tap closes that loop: a privacy-respecting opt-in
//! sampler captures a slice of real coach replies, redacts PII, and
//! routes them to a triage queue where flagged ones become new
//! `tests/scenarios/*.yaml` seeds.
//!
//! This module ships:
//!
//! 1. The [`ReplyTap`] trait — what the chat pipeline calls when a
//!    reply leaves the dispatch path.
//! 2. [`NoopReplyTap`] — production default, zero-overhead, no
//!    sampling.
//! 3. [`InMemoryReplyTap`] — test/dev default; collects samples in
//!    memory so the framework can be exercised end-to-end.
//! 4. [`ReplySample`] — the data the tap carries to the triage
//!    backend (PII-redacted reply text + minimum context the dev
//!    needs to reconstruct a scenario).
//!
//! The durable backend (`BigQuery` / Datadog / `Postgres` jsonb table,
//! plus the redactor wiring + admin triage UI) is the P5 follow-up
//! gap-analysis P5 item. This trait + the two default impls let the
//! chat pipeline opt-in *today* — the tap is a stable seam even
//! before the backend lands.

use std::mem;
use std::sync::Mutex;

/// One captured coach reply.
#[derive(Debug, Clone)]
pub struct ReplySample {
    /// Tenant + redacted user identifier (hashed before storage so
    /// the triage queue cannot re-identify the user).
    pub hashed_user_id: String,
    /// BCP-47 short locale resolved at dispatch.
    pub locale: String,
    /// Conversation turn id — the same one that threads
    /// `/internal/conversation-turn/{id}` observability.
    pub turn_id: String,
    /// Coach personality id at dispatch time (`endurance`,
    /// `strength`, …). `None` for messaging-only users without an
    /// assigned coach.
    pub coach_id: Option<String>,
    /// PII-redacted user message verbatim — the URL-credential and
    /// PII redactor middleware has already run.
    pub user_message: String,
    /// PII-redacted coach reply verbatim.
    pub coach_reply: String,
    /// Operator-set tag for triage (`flagged_by_user`,
    /// `random_sample`, `vocabulary_contract_miss`, …).
    pub triage_tag: String,
}

/// Sink interface for [`ReplySample`]s produced by the chat pipeline.
///
/// Implementations decide whether to drop (production sampling
/// gate), buffer in-memory (dev / tests), or persist (durable
/// backend). The trait is `Send + Sync` so the chat pipeline can
/// hand it to spawned tasks without ceremony.
pub trait ReplyTap: Send + Sync {
    /// Record one sample. Implementations must not block the chat
    /// pipeline — long-running side effects belong on a background
    /// task spawned by the implementation itself.
    fn record(&self, sample: ReplySample);

    /// Snapshot helper for tests + the triage admin endpoint.
    /// Default: returns empty (no observability into the sink).
    fn drain_for_inspection(&self) -> Vec<ReplySample> {
        Vec::new()
    }
}

/// Zero-overhead production default — drops every sample.
#[derive(Debug, Default)]
pub struct NoopReplyTap;

impl ReplyTap for NoopReplyTap {
    fn record(&self, _sample: ReplySample) {}
}

/// In-memory tap for dev + framework self-tests.
#[derive(Debug, Default)]
pub struct InMemoryReplyTap {
    samples: Mutex<Vec<ReplySample>>,
}

impl ReplyTap for InMemoryReplyTap {
    fn record(&self, sample: ReplySample) {
        if let Ok(mut guard) = self.samples.lock() {
            guard.push(sample);
        }
    }

    fn drain_for_inspection(&self) -> Vec<ReplySample> {
        self.samples
            .lock()
            .map_or_else(|_| Vec::new(), |mut g| mem::take(&mut *g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ReplySample {
        ReplySample {
            hashed_user_id: "h_abc".to_owned(),
            locale: "fr".to_owned(),
            turn_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            coach_id: Some("endurance".to_owned()),
            user_message: "Combien de km aujourd'hui ?".to_owned(),
            coach_reply: "Aujourd'hui, 5 à 8 km très facile.".to_owned(),
            triage_tag: "random_sample".to_owned(),
        }
    }

    #[test]
    fn noop_tap_drops_everything() {
        let tap = NoopReplyTap;
        tap.record(sample());
        assert!(tap.drain_for_inspection().is_empty());
    }

    #[test]
    fn in_memory_tap_collects_samples_and_drains_them() {
        let tap = InMemoryReplyTap::default();
        tap.record(sample());
        tap.record(sample());
        let drained = tap.drain_for_inspection();
        assert_eq!(drained.len(), 2);
        // Drain empties the buffer.
        assert!(tap.drain_for_inspection().is_empty());
    }

    #[test]
    fn drained_sample_round_trips_every_field() {
        // Asserts the full `ReplySample` shape is preserved through the
        // tap. Every field on the struct is part of the contract with
        // the durable triage backend (P5 follow-up), so any drop here
        // would silently break the triage pipeline.
        let tap = InMemoryReplyTap::default();
        tap.record(sample());
        let drained = tap.drain_for_inspection();
        let s = drained.first().expect("one sample recorded");
        assert_eq!(s.hashed_user_id, "h_abc");
        assert_eq!(s.locale, "fr");
        assert_eq!(s.turn_id, "00000000-0000-0000-0000-000000000001");
        assert_eq!(s.coach_id.as_deref(), Some("endurance"));
        assert_eq!(s.user_message, "Combien de km aujourd'hui ?");
        assert_eq!(s.coach_reply, "Aujourd'hui, 5 à 8 km très facile.");
        assert_eq!(s.triage_tag, "random_sample");
    }
}
