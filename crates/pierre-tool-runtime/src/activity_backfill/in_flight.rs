// ABOUTME: The process-local hold on a job's in-flight key: one run per (user, provider, kind) here, watched by asks that join it
// ABOUTME: A live read that landed over a narrower window is succeeded by the ask it does not cover, and only a claim's holder releases it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! In-flight job keys.
//!
//! The job row is the cross-instance truth; the in-flight key is this
//! process's: one run per `(user, provider, kind)`, which an ask that finds
//! it held watches instead of running twice ([`super::live_read::JoinedRead`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};

use pierre_database::repositories::BackfillJobKind;
use pierre_providers::core::ActivityQueryParams;
use tokio::sync::watch;
use uuid::Uuid;

/// In-flight job keys (`user_id:provider:kind`) so repeated asks while a job is
/// already paginating a user's history don't stack duplicate scrapes. Each
/// kind holds its own key, as it holds its own row: a chat's live read never
/// waits behind a history walk of the same provider. The key is held for the
/// whole run, a landed live read's follow-up included, and tells whoever
/// watches it how far the run has got — unless an ask the landed read does
/// not cover succeeds it ([`claim`]).
static IN_FLIGHT_BACKFILLS: LazyLock<Mutex<HashMap<String, InFlightRun>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The source of [`InFlightClaim`] ids.
static NEXT_IN_FLIGHT_CLAIM: AtomicU64 = AtomicU64::new(1);

/// The run holding an in-flight key: which claim it holds it by, and how far
/// it has got.
struct InFlightRun {
    claim: u64,
    state: watch::Sender<InFlight>,
    /// The landed live read this run succeeded, until that run releases its
    /// key: its own follow-up's re-ask, and every ask its landed window
    /// covers, watches it and is answered from the rows it landed, never
    /// joining this run and rebinding its row ([`claim`]).
    succeeded: Option<Succeeded>,
}

/// A landed run its successor holds the key over: its claim and its state.
struct Succeeded {
    claim: u64,
    state: watch::Sender<InFlight>,
}

/// How far a run holding its in-flight key has got. A watcher keeps the last
/// state after the run released its key.
#[derive(Debug, Clone)]
pub(crate) enum InFlight {
    /// The run is reading.
    Running,
    /// A live read landed: it read `params` and its provider answered
    /// `count` activities, written through — under the grant that answered
    /// in the session's place, when `served_by_grant` names one. Its row is
    /// already finished: what is left of the run is its follow-up, which
    /// reads that window again.
    Landed {
        params: ActivityQueryParams,
        count: usize,
        served_by_grant: Option<String>,
    },
    /// A live read spent its last attempt and its row is gone: what is left
    /// of the run is the follow-up it owed the ask it was bound to. That
    /// follow-up's re-ask, and every ask meanwhile, is answered from what is
    /// stored, the provider unavailable: nothing reads it again, or a
    /// provider that keeps failing would be re-asked in a loop.
    GaveUp,
}

/// A run's hold on its in-flight key. Only the claim that holds the key
/// releases it or marks it, never a run an ask succeeded ([`claim`]).
pub(super) struct InFlightClaim {
    key: String,
    id: u64,
}

/// Taking an in-flight key.
pub(super) enum Claim {
    /// This caller holds the key now.
    Claimed(InFlightClaim),
    /// A run here holds it; how far it has got, from now on.
    Held(watch::Receiver<InFlight>),
}

pub(super) fn in_flight_key(user_id: Uuid, provider: &str, kind: BackfillJobKind) -> String {
    format!("{user_id}:{provider}:{kind}")
}

/// Take `key`, or watch the run in this process that holds it: one lock, so
/// a watcher never misses the run it lost the key to. A held key is taken
/// over when `succeeds` says the run holding it reads nothing more for this
/// ask (a live read that landed over a narrower window): its watchers keep
/// its last state, and its release leaves the successor's claim alone.
///
/// A landed run that was succeeded and has not released its key yet is
/// watched first, by an ask it does not succeed: `succeeds` is false for a
/// landed run exactly when its window covers the ask, whose rows are then
/// already landed. That is how a landed read's own follow-up re-asks
/// without joining the wider read that succeeded it.
pub(super) fn claim(key: &str, succeeds: impl Fn(&InFlight) -> bool) -> Claim {
    let mut runs = IN_FLIGHT_BACKFILLS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(run) = runs.get(key) {
        if let Some(landed) = &run.succeeded {
            if !succeeds(&landed.state.borrow()) {
                return Claim::Held(landed.state.subscribe());
            }
        }
        if !succeeds(&run.state.borrow()) {
            return Claim::Held(run.state.subscribe());
        }
    }
    // Only a landed run is ever succeeded.
    let succeeded = runs.remove(key).map(|run| Succeeded {
        claim: run.claim,
        state: run.state,
    });
    let id = NEXT_IN_FLIGHT_CLAIM.fetch_add(1, Ordering::Relaxed);
    runs.insert(
        key.to_owned(),
        InFlightRun {
            claim: id,
            state: watch::Sender::new(InFlight::Running),
            succeeded,
        },
    );
    Claim::Claimed(InFlightClaim {
        key: key.to_owned(),
        id,
    })
}

