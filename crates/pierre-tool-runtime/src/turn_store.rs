// ABOUTME: The one per-turn signal store: a value per turn, folded by the caller's write rule, taken once
// ABOUTME: Bounded by the shared turn TTL and one cap, so a turn that dies mid-flight cannot grow it forever

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The store a per-turn signal leaves the ACP subprocess's loopback through.
//!
//! The Copilot-headless loop's tools run on the tool-host surface's own
//! executor, whose return value goes to the subprocess, never to the loop. A
//! signal a dispatch raises there (a reconnect offer, an incomplete window,
//! an envelope notice, a Guardian block) reaches the turn's loop only through
//! a process-wide store keyed by the turn: the dispatch records into it and
//! the loop takes the turn's entry once when it returns. [`TurnStore`] is that
//! store, one typed instance per signal, each caller keeping its own write
//! rule and return type.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::guardian::state::TURN_TTL;
use crate::guardian::TurnKey;

/// Hard cap on retained untaken entries, per store. A turn takes its entry
/// when its loop returns, so an entry outlives its turn only when the loop
/// failed or a stage dispatched after it: the cap is unreachable in practice.
const TURN_STORE_CAP: usize = 10_000;

/// Which write's time an entry keeps when its turn writes to it again. The
/// entry is swept on overflow once that time is [`TURN_TTL`] old.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stamp {
    /// The latest write's: the turn is alive as long as it keeps writing.
    Latest,
    /// The first write's: a later write changes neither the value's age nor,
    /// when the merge keeps the held value, the value.
    First,
}

/// One signal's values, one per turn, taken once.
///
/// A poisoned lock is skipped on both sides: the turn then loses the signal,
/// never panics, and each caller documents what losing it costs.
#[derive(Debug)]
pub(crate) struct TurnStore<V> {
    entries: Mutex<HashMap<TurnKey, (Instant, V)>>,
}

impl<V> TurnStore<V> {
    /// Create an empty store.
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Fold a write into what `key`'s turn holds: `merge` receives the held
    /// value, `None` for a turn that holds nothing, and returns the value the
    /// turn holds after the write. `stamp` says which write's time it keeps.
    pub(crate) fn record(&self, key: &TurnKey, stamp: Stamp, merge: impl FnOnce(Option<V>) -> V) {
        self.record_at(key, stamp, merge, Instant::now());
    }

    /// [`Self::record`] as of `now`.
    ///
    /// Bounded only when the turn is new to the store, so a live turn writing
    /// again never sweeps it: past [`TURN_STORE_CAP`] the entries past
    /// [`TURN_TTL`] go first, every entry only if that was not enough.
    fn record_at(
        &self,
        key: &TurnKey,
        stamp: Stamp,
        merge: impl FnOnce(Option<V>) -> V,
        now: Instant,
    ) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let held = entries.remove(key);
        if held.is_none() && entries.len() >= TURN_STORE_CAP {
            entries.retain(|_, (at, _)| now.duration_since(*at) < TURN_TTL);
            if entries.len() >= TURN_STORE_CAP {
                entries.clear();
            }
        }
        let (at, held) = match held {
            Some((first, value)) if stamp == Stamp::First => (first, Some(value)),
            Some((_, value)) => (now, Some(value)),
            None => (now, None),
        };
        entries.insert(key.clone(), (at, merge(held)));
    }

    /// Consume and return what `key`'s turn holds, if anything.
    pub(crate) fn take(&self, key: &TurnKey) -> Option<V> {
        self.entries
            .lock()
            .ok()?
            .remove(key)
            .map(|(_, value)| value)
    }
}

