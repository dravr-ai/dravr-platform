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
//!
//! A turn that ends before its subprocess does — the athlete stopped it — can
//! still be written to: a tool call already running on the tool host finishes
//! after the turn took its entry, and nothing would take that write again.
//! Such a turn seals its key as it takes ([`TurnStore::seal`]): a write that
//! arrives later is handed to the sink the turn left instead of being held,
//! so it is neither dropped nor left behind (carnet#1519).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
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

/// Where a sealed turn's late writes go: called once per write, with the
/// value the write's merge makes of nothing held.
pub(crate) type LateSink<V> = Arc<dyn Fn(V) + Send + Sync>;

/// What one store holds, under one lock so a write and a seal never cross.
struct Held<V> {
    /// Each turn's value, with the time its stamp keeps.
    entries: HashMap<TurnKey, (Instant, V)>,
    /// The turns that ended and sealed their key, with when they did and the
    /// sink their late writes go to.
    sealed: HashMap<TurnKey, (Instant, LateSink<V>)>,
}

/// One signal's values, one per turn, taken once.
///
/// A poisoned lock is skipped on both sides: the turn then loses the signal,
/// never panics, and each caller documents what losing it costs.
pub(crate) struct TurnStore<V> {
    held: Mutex<Held<V>>,
}

impl<V> fmt::Debug for TurnStore<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `try_lock`, as `Mutex`'s own `Debug` does: formatting the store
        // while this thread holds the lock must not deadlock.
        let (entries, sealed) = self
            .held
            .try_lock()
            .map_or((0, 0), |held| (held.entries.len(), held.sealed.len()));
        f.debug_struct("TurnStore")
            .field("entries", &entries)
            .field("sealed", &sealed)
            .finish()
    }
}

/// Keep `map` under [`TURN_STORE_CAP`] before a new key goes in: the entries
/// past [`TURN_TTL`] go first, every entry only if that was not enough.
fn make_room<T>(map: &mut HashMap<TurnKey, (Instant, T)>, now: Instant) {
    if map.len() >= TURN_STORE_CAP {
        map.retain(|_, (at, _)| now.duration_since(*at) < TURN_TTL);
        if map.len() >= TURN_STORE_CAP {
            map.clear();
        }
    }
}