/// Take the process-local in-flight claim for `key`; `None` when a run in
/// this process already holds it.
pub(super) fn claim_in_flight(key: &str) -> Option<InFlightClaim> {
    match claim(key, |_| false) {
        Claim::Claimed(claim) => Some(claim),
        Claim::Held(_) => None,
    }
}

/// Give the key `claim` holds back. Its watchers see the channel close,
/// after the last state the run reached. A key a successor holds now is
/// left to it, and no longer answers from this run's landed rows.
pub(super) fn release_in_flight(claim: &InFlightClaim) {
    let mut runs = IN_FLIGHT_BACKFILLS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(run) = runs.get_mut(&claim.key) else {
        return;
    };
    if run.claim == claim.id {
        runs.remove(&claim.key);
    } else if run
        .succeeded
        .as_ref()
        .is_some_and(|landed| landed.claim == claim.id)
    {
        run.succeeded = None;
    }
}

/// Mark how far the run holding `claim` got: [`InFlight::Landed`] or
/// [`InFlight::GaveUp`]. A key the claim no longer holds is left alone: only
/// its holder settles it.
pub(super) fn mark_in_flight(claim: &InFlightClaim, state: InFlight) {
    if let Some(run) = IN_FLIGHT_BACKFILLS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&claim.key)
        .filter(|run| run.claim == claim.id)
    {
        run.state.send_replace(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn landed() -> InFlight {
        InFlight::Landed {
            params: ActivityQueryParams::default(),
            count: 0,
            served_by_grant: None,
        }
    }

    #[test]
    fn a_held_key_is_watched_until_its_run_lands_and_is_succeeded() {
        let key = format!("{}:sciotte:live_read", Uuid::new_v4());
        let Claim::Claimed(first) = claim(&key, |_| true) else {
            panic!("a free key is claimed");
        };
        let Claim::Held(watcher) = claim(&key, |_| false) else {
            panic!("bug: a held key is watched, not taken");
        };
        mark_in_flight(&first, landed());
        let Claim::Claimed(successor) = claim(&key, |run| matches!(run, InFlight::Landed { .. }))
        else {
            panic!("bug: a landed run an ask succeeds gives the key up");
        };
        assert!(
            matches!(*watcher.borrow(), InFlight::Landed { .. }),
            "a watcher of the run succeeded keeps its last state"
        );

        release_in_flight(&first);
        mark_in_flight(&first, landed());
        let Claim::Held(after) = claim(&key, |_| false) else {
            panic!("bug: the run succeeded released its successor's key");
        };
        assert!(
            matches!(*after.borrow(), InFlight::Running),
            "bug: the run succeeded marked its successor's key"
        );

        release_in_flight(&successor);
        assert!(
            claim_in_flight(&key).is_some(),
            "the successor's release frees the key"
        );
    }

    /// An ask the landed run's window covers — that run's own follow-up
    /// re-asking — watches the landed run, not the wider run that succeeded
    /// it, until the landed run releases its key.
    #[test]
    fn an_ask_a_succeeded_run_covers_watches_it_until_it_releases() {
        let key = format!("{}:sciotte:live_read", Uuid::new_v4());
        let Claim::Claimed(first) = claim(&key, |_| true) else {
            panic!("a free key is claimed");
        };
        mark_in_flight(&first, landed());
        let Claim::Claimed(successor) = claim(&key, |run| matches!(run, InFlight::Landed { .. }))
        else {
            panic!("a landed run a wider ask succeeds gives the key up");
        };
        let Claim::Held(covered) = claim(&key, |_| false) else {
            panic!("bug: a covered ask took the key from the successor");
        };
        assert!(
            matches!(*covered.borrow(), InFlight::Landed { .. }),
            "bug: an ask the landed run covers joins the run that succeeded it"
        );

        release_in_flight(&first);
        let Claim::Held(after) = claim(&key, |_| false) else {
            panic!("bug: the landed run's release freed its successor's key");
        };
        assert!(
            matches!(*after.borrow(), InFlight::Running),
            "once the landed run released its key, an ask joins the successor"
        );
        release_in_flight(&successor);
    }
}
