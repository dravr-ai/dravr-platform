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
//! run inside an ACP subprocess that calls Dravr back over `/mcp` as a separate
//! HTTP task, and that task's return value goes to the subprocess, never to the
//! loop. So the signal leaves it the way a Guardian block does — through a
//! shared store keyed by the turn.
//!
//! [`offer_in_payload`] is the one reader both shapes go through, and
//! [`ReconnectOffers`] is that store, on the same key and the same
//! record/clear/take discipline as the headless block channel in
//! [`crate::guardian::GuardianTurns`].

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;

use crate::guardian::TurnKey;

/// Hard cap on retained unconsumed offers. Only the headless loop consumes
/// them, so an offer stamped by a chat or MCP-direct dispatch for a user who
/// never runs a headless turn would otherwise linger. It is one entry per
/// `(tenant, user)`, and only for an athlete carrying a dead connection, so the
/// cap is unreachable in practice; overflow drops the abandoned entries, and a
/// dropped offer costs the clickable control, not the answer.
const OFFER_CAP: usize = 10_000;

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
        .get("reconnect_required")?
        .get("provider_slug")?
        .as_str()
        .filter(|slug| !slug.is_empty())
        .map(str::to_owned)
}

/// Process-wide store of the served-without-a-provider offer raised during one
/// headless turn.
///
/// Shared via the runtime so the per-`/mcp`-request executors the Copilot ACP
/// subprocess drives write into the same bucket the headless loop reads. Keyed
/// by `(tenant, user)`: one user runs one headless turn at a time, so that pair
/// identifies it, and the loop clears the key before the subprocess starts and
/// takes it after the turn ends, bounding an entry to the turn that raised it.
///
/// Its own lock rather than a field on the Guardian store: the two signals share
/// a key and a lifetime but nothing else, and this one is touched on the
/// ordinary success path of a `get_activities` dispatch.
#[derive(Debug, Default)]
pub struct ReconnectOffers {
    offers: Mutex<HashMap<TurnKey, String>>,
}

impl ReconnectOffers {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the backend slug a dispatch served a window without, under `key`
    /// (a `(tenant, user)` headless key). Overwrites any prior unconsumed offer
    /// for the key — the latest dead connection of the turn is the one the
    /// athlete is asked to restore. A poisoned lock is skipped: the turn then
    /// carries the model's own sentence about the missing source and no
    /// control, never a panic.
    pub fn record_offer(&self, key: &TurnKey, provider_slug: String) {
        if let Ok(mut offers) = self.offers.lock() {
            if offers.len() >= OFFER_CAP {
                offers.clear();
            }
            offers.insert(key.clone(), provider_slug);
        }
    }

    /// Consume and return the offer recorded under `key`, if any.
    #[must_use]
    pub fn take_offer(&self, key: &TurnKey) -> Option<String> {
        self.offers.lock().ok()?.remove(key)
    }

    /// Drop any stale offer for `key` before a headless turn starts, so only an
    /// offer raised during *this* turn's subprocess is later taken.
    pub fn clear_offer(&self, key: &TurnKey) {
        if let Ok(mut offers) = self.offers.lock() {
            offers.remove(key);
        }
    }
}