impl<V> TurnStore<V> {
    /// Create an empty store.
    pub(crate) fn new() -> Self {
        Self {
            held: Mutex::new(Held {
                entries: HashMap::new(),
                sealed: HashMap::new(),
            }),
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
    ///
    /// A write to a sealed turn goes to its sink, called outside the lock,
    /// while the seal is younger than [`TURN_TTL`]: no tool call outlives a
    /// turn by that long, so an older seal is dropped and the write is held.
    fn record_at(
        &self,
        key: &TurnKey,
        stamp: Stamp,
        merge: impl FnOnce(Option<V>) -> V,
        now: Instant,
    ) {
        let Ok(mut held) = self.held.lock() else {
            return;
        };
        if let Some((sealed_at, sink)) = held.sealed.get(key) {
            if now.duration_since(*sealed_at) < TURN_TTL {
                let sink = Arc::clone(sink);
                drop(held);
                sink(merge(None));
                return;
            }
            held.sealed.remove(key);
        }
        let entries = &mut held.entries;
        let previous = entries.remove(key);
        if previous.is_none() {
            make_room(entries, now);
        }
        let (at, previous) = match previous {
            Some((first, value)) if stamp == Stamp::First => (first, Some(value)),
            Some((_, value)) => (now, Some(value)),
            None => (now, None),
        };
        entries.insert(key.clone(), (at, merge(previous)));
    }

    /// Consume and return what `key`'s turn holds, if anything.
    pub(crate) fn take(&self, key: &TurnKey) -> Option<V> {
        self.held
            .lock()
            .ok()?
            .entries
            .remove(key)
            .map(|(_, value)| value)
    }

    /// [`Self::take`] for a turn that ended while its tool calls may still
    /// run: every later write to `key` goes to `late` instead of the store.
    /// Sealing again replaces the sink.
    pub(crate) fn seal(&self, key: &TurnKey, late: LateSink<V>) -> Option<V> {
        self.seal_at(key, late, Instant::now())
    }

    /// [`Self::seal`] as of `now`. The seals are bounded as the entries are.
    fn seal_at(&self, key: &TurnKey, late: LateSink<V>, now: Instant) -> Option<V> {
        let mut held = self.held.lock().ok()?;
        if !held.sealed.contains_key(key) {
            make_room(&mut held.sealed, now);
        }
        held.sealed.insert(key.clone(), (now, late));
        held.entries.remove(key).map(|(_, value)| value)
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
        store.held.lock().expect("lock").entries.len()
    }

    /// Fill `store` to its cap with entries stamped `at`.
    fn fill_to_cap(store: &TurnStore<&'static str>, at: Instant) {
        let mut held = store.held.lock().expect("lock");
        let entries = &mut held.entries;
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
        assert_eq!(
            store.take(&turn("turn-a")),
            Some("a"),
            "bug: the recording turn must take its value"
        );
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
        assert_eq!(
            store.take(&key),
            Some(vec![1, 2]),
            "bug: the turn holds what the merge returned"
        );
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
        assert_eq!(
            store.take(&held),
            Some("again"),
            "bug: the turn writing again keeps its latest value"
        );
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
        assert_eq!(
            store.take(&next),
            Some("next"),
            "bug: the turn that overflowed the store keeps its own"
        );
    }

    /// What a sealed turn's late writes received.
    fn collecting() -> (Arc<Mutex<Vec<&'static str>>>, LateSink<&'static str>) {
        let late = Arc::new(Mutex::new(Vec::new()));
        let sink_late = Arc::clone(&late);
        let sink: LateSink<&'static str> =
            Arc::new(move |value| sink_late.lock().expect("lock").push(value));
        (late, sink)
    }

    /// A turn that ended takes what it holds as it seals; a write that lands
    /// after goes to its sink, never back into the store, and other turns
    /// write as before.
    #[test]
    fn a_sealed_turn_hands_its_late_writes_to_its_sink() {
        let store = TurnStore::new();
        let ended = turn("turn-ended");
        let other = turn("turn-other");
        store.record(&ended, Stamp::Latest, latest("before"));
        let (late, sink) = collecting();

        assert_eq!(
            store.seal(&ended, sink),
            Some("before"),
            "bug: sealing takes what the turn held"
        );
        store.record(&ended, Stamp::Latest, |held| {
            assert!(
                held.is_none(),
                "bug: a sealed turn's merge starts from nothing"
            );
            "after"
        });
        store.record(&other, Stamp::Latest, latest("other"));

        assert_eq!(
            *late.lock().expect("lock"),
            vec!["after"],
            "bug: a write after the seal reaches the sink"
        );
        assert_eq!(
            store.take(&ended),
            None,
            "bug: a sealed turn's late write must not wait in the store for nobody"
        );
        assert_eq!(
            store.take(&other),
            Some("other"),
            "bug: a seal redirects only its own turn's writes"
        );
    }

    /// A seal older than any turn's tool call stops redirecting, and the seals
    /// stay under the cap like the entries do.
    #[test]
    fn seals_age_out_and_stay_bounded() {
        let store = TurnStore::new();
        let start = Instant::now();
        let later = start + TURN_TTL + Duration::from_secs(1);
        let old = turn("turn-old");
        let (late, sink) = collecting();
        store.seal_at(&old, sink, start);
        store.record_at(&old, Stamp::Latest, latest("much later"), later);
        assert!(
            late.lock().expect("lock").is_empty(),
            "bug: a seal past the turn TTL redirects nothing"
        );
        assert_eq!(
            store.take(&old),
            Some("much later"),
            "bug: a write past an aged-out seal is held for its turn like any other"
        );

        for n in 0..=TURN_STORE_CAP {
            let (_, sink) = collecting();
            store.seal(&turn(&format!("sealed-{n}")), sink);
        }
        assert!(
            store.held.lock().expect("lock").sealed.len() <= TURN_STORE_CAP,
            "bug: the seals must stay under the cap"
        );
    }
}