impl<V> Default for TurnStore<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn turn(name: &str) -> TurnKey {
        TurnKey::new(None, name)
    }

    fn latest(value: &'static str) -> impl FnOnce(Option<&'static str>) -> &'static str {
        move |_| value
    }

    fn held_len<V>(store: &TurnStore<V>) -> usize {
        store.entries.lock().expect("lock").len()
    }

    /// Fill `store` to its cap with entries stamped `at`.
    fn fill_to_cap(store: &TurnStore<&'static str>, at: Instant) {
        let mut entries = store.entries.lock().expect("lock");
        let mut n = 0;
        while entries.len() < TURN_STORE_CAP {
            entries.insert(turn(&format!("filler-{n}")), (at, "filler"));
            n += 1;
        }
    }

    #[test]
    fn a_turn_takes_its_own_value_once() {
        let store = TurnStore::new();
        store.record(&turn("turn-a"), Stamp::Latest, latest("a"));
        assert_eq!(
            store.take(&turn("turn-b")),
            None,
            "bug: another turn must not take this turn's value"
        );
        assert_eq!(store.take(&turn("turn-a")), Some("a"));
        assert_eq!(store.take(&turn("turn-a")), None, "bug: taken once");
    }

    #[test]
    fn the_merge_sees_what_the_turn_holds() {
        let store = TurnStore::new();
        let key = turn("turn");
        store.record(&key, Stamp::Latest, |held: Option<Vec<u8>>| {
            assert!(held.is_none(), "bug: a new turn holds nothing");
            vec![1]
        });
        store.record(&key, Stamp::Latest, |held| {
            let mut held = held.expect("bug: the turn holds its first write");
            held.push(2);
            held
        });
        assert_eq!(store.take(&key), Some(vec![1, 2]));
    }

    /// Ages the entries by reading them from a later `now`, because an
    /// `Instant` cannot reach back past a freshly booted machine's clock start.
    #[test]
    fn overflow_drops_abandoned_entries_before_a_live_one() {
        let store = TurnStore::new();
        let later = Instant::now() + TURN_TTL + Duration::from_secs(1);
        let live = turn("turn-live");
        store.record_at(&live, Stamp::Latest, latest("live"), later);
        fill_to_cap(&store, Instant::now());

        let next = turn("turn-next");
        store.record_at(&next, Stamp::Latest, latest("next"), later);

        assert_eq!(held_len(&store), 2, "bug: only the abandoned entries went");
        assert_eq!(
            store.take(&live),
            Some("live"),
            "bug: a live turn keeps its value through an overflow"
        );
        assert_eq!(
            store.take(&next),
            Some("next"),
            "bug: the turn that overflowed the store keeps its own"
        );
    }

    #[test]
    fn entries_stay_bounded_when_none_has_been_abandoned() {
        let store = TurnStore::new();
        for n in 0..TURN_STORE_CAP {
            store.record(&turn(&format!("turn-{n}")), Stamp::Latest, latest("held"));
        }
        let past_the_cap = turn("turn-past-the-cap");
        store.record(&past_the_cap, Stamp::Latest, latest("newest"));
        assert!(
            held_len(&store) <= TURN_STORE_CAP,
            "bug: the store must stay under its cap"
        );
        assert_eq!(
            store.take(&past_the_cap),
            Some("newest"),
            "bug: the newest value survives the overflow"
        );
    }

    #[test]
    fn a_turn_already_held_at_the_cap_sweeps_nothing() {
        let store = TurnStore::new();
        let later = Instant::now() + TURN_TTL + Duration::from_secs(1);
        let held = turn("filler-0");
        fill_to_cap(&store, Instant::now());

        store.record_at(&held, Stamp::Latest, latest("again"), later);

        assert_eq!(
            held_len(&store),
            TURN_STORE_CAP,
            "bug: a turn writing again must not sweep the store"
        );
        assert_eq!(store.take(&held), Some("again"));
    }

    /// The stamp is what decides a rewritten turn's fate in the sweep: the
    /// latest write's keeps it, the first write's lets it age out.
    #[test]
    fn the_stamp_decides_which_write_a_turn_ages_from() {
        let store = TurnStore::new();
        let start = Instant::now();
        let later = start + TURN_TTL + Duration::from_secs(1);
        let refreshed = turn("turn-refreshed");
        let kept = turn("turn-kept");
        store.record_at(&refreshed, Stamp::Latest, latest("first"), start);
        store.record_at(&kept, Stamp::First, latest("first"), start);
        store.record_at(&refreshed, Stamp::Latest, latest("second"), later);
        store.record_at(&kept, Stamp::First, |held| held.unwrap_or("second"), later);
        fill_to_cap(&store, start);

        let next = turn("turn-next");
        store.record_at(&next, Stamp::Latest, latest("next"), later);

        assert_eq!(
            store.take(&refreshed),
            Some("second"),
            "bug: a turn stamped by its latest write is alive since that write"
        );
        assert_eq!(
            store.take(&kept),
            None,
            "bug: a turn stamped by its first write ages from it, whatever came after"
        );
        assert_eq!(store.take(&next), Some("next"));
    }
}
