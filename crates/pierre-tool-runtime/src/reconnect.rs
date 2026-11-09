// ABOUTME: The served-without-a-provider signal: one reader for the sidecar, one store per turn
// ABOUTME: The store is how the signal leaves the ACP subprocess's loopback, which returns nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A window a sibling connection served still owes the athlete a reconnect.
//!
//! `get_activities` stamps `reconnect_required` into its own result when the
//! athlete's healthy connections answered a window the elected provider could
//! not. Two shapes of caller read that stamp. The `ReAct` and planned loops hold
//! the tool payloads themselves and read it straight off them (the
//! `tool_results` readers). The Copilot-headless loop holds nothing: its tools
//! run inside an ACP subprocess that calls Dravr back through the tool host on a
//! separate task, and that task's return value goes to the subprocess, never to
//! the loop. So the signal leaves it the way a Guardian block does — through a
//! shared store keyed by the turn.
//!
//! [`offer_in_payload`] is the one reader both shapes go through, and
//! [`ReconnectOffers`] is that store, on the same key and the same record/take
//! discipline as the block channel in [`crate::guardian::GuardianTurns`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use serde_json::Value;

use crate::guardian::state::TURN_TTL;
use crate::guardian::TurnKey;

/// Hard cap on retained unconsumed offers. The tool loop takes a turn's offer
/// when it returns, so an entry outlives its turn only when the loop failed or a
/// stage read `get_activities` after it (the capability recovery's verification
/// fetch), and only for an athlete carrying a dead connection: the cap is
/// unreachable in practice. Overflow drops the entries past [`TURN_TTL`] first,
/// every entry only if that was not enough, and a dropped offer costs the
/// clickable control, not the answer.
const OFFER_CAP: usize = 10_000;

/// The `reconnect_required` key holding the dead connection's backend.
///
/// It is the chat pipeline's to mint a reconnect link from, never the
/// model's to read: a mirror backend (`sciotte_garmin`) is a name the athlete
/// never chose. [`crate::tool_results::project_activities_payload`] drops it
/// from what the model reads; the sidecar's `provider` names the connection.
pub(crate) const RECONNECT_BACKEND_KEY: &str = "provider_slug";

/// The `get_activities` sidecar a window served without a dead connection
/// carries.
pub(crate) const RECONNECT_REQUIRED_KEY: &str = "reconnect_required";

/// Backend key of a provider one tool payload was served WITHOUT, if any.
///
/// `get_activities` stamps `reconnect_required` into its own RESULT when the
/// athlete's healthy connections answered a window the elected provider could
/// not. That is deliberately not the dispatch metadata
/// `function_dispatch::execute_function_calls` scans for
/// `auth_required_provider`: the metadata stamp is captured whether the call
/// succeeded or not and aborts the turn into the deterministic reconnect reply,
/// which is the blanking a served window exists to avoid. Read off the payload,
/// the same signal accompanies the answer instead of replacing it.
///
/// Keyed on the tool name so only the envelope that carries this contract can
/// raise it — a nested field of some other tool's output cannot reach the chat
/// pipeline's mint.
#[must_use]
pub fn offer_in_payload(tool_name: &str, payload: &Value) -> Option<String> {
    if tool_name != "get_activities" {
        return None;
    }
    payload
        .get(RECONNECT_REQUIRED_KEY)?
        .get(RECONNECT_BACKEND_KEY)?
        .as_str()
        .filter(|slug| !slug.is_empty())
        .map(str::to_owned)
}

/// Process-wide store of the served-without-a-provider offer raised during one
/// headless turn.
///
/// Shared via the runtime so the tool-host surface's executor the Copilot ACP
/// subprocess drives writes into the same bucket the turn's loop reads. Keyed by
/// the turn: one athlete can run overlapping turns, and a slot per athlete would
/// let one turn clear or take another's offer, and a one-shot MCP call land in a
/// chat turn (carnet#1404). A one-shot call records nothing, and neither does a
/// window the pipeline prefetched: only a read the model asked for raises an offer.
///
/// Its own lock rather than a field on the Guardian store: the two signals share
/// a key and a lifetime but nothing else, and this one is touched on the
/// ordinary success path of a `get_activities` dispatch.
#[derive(Debug, Default)]
pub struct ReconnectOffers {
    offers: Mutex<HashMap<TurnKey, (Instant, String)>>,
}

impl ReconnectOffers {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the backend slug a dispatch served a window without, under `key`
    /// (the turn it served). Overwrites any prior unconsumed offer
    /// for the key — the latest dead connection of the turn is the one the
    /// athlete is asked to restore. A poisoned lock is skipped: the turn then
    /// carries the model's own sentence about the missing source and no
    /// control, never a panic.
    pub fn record_offer(&self, key: &TurnKey, provider_slug: String) {
        self.record_offer_at(key, provider_slug, Instant::now());
    }

    /// [`Self::record_offer`] as of `now`.
    fn record_offer_at(&self, key: &TurnKey, provider_slug: String, now: Instant) {
        if let Ok(mut offers) = self.offers.lock() {
            if offers.len() >= OFFER_CAP {
                offers.retain(|_, (at, _)| now.duration_since(*at) < TURN_TTL);
                if offers.len() >= OFFER_CAP {
                    offers.clear();
                }
            }
            offers.insert(key.clone(), (now, provider_slug));
        }
    }

    /// Consume and return the offer recorded under `key`, if any.
    #[must_use]
    pub fn take_offer(&self, key: &TurnKey) -> Option<String> {
        self.offers.lock().ok()?.remove(key).map(|(_, slug)| slug)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Ages the entries by reading them from a later `now`, because an `Instant`
    /// cannot reach back past a freshly booted machine's clock start.
    #[test]
    fn overflow_drops_abandoned_offers_before_a_live_one() {
        let store = ReconnectOffers::new();
        let later = Instant::now() + TURN_TTL + Duration::from_secs(1);
        let live = TurnKey::new(None, "turn-live");
        store.record_offer_at(&live, "strava".to_owned(), later);
        for turn in 0..OFFER_CAP {
            store.offers.lock().expect("lock").insert(
                TurnKey::new(None, format!("turn-{turn}")),
                (Instant::now(), "garmin".to_owned()),
            );
        }

        let next = TurnKey::new(None, "turn-next");
        store.record_offer_at(&next, "wahoo".to_owned(), later);

        assert_eq!(
            store.offers.lock().expect("lock").len(),
            2,
            "only the abandoned entries went"
        );
        assert_eq!(store.take_offer(&live).as_deref(), Some("strava"));
        assert_eq!(store.take_offer(&next).as_deref(), Some("wahoo"));
    }

    #[test]
    fn offers_stay_bounded_when_none_has_been_abandoned() {
        let store = ReconnectOffers::new();
        for turn in 0..OFFER_CAP {
            store.record_offer(
                &TurnKey::new(None, format!("turn-{turn}")),
                "garmin".to_owned(),
            );
        }
        let past_the_cap = TurnKey::new(None, "turn-past-the-cap");
        store.record_offer(&past_the_cap, "wahoo".to_owned());
        assert!(store.offers.lock().expect("lock").len() <= OFFER_CAP);
        assert_eq!(store.take_offer(&past_the_cap).as_deref(), Some("wahoo"));
    }
}
