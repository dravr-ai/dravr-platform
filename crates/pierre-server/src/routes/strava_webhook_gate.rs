// ABOUTME: What a Strava push event must pass before it spends the shared Strava quota
// ABOUTME: The registered subscription id, and one activities fetch per owner per debounce window

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guards on `POST /webhooks/strava`.
//!
//! Strava does not sign push events, and the route is public, so a body is
//! only a claim. Two things stand between that claim and a fetch that spends
//! the shared Strava app's read quota with an athlete's token (carnet#557):
//!
//! - **The subscription id.** Every genuine event carries the id Strava gave
//!   back when `pierre-cli strava-webhook subscribe` registered the app's one
//!   subscription. The server reads the registered ids from
//!   `STRAVA_WEBHOOK_SUBSCRIPTION_ID` (comma-separated, one per app that
//!   subscribed) and refuses any other. Unset, every event is refused: an
//!   unguarded route is the bug, not a default.
//! - **A per-owner debounce.** Subscription ids are small integers, so the
//!   check raises the bar without closing the door. The fetch reads a
//!   week-long window, so a second event inside [`OWNER_FETCH_WINDOW`] learns
//!   nothing the first fetch did not; it is folded into one trailing fetch at
//!   the window's end, which still picks up an activity uploaded in between.
//!   However many events name an owner, their token is spent at most once per
//!   window plus one trailing fetch.

use std::collections::HashMap;
use std::env;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

/// The variable holding the subscription ids Strava returned at subscribe.
pub const SUBSCRIPTION_ID_VAR: &str = "STRAVA_WEBHOOK_SUBSCRIPTION_ID";

/// The shortest gap between two webhook-triggered fetches for one owner.
pub const OWNER_FETCH_WINDOW: Duration = Duration::from_mins(5);

/// The subscription ids this deployment registered with Strava.
///
/// `None` when [`SUBSCRIPTION_ID_VAR`] is unset or holds no id — including
/// the Secret Manager placeholder a fresh deployment starts with — in which
/// case every event is refused.
#[must_use]
pub fn registered_subscription_ids() -> Option<Vec<u64>> {
    let raw = env::var(SUBSCRIPTION_ID_VAR).ok()?;
    let ids: Vec<u64> = raw
        .split(',')
        .filter_map(|id| id.trim().parse().ok())
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// Why an event naming `subscription_id` is refused, or `None` to accept it.
///
/// 503 while no subscription id is configured (Strava retries, and the
/// refusal is this deployment's to fix); 403 for an id this deployment did not
/// register.
#[must_use]
pub fn subscription_refusal(subscription_id: u64) -> Option<StatusCode> {
    let Some(registered) = registered_subscription_ids() else {
        warn!(
            "Strava webhook refused: {SUBSCRIPTION_ID_VAR} is unset, so no event can be told \
             from a forgery"
        );
        return Some(StatusCode::SERVICE_UNAVAILABLE);
    };
    if registered.contains(&subscription_id) {
        return None;
    }
    warn!(
        subscription_id = %subscription_id,
        "Strava webhook refused: not this deployment's subscription"
    );
    Some(StatusCode::FORBIDDEN)
}

/// When a webhook-triggered fetch for one owner may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchTiming {
    /// No fetch inside the window: fetch now.
    Now,
    /// A fetch ran inside the window and none is pending: fetch once this
    /// long from now, then call [`OwnerFetchGate::trailing_started`].
    After(Duration),
    /// A trailing fetch is already pending and will cover this event.
    Coalesced,
}

/// Per-owner fetch bookkeeping.
#[derive(Debug, Clone, Copy)]
struct OwnerSlot {
    /// When the owner's latest fetch ran, or will run if one is pending.
    last: Instant,
    /// Whether a trailing fetch is scheduled and has not started yet.
    trailing_pending: bool,
}

/// Debounces webhook-triggered fetches per owner; see the module docs.
#[derive(Debug)]
pub struct OwnerFetchGate {
    window: Duration,
    slots: Mutex<HashMap<Uuid, OwnerSlot>>,
}

impl OwnerFetchGate {
    /// A gate allowing one fetch per owner per `window`, plus one trailing.
    #[must_use]
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// Decide when the fetch an event for `owner` asks for may run.
    ///
    /// Slots older than the window with nothing pending are dropped on the
    /// way, so the map holds only owners fetched within the last window.
    pub fn claim(&self, owner: Uuid, now: Instant) -> FetchTiming {
        let Ok(mut slots) = self.slots.lock() else {
            // A poisoned lock means a panic mid-update; fetching is the
            // behaviour the route had before the gate existed.
            return FetchTiming::Now;
        };
        let window = self.window;
        slots.retain(|_, slot| {
            slot.trailing_pending || now.saturating_duration_since(slot.last) < window
        });

        match slots.get_mut(&owner) {
            None => {
                slots.insert(
                    owner,
                    OwnerSlot {
                        last: now,
                        trailing_pending: false,
                    },
                );
                FetchTiming::Now
            }
            Some(slot) if slot.trailing_pending => FetchTiming::Coalesced,
            Some(slot) => {
                let wait = window.saturating_sub(now.saturating_duration_since(slot.last));
                slot.last = now + wait;
                slot.trailing_pending = true;
                FetchTiming::After(wait)
            }
        }
    }

    /// Wait until `owner` may be fetched; `false` means skip the fetch.
    ///
    /// Returns at once for [`FetchTiming::Now`], `false` for
    /// [`FetchTiming::Coalesced`], and after the wait for
    /// [`FetchTiming::After`] — or `false` if `drain` fires first, since a
    /// trailing fetch must not hold a shutting-down process; the next sync
    /// picks its activity up.
    pub async fn wait_for_turn(&self, owner: Uuid, drain: CancellationToken) -> bool {
        match self.claim(owner, Instant::now()) {
            FetchTiming::Now => true,
            FetchTiming::After(wait) => {
                info!(
                    user_id = %owner,
                    wait_secs = wait.as_secs(),
                    "Strava webhook fetch deferred: the owner was fetched within the debounce window"
                );
                tokio::select! {
                    () = sleep(wait) => {}
                    () = drain.cancelled() => return false,
                }
                self.trailing_started(owner);
                true
            }
            FetchTiming::Coalesced => {
                info!(
                    user_id = %owner,
                    "Strava webhook event folded into the owner's pending fetch"
                );
                false
            }
        }
    }

    /// Record that the trailing fetch [`FetchTiming::After`] scheduled for
    /// `owner` is starting, so the next event opens a new window after it.
    pub fn trailing_started(&self, owner: Uuid) {
        if let Ok(mut slots) = self.slots.lock() {
            if let Some(slot) = slots.get_mut(&owner) {
                slot.trailing_pending = false;
            }
        }
    }
}

/// The process-wide gate the Strava webhook route fetches through.
pub static STRAVA_OWNER_FETCH_GATE: LazyLock<OwnerFetchGate> =
    LazyLock::new(|| OwnerFetchGate::new(OWNER_FETCH_WINDOW));
